#[path = "inference_legacy.rs"]
mod legacy;

pub use legacy::{
    ActiveGenerations, InferenceMedia, InferenceMode, StreamChunkEvent, StreamDoneEvent,
    StreamRequest, StreamStartedEvent,
};

#[cfg(feature = "native-cxx-llama")]
pub use legacy::InferenceMetrics;

#[cfg(feature = "native-cxx-llama")]
#[path = "native_chat_router.rs"]
mod native_chat_router;

#[cfg(not(feature = "native-cxx-llama"))]
pub use legacy::stream_chat_completion;

#[cfg(feature = "native-cxx-llama")]
pub async fn stream_chat_completion(
    request: StreamRequest<'_>,
) -> Result<InferenceMetrics, crate::app_error::AppError> {
    if let Some(native_result) = native_chat_router::try_stream_native(&request).await {
        match native_result {
            Ok(metrics) => {
                tracing::info!(
                    model = request.model,
                    time_to_first_token_ms = ?metrics.time_to_first_token_ms,
                    elapsed_ms = metrics.elapsed_ms,
                    "native CXX chat inference completed"
                );
                return Ok(metrics);
            }
            Err(error) if error.can_retry() => {
                tracing::warn!(
                    model = request.model,
                    error = %error.error,
                    "native inference failed before output; falling back to llama-server"
                );
            }
            Err(error) => return Err(error.error),
        }
    }

    legacy::stream_chat_completion(request).await
}
