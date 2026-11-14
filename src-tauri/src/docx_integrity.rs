//! Integrity checks for OpenMindAI's seekable, UTF-8 DOCX export packages.
use crate::app_error::AppError;
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Component, Path},
};
const BUDGET: u64 = 128 * 1024 * 1024;
const TYPES: &str = "http://schemas.openxmlformats.org/package/2006/content-types";
const RELS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const OFFICE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument";
const WORD: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const MAIN_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml";
const REQUIRED: [&str; 3] = ["[Content_Types].xml", "_rels/.rels", "word/document.xml"];
fn xml(text: &str) -> Result<roxmltree::Document<'_>, roxmltree::Error> {
    roxmltree::Document::parse_with_options(
        text,
        roxmltree::ParsingOptions {
            allow_dtd: false,
            nodes_limit: 1_000_000,
            ..Default::default()
        },
    )
}

pub(super) fn validate(source: &mut File) -> Result<(), AppError> {
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        if source.metadata()?.len() > BUDGET {
            return Err("DOCX archive exceeds validation budget".into());
        }
        source.seek(SeekFrom::Start(0))?;
        let mut archive = zip::ZipArchive::new(&mut *source)?;
        if archive.offset() != 0 {
            return Err("unexpected ZIP prefix".into());
        }
        let mut metadata = HashMap::new();
        let mut parts = HashMap::new();
        let mut remaining = BUDGET;
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index)?;
            let name = entry.name()?.into_owned();
            if name.contains(['\\', ':'])
                || Path::new(&name)
                    .components()
                    .any(|p| !matches!(p, Component::Normal(_)))
                || name.is_empty()
                || entry.encrypted()
                || entry
                    .unix_mode()
                    .is_some_and(|mode| mode & 0o170000 == 0o120000)
            {
                return Err("unsafe DOCX ZIP entry".into());
            }
            let size = entry.size();
            let crc = entry.crc32();
            if size > remaining || metadata.insert(name.clone(), (size, crc)).is_some() {
                return Err("DOCX archive budget or duplicate entry".into());
            }
            // Reading to EOF validates CRC, including parts not used by the renderer.
            let count = if name.ends_with(".xml") || name.ends_with(".rels") {
                let mut bytes = Vec::new();
                (&mut entry).take(remaining + 1).read_to_end(&mut bytes)?;
                let count = bytes.len() as u64;
                if count > remaining {
                    return Err("DOCX expansion exceeds budget".into());
                }
                let text = String::from_utf8(bytes)?;
                xml(&text)?;
                if REQUIRED.contains(&name.as_str()) {
                    parts.insert(name, text);
                }
                count
            } else {
                std::io::copy(&mut (&mut entry).take(remaining + 1), &mut std::io::sink())?
            };
            if count != size || count > remaining {
                return Err("DOCX entry size mismatch".into());
            }
            remaining -= count;
        }
        drop(archive);
        // zip's central-directory index collapses duplicate names. Independently
        // walk local headers to reject duplicates and local/central disagreement.
        source.seek(SeekFrom::Start(0))?;
        let mut local = HashSet::new();
        let mut remaining = BUDGET;
        while let Some(mut entry) = zip::read::read_zipfile_from_stream(&mut *source)? {
            let name = entry.name()?.into_owned();
            if !local.insert(name.clone())
                || metadata.get(&name) != Some(&(entry.size(), entry.crc32()))
            {
                return Err("inconsistent DOCX local headers".into());
            }
            if entry.size() > remaining {
                return Err("DOCX expansion exceeds budget".into());
            }
            let count = std::io::copy(&mut (&mut entry).take(remaining + 1), &mut std::io::sink())?;
            if count != entry.size() || count > remaining {
                return Err("DOCX entry size mismatch".into());
            }
            remaining -= count;
        }
        if local.len() != metadata.len() {
            return Err("missing DOCX local entry".into());
        }
        let required = |name: &str| parts.get(name).ok_or("missing required DOCX part");
        let types = xml(required(REQUIRED[0])?)?;
        let root = types.root_element();
        if !root.has_tag_name((TYPES, "Types")) {
            return Err("invalid content types".into());
        }
        let main: Vec<_> = root
            .children()
            .filter(|n| {
                n.has_tag_name((TYPES, "Override"))
                    && n.attribute("PartName") == Some("/word/document.xml")
            })
            .collect();
        if main.len() != 1 || main[0].attribute("ContentType") != Some(MAIN_TYPE) {
            return Err("invalid main document content type".into());
        }
        let rels = xml(required(REQUIRED[1])?)?;
        let root = rels.root_element();
        if !root.has_tag_name((RELS, "Relationships")) {
            return Err("invalid package relationships".into());
        }
        let main: Vec<_> = root
            .children()
            .filter(|n| {
                n.has_tag_name((RELS, "Relationship")) && n.attribute("Type") == Some(OFFICE)
            })
            .collect();
        if main.len() != 1
            || !matches!(
                main[0].attribute("Target"),
                Some("word/document.xml" | "/word/document.xml")
            )
            || !matches!(main[0].attribute("TargetMode"), None | Some("Internal"))
        {
            return Err("invalid main document relationship".into());
        }
        let document = xml(required(REQUIRED[2])?)?;
        let root = document.root_element();
        if !root.has_tag_name((WORD, "document"))
            || root
                .children()
                .filter(|n| n.has_tag_name((WORD, "body")))
                .count()
                != 1
        {
            return Err("invalid Word document body".into());
        }
        Ok(())
    })();
    source.seek(SeekFrom::Start(0))?;
    result.map_err(|_| {
        AppError::ArtifactGenerationFailed(
            "Generated DOCX is corrupt, unsupported or exceeds validation limits (128 MiB / 1,000,000 XML nodes)"
                .into(),
        )
    })
}

#[cfg(test)]
fn parts() -> Vec<(String, String)> {
    vec![
        (
            REQUIRED[0].into(),
            format!(
                r#"<Types xmlns="{TYPES}"><Override PartName="/word/document.xml" ContentType="{MAIN_TYPE}"/></Types>"#
            ),
        ),
        (
            REQUIRED[1].into(),
            format!(
                r#"<Relationships xmlns="{RELS}"><Relationship Id="rId1" Type="{OFFICE}" Target="word/document.xml"/></Relationships>"#
            ),
        ),
        (
            REQUIRED[2].into(),
            format!(
                r#"<w:document xmlns:w="{WORD}"><w:body><w:p><w:r><w:t>Hello বাংলা Русский &amp; text</w:t></w:r></w:p></w:body></w:document>"#
            ),
        ),
    ]
}
#[cfg(test)]
fn package(parts: Vec<(String, String)>, method: zip::CompressionMethod) -> Vec<u8> {
    use std::io::Write;
    let mut output = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, text) in parts {
        output
            .start_file(
                name,
                zip::write::SimpleFileOptions::default().compression_method(method),
            )
            .unwrap();
        output.write_all(text.as_bytes()).unwrap();
    }
    output.finish().unwrap().into_inner()
}
#[cfg(test)]
pub(super) fn fixture_docx() -> Vec<u8> {
    package(parts(), zip::CompressionMethod::Deflated)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn check(bytes: &[u8]) -> Result<(), AppError> {
        use std::io::Write;
        let mut file = tempfile::tempfile().unwrap();
        file.write_all(bytes).unwrap();
        validate(&mut file)
    }
    #[test]
    fn real_zip_utf8_parts_and_deflate_pass_and_rewind() {
        use std::io::Write;
        let mut f = tempfile::tempfile().unwrap();
        f.write_all(&fixture_docx()).unwrap();
        validate(&mut f).unwrap();
        assert_eq!(f.stream_position().unwrap(), 0);
    }
    #[test]
    fn fake_header_and_truncated_zip_are_rejected() {
        assert!(check(b"PK\x03\x04fake").is_err());
        let mut bytes = fixture_docx();
        bytes.truncate(bytes.len() - 15);
        assert!(check(&bytes).is_err());
    }
    #[test]
    fn required_parts_and_relationship_targets_are_checked() {
        for index in 0..3 {
            let mut p = parts();
            p.remove(index);
            assert!(check(&package(p, zip::CompressionMethod::Stored)).is_err());
        }
        for replacement in ["https://outside.invalid/file", "../document.xml"] {
            let mut p = parts();
            p[1].1 = p[1].1.replace(
                "Target=\"word/document.xml\"",
                &format!("Target=\"{replacement}\""),
            );
            assert!(check(&package(p, zip::CompressionMethod::Stored)).is_err());
        }
    }
    #[test]
    fn xml_wellformedness_namespace_body_and_dtd_are_checked() {
        for content in [
            "<broken>",
            "<document><body/></document>",
            &format!(r#"<w:document xmlns:w="{WORD}"/>"#),
            &format!(
                r#"<!DOCTYPE x [<!ENTITY a "expanded">]><w:document xmlns:w="{WORD}"><w:body>&a;</w:body></w:document>"#
            ),
        ] {
            let mut p = parts();
            p[2].1 = content.into();
            assert!(check(&package(p, zip::CompressionMethod::Stored)).is_err());
        }
    }
    #[test]
    fn correct_xml_with_wrong_crc_is_rejected() {
        let mut bytes = package(parts(), zip::CompressionMethod::Stored);
        let at = bytes.windows(5).position(|x| x == b"Hello").unwrap();
        bytes[at] = b'J';
        assert!(check(&bytes).is_err());
    }
    #[test]
    fn duplicate_local_entries_and_unsafe_paths_are_rejected() {
        let mut p = parts();
        p.push(("word/dupument.xml".into(), p[2].1.clone()));
        let mut bytes = package(p, zip::CompressionMethod::Stored);
        for at in (0..=bytes.len() - 17).collect::<Vec<_>>() {
            if &bytes[at..at + 17] == b"word/dupument.xml" {
                bytes[at..at + 17].copy_from_slice(b"word/document.xml");
            }
        }
        assert!(check(&bytes).is_err());
        let mut p = parts();
        p.push(("../outside.xml".into(), "<x/>".into()));
        assert!(check(&package(p, zip::CompressionMethod::Stored)).is_err());
    }
    #[test]
    fn archive_budget_rejects_sparse_oversized_input() {
        let mut f = tempfile::tempfile().unwrap();
        f.set_len(BUDGET + 1).unwrap();
        assert!(validate(&mut f).is_err());
    }
}
