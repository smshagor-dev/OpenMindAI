use std::{
    fs,
    path::{Component, Path, PathBuf},
};

use serde_json::{json, Value};
use tree_sitter::{Language, Node, Parser, Point};

use crate::app_error::AppError;

const MAX_SOURCE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_AST_FILES: usize = 1_200;
const MAX_AST_SCAN_DEPTH: usize = 18;
const MAX_SYMBOL_RESULTS: usize = 100;
const MAX_DOCUMENT_SYMBOL_RESULTS: usize = 200;
const MAX_REFERENCE_RESULTS: usize = 200;
const MAX_PREVIEW_CHARS: usize = 320;

#[derive(Debug, Clone)]
struct AstSymbol {
    name: String,
    kind: &'static str,
    start: Point,
    end: Point,
}

pub fn document_symbols(root: &Path, relative_path: &str) -> Result<Option<Value>, AppError> {
    let root = canonical_root(root)?;
    let file = resolve_source_file(&root, relative_path)?;
    let text = read_source(&file)?;
    let Some(symbols) = symbols_for_file(&file, &text)? else {
        return Ok(None);
    };

    let items = symbols
        .into_iter()
        .take(MAX_DOCUMENT_SYMBOL_RESULTS)
        .map(|symbol| symbol_json(&root, &file, &text, &symbol, false))
        .collect::<Vec<_>>();
    Ok(Some(Value::Array(items)))
}

pub fn workspace_symbols(root: &Path, query: &str) -> Result<Option<Value>, AppError> {
    let root = canonical_root(root)?;
    let query = query.trim().to_ascii_lowercase();
    if query.is_empty() {
        return Err(AppError::internal("symbol_search query cannot be empty"));
    }

    let mut parsed_any = false;
    let mut results = Vec::new();
    for file in collect_source_files(&root)? {
        let Ok(text) = read_source(&file) else {
            continue;
        };
        let Some(symbols) = symbols_for_file(&file, &text)? else {
            continue;
        };
        parsed_any = true;
        for symbol in symbols {
            if symbol.name.to_ascii_lowercase().contains(&query) {
                results.push(symbol_json(&root, &file, &text, &symbol, false));
                if results.len() >= MAX_SYMBOL_RESULTS {
                    return Ok(Some(Value::Array(results)));
                }
            }
        }
    }

    Ok(parsed_any.then_some(Value::Array(results)))
}

pub fn definition(
    root: &Path,
    relative_path: &str,
    line: u64,
    character: u64,
) -> Result<Option<Value>, AppError> {
    let (root, _file, symbol) = navigation_symbol(root, relative_path, line, character)?;
    let mut parsed_any = false;
    let mut results = Vec::new();

    for file in collect_source_files(&root)? {
        let Ok(text) = read_source(&file) else {
            continue;
        };
        let Some(symbols) = symbols_for_file(&file, &text)? else {
            continue;
        };
        parsed_any = true;
        for candidate in symbols {
            if candidate.name == symbol {
                results.push(symbol_json(&root, &file, &text, &candidate, false));
                if results.len() >= MAX_SYMBOL_RESULTS {
                    return Ok(Some(Value::Array(results)));
                }
            }
        }
    }

    Ok(parsed_any.then_some(Value::Array(results)))
}

pub fn hover(
    root: &Path,
    relative_path: &str,
    line: u64,
    character: u64,
) -> Result<Option<Value>, AppError> {
    let (root, _file, symbol) = navigation_symbol(root, relative_path, line, character)?;
    let mut parsed_any = false;
    let mut results = Vec::new();

    for file in collect_source_files(&root)? {
        let Ok(text) = read_source(&file) else {
            continue;
        };
        let Some(symbols) = symbols_for_file(&file, &text)? else {
            continue;
        };
        parsed_any = true;
        for candidate in symbols {
            if candidate.name == symbol {
                results.push(symbol_json(&root, &file, &text, &candidate, true));
                if results.len() >= 10 {
                    return Ok(Some(Value::Array(results)));
                }
            }
        }
    }

    Ok(parsed_any.then_some(Value::Array(results)))
}

pub fn references(
    root: &Path,
    relative_path: &str,
    line: u64,
    character: u64,
) -> Result<Option<Value>, AppError> {
    let (root, file, symbol) = navigation_symbol(root, relative_path, line, character)?;
    if language_for_file(&file).is_none() {
        return Ok(None);
    }

    let mut parsed_any = false;
    let mut results = Vec::new();
    for file in collect_source_files(&root)? {
        let Ok(text) = read_source(&file) else {
            continue;
        };
        let Some(tree) = parse_file(&file, &text)? else {
            continue;
        };
        parsed_any = true;
        collect_reference_nodes(
            tree.root_node(),
            text.as_bytes(),
            &symbol,
            &root,
            &file,
            &text,
            &mut results,
        );
        if results.len() >= MAX_REFERENCE_RESULTS {
            results.truncate(MAX_REFERENCE_RESULTS);
            return Ok(Some(Value::Array(results)));
        }
    }

    Ok(parsed_any.then_some(Value::Array(results)))
}

fn navigation_symbol(
    root: &Path,
    relative_path: &str,
    line: u64,
    character: u64,
) -> Result<(PathBuf, PathBuf, String), AppError> {
    if line == 0 {
        return Err(AppError::internal(
            "symbol navigation line is 1-based and must be >= 1",
        ));
    }
    let root = canonical_root(root)?;
    let file = resolve_source_file(&root, relative_path)?;
    if language_for_file(&file).is_none() {
        return Err(AppError::internal(
            "Tree-sitter fallback does not support this source language",
        ));
    }
    let text = read_source(&file)?;
    let line_index = usize::try_from(line)
        .map_err(|_| AppError::internal("symbol navigation line is too large"))?;
    let character_index = usize::try_from(character)
        .map_err(|_| AppError::internal("symbol navigation character is too large"))?;
    let symbol = symbol_at_position(&text, line_index, character_index)?;
    Ok((root, file, symbol))
}

fn parse_file(file: &Path, text: &str) -> Result<Option<tree_sitter::Tree>, AppError> {
    let Some(language) = language_for_file(file) else {
        return Ok(None);
    };
    let mut parser = Parser::new();
    parser.set_language(&language).map_err(|error| {
        AppError::internal(format!("failed to load Tree-sitter grammar: {error}"))
    })?;
    let tree = parser
        .parse(text, None)
        .ok_or_else(|| AppError::internal("Tree-sitter parser returned no syntax tree"))?;
    Ok(Some(tree))
}

fn symbols_for_file(file: &Path, text: &str) -> Result<Option<Vec<AstSymbol>>, AppError> {
    let Some(tree) = parse_file(file, text)? else {
        return Ok(None);
    };
    let extension = extension(file);
    let bytes = text.as_bytes();
    let mut results = Vec::new();
    let mut stack = vec![tree.root_node()];

    while let Some(node) = stack.pop() {
        if let Some(kind) = declaration_kind(&extension, node) {
            if let Some(name_node) = declaration_name_node(node) {
                if let Ok(name) = name_node.utf8_text(bytes) {
                    let name = name.trim();
                    if !name.is_empty() && name.chars().count() <= 512 {
                        results.push(AstSymbol {
                            name: name.to_string(),
                            kind,
                            start: name_node.start_position(),
                            end: name_node.end_position(),
                        });
                    }
                }
            }
        }

        let mut cursor = node.walk();
        let children = node.children(&mut cursor).collect::<Vec<_>>();
        stack.extend(children.into_iter().rev());
    }

    Ok(Some(results))
}

fn declaration_kind(extension: &str, node: Node<'_>) -> Option<&'static str> {
    let kind = node.kind();
    match extension {
        "rs" => match kind {
            "function_item" => Some("function"),
            "struct_item" => Some("struct"),
            "enum_item" => Some("enum"),
            "trait_item" => Some("trait"),
            "type_item" => Some("type"),
            "const_item" => Some("constant"),
            "static_item" => Some("static"),
            "mod_item" => Some("module"),
            "macro_definition" => Some("macro"),
            _ => None,
        },
        "js" | "jsx" | "ts" | "tsx" => match kind {
            "function_declaration" | "generator_function_declaration" => Some("function"),
            "class_declaration" => Some("class"),
            "method_definition" => Some("method"),
            "interface_declaration" => Some("interface"),
            "type_alias_declaration" => Some("type"),
            "enum_declaration" => Some("enum"),
            "internal_module" => Some("namespace"),
            "variable_declarator" if is_module_level_variable(node) => Some("variable"),
            _ => None,
        },
        "py" => match kind {
            "function_definition" => Some("function"),
            "class_definition" => Some("class"),
            _ => None,
        },
        "go" => match kind {
            "function_declaration" => Some("function"),
            "method_declaration" => Some("method"),
            "type_spec" => Some("type"),
            _ => None,
        },
        "c" | "h" => match kind {
            "function_definition" => Some("function"),
            "struct_specifier" => Some("struct"),
            "enum_specifier" => Some("enum"),
            "type_definition" => Some("type"),
            _ => None,
        },
        "cc" | "cpp" | "cxx" | "hh" | "hpp" | "hxx" => match kind {
            "function_definition" => Some("function"),
            "class_specifier" => Some("class"),
            "struct_specifier" => Some("struct"),
            "enum_specifier" => Some("enum"),
            "namespace_definition" => Some("namespace"),
            "type_definition" => Some("type"),
            _ => None,
        },
        _ => None,
    }
}

fn is_module_level_variable(node: Node<'_>) -> bool {
    let mut parent = node.parent();
    while let Some(current) = parent {
        match current.kind() {
            "lexical_declaration" | "variable_declaration" | "export_statement" => {
                parent = current.parent();
            }
            "program" => return true,
            _ => return false,
        }
    }
    false
}

fn declaration_name_node(node: Node<'_>) -> Option<Node<'_>> {
    if let Some(name) = node.child_by_field_name("name") {
        return Some(name);
    }
    if let Some(declarator) = node.child_by_field_name("declarator") {
        if let Some(identifier) = find_identifier_descendant(declarator, 0) {
            return Some(identifier);
        }
    }
    find_identifier_descendant(node, 0)
}

fn find_identifier_descendant(node: Node<'_>, depth: usize) -> Option<Node<'_>> {
    if depth > 10 {
        return None;
    }
    if is_identifier_node(node.kind()) {
        return Some(node);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(identifier) = find_identifier_descendant(child, depth + 1) {
            return Some(identifier);
        }
    }
    None
}

fn collect_reference_nodes(
    node: Node<'_>,
    bytes: &[u8],
    symbol: &str,
    root: &Path,
    file: &Path,
    text: &str,
    output: &mut Vec<Value>,
) {
    if output.len() >= MAX_REFERENCE_RESULTS {
        return;
    }
    if is_identifier_node(node.kind()) && node.utf8_text(bytes).is_ok_and(|value| value == symbol) {
        let start = node.start_position();
        output.push(json!({
            "path": relative_display(root, file),
            "line": start.row + 1,
            "character": character_column(text, start),
            "preview": line_preview(text, start.row),
        }));
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_reference_nodes(child, bytes, symbol, root, file, text, output);
        if output.len() >= MAX_REFERENCE_RESULTS {
            break;
        }
    }
}

fn is_identifier_node(kind: &str) -> bool {
    matches!(
        kind,
        "identifier"
            | "type_identifier"
            | "field_identifier"
            | "namespace_identifier"
            | "shorthand_property_identifier"
            | "shorthand_property_identifier_pattern"
    )
}

fn language_for_file(file: &Path) -> Option<Language> {
    match extension(file).as_str() {
        "rs" => Some(tree_sitter_rust::LANGUAGE.into()),
        "js" | "jsx" => Some(tree_sitter_javascript::LANGUAGE.into()),
        "ts" => Some(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
        "tsx" => Some(tree_sitter_typescript::LANGUAGE_TSX.into()),
        "py" => Some(tree_sitter_python::LANGUAGE.into()),
        "go" => Some(tree_sitter_go::LANGUAGE.into()),
        "c" | "h" => Some(tree_sitter_c::LANGUAGE.into()),
        "cc" | "cpp" | "cxx" | "hh" | "hpp" | "hxx" => Some(tree_sitter_cpp::LANGUAGE.into()),
        _ => None,
    }
}

fn symbol_json(root: &Path, file: &Path, text: &str, symbol: &AstSymbol, preview: bool) -> Value {
    let mut value = json!({
        "name": symbol.name,
        "kind": symbol.kind,
        "path": relative_display(root, file),
        "line": symbol.start.row + 1,
        "character": character_column(text, symbol.start),
        "endLine": symbol.end.row + 1,
        "endCharacter": character_column(text, symbol.end),
    });
    if preview {
        if let Value::Object(ref mut object) = value {
            object.insert(
                "preview".to_string(),
                Value::String(line_preview(text, symbol.start.row)),
            );
        }
    }
    value
}

fn character_column(text: &str, point: Point) -> usize {
    let Some(line) = text.lines().nth(point.row) else {
        return point.column;
    };
    if point.column >= line.len() {
        return line.chars().count();
    }
    let mut column = point.column;
    while column > 0 && !line.is_char_boundary(column) {
        column -= 1;
    }
    line[..column].chars().count()
}

fn line_preview(text: &str, row: usize) -> String {
    text.lines()
        .nth(row)
        .map(str::trim)
        .map(|line| truncate_preview(line, MAX_PREVIEW_CHARS))
        .unwrap_or_default()
}

fn symbol_at_position(text: &str, line: usize, character: usize) -> Result<String, AppError> {
    let line_text = text
        .lines()
        .nth(line.saturating_sub(1))
        .ok_or_else(|| AppError::internal("symbol navigation line is outside the file"))?;
    let chars = line_text.chars().collect::<Vec<_>>();
    if character > chars.len() {
        return Err(AppError::internal(
            "symbol navigation character is outside the line",
        ));
    }

    let mut index = character.min(chars.len().saturating_sub(1));
    if chars
        .get(index)
        .is_none_or(|character| !is_identifier_char(*character))
    {
        if index > 0 && is_identifier_char(chars[index - 1]) {
            index -= 1;
        } else {
            return Err(AppError::internal(
                "no symbol found at the requested position",
            ));
        }
    }

    let mut start = index;
    while start > 0 && is_identifier_char(chars[start - 1]) {
        start -= 1;
    }
    let mut end = index + 1;
    while end < chars.len() && is_identifier_char(chars[end]) {
        end += 1;
    }
    Ok(chars[start..end].iter().collect())
}

fn collect_source_files(root: &Path) -> Result<Vec<PathBuf>, AppError> {
    let mut files = Vec::new();
    let mut directories = vec![(root.to_path_buf(), 0usize)];
    while let Some((directory, depth)) = directories.pop() {
        if files.len() >= MAX_AST_FILES {
            break;
        }
        let mut entries = fs::read_dir(&directory)?.collect::<Result<Vec<_>, _>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries.into_iter().rev() {
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_dir() {
                if depth < MAX_AST_SCAN_DEPTH && !ignored_directory(&path) {
                    directories.push((path, depth + 1));
                }
            } else if metadata.is_file()
                && language_for_file(&path).is_some()
                && metadata.len() <= MAX_SOURCE_BYTES
            {
                files.push(path);
                if files.len() >= MAX_AST_FILES {
                    break;
                }
            }
        }
    }
    files.sort();
    Ok(files)
}

fn resolve_source_file(root: &Path, relative_path: &str) -> Result<PathBuf, AppError> {
    let relative = Path::new(relative_path.trim());
    if relative.as_os_str().is_empty()
        || relative.is_absolute()
        || relative.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(AppError::internal(
            "Tree-sitter navigation requires a workspace-relative file path",
        ));
    }
    let joined = root.join(relative);
    if joined.exists() && fs::symlink_metadata(&joined)?.file_type().is_symlink() {
        return Err(AppError::internal(
            "Tree-sitter navigation refuses symlink source files",
        ));
    }
    let file = fs::canonicalize(joined)?;
    if !file.starts_with(root) || !file.is_file() {
        return Err(AppError::internal(
            "Tree-sitter navigation file is outside the workspace or unsafe",
        ));
    }
    Ok(file)
}

fn read_source(path: &Path) -> Result<String, AppError> {
    let metadata = fs::metadata(path)?;
    if metadata.len() > MAX_SOURCE_BYTES {
        return Err(AppError::internal(
            "Tree-sitter navigation file exceeds the read limit",
        ));
    }
    fs::read_to_string(path)
        .map_err(|_| AppError::internal("Tree-sitter navigation supports UTF-8 source files only"))
}

fn canonical_root(root: &Path) -> Result<PathBuf, AppError> {
    let root = fs::canonicalize(root)?;
    if !root.is_dir() {
        return Err(AppError::internal(
            "Tree-sitter navigation root is not a directory",
        ));
    }
    Ok(root)
}

fn ignored_directory(path: &Path) -> bool {
    matches!(
        path.file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default(),
        ".git"
            | "node_modules"
            | "target"
            | "dist"
            | "build"
            | ".next"
            | ".venv"
            | "vendor"
            | ".openmindai-patch-transactions"
            | ".idea"
            | ".gradle"
            | ".cache"
            | "coverage"
            | "Pods"
            | "DerivedData"
    )
}

fn extension(path: &Path) -> String {
    path.extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

fn is_identifier_char(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

fn relative_display(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn truncate_preview(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let mut output = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        output.push('…');
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_outline_ignores_comment_and_string_decoys() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("lib.rs");
        fs::write(
            &file,
            "// fn fake() {}\nconst TEXT: &str = \"struct Ghost {}\";\npub struct Router {}\nimpl Router { fn dispatch(&self) {} }\n",
        )
        .unwrap();
        let result = document_symbols(temp.path(), "lib.rs").unwrap().unwrap();
        let text = result.to_string();
        assert!(text.contains("Router"));
        assert!(text.contains("dispatch"));
        assert!(!text.contains("fake"));
        assert!(!text.contains("Ghost"));
    }

    #[test]
    fn typescript_outline_captures_module_level_arrow_function() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join("api.ts"),
            "export const loadUser = async () => {};\nfunction helper() {}\n",
        )
        .unwrap();
        let result = document_symbols(temp.path(), "api.ts").unwrap().unwrap();
        let text = result.to_string();
        assert!(text.contains("loadUser"));
        assert!(text.contains("helper"));
    }

    #[test]
    fn references_exclude_comments_and_string_literals() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join("lib.rs"),
            "pub struct Router {}\nfn use_it() { let _ = Router {}; let _ = \"Router\"; }\n// Router\n",
        )
        .unwrap();
        let result = references(temp.path(), "lib.rs", 1, 12).unwrap().unwrap();
        let items = result.as_array().unwrap();
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn unsupported_language_returns_none_for_ast_outline() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("Main.java"), "class Main {}\n").unwrap();
        assert!(document_symbols(temp.path(), "Main.java")
            .unwrap()
            .is_none());
    }
}
