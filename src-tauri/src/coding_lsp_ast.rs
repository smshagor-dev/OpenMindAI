use std::path::Path;

use serde_json::Value;

use crate::{app_error::AppError, coding_ast};

mod legacy {
    include!("coding_lsp.rs");
}

pub use legacy::{
    diagnostics, incoming_calls, outgoing_calls, shutdown_pooled_sessions, NavigationResult,
};

pub async fn workspace_symbols(
    root: &Path,
    query: &str,
    allow_language_server: bool,
) -> Result<NavigationResult, AppError> {
    let fallback = legacy::workspace_symbols(root, query, allow_language_server).await?;
    if !uses_lexical_fallback(&fallback) {
        return Ok(fallback);
    }
    match coding_ast::workspace_symbols(root, query) {
        Ok(Some(result)) => Ok(ast_navigation(fallback, result)),
        Ok(None) | Err(_) => Ok(fallback),
    }
}

pub async fn document_symbols(
    root: &Path,
    relative_path: &str,
    allow_language_server: bool,
) -> Result<NavigationResult, AppError> {
    let fallback = legacy::document_symbols(root, relative_path, allow_language_server).await?;
    if !uses_lexical_fallback(&fallback) {
        return Ok(fallback);
    }
    match coding_ast::document_symbols(root, relative_path) {
        Ok(Some(result)) => Ok(ast_navigation(fallback, result)),
        Ok(None) | Err(_) => Ok(fallback),
    }
}

pub async fn definition(
    root: &Path,
    relative_path: &str,
    line: u64,
    character: u64,
    allow_language_server: bool,
) -> Result<NavigationResult, AppError> {
    let fallback =
        legacy::definition(root, relative_path, line, character, allow_language_server).await?;
    if !uses_lexical_fallback(&fallback) {
        return Ok(fallback);
    }
    match coding_ast::definition(root, relative_path, line, character) {
        Ok(Some(result)) => Ok(ast_navigation(fallback, result)),
        Ok(None) | Err(_) => Ok(fallback),
    }
}

pub async fn references(
    root: &Path,
    relative_path: &str,
    line: u64,
    character: u64,
    allow_language_server: bool,
) -> Result<NavigationResult, AppError> {
    let fallback =
        legacy::references(root, relative_path, line, character, allow_language_server).await?;
    if !uses_lexical_fallback(&fallback) {
        return Ok(fallback);
    }
    match coding_ast::references(root, relative_path, line, character) {
        Ok(Some(result)) => Ok(ast_navigation(fallback, result)),
        Ok(None) | Err(_) => Ok(fallback),
    }
}

pub async fn hover(
    root: &Path,
    relative_path: &str,
    line: u64,
    character: u64,
    allow_language_server: bool,
) -> Result<NavigationResult, AppError> {
    let fallback =
        legacy::hover(root, relative_path, line, character, allow_language_server).await?;
    if !uses_lexical_fallback(&fallback) {
        return Ok(fallback);
    }
    match coding_ast::hover(root, relative_path, line, character) {
        Ok(Some(result)) => Ok(ast_navigation(fallback, result)),
        Ok(None) | Err(_) => Ok(fallback),
    }
}

fn uses_lexical_fallback(result: &NavigationResult) -> bool {
    result.engine == "lexical-fallback" || result.engine.ends_with("+lexical-fallback")
}

fn ast_navigation(previous: NavigationResult, result: Value) -> NavigationResult {
    let engine = if previous.server.is_some() && previous.engine.starts_with("lsp") {
        "lsp+tree-sitter-fallback"
    } else {
        "tree-sitter-fallback"
    };
    NavigationResult {
        engine: engine.to_string(),
        server: previous.server,
        result,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn ast_engine_preserves_lsp_server_context() {
        let navigation = ast_navigation(
            NavigationResult {
                engine: "lsp+lexical-fallback".to_string(),
                server: Some("rust-analyzer@.".to_string()),
                result: Value::Array(Vec::new()),
            },
            json!([{"name":"Router"}]),
        );
        assert_eq!(navigation.engine, "lsp+tree-sitter-fallback");
        assert_eq!(navigation.server.as_deref(), Some("rust-analyzer@."));
    }

    #[test]
    fn pure_fallback_is_labeled_tree_sitter() {
        let navigation = ast_navigation(
            NavigationResult {
                engine: "lexical-fallback".to_string(),
                server: None,
                result: Value::Array(Vec::new()),
            },
            json!([]),
        );
        assert_eq!(navigation.engine, "tree-sitter-fallback");
        assert!(navigation.server.is_none());
    }
}
