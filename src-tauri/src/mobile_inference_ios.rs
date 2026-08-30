use serde::Serialize;
use tauri::State;

use crate::{app_error::AppError, AppState};

#[derive(Clone, Default)]
pub(crate) struct MobileInferenceState;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NativeInferenceProbeResult {
    model_id: String,
    output: String,
    prompt_tokens: usize,
    generated_tokens: u32,
    elapsed_ms: u128,
}

#[tauri::command]
pub(crate) async fn mobile_native_inference_probe(
    model_id: String,
    prompt: String,
    max_tokens: Option<u32>,
    state: State<'_, AppState>,
    native: State<'_, MobileInferenceState>,
) -> Result<NativeInferenceProbeResult, AppError> {
    let _ = (model_id, prompt, max_tokens, state, native);
    Err(AppError::ModelUnsupported(
        "embedded local inference is currently implemented for Android only".to_string(),
    ))
}
