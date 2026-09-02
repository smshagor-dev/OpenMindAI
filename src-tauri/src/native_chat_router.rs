use std::{
    env,
    sync::{Arc, OnceLock},
};

use crate::{
    app_error::AppError,
    chat::ChatRepository,
    hardware::{HardwareProfile, HardwareProfiler},
    inference::{InferenceMetrics, InferenceMode, StreamRequest},
    launch_planner::ModelLaunchPlanner,
    model_registry::ModelRegistry,
    native_bridge::GenerationConfig,
    native_stream::{stream_native_completion, NativeStreamError, NativeStreamRequest},
    native_supervisor::{NativeInferenceSupervisor, NativeModelSpec},
    portable_root::PortableRootManager,
};

static NATIVE_SUPERVISOR: OnceLock<Arc<NativeInferenceSupervisor>> = OnceLock::new();
static NATIVE_HARDWARE: OnceLock<HardwareProfile> = OnceLock::new();
static NATIVE_ROOT: OnceLock<PortableRootManager> = OnceLock::new();

pub async fn try_stream_native(
    request: &StreamRequest<'_>,
) -> Option<Result<InferenceMetrics, NativeStreamError>> {
    match native_eligible(request) {
        Ok(false) => return None,
        Err(error) => return Some(Err(before_output(error))),
        Ok(true) => {}
    }

    let prepared = prepare_native_request(request);
    let (model, config) = match prepared {
        Ok(value) => value,
        Err(error) => return Some(Err(before_output(error))),
    };

    let supervisor = Arc::clone(
        NATIVE_SUPERVISOR.get_or_init(|| Arc::new(NativeInferenceSupervisor::start())),
    );
    Some(
        stream_native_completion(NativeStreamRequest {
            app: request.app,
            database: request.database,
            active: request.active,
            supervisor,
            model,
            conversation_id: request.conversation_id,
            assistant: request.assistant,
            config,
        })
        .await,
    )
}

fn native_eligible(request: &StreamRequest<'_>) -> Result<bool, AppError> {
    if !native_chat_enabled() || !request.media.is_empty() {
        return Ok(false);
    }
    if !matches!(&request.mode, InferenceMode::Chat) {
        return Ok(false);
    }

    let db = request
        .database
        .lock()
        .map_err(|_| AppError::internal("database lock poisoned"))?;
    let latest_user = ChatRepository::new(&db)
        .list_messages(request.conversation_id)?
        .into_iter()
        .rev()
        .find(|message| message.role == "user" && message.status == "completed");
    let Some(message) = latest_user else {
        return Ok(false);
    };
    let content = message.content.to_ascii_lowercase();
    if content.contains("[mode: web search]")
        || content.contains("[mode: deep research]")
        || content.contains("data:image/")
    {
        return Ok(false);
    }
    Ok(true)
}

fn prepare_native_request(
    request: &StreamRequest<'_>,
) -> Result<(NativeModelSpec, GenerationConfig), AppError> {
    let root = native_root()?;
    let model = {
        let db = request
            .database
            .lock()
            .map_err(|_| AppError::internal("database lock poisoned"))?;
        ModelRegistry::new(&db, root)
            .list_models()?
            .into_iter()
            .find(|model| model.id == request.model && model.enabled && model.format == "gguf")
            .ok_or_else(|| AppError::ModelNotFound(request.model.to_string()))?
    };

    let model_path = root.resolve_relative(&model.path)?;
    let hardware = NATIVE_HARDWARE.get_or_init(HardwareProfiler::detect);
    let plan = ModelLaunchPlanner::plan(&model, hardware, 0);
    let n_threads = i32::try_from(plan.config.threads).unwrap_or(i32::MAX).max(1);

    let config = GenerationConfig {
        temperature: 0.6,
        top_p: 0.95,
        max_tokens: 768,
        n_ctx: plan.config.context_size,
        n_batch: plan.config.batch_size,
        n_threads,
    };
    config
        .validate()
        .map_err(|message| AppError::InferenceFailed(message.to_string()))?;

    Ok((
        NativeModelSpec {
            id: model.id,
            path: model_path,
            n_gpu_layers: plan.config.gpu_layers,
        },
        config,
    ))
}

fn native_root() -> Result<&'static PortableRootManager, AppError> {
    if let Some(root) = NATIVE_ROOT.get() {
        return Ok(root);
    }
    let resolved = PortableRootManager::resolve()?;
    let _ = NATIVE_ROOT.set(resolved);
    NATIVE_ROOT
        .get()
        .ok_or_else(|| AppError::internal("failed to initialize native root"))
}

fn native_chat_enabled() -> bool {
    env::var("OPENMINDAI_NATIVE_CHAT")
        .ok()
        .is_some_and(|value| matches!(value.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
}

fn before_output(error: AppError) -> NativeStreamError {
    NativeStreamError {
        error,
        emitted_output: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_flag_is_off_unless_explicitly_enabled() {
        let previous = env::var_os("OPENMINDAI_NATIVE_CHAT");
        env::remove_var("OPENMINDAI_NATIVE_CHAT");
        assert!(!native_chat_enabled());
        if let Some(previous) = previous {
            env::set_var("OPENMINDAI_NATIVE_CHAT", previous);
        }
    }
}
