#[cfg(target_os = "android")]
use std::{
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

#[cfg(target_os = "android")]
use llama_cpp_2::{
    llama_backend::LlamaBackend,
    model::{params::LlamaModelParams, LlamaModel},
};

#[cfg(target_os = "android")]
use crate::app_error::AppError;

#[cfg(target_os = "android")]
struct CachedModel {
    path: PathBuf,
    model: LlamaModel,
}

#[cfg(target_os = "android")]
struct CacheState {
    backend: LlamaBackend,
    model: Option<CachedModel>,
}

#[cfg(target_os = "android")]
static MOBILE_MODEL_CACHE: OnceLock<Mutex<Option<CacheState>>> = OnceLock::new();

#[cfg(target_os = "android")]
fn cache() -> &'static Mutex<Option<CacheState>> {
    MOBILE_MODEL_CACHE.get_or_init(|| Mutex::new(None))
}

#[cfg(target_os = "android")]
pub(crate) fn with_cached_model<T>(
    model_path: &Path,
    operation: impl FnOnce(&LlamaBackend, &LlamaModel) -> Result<T, AppError>,
) -> Result<T, AppError> {
    let mut guard = cache()
        .lock()
        .map_err(|_| AppError::ModelLoadFailed("Android model cache lock poisoned".to_string()))?;

    if guard.is_none() {
        let backend = LlamaBackend::init().map_err(|error| {
            AppError::ModelLoadFailed(format!("llama backend init failed: {error}"))
        })?;
        *guard = Some(CacheState {
            backend,
            model: None,
        });
    }

    let state = guard
        .as_mut()
        .ok_or_else(|| AppError::ModelLoadFailed("Android model cache unavailable".to_string()))?;
    let cache_hit = state
        .model
        .as_ref()
        .is_some_and(|cached| cached.path == model_path);

    if !cache_hit {
        // Keep one model resident at a time. Dropping the old model before loading
        // the replacement avoids carrying two GGUF allocations on memory-constrained phones.
        state.model = None;
        let model = LlamaModel::load_from_file(
            &state.backend,
            model_path,
            &LlamaModelParams::default(),
        )
        .map_err(|error| AppError::ModelLoadFailed(format!("failed to load GGUF: {error}")))?;
        state.model = Some(CachedModel {
            path: model_path.to_path_buf(),
            model,
        });
    }

    let cached = state.model.as_ref().ok_or_else(|| {
        AppError::ModelLoadFailed("Android model cache did not retain the loaded model".to_string())
    })?;
    operation(&state.backend, &cached.model)
}
