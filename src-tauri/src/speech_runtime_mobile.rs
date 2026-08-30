use std::path::Path;

use serde::Serialize;

use crate::{app_error::AppError, portable_root::PortableRootManager};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptionResult {
    pub text: String,
    pub language: Option<String>,
    pub duration_seconds: f64,
}

pub async fn transcribe_data_url(
    root: &PortableRootManager,
    model_path: &Path,
    data_url: &str,
    source_name: &str,
) -> Result<TranscriptionResult, AppError> {
    let _ = (root, model_path, data_url, source_name);
    Err(AppError::InferenceFailed(
        "Local OpenMindAI Hear is not enabled in the Android/iOS app shell yet. Mobile speech transcription is planned for the mobile inference phase."
            .to_string(),
    ))
}
