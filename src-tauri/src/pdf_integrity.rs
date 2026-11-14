//! Structural validation for locally generated PDF exports, before publication.
use crate::app_error::AppError;
use lopdf::{Document, Object, ObjectId};
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::{Read, Seek, SeekFrom},
};

fn invalid() -> AppError {
    AppError::ArtifactGenerationFailed("Generated PDF is incomplete or structurally invalid".into())
}

pub(super) fn validate(source: &mut File) -> Result<(), AppError> {
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        let length = source.metadata()?.len();
        source.seek(SeekFrom::Start(length.saturating_sub(512)))?;
        let mut tail = Vec::new();
        source.read_to_end(&mut tail)?;
        if !tail.trim_ascii_end().ends_with(b"%%EOF") {
            return Err("missing PDF end marker".into());
        }
        source.seek(SeekFrom::Start(0))?;
        let doc = Document::load_from(&mut *source)?;
        if doc.is_encrypted() {
            return Err("encrypted export".into());
        }
        let catalog = doc.trailer.get(b"Root")?.as_reference()?;
        let catalog = doc
            .objects
            .get(&catalog)
            .ok_or("missing catalog")?
            .as_dict()?;
        if catalog.get(b"Type")?.as_name()? != b"Catalog" {
            return Err("invalid catalog".into());
        }
        let pages = catalog.get(b"Pages")?.as_reference()?;
        // Traverse iteratively and reject cycles/shared nodes before walking
        // content. A malicious tree cannot recurse forever or overflow a stack.
        let mut stack = vec![(pages, false)];
        let mut seen = HashSet::new();
        let mut counts = HashMap::<ObjectId, i64>::new();
        while let Some((id, exit)) = stack.pop() {
            let dict = doc.objects.get(&id).ok_or("missing page node")?.as_dict()?;
            let kind = dict.get(b"Type")?.as_name()?;
            if exit {
                let mut count = 0_i64;
                for child in dict.get(b"Kids")?.as_array()? {
                    count = count
                        .checked_add(
                            *counts
                                .get(&child.as_reference()?)
                                .ok_or("missing child count")?,
                        )
                        .ok_or("page count overflow")?;
                }
                if dict.get(b"Count")?.as_i64()? != count {
                    return Err("incorrect page count".into());
                }
                counts.insert(id, count);
                continue;
            }
            if !seen.insert(id) {
                return Err("cyclic or shared page tree".into());
            }
            match kind {
                b"Pages" => {
                    stack.push((id, true));
                    for child in dict.get(b"Kids")?.as_array()?.iter().rev() {
                        stack.push((child.as_reference()?, false));
                    }
                }
                b"Page" => {
                    if let Ok(contents) = dict.get(b"Contents") {
                        let refs = match contents {
                            Object::Reference(id) => vec![*id],
                            Object::Array(items) => items
                                .iter()
                                .map(Object::as_reference)
                                .collect::<Result<Vec<_>, _>>()?,
                            _ => return Err("invalid page content reference".into()),
                        };
                        let mut bytes = Vec::new();
                        for reference in refs {
                            let stream = doc
                                .objects
                                .get(&reference)
                                .ok_or("missing page content")?
                                .as_stream()?;
                            if stream.dict.has(b"Filter") {
                                bytes.extend(stream.decompressed_content()?);
                            } else {
                                bytes.extend_from_slice(&stream.content);
                            }
                            bytes.push(b'\n');
                        }
                        lopdf::content::Content::decode(&bytes)?;
                    }
                    counts.insert(id, 1);
                }
                _ => return Err("invalid page tree node".into()),
            }
        }
        if counts.get(&pages).copied().unwrap_or(0) == 0 {
            return Err("empty PDF export".into());
        }
        Ok(())
    })();
    source.seek(SeekFrom::Start(0))?;
    result.map_err(|_| invalid())
}

#[cfg(test)]
fn fixture_document() -> (Document, ObjectId, ObjectId) {
    use lopdf::{dictionary, Stream};
    let mut doc = Document::with_version("1.7");
    let pages = doc.new_object_id();
    let stream = doc.add_object(Stream::new(dictionary! {}, b"q Q".to_vec()));
    let page=doc.add_object(dictionary!{"Type"=>"Page","Parent"=>pages,"MediaBox"=>vec![0.into(),0.into(),100.into(),100.into()],"Contents"=>stream});
    doc.objects.insert(
        pages,
        dictionary! {"Type"=>"Pages","Kids"=>vec![page.into()],"Count"=>1}.into(),
    );
    let root = doc.add_object(dictionary! {"Type"=>"Catalog","Pages"=>pages});
    doc.trailer.set("Root", root);
    (doc, pages, stream)
}
#[cfg(test)]
pub(super) fn fixture_pdf() -> Vec<u8> {
    let (mut doc, _, _) = fixture_document();
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).unwrap();
    bytes
}
#[cfg(test)]
mod tests {
    use super::*;
    fn check(bytes: &[u8]) -> Result<(), AppError> {
        let mut file = tempfile::tempfile().unwrap();
        use std::io::Write;
        file.write_all(bytes).unwrap();
        validate(&mut file)
    }
    fn serialized(mut doc: Document) -> Vec<u8> {
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).unwrap();
        bytes
    }
    #[test]
    fn valid_pdf_is_parsed_and_source_rewound() {
        let mut file = tempfile::tempfile().unwrap();
        use std::io::Write;
        file.write_all(&fixture_pdf()).unwrap();
        validate(&mut file).unwrap();
        assert_eq!(file.stream_position().unwrap(), 0);
    }
    #[test]
    fn truncated_and_signature_only_documents_are_rejected() {
        assert!(check(b"%PDF-fake\n%%EOF").is_err());
        let mut bytes = fixture_pdf();
        bytes.truncate(bytes.len() - 8);
        assert!(check(&bytes).is_err());
    }
    #[test]
    fn empty_and_incorrect_page_counts_are_rejected() {
        let (mut doc, pages, _) = fixture_document();
        doc.objects
            .get_mut(&pages)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set("Count", 2);
        assert!(check(&serialized(doc)).is_err());
        let (mut doc, pages, _) = fixture_document();
        let dict = doc.objects.get_mut(&pages).unwrap().as_dict_mut().unwrap();
        dict.set("Kids", Vec::<Object>::new());
        dict.set("Count", 0);
        assert!(check(&serialized(doc)).is_err());
    }
    #[test]
    fn cyclic_page_tree_is_rejected_without_recursive_traversal() {
        let (mut doc, pages, _) = fixture_document();
        doc.objects
            .get_mut(&pages)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set("Kids", vec![Object::Reference(pages)]);
        assert!(check(&serialized(doc)).is_err());
    }
    #[test]
    fn missing_and_malformed_content_streams_are_rejected() {
        let (mut doc, _, stream) = fixture_document();
        doc.objects.remove(&stream);
        assert!(check(&serialized(doc)).is_err());
        let (mut doc, _, stream) = fixture_document();
        doc.objects
            .get_mut(&stream)
            .unwrap()
            .as_stream_mut()
            .unwrap()
            .content = b"(unterminated".to_vec();
        assert!(check(&serialized(doc)).is_err());
    }
}
