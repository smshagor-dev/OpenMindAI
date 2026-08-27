use serde::Serialize;
use thiserror::Error;

#[allow(dead_code)]
#[derive(Debug, Error, Serialize)]
#[serde(tag = "code", content = "message")]
pub enum AppError {
    #[error("ROOT_NOT_WRITABLE: {0}")]
    RootNotWritable(String),
    #[error("DATABASE_INIT_FAILED: {0}")]
    DatabaseInitFailed(String),
    #[error("MODEL_NOT_FOUND: {0}")]
    ModelNotFound(String),
    #[error("MODEL_INVALID: {0}")]
    ModelInvalid(String),
    #[error("MODEL_DOWNLOAD_FAILED: {0}")]
    ModelDownloadFailed(String),
    #[error("MODEL_CHECKSUM_FAILED: {0}")]
    ModelChecksumFailed(String),
    #[error("MODEL_LOAD_FAILED: {0}")]
    ModelLoadFailed(String),
    #[error("MODEL_OUT_OF_MEMORY: {0}")]
    ModelOutOfMemory(String),
    #[error("MODEL_UNSUPPORTED: {0}")]
    ModelUnsupported(String),
    #[error("CONTEXT_OVERFLOW: {0}")]
    ContextOverflow(String),
    #[error("INFERENCE_SERVER_UNAVAILABLE: {0}")]
    InferenceServerUnavailable(String),
    #[error("INFERENCE_FAILED: {0}")]
    InferenceFailed(String),
    #[error("INFERENCE_CANCELLED: {0}")]
    InferenceCancelled(String),
    #[error("STREAM_FAILED: {0}")]
    StreamFailed(String),
    #[error("RUNTIME_NOT_FOUND: {0}")]
    RuntimeNotFound(String),
    #[error("RUNTIME_START_FAILED: {0}")]
    RuntimeStartFailed(String),
    #[error("RUNTIME_INSTALL_FAILED: {0}")]
    RuntimeInstallFailed(String),
    #[error("GPU_BACKEND_UNAVAILABLE: {0}")]
    GpuBackendUnavailable(String),
    #[error("INSUFFICIENT_STORAGE: {0}")]
    InsufficientStorage(String),
    #[error("ARTIFACT_NOT_FOUND: {0}")]
    ArtifactNotFound(String),
    #[error("ARTIFACT_GENERATION_FAILED: {0}")]
    ArtifactGenerationFailed(String),
    #[error("GITHUB_API_ERROR: {0}")]
    GithubApiError(String),
    #[error("SECRET_STORE_FAILED: {0}")]
    SecretStoreFailed(String),
    #[error("SETUP_REQUIRED: {0}")]
    SetupRequired(String),
    #[error("INTERNAL: {0}")]
    Internal(String),
}

impl AppError {
    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal(message.into())
    }
}

impl From<rusqlite::Error> for AppError {
    fn from(value: rusqlite::Error) -> Self {
        Self::DatabaseInitFailed(value.to_string())
    }
}

impl From<std::io::Error> for AppError {
    fn from(value: std::io::Error) -> Self {
        Self::Internal(value.to_string())
    }
}
