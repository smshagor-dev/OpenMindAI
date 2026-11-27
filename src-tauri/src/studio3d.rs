use base64::{engine::general_purpose::STANDARD, Engine as _};
use chrono::Utc;
use rusqlite::{params, OptionalExtension};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs,
    io::Read,
    path::PathBuf,
    sync::{Mutex, OnceLock},
};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{app_error::AppError, AppState};

const MAX_GLB_BYTES: usize = 30 * 1024 * 1024;
const MAX_MESH_BYTES: usize = 30 * 1024 * 1024;
const MAX_GLTF_BUNDLE_BYTES: usize = 30 * 1024 * 1024;
const MAX_SCENE_JSON_BYTES: usize = 2 * 1024 * 1024;
const MAX_IMAGE_BYTES: usize = 30 * 1024 * 1024;
const MAX_IMAGE_PIXELS: u64 = 40_000_000;
const STUDIO3D_MAX_CONCURRENT_JOBS: usize = 1;
const STUDIO3D_LOCAL_JOB_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(8 * 60);
static STUDIO3D_SCHEDULER: OnceLock<Semaphore> = OnceLock::new();
static STUDIO3D_CANCELLATIONS: OnceLock<Mutex<HashMap<String, CancellationToken>>> =
    OnceLock::new();

fn studio3d_scheduler() -> &'static Semaphore {
    STUDIO3D_SCHEDULER.get_or_init(|| Semaphore::new(STUDIO3D_MAX_CONCURRENT_JOBS))
}

// Both diagnostics and the scheduler resolve from the current portable root.
fn planning_model(state: &AppState) -> Result<ScenePlanner, AppError> {
    let models = {
        let database = state
            .database
            .lock()
            .map_err(|_| AppError::internal("database lock poisoned"))?;
        crate::model_registry::ModelRegistry::new(&database, &state.root).discover_gguf_models()?
    };
    select_planning_model(&state.root, models)
}

type ScenePlanner = (Option<crate::model_registry::ModelRecord>, Vec<Value>);

fn select_planning_model(
    root: &crate::portable_root::PortableRootManager,
    mut models: Vec<crate::model_registry::ModelRecord>,
) -> Result<ScenePlanner, AppError> {
    use crate::model_registry::ModelLifecycleState;
    models.sort_by_key(|model| {
        let qwen = model.source_repository.as_deref() == Some("Qwen/Qwen3-4B-GGUF");
        (!qwen, model.name.to_lowercase(), model.id.clone())
    });
    let mut unavailable = Vec::new();
    for model in models {
        let (status, reason) = match model.state {
            ModelLifecycleState::NotInstalled => {
                ("registeredMissing", "Registered file is missing".into())
            }
            ModelLifecycleState::Failed => (
                "temporarilyUnavailable",
                "File or package is unreadable or invalid".into(),
            ),
            _ if !model.enabled => ("temporarilyUnavailable", "Model is disabled".into()),
            _ if !serde_json::from_str::<Vec<String>>(&model.capabilities)
                .is_ok_and(|capabilities| capabilities.iter().any(|item| item == "chat")) =>
            {
                (
                    "temporarilyUnavailable",
                    "Model does not advertise chat capability".into(),
                )
            }
            _ => match crate::model_registry::validate_scene_planner(root, &model.path) {
                Ok(true) => return Ok((Some(model), unavailable)),
                Ok(false) => (
                    "installedUnvalidated",
                    "Verified file checksum is unavailable or mismatched".into(),
                ),
                Err(error) => ("temporarilyUnavailable", error.to_string()),
            },
        };
        unavailable.push(serde_json::json!({
            "id": model.id, "model": model.name, "path": model.path,
            "status": status, "reason": reason
        }));
    }
    Ok((None, unavailable))
}

async fn resolve_scene_planner(app: AppHandle) -> Result<ScenePlanner, AppError> {
    tauri::async_runtime::spawn_blocking(move || planning_model(&app.state::<AppState>()))
        .await
        .map_err(|error| AppError::internal(format!("Scene planner inspection failed: {error}")))?
}

pub(crate) async fn scene_planner_for_generation(
    app: AppHandle,
) -> Result<crate::model_registry::ModelRecord, AppError> {
    let (model, unavailable) = resolve_scene_planner(app).await?;
    model.ok_or_else(|| {
        AppError::ModelNotFound(format!(
            "No installed, validated local language model is ready for 3D scene planning ({})",
            unavailable
                .first()
                .and_then(|item| item["reason"].as_str())
                .unwrap_or("no language models found in the active portable root")
        ))
    })
}

async fn local_scene_readiness(app: AppHandle) -> Result<Value, AppError> {
    let (model, unavailable) = resolve_scene_planner(app.clone()).await?;
    let state = app.state::<AppState>();
    let hardware = crate::hardware::HardwareProfiler::for_inference(&state.hardware);
    let mut runtime = state
        .runtime
        .lock()
        .map_err(|_| AppError::internal("runtime lock poisoned"))?;
    let inventory = runtime.inventory(&hardware)?;
    let (status, detail) = match &model {
        None => (
            unavailable
                .first()
                .and_then(|item| item["status"].as_str())
                .unwrap_or("notInstalled"),
            unavailable
                .first()
                .and_then(|item| item["reason"].as_str())
                .unwrap_or("No language models found in the active portable root"),
        ),
        Some(_) if inventory.selected.is_none() => {
            ("missingRuntime", "No usable llama.cpp runtime installed")
        }
        Some(model) => {
            let path = runtime.resolved_model_path(&model.path)?;
            runtime.scene_planner_status(&path)
        }
    };
    Ok(serde_json::json!({
        "status": status, "detail": detail, "model": model.as_ref().map(|item| &item.name),
        "modelId": model.as_ref().map(|item| &item.id), "modelPath": model.as_ref().map(|item| &item.path),
        "verification": model.as_ref().map(|_| "checksumVerified"),
        "runtimeAvailable": inventory.selected.is_some(),
        "runtime": inventory.selected, "serverRuntime": runtime.scene_planner_runtime(),
        "serverState": runtime.state(), "serverModelPath": runtime.loaded_model_path(),
        "unavailableModels": unavailable, "scope": "Text and schema-limited scene planning"
    }))
}

#[tauri::command]
pub(crate) async fn studio3d_neural_capabilities(app: AppHandle) -> Result<Value, AppError> {
    let local_scene_reasoning = local_scene_readiness(app.clone()).await?;
    let state = app.state::<AppState>();
    let hardware = crate::hardware::HardwareProfiler::for_inference(&state.hardware);
    let model_root = state.root.root().join("models/3d");
    let model_root_exists = model_root.is_dir();
    let hardware_json = serde_json::to_value(&hardware)
        .map_err(|_| studio_error("Could not serialize the detected hardware profile"))?;
    Ok(serde_json::json!({
        "hardware": hardware_json,
        "modelRoot": "models/3d",
        "modelRootExists": model_root_exists,
        "engines": [
            {
                "id": "triposr",
                "name": "TripoSR",
                "status": "Unsupported",
                "reason": "OpenMindAI Desktop has no TripoSR adapter, reviewed runtime lock, or installed weights. Upstream documents a CPU fallback; an app-ready setup is not verified here.",
                "requirements": "Python 3.8+ and PyTorch; CUDA is recommended. Upstream documents about 6 GB VRAM for its default GPU path and a CPU fallback.",
                "license": "MIT (upstream code and pretrained weights)",
                "weightStorage": "Not verified; no download will be started",
                "source": "https://github.com/VAST-AI-Research/TripoSR"
            },
            {
                "id": "hunyuan3d2",
                "name": "Hunyuan3D-2",
                "status": "Unsupported",
                "reason": "OpenMindAI Desktop has no Hunyuan3D adapter or verified runtime/weight package. Its upstream Windows support does not establish compatibility with this app runtime.",
                "requirements": "PyTorch plus a large model runtime and weights; GPU path not verified on this machine.",
                "license": "Tencent Hunyuan Community License (non-commercial terms; review before use)",
                "weightStorage": "Not verified; no download will be started",
                "source": "https://github.com/Tencent-Hunyuan/Hunyuan3D-2"
            },
            {
                "id": "trellis",
                "name": "Microsoft TRELLIS",
                "status": "Unsupported",
                "reason": "Upstream currently targets Linux and requires an NVIDIA GPU with at least 16 GB VRAM; the detected host is Windows with Intel integrated graphics.",
                "requirements": "Linux, NVIDIA GPU with 16 GB+ VRAM, CUDA 11.8/12.2, Python 3.8+.",
                "license": "MIT for repository code; model-weight terms require separate review",
                "weightStorage": "Not verified; no download will be started",
                "source": "https://github.com/microsoft/TRELLIS"
            },
            {
                "id": "cloud-image-to-3d",
                "name": "OpenMindAI Cloud image-to-3D",
                "status": "Cloud Permission Required",
                "reason": "The dedicated Cloud 3D engine catalog previously returned HTTP 403. It remains disabled; no alternate endpoint is used.",
                "requirements": "An authorized specialized image-to-mesh endpoint and explicit image-upload approval.",
                "license": "Provider/model license not advertised",
                "weightStorage": "Cloud managed; not checked",
                "source": "https://cloud.openmindai.org/"
            }
        ],
        "localSceneReasoning": local_scene_reasoning,
        "proceduralGeometry": {
            "status": "Installed & Ready",
            "scope": "Editable CPU procedural triangle meshes; not neural reconstruction"
        }
    }))
}

fn studio3d_cancellations() -> &'static Mutex<HashMap<String, CancellationToken>> {
    STUDIO3D_CANCELLATIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn validate_scene_spec(value: Value) -> Result<Value, AppError> {
    if value.get("schemaVersion").and_then(Value::as_u64) != Some(1) {
        return Err(studio_error("scene plan schemaVersion must be 1"));
    }
    if value.as_object().is_none_or(|object| {
        object
            .keys()
            .any(|key| !matches!(key.as_str(), "schemaVersion" | "objects"))
    }) {
        return Err(studio_error(
            "scene plan contains unsupported top-level fields",
        ));
    }
    let objects = value
        .get("objects")
        .and_then(Value::as_array)
        .filter(|items| !items.is_empty() && items.len() <= 16)
        .ok_or_else(|| studio_error("local model scene plan must contain 1 to 16 objects"))?;
    let mut names = std::collections::HashSet::new();
    for object in objects {
        if object.as_object().is_none_or(|fields| {
            fields.keys().any(|key| {
                !matches!(
                    key.as_str(),
                    "name"
                        | "primitive"
                        | "dimensions"
                        | "position"
                        | "rotation"
                        | "color"
                        | "metallic"
                        | "roughness"
                        | "parent"
                )
            })
        }) {
            return Err(studio_error(
                "scene plan object contains unsupported fields",
            ));
        }
        let name = object
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| {
                !name.trim().is_empty() && name.len() <= 80 && !name.chars().any(char::is_control)
            })
            .ok_or_else(|| studio_error("scene plan contains an invalid object name"))?;
        if !names.insert(name.to_owned()) {
            return Err(studio_error("scene plan object names must be unique"));
        }
        if !matches!(
            object.get("primitive").and_then(Value::as_str),
            Some("box" | "cylinder" | "sphere" | "cone" | "wheel")
        ) {
            return Err(studio_error(
                "scene plan uses an unsupported geometry primitive",
            ));
        }
        for (field, minimum, maximum, strictly_positive) in [
            ("dimensions", 0.0, 10.0, true),
            ("position", -20.0, 20.0, false),
            ("rotation", -6.28319, 6.28319, false),
            ("color", 0.0, 1.0, false),
        ] {
            let vector = object
                .get(field)
                .and_then(Value::as_array)
                .filter(|items| items.len() == 3)
                .ok_or_else(|| {
                    studio_error(format!(
                        "scene plan {field} must have three numeric components"
                    ))
                })?;
            for component in vector {
                let number = component
                    .as_f64()
                    .filter(|number| number.is_finite())
                    .ok_or_else(|| {
                        studio_error(format!("scene plan {field} contains a non-finite number"))
                    })?;
                if number < minimum || number > maximum || (strictly_positive && number <= 0.0) {
                    return Err(studio_error(format!(
                        "scene plan {field} is outside its allowed range"
                    )));
                }
            }
        }
        for field in ["metallic", "roughness"] {
            object
                .get(field)
                .and_then(Value::as_f64)
                .filter(|number| number.is_finite() && (0.0..=1.0).contains(number))
                .ok_or_else(|| {
                    studio_error(format!("scene plan {field} must be between 0 and 1"))
                })?;
        }
        if let Some(parent) = object.get("parent").filter(|parent| !parent.is_null()) {
            if parent
                .as_str()
                .is_none_or(|parent| parent == name || parent.len() > 80)
            {
                return Err(studio_error(
                    "scene plan contains an invalid parent reference",
                ));
            }
        }
    }
    for object in objects {
        if let Some(parent) = object.get("parent").and_then(Value::as_str) {
            if !names.contains(parent) {
                return Err(studio_error("scene plan parent reference does not exist"));
            }
        }
    }
    let parents: HashMap<String, Option<String>> = objects
        .iter()
        .map(|object| {
            let name = object["name"].as_str().unwrap().to_owned();
            let parent = object
                .get("parent")
                .and_then(Value::as_str)
                .map(str::to_owned);
            (name, parent)
        })
        .collect();
    for name in names.iter() {
        let mut seen = std::collections::HashSet::new();
        let mut current = Some(name.as_str());
        while let Some(node) = current {
            if !seen.insert(node) {
                return Err(studio_error("scene plan hierarchy contains a parent cycle"));
            }
            current = parents.get(node).and_then(Option::as_deref);
        }
    }
    Ok(value)
}

pub(crate) fn parse_architecture_spec_response(response: &str) -> Result<Value, AppError> {
    if response.len() > 64 * 1024 {
        return Err(studio_error("architectural plan exceeds the 64 KB limit"));
    }
    let value: Value = serde_json::from_str(response.trim()).map_err(|error| {
        studio_error(format!(
            "local model returned invalid architectural JSON at line {}, column {}",
            error.line(),
            error.column()
        ))
    })?;
    let footprint_ok = value
        .get("footprint")
        .and_then(Value::as_array)
        .is_some_and(|v| {
            v.len() == 2 && v[0].as_f64() == Some(12.0) && v[1].as_f64() == Some(10.0)
        });
    if value.get("schemaVersion").and_then(Value::as_u64) != Some(2) || !footprint_ok {
        return Err(studio_error(
            "architectural plan must use schemaVersion 2 and a 12 by 10 metre footprint",
        ));
    }
    let _story_height = value
        .get("storyHeight")
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite() && (2.8..=3.8).contains(v))
        .ok_or_else(|| studio_error("architectural story height must be 2.8 to 3.8 metres"))?;
    if value.as_object().is_none_or(|o| {
        o.keys().any(|k| {
            !matches!(
                k.as_str(),
                "schemaVersion" | "footprint" | "storyHeight" | "rooms"
            )
        })
    }) {
        return Err(studio_error(
            "architectural plan contains unsupported top-level fields",
        ));
    }
    let rooms = value
        .get("rooms")
        .and_then(Value::as_array)
        .filter(|r| r.len() == 9)
        .ok_or_else(|| studio_error("architectural plan must contain exactly nine rooms"))?;
    let mut names = std::collections::HashSet::new();
    for room in rooms {
        if room.as_object().is_none_or(|o| {
            o.keys()
                .any(|k| !matches!(k.as_str(), "name" | "floor" | "x" | "z" | "width" | "depth"))
        }) {
            return Err(studio_error(
                "architectural room contains unsupported fields",
            ));
        }
        let name = room
            .get("name")
            .and_then(Value::as_str)
            .filter(|n| !n.trim().is_empty() && n.len() <= 80)
            .ok_or_else(|| studio_error("architectural room name is invalid"))?;
        if !names.insert(name.to_lowercase()) {
            return Err(studio_error("architectural room names must be unique"));
        }
        let floor = room
            .get("floor")
            .and_then(Value::as_str)
            .filter(|f| matches!(*f, "ground" | "upper"))
            .ok_or_else(|| studio_error("architectural room floor must be ground or upper"))?;
        let number = |key: &str| {
            room.get(key)
                .and_then(Value::as_f64)
                .filter(|v| v.is_finite())
        };
        let x = number("x").ok_or_else(|| studio_error("architectural room x is invalid"))?;
        let z = number("z").ok_or_else(|| studio_error("architectural room z is invalid"))?;
        let _width = number("width")
            .filter(|v| (1.0..=12.0).contains(v))
            .ok_or_else(|| studio_error("architectural room width must be 1 to 12 metres"))?;
        let _depth = number("depth")
            .filter(|v| (1.0..=10.0).contains(v))
            .ok_or_else(|| studio_error("architectural room depth must be 1 to 10 metres"))?;
        let _ = (x, z, floor);
    }
    let ground_names = rooms
        .iter()
        .filter(|r| r["floor"] == "ground")
        .filter_map(|r| r["name"].as_str())
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    let upper_rooms = rooms
        .iter()
        .filter(|r| r["floor"] == "upper")
        .collect::<Vec<_>>();
    let upper_names = upper_rooms
        .iter()
        .filter_map(|r| r["name"].as_str())
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    if !["living", "dining", "kitchen", "bathroom"]
        .iter()
        .all(|part| ground_names.contains(part))
        || upper_rooms
            .iter()
            .filter(|r| {
                r["name"]
                    .as_str()
                    .is_some_and(|n| n.to_lowercase().contains("bedroom"))
            })
            .count()
            < 3
        || !upper_names.contains("bathroom")
        || !upper_names.contains("landing")
    {
        return Err(studio_error("architectural plan is missing required ground-floor rooms or three upper bedrooms and a bathroom"));
    }
    normalize_architecture_room_layout(value)
}

/// Resolve approximate Qwen room coordinates into non-overlapping rectangles
/// inside the requested envelope. Names, floor assignments and room sizes stay
/// AI-authored; this bounded placement pass makes its rough coordinates usable
/// and records every moved room in the durable plan.
fn normalize_architecture_room_layout(mut value: Value) -> Result<Value, AppError> {
    let rooms = value
        .get_mut("rooms")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| studio_error("architectural plan has no room array"))?;
    let mut adjusted = Vec::new();
    for floor in ["ground", "upper"] {
        let mut indices = rooms
            .iter()
            .enumerate()
            .filter(|(_, room)| room["floor"] == floor)
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        indices.sort_by(|a, b| {
            let area = |index: usize| {
                rooms[index]["width"].as_f64().unwrap_or(0.0)
                    * rooms[index]["depth"].as_f64().unwrap_or(0.0)
            };
            area(*b).total_cmp(&area(*a))
        });
        let mut placed: Vec<(f64, f64, f64, f64)> = Vec::new();
        for index in indices {
            let room = &rooms[index];
            let name = room["name"].as_str().unwrap_or("Room").to_owned();
            let width = room["width"].as_f64().unwrap();
            let depth = room["depth"].as_f64().unwrap();
            let original_x = room["x"].as_f64().unwrap();
            let original_z = room["z"].as_f64().unwrap();
            let x_min = -6.0 + width / 2.0;
            let x_max = 6.0 - width / 2.0;
            let z_min = -5.0 + depth / 2.0;
            let z_max = 5.0 - depth / 2.0;
            let mut best: Option<(f64, f64, f64)> = None;
            let x_steps = ((x_max - x_min) / 0.25).ceil() as usize;
            let z_steps = ((z_max - z_min) / 0.25).ceil() as usize;
            for xi in 0..=x_steps {
                let x = (x_min + xi as f64 * 0.25).min(x_max);
                for zi in 0..=z_steps {
                    let z = (z_min + zi as f64 * 0.25).min(z_max);
                    let bounds = (
                        x - width / 2.0,
                        x + width / 2.0,
                        z - depth / 2.0,
                        z + depth / 2.0,
                    );
                    let overlaps = placed.iter().any(|(ax1, ax2, az1, az2)| {
                        bounds.0 < *ax2 + 0.08
                            && bounds.1 > *ax1 - 0.08
                            && bounds.2 < *az2 + 0.08
                            && bounds.3 > *az1 - 0.08
                    });
                    if overlaps {
                        continue;
                    }
                    let score = (x - original_x).powi(2) + (z - original_z).powi(2);
                    if best.is_none_or(|(_, _, best_score)| score < best_score) {
                        best = Some((x, z, score));
                    }
                }
            }
            let (x,z,_) = best.ok_or_else(|| studio_error(format!("could not fit {name:?} on the {floor} floor without overlap; reduce its AI-planned area")))?;
            if (x - original_x).abs() > 0.01 || (z - original_z).abs() > 0.01 {
                adjusted.push(Value::String(name));
            }
            rooms[index]["x"] = Value::from(x);
            rooms[index]["z"] = Value::from(z);
            placed.push((
                x - width / 2.0,
                x + width / 2.0,
                z - depth / 2.0,
                z + depth / 2.0,
            ));
        }
    }
    value["layoutAdjustments"] = Value::Array(adjusted);
    Ok(value)
}

pub(crate) fn parse_scene_spec_response(response: &str) -> Result<Value, AppError> {
    if response.len() > 64 * 1024 {
        return Err(studio_error("local scene plan exceeds the 64 KB limit"));
    }
    let value: Value = serde_json::from_str(response.trim()).map_err(|error| {
        let line = error.line();
        let column = error.column();
        tracing::warn!(parse_error = %error, response_bytes = response.len(), line, column, "local 3D scene response was not a single JSON document");
        studio_error(format!("local model returned invalid scene JSON at line {line}, column {column}"))
    })?;
    normalize_scene_spec(value).and_then(validate_scene_spec)
}

/// Validate a single-object patch returned by the local scene planner. Existing
/// scene geometry is never sent back to the geometry worker: the UI applies
/// this bounded specification to the selected node and retains all other nodes.
#[tauri::command]
pub(crate) async fn studio3d_ai_edit_object(
    app: AppHandle,
    prompt: String,
    operation: String,
    target: Value,
    state: State<'_, AppState>,
) -> Result<Value, AppError> {
    if prompt.trim().len() < 5 || prompt.len() > 2_000 {
        return Err(studio_error(
            "AI edit prompt must contain 5 to 2,000 characters",
        ));
    }
    if !matches!(operation.as_str(), "edit" | "add") {
        return Err(studio_error("AI edit operation must be edit or add"));
    }
    let target_name = target
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.trim().is_empty() && name.len() <= 120)
        .ok_or_else(|| studio_error("select a named scene object or room before editing"))?;
    let target_context = serde_json::to_string(&target)
        .map_err(|_| studio_error("selected object context is invalid"))?;
    if target_context.len() > 4_000 {
        return Err(studio_error(
            "selected object context exceeds the 4 KB limit",
        ));
    }
    let spatial_guidance = match target.pointer("/spatialContext/kind").and_then(Value::as_str) {
        Some("balcony_assembly_resize") => " Spatial constraints: the selected object is the balcony slab and spatialContext lists its stable-ID railing and baluster dependents. Return dimensions[0] widened by 2% to 35%; preserve dimensions[1], dimensions[2], position and rotation. The desktop validates and updates the complete assembly atomically; do not invent or rename members.",
        Some("room_aware_sofa_move") => " Spatial constraints: use spatialContext.worldBounds, room.bounds, doorwayClearanceBounds and nearbyFurniture. Return position as the selected object's local translation (not the world-space center), move at least 0.25 m toward a free opposite room wall, stay inside the room, clear every doorway and furniture item, and preserve dimensions, rotation and floor height. If spatialContext.safeCandidatePosition is non-null, use that exact position array; the desktop already preflighted that candidate against the room, doorways and furniture.",
        Some("connected_staircase_move") => " Spatial constraints: use spatialContext.worldBounds and the supplied floor, upper landing and entrance bounds. Return position as the selected group's local translation, move no more than 0.2 m closer to the main entrance, preserve vertical position, dimensions and rotation so both floor connections remain aligned. If spatialContext.safeCandidatePosition is non-null, use that exact position array; the desktop already preflighted both floor connections and door clearances.",
        _ => "",
    };
    let instruction = if operation == "edit" {
        format!(
            "Edit exactly the selected scene object in place. Preserve its exact name, primitive and local coordinate frame. Return one object only. Current selected object: {target_context}. Requested change: {}.{} Return a schemaVersion 1 scene plan with exactly one object and no other objects. The object must be named {:?}, parent must be JSON null in this patch; the desktop retains the existing hierarchy. Dimensions, position, rotation, color, metallic and roughness must be complete and valid. JSON only.",
            prompt.trim(), spatial_guidance, target_name
        )
    } else {
        format!(
            "Add exactly one new editable primitive as a child of the selected scene container. Do not modify or regenerate the selected container or any other scene object. Selected container: {target_context}. Requested addition: {}. Express position in the selected container's local coordinates. Return a schemaVersion 1 scene plan with exactly one object and no other objects. Use parent JSON null; the desktop attaches the new object under the selected container. Give the new object a concise unique name and complete valid primitive, dimensions, position, rotation, color, metallic and roughness. JSON only.",
            prompt.trim()
        )
    };
    let (model_id, response) =
        crate::studio3d_local_scene_spec(&app, &state, &instruction, false).await?;
    let plan = parse_scene_spec_response(&response)?;
    let object = validate_single_object_ai_patch(&plan, target_name, &operation)?;
    Ok(serde_json::json!({"modelId":model_id,"operation":operation,"object":object}))
}

#[tauri::command]
pub(crate) async fn studio3d_cloud_edit_object(
    prompt: String,
    operation: String,
    target: Value,
) -> Result<Value, AppError> {
    if prompt.trim().len() < 5 || prompt.len() > 2_000 {
        return Err(studio_error(
            "Cloud edit prompt must contain 5 to 2,000 characters",
        ));
    }
    if !matches!(operation.as_str(), "edit" | "add") {
        return Err(studio_error("Cloud edit operation must be edit or add"));
    }
    let target_name = target
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.trim().is_empty() && name.len() <= 120)
        .ok_or_else(|| studio_error("select a named scene object or room before editing"))?;
    let target_context = serde_json::to_string(&target)
        .map_err(|_| studio_error("selected object context is invalid"))?;
    if target_context.len() > 4_000 {
        return Err(studio_error(
            "selected object context exceeds the 4 KB limit",
        ));
    }
    let system = "You are an OpenMindAI 3D scene reasoning service. Return one schemaVersion 1 JSON scene plan with exactly one object. Use only box, cylinder, sphere, cone, wheel. Dimensions are full metres, rotations are radians, positions use the supplied local frame, colors and PBR values are 0..1. Never return code, prose, markdown or unsupported fields.";
    let task = if operation == "edit" {
        format!("Update the selected object's fields for this requested edit: {}. Preserve the exact name {:?}, the existing primitive and local coordinate frame. parent must be JSON null; the desktop retains the current hierarchy. Selected object: {target_context}", prompt.trim(), target_name)
    } else {
        format!("Add one new primitive to the selected container for this request: {}. Do not modify the selected container. Give the added object a new concise name. Set parent to JSON null; the desktop attaches it to the selected container. Express position in that container's local coordinates. Selected container: {target_context}", prompt.trim())
    };
    let model = crate::cloud_bridge::research_cloud_model().await?;
    let request_id = Uuid::new_v4().to_string();
    let completion = crate::cloud_bridge::research_cloud_completion(
        &model,
        &[
            serde_json::json!({"role":"system","content":system}),
            serde_json::json!({"role":"user","content":task}),
        ],
        4_096,
        &request_id,
    )
    .await
    .map_err(|error| {
        let detail = error.to_string();
        let provider_request_id = cloud_provider_request_id(&detail);
        studio_error(cloud_edit_diagnostic(
            &model.model_id,
            &request_id,
            provider_request_id.as_deref(),
            cloud_http_status(&detail),
            None,
            cloud_error_code(&detail),
            &detail,
        ))
    })?;
    let provider_request_id = completion.upstream_request_id.clone();
    let finish_reason = completion
        .data
        .pointer("/choices/0/finish_reason")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let usage = completion.data.get("usage").cloned();
    let response = completion
        .data
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            studio_error(cloud_edit_diagnostic(
                &model.model_id,
                &request_id,
                provider_request_id.as_deref(),
                Some(completion.http_status),
                finish_reason.as_deref(),
                "EMPTY_COMPLETION",
                "Cloud returned no text scene response",
            ))
        })?;
    let plan = parse_scene_spec_response(response).map_err(|error| {
        studio_error(cloud_edit_diagnostic(
            &model.model_id,
            &request_id,
            provider_request_id.as_deref(),
            Some(completion.http_status),
            finish_reason.as_deref(),
            if finish_reason.as_deref() == Some("length") {
                "TRUNCATED_COMPLETION"
            } else {
                "INVALID_JSON"
            },
            &error.to_string(),
        ))
    })?;
    let object =
        validate_single_object_ai_patch(&plan, target_name, &operation).map_err(|error| {
            studio_error(cloud_edit_diagnostic(
                &model.model_id,
                &request_id,
                provider_request_id.as_deref(),
                Some(completion.http_status),
                finish_reason.as_deref(),
                "PATCH_REJECTED",
                &error.to_string(),
            ))
        })?;
    Ok(serde_json::json!({
        "modelId":model.model_id,
        "operation":operation,
        "clientRequestId":request_id,
        "providerRequestId":provider_request_id,
        "httpStatus":completion.http_status,
        "finishReason":finish_reason,
        "editApplied":false,
        "usage":usage,
        "object":object
    }))
}

fn cloud_error_code(detail: &str) -> &'static str {
    if detail.contains("HTTP 401") {
        "HTTP_401"
    } else if detail.contains("HTTP 403") {
        "HTTP_403"
    } else if detail.contains("HTTP 429") {
        "HTTP_429"
    } else if detail.to_ascii_lowercase().contains("timed out") {
        "TIMEOUT_OUTCOME_UNKNOWN"
    } else if detail.to_ascii_lowercase().contains("network")
        || detail.to_ascii_lowercase().contains("unreachable")
    {
        "TRANSPORT_OUTCOME_UNKNOWN"
    } else {
        "CLOUD_REQUEST_FAILED"
    }
}

fn cloud_http_status(detail: &str) -> Option<u16> {
    let marker = detail.find("HTTP ")? + 5;
    detail.get(marker..marker + 3)?.parse().ok()
}

fn cloud_provider_request_id(detail: &str) -> Option<String> {
    let marker = "upstream request ID: ";
    let start = detail.find(marker)? + marker.len();
    let value = detail
        .get(start..)?
        .split(|ch: char| ch.is_whitespace() || ch == ';')
        .next()?;
    (!value.is_empty()).then(|| value.to_owned())
}

fn cloud_edit_diagnostic(
    model_id: &str,
    client_request_id: &str,
    provider_request_id: Option<&str>,
    http_status: Option<u16>,
    finish_reason: Option<&str>,
    error_code: &str,
    detail: &str,
) -> String {
    format!(
        "Cloud edit diagnostics: model={model_id}; client_request_id={client_request_id}; provider_request_id={}; gateway_status={}; finish_reason={}; error_code={error_code}; edit_applied=false; usage=unknown; detail={detail}",
        provider_request_id.unwrap_or("unavailable"),
        http_status.map(|status| status.to_string()).unwrap_or_else(|| "unknown".into()),
        finish_reason.unwrap_or("unknown"),
    )
}

fn normalize_scene_spec(mut value: Value) -> Result<Value, AppError> {
    // Qwen sometimes serializes a null root parent as the literal string
    // "null". This single exact alias is unambiguous; all other invalid
    // references remain validation errors.
    if let Some(objects) = value.get_mut("objects").and_then(Value::as_array_mut) {
        for object in objects {
            if object.get("parent").and_then(Value::as_str) == Some("null") {
                object["parent"] = Value::Null;
            }
        }
    }
    Ok(value)
}

fn validate_single_object_ai_patch(
    plan: &Value,
    target_name: &str,
    operation: &str,
) -> Result<Value, AppError> {
    let objects = plan
        .get("objects")
        .and_then(Value::as_array)
        .filter(|objects| objects.len() == 1)
        .ok_or_else(|| studio_error("AI edit must return exactly one object patch"))?;
    let object = &objects[0];
    let name = object
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match operation {
        "edit" if name != target_name => Err(studio_error(
            "AI edit changed the selected object's identity; no changes were applied",
        )),
        "add" if name == target_name => Err(studio_error("AI addition must use a new object name")),
        "edit" | "add" => Ok(object.clone()),
        _ => Err(studio_error("AI edit operation must be edit or add")),
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Studio3DAsset {
    id: String,
    project_id: String,
    kind: String,
    original_name: String,
    media_type: String,
    size_bytes: u64,
    content_sha256: String,
    width: Option<u32>,
    height: Option<u32>,
    created_at: String,
    preview_base64: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LoadedStudio3DAsset {
    #[serde(flatten)]
    asset: Studio3DAsset,
    data_base64: String,
    preview_base64: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Studio3DJob {
    id: String,
    project_id: String,
    generation_mode: String,
    inference_route: String,
    model_id: String,
    engine_id: String,
    input_asset_ids: Value,
    current_stage: String,
    status: String,
    input_sha256: String,
    output_asset_id: Option<String>,
    output_sha256: String,
    output_object_count: u32,
    error: String,
    recovery: Value,
    created_at: String,
    updated_at: String,
    started_at: Option<String>,
    completed_at: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Studio3DProject {
    id: String,
    name: String,
    created_at: String,
    updated_at: String,
    scene_count: i64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Studio3DSceneSummary {
    id: String,
    project_id: String,
    title: String,
    prompt: String,
    mode: String,
    engine: String,
    content_sha256: String,
    revision: i64,
    created_at: String,
    updated_at: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Studio3DScene {
    #[serde(flatten)]
    summary: Studio3DSceneSummary,
    scene_json: Value,
    glb_base64: String,
}

fn studio_error(message: impl Into<String>) -> AppError {
    AppError::internal(format!("3D Studio: {}", message.into()))
}

fn decode_glb(encoded: &str) -> Result<Vec<u8>, AppError> {
    if encoded.is_empty() || encoded.len() > ((MAX_GLB_BYTES + 2) / 3) * 4 + 8 {
        return Err(studio_error("GLB exceeds the 30 MB project asset limit."));
    }
    let bytes = STANDARD
        .decode(encoded)
        .map_err(|_| studio_error("GLB payload is not valid base64."))?;
    if bytes.len() < 12 || bytes.len() > MAX_GLB_BYTES {
        return Err(studio_error("GLB must be between 12 bytes and 30 MB."));
    }
    let magic = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
    let version = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
    let declared = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
    if magic != 0x4654_6c67 || version != 2 || declared != bytes.len() {
        return Err(studio_error("Asset is not a complete GLB 2.0 file."));
    }
    Ok(bytes)
}

fn validate_generated_mesh_glb(bytes: &[u8]) -> Result<usize, AppError> {
    let encoded = STANDARD.encode(bytes);
    decode_glb(&encoded)?;
    if bytes.len() < 20 {
        return Err(studio_error("generated GLB has no JSON chunk"));
    }
    let json_length = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
    let json_kind = u32::from_le_bytes(bytes[16..20].try_into().unwrap());
    let json_end = 20usize
        .checked_add(json_length)
        .filter(|end| *end <= bytes.len())
        .ok_or_else(|| studio_error("generated GLB JSON chunk is truncated"))?;
    if json_kind != 0x4e4f_534a {
        return Err(studio_error(
            "generated GLB does not start with a JSON chunk",
        ));
    }
    let document: Value = serde_json::from_slice(&bytes[20..json_end])
        .map_err(|_| studio_error("generated GLB JSON is malformed"))?;
    if document.pointer("/asset/version").and_then(Value::as_str) != Some("2.0") {
        return Err(studio_error("generated mesh is not glTF 2.0"));
    }
    let mesh_count = document
        .get("meshes")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or_default();
    let accessors = document
        .get("accessors")
        .and_then(Value::as_array)
        .ok_or_else(|| studio_error("generated GLB has no geometry accessors"))?;
    let views = document
        .get("bufferViews")
        .and_then(Value::as_array)
        .ok_or_else(|| studio_error("generated GLB has no geometry buffers"))?;
    let bin_header_end = json_end
        .checked_add(8)
        .filter(|end| *end <= bytes.len())
        .ok_or_else(|| studio_error("generated GLB has no binary chunk"))?;
    let bin_length = u32::from_le_bytes(bytes[json_end..json_end + 4].try_into().unwrap()) as usize;
    let bin_kind = u32::from_le_bytes(bytes[json_end + 4..bin_header_end].try_into().unwrap());
    let bin_end = bin_header_end
        .checked_add(bin_length)
        .filter(|end| *end == bytes.len())
        .ok_or_else(|| {
            studio_error("generated GLB binary chunk is truncated or has trailing bytes")
        })?;
    if bin_kind != 0x004e_4942 || mesh_count == 0 {
        return Err(studio_error("generated GLB has no binary mesh geometry"));
    }
    let byte_range =
        |accessor: &Value, expected_type: &str, component_type: u64| -> Result<(), AppError> {
            if accessor.get("type").and_then(Value::as_str) != Some(expected_type)
                || accessor.get("componentType").and_then(Value::as_u64) != Some(component_type)
                || accessor
                    .get("count")
                    .and_then(Value::as_u64)
                    .is_none_or(|count| count < 3)
            {
                return Err(studio_error(
                    "generated GLB has an invalid geometry accessor",
                ));
            }
            let view_index = accessor
                .get("bufferView")
                .and_then(Value::as_u64)
                .ok_or_else(|| studio_error("generated geometry accessor has no buffer view"))?
                as usize;
            let view = views.get(view_index).ok_or_else(|| {
                studio_error("generated geometry references a missing buffer view")
            })?;
            if view.get("buffer").and_then(Value::as_u64) != Some(0) {
                return Err(studio_error(
                    "generated GLB references an unsupported buffer",
                ));
            }
            let view_offset = view.get("byteOffset").and_then(Value::as_u64).unwrap_or(0) as usize;
            let view_length = view
                .get("byteLength")
                .and_then(Value::as_u64)
                .ok_or_else(|| studio_error("generated buffer view has no length"))?
                as usize;
            let accessor_offset = accessor
                .get("byteOffset")
                .and_then(Value::as_u64)
                .unwrap_or(0) as usize;
            let count = accessor.get("count").and_then(Value::as_u64).unwrap() as usize;
            let width = if expected_type == "VEC3" { 12 } else { 4 };
            let end = view_offset
                .checked_add(accessor_offset)
                .and_then(|start| start.checked_add(count.saturating_mul(width)))
                .ok_or_else(|| studio_error("generated geometry range overflows"))?;
            if accessor_offset.saturating_add(count.saturating_mul(width)) > view_length
                || end > bin_length
            {
                return Err(studio_error(
                    "generated geometry accessor exceeds its binary buffer",
                ));
            }
            Ok(())
        };
    let mut primitive_count = 0usize;
    for mesh in document["meshes"].as_array().into_iter().flatten() {
        for primitive in mesh
            .get("primitives")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if primitive
                .get("mode")
                .and_then(Value::as_u64)
                .is_some_and(|mode| mode != 4)
            {
                return Err(studio_error(
                    "generated mesh primitive is not a triangle list",
                ));
            }
            let position = primitive
                .pointer("/attributes/POSITION")
                .and_then(Value::as_u64)
                .ok_or_else(|| studio_error("generated mesh has no position attribute"))?
                as usize;
            let indices = primitive
                .get("indices")
                .and_then(Value::as_u64)
                .ok_or_else(|| studio_error("generated mesh has no index buffer"))?
                as usize;
            let position_accessor = accessors
                .get(position)
                .ok_or_else(|| studio_error("generated mesh position accessor is missing"))?;
            let index_accessor = accessors
                .get(indices)
                .ok_or_else(|| studio_error("generated mesh index accessor is missing"))?;
            byte_range(position_accessor, "VEC3", 5126)?;
            byte_range(index_accessor, "SCALAR", 5125)?;
            if index_accessor["count"]
                .as_u64()
                .is_none_or(|count| count % 3 != 0)
            {
                return Err(studio_error(
                    "generated mesh index count is not a triangle list",
                ));
            }
            primitive_count += 1;
        }
    }
    if primitive_count == 0 || bin_end != bytes.len() {
        return Err(studio_error(
            "generated GLB contains no complete triangle geometry",
        ));
    }
    Ok(mesh_count)
}

pub(crate) fn generated_glb_object_count(bytes: &[u8]) -> Result<usize, AppError> {
    validate_generated_mesh_glb(bytes)
}

fn summary_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Studio3DSceneSummary> {
    Ok(Studio3DSceneSummary {
        id: row.get(0)?,
        project_id: row.get(1)?,
        title: row.get(2)?,
        prompt: row.get(3)?,
        mode: row.get(4)?,
        engine: row.get(5)?,
        content_sha256: row.get(6)?,
        revision: row.get(7)?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
    })
}

fn checked_image_dimensions(width: u32, height: u32) -> Result<(u32, u32), AppError> {
    let pixels = u64::from(width).saturating_mul(u64::from(height));
    if width == 0 || height == 0 || width > 20_000 || height > 20_000 || pixels > MAX_IMAGE_PIXELS {
        return Err(studio_error(
            "image dimensions exceed the 40 megapixel decode budget",
        ));
    }
    Ok((width, height))
}

fn image_properties(
    extension: &str,
    bytes: &[u8],
) -> Result<(&'static str, u32, u32, String), AppError> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| studio_error("image format could not be detected"))?;
    let format = reader
        .format()
        .ok_or_else(|| studio_error("image format signature is missing"))?;
    let (expected, media_type) = match extension {
        "jpg" | "jpeg" => (image::ImageFormat::Jpeg, "image/jpeg"),
        "png" => (image::ImageFormat::Png, "image/png"),
        "webp" => (image::ImageFormat::WebP, "image/webp"),
        "bmp" => (image::ImageFormat::Bmp, "image/bmp"),
        "tif" | "tiff" => (image::ImageFormat::Tiff, "image/tiff"),
        _ => return Err(studio_error("unsupported image extension")),
    };
    if format != expected {
        return Err(studio_error(
            "image extension does not match the detected file format",
        ));
    }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(20_000);
    limits.max_image_height = Some(20_000);
    limits.max_alloc = Some(160 * 1024 * 1024);
    reader.limits(limits);
    let decoded = reader
        .decode()
        .map_err(|_| studio_error("image is malformed or exceeds the decode limits"))?;
    let (width, height) = checked_image_dimensions(decoded.width(), decoded.height())?;
    let preview = decoded.thumbnail(320, 320);
    let mut png = std::io::Cursor::new(Vec::new());
    preview
        .write_to(&mut png, image::ImageFormat::Png)
        .map_err(|_| studio_error("image preview could not be encoded"))?;
    Ok((media_type, width, height, STANDARD.encode(png.into_inner())))
}

fn validate_gltf_texture(mime: &str, bytes: &[u8]) -> Result<(), AppError> {
    let expected = match mime {
        "image/png" => image::ImageFormat::Png,
        "image/jpeg" => image::ImageFormat::Jpeg,
        "image/webp" => image::ImageFormat::WebP,
        _ => return Err(studio_error("glTF texture MIME type is unsupported")),
    };
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| studio_error("glTF texture format could not be detected"))?;
    if reader.format() != Some(expected) {
        return Err(studio_error(
            "glTF texture MIME type does not match its data",
        ));
    }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(20_000);
    limits.max_image_height = Some(20_000);
    limits.max_alloc = Some(160 * 1024 * 1024);
    reader.limits(limits);
    let image = reader
        .decode()
        .map_err(|_| studio_error("glTF texture is malformed or exceeds the decode limits"))?;
    checked_image_dimensions(image.width(), image.height())?;
    Ok(())
}

fn read_studio3d_file(path: &str, maximum: usize) -> Result<(PathBuf, String, Vec<u8>), AppError> {
    let path = PathBuf::from(path);
    let metadata = fs::symlink_metadata(&path)?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() == 0
        || metadata.len() > maximum as u64
    {
        return Err(studio_error(
            "selected asset must be a regular local file within its size limit",
        ));
    }
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| {
            !value.is_empty()
                && value.chars().count() <= 255
                && !value.chars().any(char::is_control)
        })
        .ok_or_else(|| studio_error("selected filename is invalid"))?
        .to_owned();
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    fs::File::open(&path)?
        .take(maximum as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 != metadata.len() || bytes.len() > maximum {
        return Err(studio_error(
            "selected asset changed during import or exceeded its size limit",
        ));
    }
    Ok((path, name, bytes))
}

fn asset_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Studio3DAsset> {
    Ok(Studio3DAsset {
        id: row.get(0)?,
        project_id: row.get(1)?,
        kind: row.get(2)?,
        original_name: row.get(3)?,
        media_type: row.get(4)?,
        size_bytes: row.get::<_, i64>(5)?.max(0) as u64,
        content_sha256: row.get(6)?,
        width: row.get(7)?,
        height: row.get(8)?,
        created_at: row.get(9)?,
        preview_base64: None,
    })
}

fn asset_with_preview_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Studio3DAsset> {
    let mut asset = asset_from_row(row)?;
    asset.preview_base64 = row
        .get::<_, Option<Vec<u8>>>(10)?
        .map(|bytes| STANDARD.encode(bytes));
    Ok(asset)
}

fn store_studio3d_asset(
    project_id: &str,
    kind: &str,
    name: &str,
    media_type: &str,
    width: Option<u32>,
    height: Option<u32>,
    bytes: Vec<u8>,
    preview_base64: Option<String>,
    state: &State<'_, AppState>,
) -> Result<Studio3DAsset, AppError> {
    let digest = format!("{:x}", Sha256::digest(&bytes));
    let db = state
        .database
        .lock()
        .map_err(|_| studio_error("database lock is unavailable"))?;
    let connection = db.connection();
    let exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM studio3d_projects WHERE id=?1)",
        params![project_id],
        |row| row.get(0),
    )?;
    if !exists {
        return Err(studio_error("project not found"));
    }
    if let Some(existing) = connection.query_row(
        "SELECT id,project_id,kind,original_name,media_type,size_bytes,content_sha256,width,height,created_at FROM studio3d_assets WHERE project_id=?1 AND content_sha256=?2",
        params![project_id,digest], asset_from_row,
    ).optional()? { return Ok(existing); }
    let (count, total): (i64, i64) = connection.query_row(
        "SELECT COUNT(*),COALESCE(SUM(size_bytes),0) FROM studio3d_assets WHERE project_id=?1",
        params![project_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if count >= 100 || total.saturating_add(bytes.len() as i64) > 500 * 1024 * 1024 {
        return Err(studio_error(
            "project asset limit reached (100 assets or 500 MB)",
        ));
    }
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    let tx = connection.unchecked_transaction()?;
    let preview = preview_base64
        .as_deref()
        .map(|encoded| {
            STANDARD
                .decode(encoded)
                .map_err(|_| studio_error("generated preview is invalid"))
        })
        .transpose()?;
    tx.execute(
        "INSERT INTO studio3d_assets(id,project_id,kind,original_name,media_type,size_bytes,content_sha256,width,height,content_blob,preview_blob,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
        params![id,project_id,kind,name,media_type,bytes.len() as i64,digest,width,height,bytes,preview,now],
    )?;
    tx.execute(
        "UPDATE studio3d_projects SET updated_at=?1 WHERE id=?2",
        params![now, project_id],
    )?;
    tx.commit()?;
    Ok(Studio3DAsset {
        id,
        project_id: project_id.to_owned(),
        kind: kind.to_owned(),
        original_name: name.to_owned(),
        media_type: media_type.to_owned(),
        size_bytes: bytes.len() as u64,
        content_sha256: digest,
        width,
        height,
        created_at: now,
        preview_base64,
    })
}

#[tauri::command]
pub(crate) fn import_studio3d_image(
    project_id: String,
    path: String,
    state: State<'_, AppState>,
) -> Result<Studio3DAsset, AppError> {
    Uuid::parse_str(&project_id).map_err(|_| studio_error("invalid project id"))?;
    let (path, name, bytes) = read_studio3d_file(&path, MAX_IMAGE_BYTES)?;
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let (media_type, width, height, preview) = image_properties(&extension, &bytes)?;
    store_studio3d_asset(
        &project_id,
        "image",
        &name,
        media_type,
        Some(width),
        Some(height),
        bytes,
        Some(preview),
        &state,
    )
}

#[tauri::command]
pub(crate) fn import_studio3d_glb(
    project_id: String,
    path: String,
    state: State<'_, AppState>,
) -> Result<Studio3DAsset, AppError> {
    Uuid::parse_str(&project_id).map_err(|_| studio_error("invalid project id"))?;
    let (_path, name, bytes) = read_studio3d_file(&path, MAX_GLB_BYTES)?;
    decode_glb(&STANDARD.encode(&bytes))?;
    store_studio3d_asset(
        &project_id,
        "mesh",
        &name,
        "model/gltf-binary",
        None,
        None,
        bytes,
        None,
        &state,
    )
}

#[tauri::command]
pub(crate) fn store_studio3d_generated_glb(
    project_id: String,
    filename: String,
    data_base64: String,
    state: State<'_, AppState>,
) -> Result<Studio3DAsset, AppError> {
    store_studio3d_generated_glb_inner(&project_id, &filename, &data_base64, &state)
}

fn store_studio3d_generated_glb_inner(
    project_id: &str,
    filename: &str,
    data_base64: &str,
    state: &State<'_, AppState>,
) -> Result<Studio3DAsset, AppError> {
    Uuid::parse_str(&project_id).map_err(|_| studio_error("invalid project id"))?;
    if filename.is_empty()
        || filename.len() > 180
        || filename.chars().any(|ch| matches!(ch, '/' | '\\' | ':'))
        || data_base64.len() > (MAX_GLB_BYTES * 4 / 3 + 8)
    {
        return Err(studio_error("generated GLB filename or payload is invalid"));
    }
    let bytes = STANDARD
        .decode(data_base64)
        .map_err(|_| studio_error("generated GLB is not valid base64"))?;
    validate_generated_mesh_glb(&bytes)?;
    store_studio3d_asset(
        project_id,
        "mesh",
        filename,
        "model/gltf-binary",
        None,
        None,
        bytes,
        None,
        state,
    )
}

fn validate_obj(bytes: &[u8]) -> Result<(), AppError> {
    let text = std::str::from_utf8(bytes).map_err(|_| studio_error("OBJ must be UTF-8 text"))?;
    let mut vertices = 0usize;
    let mut faces = 0usize;
    for line in text.lines().take(2_000_000) {
        let mut parts = line.split_whitespace();
        match parts.next() {
            Some("v") => {
                let values: Vec<_> = parts.take(4).collect();
                if values.len() < 3
                    || values[..3]
                        .iter()
                        .any(|v| v.parse::<f32>().map_or(true, |n| !n.is_finite()))
                {
                    return Err(studio_error("OBJ contains malformed vertex coordinates"));
                }
                vertices += 1;
            }
            Some("f") => {
                let refs: Vec<_> = parts.collect();
                if refs.len() < 3
                    || refs.iter().any(|r| {
                        r.split('/')
                            .next()
                            .and_then(|n| n.parse::<i64>().ok())
                            .filter(|n| *n != 0)
                            .is_none()
                    })
                {
                    return Err(studio_error("OBJ contains malformed face indices"));
                }
                faces += 1;
            }
            _ => {}
        }
    }
    if vertices == 0 || faces == 0 {
        return Err(studio_error("OBJ has no usable triangle geometry"));
    }
    Ok(())
}

fn validate_stl(bytes: &[u8]) -> Result<(), AppError> {
    let binary_facets = if bytes.len() >= 84 {
        Some(u32::from_le_bytes(bytes[80..84].try_into().unwrap()) as usize)
    } else {
        None
    };
    let binary_valid = binary_facets.is_some_and(|n| {
        n > 0 && n <= 2_000_000 && 84usize.saturating_add(n.saturating_mul(50)) == bytes.len()
    });
    if binary_valid {
        return Ok(());
    }
    let text = std::str::from_utf8(bytes)
        .map_err(|_| studio_error("STL is neither valid binary nor ASCII data"))?;
    let lower = text.to_ascii_lowercase();
    if lower.trim_start().starts_with("solid")
        && lower.contains("facet normal")
        && lower.contains("vertex")
        && lower.contains("endsolid")
    {
        Ok(())
    } else {
        Err(studio_error(
            "STL geometry is malformed or exceeds the facet limit",
        ))
    }
}

#[tauri::command]
pub(crate) fn import_studio3d_mesh(
    project_id: String,
    path: String,
    state: State<'_, AppState>,
) -> Result<Studio3DAsset, AppError> {
    Uuid::parse_str(&project_id).map_err(|_| studio_error("invalid project id"))?;
    let (path, name, bytes) = read_studio3d_file(&path, MAX_MESH_BYTES)?;
    let extension = path
        .extension()
        .and_then(|v| v.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let media_type = match extension.as_str() {
        "obj" => {
            validate_obj(&bytes)?;
            "model/obj"
        }
        "stl" => {
            validate_stl(&bytes)?;
            "model/stl"
        }
        _ => return Err(studio_error(
            "supported mesh imports are GLB, OBJ and STL; glTF requires bundled external resources",
        )),
    };
    store_studio3d_asset(
        &project_id,
        "mesh",
        &name,
        media_type,
        None,
        None,
        bytes,
        None,
        &state,
    )
}

fn bundle_gltf_resources(
    path: &str,
    approved_directory: &str,
) -> Result<(String, Vec<u8>), AppError> {
    let source_path = PathBuf::from(path);
    let scope_path = PathBuf::from(approved_directory);
    let source_meta = fs::symlink_metadata(&source_path)?;
    let scope_meta = fs::symlink_metadata(&scope_path)?;
    if !source_meta.is_file()
        || source_meta.file_type().is_symlink()
        || !scope_meta.is_dir()
        || scope_meta.file_type().is_symlink()
    {
        return Err(studio_error(
            "choose a regular glTF file and an approved local resource directory",
        ));
    }
    let source = fs::canonicalize(&source_path)?;
    let scope = fs::canonicalize(&scope_path)?;
    if !source.starts_with(&scope)
        || source
            .extension()
            .and_then(|v| v.to_str())
            .is_none_or(|v| !v.eq_ignore_ascii_case("gltf"))
    {
        return Err(studio_error(
            "glTF file must be inside the approved resource directory",
        ));
    }
    let name = source
        .file_name()
        .and_then(|v| v.to_str())
        .ok_or_else(|| studio_error("glTF filename is invalid"))?
        .to_owned();
    let metadata = fs::metadata(&source)?;
    if metadata.len() == 0 || metadata.len() > MAX_GLTF_BUNDLE_BYTES as u64 {
        return Err(studio_error("glTF exceeds the 30 MB bundle limit"));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    fs::File::open(&source)?
        .take(MAX_GLTF_BUNDLE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 != metadata.len() {
        return Err(studio_error("glTF changed during import"));
    }
    let mut document: Value =
        serde_json::from_slice(&bytes).map_err(|_| studio_error("glTF JSON is malformed"))?;
    if document
        .get("asset")
        .and_then(|v| v.get("version"))
        .and_then(Value::as_str)
        != Some("2.0")
    {
        return Err(studio_error("only glTF 2.0 documents can be imported"));
    }
    if let Some(required) = document.get("extensionsRequired") {
        let required = required
            .as_array()
            .ok_or_else(|| studio_error("glTF extensionsRequired must be an array"))?;
        const SUPPORTED_REQUIRED_EXTENSIONS: &[&str] = &[
            "KHR_mesh_quantization",
            "KHR_texture_transform",
            "KHR_materials_unlit",
            "KHR_materials_pbrSpecularGlossiness",
            "KHR_materials_clearcoat",
            "KHR_materials_ior",
            "KHR_materials_specular",
            "KHR_materials_transmission",
            "KHR_materials_volume",
            "KHR_materials_iridescence",
            "KHR_materials_sheen",
            "KHR_materials_emissive_strength",
            "KHR_lights_punctual",
            "EXT_mesh_gpu_instancing",
        ];
        for extension in required {
            let name = extension
                .as_str()
                .ok_or_else(|| studio_error("glTF required extension name is invalid"))?;
            if !SUPPORTED_REQUIRED_EXTENSIONS.contains(&name) {
                return Err(studio_error(format!(
                    "required glTF extension is unsupported: {name}"
                )));
            }
        }
    }
    let source_parent = source
        .parent()
        .ok_or_else(|| studio_error("glTF source has no parent directory"))?;
    let mut seen = std::collections::HashSet::new();
    let mut total = bytes.len();
    for collection in ["buffers", "images"] {
        let Some(items) = document.get_mut(collection).and_then(Value::as_array_mut) else {
            continue;
        };
        for item in items {
            let Some(uri) = item.get("uri").and_then(Value::as_str).map(str::to_owned) else {
                continue;
            };
            if uri.starts_with("data:") {
                let (mime, payload) = uri
                    .strip_prefix("data:")
                    .and_then(|v| v.split_once(","))
                    .ok_or_else(|| studio_error("malformed glTF data URI"))?;
                if !mime.ends_with(";base64") {
                    return Err(studio_error("glTF data URI must use base64 encoding"));
                }
                let actual = mime.trim_end_matches(";base64");
                let is_buffer = collection == "buffers";
                if (is_buffer && actual != "application/octet-stream")
                    || (!is_buffer && !matches!(actual, "image/png" | "image/jpeg" | "image/webp"))
                {
                    return Err(studio_error("glTF embeds an unsupported resource format"));
                }
                let decoded = STANDARD
                    .decode(payload)
                    .map_err(|_| studio_error("glTF data URI has invalid base64"))?;
                if !is_buffer {
                    validate_gltf_texture(actual, &decoded)?;
                }
                if collection == "buffers"
                    && item
                        .get("byteLength")
                        .and_then(Value::as_u64)
                        .is_none_or(|n| n as usize > decoded.len())
                {
                    return Err(studio_error("glTF embedded buffer byteLength is invalid"));
                }
                total = total.saturating_add(decoded.len());
                if total > MAX_GLTF_BUNDLE_BYTES {
                    return Err(studio_error("glTF resources exceed the 30 MB bundle limit"));
                }
                continue;
            }
            if uri.is_empty()
                || uri.starts_with('/')
                || uri.starts_with('\\')
                || uri
                    .chars()
                    .any(|ch| matches!(ch, '\\' | ':' | '?' | '#' | '%'))
                || uri
                    .split('/')
                    .any(|part| part.is_empty() || part == "." || part == "..")
            {
                return Err(studio_error("glTF resource URI is unsafe or ambiguous"));
            }
            if !seen.insert(uri.clone()) {
                return Err(studio_error("glTF contains duplicate resource paths"));
            }
            let candidate = source_parent.join(&uri);
            let candidate_meta = fs::symlink_metadata(&candidate)
                .map_err(|_| studio_error(format!("missing glTF resource: {uri}")))?;
            if candidate_meta.file_type().is_symlink() || !candidate_meta.is_file() {
                return Err(studio_error(
                    "glTF resources must be regular files without symlinks",
                ));
            }
            let resolved = fs::canonicalize(&candidate)?;
            if !resolved.starts_with(&scope) {
                return Err(studio_error(
                    "glTF resource resolves outside the approved directory",
                ));
            }
            let length = fs::metadata(&resolved)?.len();
            if length == 0
                || length > MAX_GLTF_BUNDLE_BYTES as u64
                || total.saturating_add(length as usize) > MAX_GLTF_BUNDLE_BYTES
            {
                return Err(studio_error("glTF bundle exceeds the 30 MB resource limit"));
            }
            let mut resource = Vec::with_capacity(length as usize);
            fs::File::open(&resolved)?
                .take(MAX_GLTF_BUNDLE_BYTES as u64 + 1)
                .read_to_end(&mut resource)?;
            if resource.len() as u64 != length {
                return Err(studio_error("glTF resource changed during import"));
            }
            let mime = if collection == "buffers" {
                if item
                    .get("byteLength")
                    .and_then(Value::as_u64)
                    .is_none_or(|n| n as usize > resource.len())
                {
                    return Err(studio_error("glTF buffer byteLength is invalid"));
                }
                "application/octet-stream"
            } else if resource.starts_with(b"\x89PNG\r\n\x1a\n") {
                "image/png"
            } else if resource.starts_with(b"\xff\xd8\xff") {
                "image/jpeg"
            } else if resource.len() >= 12
                && &resource[0..4] == b"RIFF"
                && &resource[8..12] == b"WEBP"
            {
                "image/webp"
            } else {
                return Err(studio_error(
                    "external glTF textures must be valid PNG, JPEG or WebP files",
                ));
            };
            if collection == "images" {
                validate_gltf_texture(mime, &resource)?;
            }
            total += resource.len();
            *item
                .get_mut("uri")
                .ok_or_else(|| studio_error("invalid glTF resource entry"))? =
                Value::String(format!("data:{mime};base64,{}", STANDARD.encode(resource)));
        }
    }
    let bundled = serde_json::to_vec(&document)
        .map_err(|_| studio_error("glTF resource bundle could not be encoded"))?;
    if bundled.len() > MAX_GLTF_BUNDLE_BYTES {
        return Err(studio_error(
            "bundled glTF exceeds the 30 MB project asset limit",
        ));
    }
    Ok((name, bundled))
}

#[tauri::command]
pub(crate) fn import_studio3d_gltf(
    project_id: String,
    path: String,
    approved_directory: String,
    state: State<'_, AppState>,
) -> Result<Studio3DAsset, AppError> {
    Uuid::parse_str(&project_id).map_err(|_| studio_error("invalid project id"))?;
    let (name, bytes) = bundle_gltf_resources(&path, &approved_directory)?;
    store_studio3d_asset(
        &project_id,
        "mesh",
        &name,
        "model/gltf+json",
        None,
        None,
        bytes,
        None,
        &state,
    )
}

#[tauri::command]
pub(crate) fn create_studio3d_job(
    project_id: String,
    generation_mode: String,
    inference_route: String,
    model_id: String,
    engine_id: String,
    input_asset_ids: Vec<String>,
    input: String,
    state: State<'_, AppState>,
) -> Result<Studio3DJob, AppError> {
    Uuid::parse_str(&project_id).map_err(|_| studio_error("invalid project id"))?;
    if !matches!(
        generation_mode.as_str(),
        "text-to-3d" | "image-to-3d" | "scene-edit" | "architecture"
    ) || !matches!(inference_route.as_str(), "local" | "cloud" | "auto")
        || model_id.len() > 200
        || engine_id.len() > 200
        || input.len() > 64 * 1024
        || input_asset_ids.len() > 64
    {
        return Err(studio_error(
            "3D job parameters are invalid or exceed their limits",
        ));
    }
    let mut deduplicated = std::collections::HashSet::new();
    for id in &input_asset_ids {
        Uuid::parse_str(id).map_err(|_| studio_error("invalid input asset id"))?;
        if !deduplicated.insert(id) {
            return Err(studio_error("duplicate input asset id"));
        }
    }
    let input_assets = serde_json::to_string(&input_asset_ids)
        .map_err(|_| studio_error("job inputs could not be encoded"))?;
    let digest = format!("{:x}", Sha256::digest(format!("{generation_mode}\n{inference_route}\n{model_id}\n{engine_id}\n{input}\n{input_assets}").as_bytes()));
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    let db = state
        .database
        .lock()
        .map_err(|_| studio_error("database lock is unavailable"))?;
    let tx = db.connection().unchecked_transaction()?;
    let project_exists: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM studio3d_projects WHERE id=?1)",
        params![project_id],
        |row| row.get(0),
    )?;
    if !project_exists {
        return Err(studio_error("3D project not found"));
    }
    for asset_id in &input_asset_ids {
        let belongs: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM studio3d_assets WHERE id=?1 AND project_id=?2)",
            params![asset_id, project_id],
            |row| row.get(0),
        )?;
        if !belongs {
            return Err(studio_error(
                "an input asset does not belong to this project",
            ));
        }
    }
    let request_json = serde_json::json!({"prompt":input,"generationMode":generation_mode,"inputAssetIds":input_asset_ids}).to_string();
    tx.execute("INSERT INTO studio3d_jobs(id,project_id,generation_mode,inference_route,model_id,engine_id,input_asset_ids_json,current_stage,status,input_sha256,recovery_json,request_json,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,'queued','queued',?8,'{}',?9,?10,?10)", params![id,project_id,generation_mode,inference_route,model_id,engine_id,input_assets,digest,request_json,now])?;
    let result = studio3d_job_by_id(&tx, &id)?;
    tx.commit()?;
    Ok(result)
}

fn studio3d_job_by_id(
    connection: &rusqlite::Connection,
    id: &str,
) -> Result<Studio3DJob, AppError> {
    connection.query_row("SELECT id,project_id,generation_mode,inference_route,model_id,engine_id,input_asset_ids_json,current_stage,status,input_sha256,output_asset_id,output_sha256,error,recovery_json,created_at,updated_at,started_at,completed_at,output_object_count FROM studio3d_jobs WHERE id=?1", params![id], |row| {
        let input: String = row.get(6)?;
        let recovery: String = row.get(13)?;
        Ok(Studio3DJob { id:row.get(0)?, project_id:row.get(1)?, generation_mode:row.get(2)?, inference_route:row.get(3)?, model_id:row.get(4)?, engine_id:row.get(5)?, input_asset_ids:serde_json::from_str(&input).unwrap_or(Value::Null), current_stage:row.get(7)?, status:row.get(8)?, input_sha256:row.get(9)?, output_asset_id:row.get(10)?, output_sha256:row.get(11)?, error:row.get(12)?, recovery:serde_json::from_str(&recovery).unwrap_or(Value::Null), created_at:row.get(14)?, updated_at:row.get(15)?, started_at:row.get(16)?, completed_at:row.get(17)?, output_object_count:row.get::<_,i64>(18)?.max(0) as u32 })
    }).map_err(AppError::from)
}

#[tauri::command]
pub(crate) fn list_studio3d_jobs(
    project_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<Studio3DJob>, AppError> {
    Uuid::parse_str(&project_id).map_err(|_| studio_error("invalid project id"))?;
    let db = state
        .database
        .lock()
        .map_err(|_| studio_error("database lock is unavailable"))?;
    let mut statement = db.connection().prepare(
        "SELECT id FROM studio3d_jobs WHERE project_id=?1 ORDER BY created_at DESC,id LIMIT 200",
    )?;
    let ids = statement
        .query_map(params![project_id], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    ids.iter()
        .map(|id| studio3d_job_by_id(db.connection(), id))
        .collect()
}

#[tauri::command]
pub(crate) fn transition_studio3d_job(
    app: AppHandle,
    job_id: String,
    status: String,
    current_stage: String,
    output_asset_id: Option<String>,
    output_sha256: String,
    error: String,
    recovery: Value,
    state: State<'_, AppState>,
) -> Result<Studio3DJob, AppError> {
    Uuid::parse_str(&job_id).map_err(|_| studio_error("invalid job id"))?;
    if !matches!(
        status.as_str(),
        "queued"
            | "running"
            | "completed"
            | "failed"
            | "interrupted"
            | "cancelled"
            | "awaiting_approval"
            | "retry_required"
    ) || current_stage.is_empty()
        || current_stage.len() > 80
        || error.len() > 4000
        || recovery.to_string().len() > 16 * 1024
        || (!output_sha256.is_empty()
            && (output_sha256.len() != 64 || !output_sha256.bytes().all(|b| b.is_ascii_hexdigit())))
    {
        return Err(studio_error(
            "3D job update is invalid or exceeds its limits",
        ));
    }
    if let Some(asset_id) = &output_asset_id {
        Uuid::parse_str(asset_id).map_err(|_| studio_error("invalid output asset id"))?;
    }
    let now = Utc::now().to_rfc3339();
    let db = state
        .database
        .lock()
        .map_err(|_| studio_error("database lock is unavailable"))?;
    let tx = db.connection().unchecked_transaction()?;
    let (project_id, route, previous, previous_recovery, previous_stage, previous_asset, previous_sha, previous_error): (String,String,String,String,String,Option<String>,String,String) = tx.query_row("SELECT project_id,inference_route,status,recovery_json,current_stage,output_asset_id,output_sha256,error FROM studio3d_jobs WHERE id=?1", params![job_id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get(6)?,row.get(7)?))).optional()?.ok_or_else(|| studio_error("3D job not found"))?;
    if previous == status
        && matches!(
            previous.as_str(),
            "completed" | "failed" | "interrupted" | "cancelled"
        )
    {
        if previous_stage == current_stage
            && previous_asset == output_asset_id
            && (output_sha256.is_empty() || previous_sha == output_sha256)
            && previous_error == error
            && previous_recovery == recovery.to_string()
        {
            return studio3d_job_by_id(&tx, &job_id);
        }
        return Err(studio_error("terminal 3D job states are immutable"));
    }
    let safe_to_resume = serde_json::from_str::<Value>(&previous_recovery)
        .ok()
        .and_then(|value| value.get("safeToResume").and_then(Value::as_bool))
        .unwrap_or(false);
    let allowed = match previous.as_str() {
        "queued" => matches!(
            status.as_str(),
            "running" | "cancelled" | "failed" | "interrupted" | "awaiting_approval"
        ),
        "awaiting_approval" => matches!(status.as_str(), "queued" | "cancelled"),
        "running" => matches!(
            status.as_str(),
            "completed" | "failed" | "interrupted" | "cancelled" | "retry_required"
        ),
        "interrupted" => {
            status == "cancelled" || (status == "queued" && route == "local" && safe_to_resume)
        }
        "retry_required" => {
            status == "cancelled"
                || (status == "queued"
                    && ((route == "local" && safe_to_resume)
                        || (route != "local"
                            && serde_json::from_str::<Value>(&previous_recovery)
                                .ok()
                                .and_then(|value| {
                                    value.get("retryAuthorized").and_then(Value::as_bool)
                                })
                                .unwrap_or(false))))
        }
        _ => false,
    };
    if !allowed && status != previous {
        return Err(studio_error("invalid 3D job state transition"));
    }
    if status == "completed" {
        let asset_id = output_asset_id
            .as_deref()
            .ok_or_else(|| studio_error("completed jobs must reference their output asset"))?;
        if output_sha256.is_empty() {
            return Err(studio_error(
                "completed jobs must include an output checksum",
            ));
        }
        let actual_digest: Option<String> = tx
            .query_row(
                "SELECT content_sha256 FROM studio3d_assets WHERE id=?1 AND project_id=?2",
                params![asset_id, project_id],
                |row| row.get(0),
            )
            .optional()?;
        if actual_digest.as_deref() != Some(output_sha256.as_str()) {
            return Err(studio_error(
                "job output checksum does not match its project asset",
            ));
        }
    }
    let mut merged_recovery =
        serde_json::from_str::<Value>(&previous_recovery).unwrap_or_else(|_| serde_json::json!({}));
    if let (Some(current), Some(incoming)) = (merged_recovery.as_object_mut(), recovery.as_object())
    {
        current.extend(
            incoming
                .iter()
                .map(|(key, value)| (key.clone(), value.clone())),
        );
    }
    let recovery_string = merged_recovery.to_string();
    if recovery_string.len() > 16 * 1024 {
        return Err(studio_error("merged 3D recovery data exceeds its limit"));
    }
    let mut output_object_count = 0usize;
    if status == "completed" {
        let asset_id = output_asset_id
            .as_deref()
            .ok_or_else(|| studio_error("completed jobs must reference their output asset"))?;
        let (actual_digest, bytes): (String, Vec<u8>) = tx.query_row(
            "SELECT content_sha256,content_blob FROM studio3d_assets WHERE id=?1 AND project_id=?2",
            params![asset_id, project_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if actual_digest != output_sha256
            || format!("{:x}", Sha256::digest(&bytes)) != output_sha256
        {
            return Err(studio_error(
                "job output checksum does not match its project asset",
            ));
        }
        output_object_count = generated_glb_object_count(&bytes)?;
        if output_object_count == 0 || output_object_count > 1024 {
            return Err(studio_error(
                "completed jobs must reference a GLB with a bounded set of mesh objects",
            ));
        }
    }
    tx.execute("UPDATE studio3d_jobs SET current_stage=?1,status=?2,output_asset_id=COALESCE(?3,output_asset_id),output_sha256=CASE WHEN ?4='' THEN output_sha256 ELSE ?4 END,error=?5,recovery_json=CASE WHEN ?2='completed' THEN json_set(?6,'$.objectCount',?9) ELSE ?6 END,updated_at=?7,started_at=CASE WHEN ?2='running' AND started_at IS NULL THEN ?7 ELSE started_at END,completed_at=CASE WHEN ?2 IN ('completed','failed','interrupted','cancelled') THEN ?7 ELSE completed_at END,output_object_count=CASE WHEN ?2='completed' THEN ?9 ELSE output_object_count END WHERE id=?8", params![current_stage,status,output_asset_id,output_sha256,error,recovery_string,now,job_id,output_object_count as i64])?;
    let updated = studio3d_job_by_id(&tx, &job_id)?;
    tx.commit()?;
    let _ = app.emit("studio3d://job-progress", &updated);
    Ok(updated)
}

/// Rust owns dispatch, bounded scheduling, polling, validation and project publication for
/// local generation jobs. The Python worker is retained as the geometry adapter.
#[tauri::command]
pub(crate) async fn execute_studio3d_local_job(
    app: AppHandle,
    job_id: String,
    state: State<'_, AppState>,
) -> Result<Studio3DAsset, AppError> {
    Uuid::parse_str(&job_id).map_err(|_| studio_error("invalid job id"))?;
    let (project_id, route, mode, job_status, request): (String, String, String, String, String) = {
        let db = state
            .database
            .lock()
            .map_err(|_| studio_error("database lock is unavailable"))?;
        db.connection().query_row(
            "SELECT project_id,inference_route,generation_mode,status,request_json FROM studio3d_jobs WHERE id=?1",
            params![job_id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?)),
        ).optional()?.ok_or_else(|| studio_error("3D job not found"))?
    };
    if job_status != "queued" {
        return Err(studio_error(
            "only queued 3D jobs may be dispatched; uncertain or started jobs are never replayed",
        ));
    }
    if route != "local" || !matches!(mode.as_str(), "text-to-3d" | "architecture") {
        return Err(studio_error(
            "the local procedural adapter only accepts local text or architecture jobs",
        ));
    }
    let request: Value = serde_json::from_str(&request)
        .map_err(|_| studio_error("stored job request is invalid"))?;
    let prompt = request
        .get("prompt")
        .and_then(Value::as_str)
        .ok_or_else(|| studio_error("stored job prompt is missing"))?;
    if prompt.trim().len() < 5 || prompt.len() > 8_000 {
        return Err(studio_error(
            "local generation prompt must contain 5 to 8,000 characters",
        ));
    }
    let token = CancellationToken::new();
    {
        let mut active = studio3d_cancellations()
            .lock()
            .map_err(|_| studio_error("job cancellation registry is unavailable"))?;
        if active.contains_key(&job_id) {
            return Err(studio_error("this 3D job is already executing"));
        }
        active.insert(job_id.clone(), token.clone());
    }
    let result = execute_studio3d_local_job_inner(
        &app,
        &job_id,
        &project_id,
        mode == "architecture",
        prompt,
        token.clone(),
        &state,
    )
    .await;
    if let Ok(mut active) = studio3d_cancellations().lock() {
        active.remove(&job_id);
    }
    match result {
        Ok(asset) => Ok(asset),
        Err(error) => {
            let (status, stage) = if token.is_cancelled() {
                ("cancelled", "cancelled")
            } else {
                ("failed", "failed")
            };
            let _ = transition_studio3d_job(
                app,
                job_id,
                status.into(),
                stage.into(),
                None,
                String::new(),
                error.to_string(),
                serde_json::json!({"safeToResume":false}),
                state,
            );
            Err(error)
        }
    }
}

async fn execute_studio3d_local_job_inner(
    app: &AppHandle,
    job_id: &str,
    project_id: &str,
    architecture: bool,
    prompt: &str,
    token: CancellationToken,
    state: &State<'_, AppState>,
) -> Result<Studio3DAsset, AppError> {
    let permit = tokio::select! {
        _ = token.cancelled() => return Err(studio_error("3D job cancelled while queued")),
        result = tokio::time::timeout(STUDIO3D_LOCAL_JOB_TIMEOUT, studio3d_scheduler().acquire()) =>
            result.map_err(|_| studio_error("timed out waiting for a local generation slot"))?
                .map_err(|_| studio_error("local generation scheduler is shutting down"))?,
    };
    let _permit = permit;
    let started = std::time::Instant::now();
    let queued_state = app.state::<AppState>();
    transition_studio3d_job(
        app.clone(),
        job_id.to_owned(),
        "running".into(),
        "scheduled".into(),
        None,
        String::new(),
        String::new(),
        serde_json::json!({"safeToResume":false,"scheduler":"native","concurrencyLimit":STUDIO3D_MAX_CONCURRENT_JOBS}),
        queued_state,
    )?;
    tokio::select! {
        _ = token.cancelled() => return Err(studio_error("3D job cancelled before dispatch")),
        result = tokio::time::timeout(STUDIO3D_LOCAL_JOB_TIMEOUT, crate::cloud_bridge::local_gpu_start(app.clone())) => {
            result.map_err(|_| studio_error("local geometry worker startup timed out"))??;
        }
    }
    transition_studio3d_job(
        app.clone(),
        job_id.to_owned(),
        "running".into(),
        "local-model-inference".into(),
        None,
        String::new(),
        String::new(),
        serde_json::json!({"safeToResume":false,"modelProvider":"local-llama.cpp"}),
        app.state::<AppState>(),
    )?;
    let key = Uuid::new_v4().to_string();
    let mode = if architecture {
        "architecture"
    } else {
        "object"
    };
    let (model_id, scene_spec_text) = tokio::select! {
        _ = token.cancelled() => return Err(studio_error("3D job cancelled during local model planning")),
        result = tokio::time::timeout(STUDIO3D_LOCAL_JOB_TIMEOUT, crate::studio3d_local_scene_spec(app, state, prompt, architecture)) =>
            result.map_err(|_| studio_error("local scene planning timed out"))??,
    };
    let scene_spec = if architecture {
        parse_architecture_spec_response(&scene_spec_text)?
    } else {
        parse_scene_spec_response(&scene_spec_text)?
    };
    let scene_spec_sha256 = format!("{:x}", Sha256::digest(scene_spec.to_string().as_bytes()));
    {
        let db = state
            .database
            .lock()
            .map_err(|_| studio_error("database lock is unavailable"))?;
        let planned_count = scene_spec["rooms"]
            .as_array()
            .or_else(|| scene_spec["objects"].as_array())
            .map(Vec::len)
            .unwrap_or_default();
        db.connection().execute("UPDATE studio3d_jobs SET model_id=?1,recovery_json=?2,updated_at=?3 WHERE id=?4 AND status='running'", params![model_id,serde_json::json!({"safeToResume":false,"scheduler":"native","modelProvider":"local-llama.cpp","sceneSpecificationSha256":scene_spec_sha256,"sceneObjectCount":planned_count,"sceneSpecification":scene_spec}).to_string(),Utc::now().to_rfc3339(),job_id])?;
    }
    transition_studio3d_job(
        app.clone(),
        job_id.to_owned(),
        "running".into(),
        "geometry-generation".into(),
        None,
        String::new(),
        String::new(),
        serde_json::json!({"safeToResume":false,"modelProvider":"local-llama.cpp","sceneSpecificationSha256":scene_spec_sha256}),
        app.state::<AppState>(),
    )?;
    let submitted = tokio::select! {
        _ = token.cancelled() => return Err(studio_error("3D job cancelled before provider submission")),
        result = crate::cloud_bridge::local_gpu_request("jobs".into(), "POST".into(), Some(serde_json::json!({"prompt":prompt,"mode":mode,"request_key":key,"scene_spec":scene_spec}))) => result?,
    };
    let worker_id = submitted
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| studio_error("local worker returned no job identifier"))?
        .to_owned();
    let status = loop {
        if started.elapsed() >= STUDIO3D_LOCAL_JOB_TIMEOUT {
            return Err(studio_error("local 3D generation timed out"));
        }
        let response = tokio::select! {
            _ = token.cancelled() => {
                let _ = crate::cloud_bridge::local_gpu_request(format!("jobs/{worker_id}/cancel"), "POST".into(), Some(serde_json::json!({"reason":"user-cancelled"}))).await;
                return Err(studio_error("3D job cancelled; worker cancellation was requested"));
            },
            result = crate::cloud_bridge::local_gpu_request(format!("jobs/{worker_id}"), "GET".into(), None) => result?,
        };
        let status = response
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let app_state = app.state::<AppState>();
        let stage = format!("worker-{}", status)
            .chars()
            .take(80)
            .collect::<String>();
        transition_studio3d_job(
            app.clone(),
            job_id.to_owned(),
            "running".into(),
            stage,
            None,
            String::new(),
            String::new(),
            serde_json::json!({"safeToResume":false,"workerJobId":worker_id,"elapsedMs":started.elapsed().as_millis()}),
            app_state,
        )?;
        match status {
            "completed" => break response,
            "failed" => {
                return Err(studio_error(
                    response
                        .get("error")
                        .and_then(Value::as_str)
                        .unwrap_or("local geometry worker failed"),
                ))
            }
            "cancelled" | "canceled" => {
                return Err(studio_error("local geometry worker cancelled the job"))
            }
            _ => {
                tokio::select! { _ = token.cancelled() => continue, _ = tokio::time::sleep(std::time::Duration::from_millis(500)) => {} }
            }
        }
    };
    let geometry_backend = status
        .pointer("/output/geometry_backend")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let triangle_count = status
        .pointer("/output/triangles")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    if triangle_count == 0 || !matches!(geometry_backend, "cpu" | "opencl") {
        return Err(studio_error(
            "local worker reported incomplete triangle geometry or an unknown geometry backend",
        ));
    }
    transition_studio3d_job(
        app.clone(),
        job_id.to_owned(),
        "running".into(),
        "output-validation".into(),
        None,
        String::new(),
        String::new(),
        serde_json::json!({"safeToResume":false,"workerJobId":worker_id}),
        app.state::<AppState>(),
    )?;
    let mesh = tokio::select! {
        _ = token.cancelled() => return Err(studio_error("job cancelled before asset publication")),
        result = crate::cloud_bridge::local_gpu_request(format!("jobs/{worker_id}/mesh"), "GET".into(), None) => result?,
    };
    let filename = mesh
        .get("filename")
        .and_then(Value::as_str)
        .unwrap_or("generated.glb");
    let base64 = mesh
        .get("data_base64")
        .and_then(Value::as_str)
        .ok_or_else(|| studio_error("local worker returned no GLB data"))?;
    let asset = store_studio3d_generated_glb_inner(project_id, filename, base64, state)?;
    transition_studio3d_job(
        app.clone(),
        job_id.to_owned(),
        "running".into(),
        "asset-publication".into(),
        None,
        String::new(),
        String::new(),
        serde_json::json!({"safeToResume":false,"workerJobId":worker_id,"assetSha256":asset.content_sha256}),
        app.state::<AppState>(),
    )?;
    let app_state = app.state::<AppState>();
    let object_count = status
        .pointer("/output/objects")
        .and_then(Value::as_u64)
        .map(|count| count as usize)
        .unwrap_or_else(|| {
            scene_spec["objects"]
                .as_array()
                .map(Vec::len)
                .unwrap_or_default()
        });
    transition_studio3d_job(
        app.clone(),
        job_id.to_owned(),
        "completed".into(),
        "published".into(),
        Some(asset.id.clone()),
        asset.content_sha256.clone(),
        String::new(),
        serde_json::json!({"safeToResume":false,"workerJobId":worker_id,"executionMs":started.elapsed().as_millis(),"geometryAdapter":"procedural triangle mesh","geometryBackend":geometry_backend,"objectCount":object_count,"triangleCount":triangle_count,"modelProvider":"local-llama.cpp","modelId":model_id,"sceneSpecificationSha256":scene_spec_sha256,"assetSha256":asset.content_sha256}),
        app_state,
    )?;
    Ok(asset)
}

#[tauri::command]
pub(crate) async fn cancel_studio3d_job(job_id: String) -> Result<(), AppError> {
    Uuid::parse_str(&job_id).map_err(|_| studio_error("invalid job id"))?;
    let active = studio3d_cancellations()
        .lock()
        .map_err(|_| studio_error("job cancellation registry is unavailable"))?;
    let token = active
        .get(&job_id)
        .ok_or_else(|| studio_error("3D job is not currently executing"))?;
    token.cancel();
    Ok(())
}

#[tauri::command]
pub(crate) fn list_studio3d_assets(
    project_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<Studio3DAsset>, AppError> {
    Uuid::parse_str(&project_id).map_err(|_| studio_error("invalid project id"))?;
    let db = state
        .database
        .lock()
        .map_err(|_| studio_error("database lock is unavailable"))?;
    let mut query = db.connection().prepare("SELECT id,project_id,kind,original_name,media_type,size_bytes,content_sha256,width,height,created_at,preview_blob FROM studio3d_assets WHERE project_id=?1 ORDER BY created_at DESC,id")?;
    let rows = query.query_map(params![project_id], asset_with_preview_from_row)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(AppError::from)
}

#[tauri::command]
pub(crate) fn load_studio3d_asset(
    asset_id: String,
    state: State<'_, AppState>,
) -> Result<LoadedStudio3DAsset, AppError> {
    Uuid::parse_str(&asset_id).map_err(|_| studio_error("invalid asset id"))?;
    let db = state
        .database
        .lock()
        .map_err(|_| studio_error("database lock is unavailable"))?;
    let (asset, bytes, preview) = db.connection().query_row(
        "SELECT id,project_id,kind,original_name,media_type,size_bytes,content_sha256,width,height,created_at,content_blob,preview_blob FROM studio3d_assets WHERE id=?1",
        params![asset_id], |row| Ok((asset_from_row(row)?,row.get::<_,Vec<u8>>(10)?,row.get::<_,Option<Vec<u8>>>(11)?)),
    ).optional()?.ok_or_else(|| studio_error("asset not found"))?;
    if format!("{:x}", Sha256::digest(&bytes)) != asset.content_sha256 {
        return Err(studio_error("asset checksum does not match"));
    }
    Ok(LoadedStudio3DAsset {
        asset,
        data_base64: STANDARD.encode(bytes),
        preview_base64: preview.map(|bytes| STANDARD.encode(bytes)),
    })
}

#[tauri::command]
pub(crate) fn delete_studio3d_asset(
    project_id: String,
    asset_id: String,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    Uuid::parse_str(&project_id).map_err(|_| studio_error("invalid project id"))?;
    Uuid::parse_str(&asset_id).map_err(|_| studio_error("invalid asset id"))?;
    let db = state
        .database
        .lock()
        .map_err(|_| studio_error("database lock is unavailable"))?;
    if db.connection().execute(
        "DELETE FROM studio3d_assets WHERE id=?1 AND project_id=?2",
        params![asset_id, project_id],
    )? == 0
    {
        return Err(studio_error("asset not found in this project"));
    }
    Ok(())
}

#[tauri::command]
pub(crate) fn create_studio3d_project(
    name: String,
    state: State<'_, AppState>,
) -> Result<Studio3DProject, AppError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 120 || name.chars().any(char::is_control) {
        return Err(studio_error(
            "Project name must contain 1 to 120 printable characters.",
        ));
    }
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    let db = state
        .database
        .lock()
        .map_err(|_| studio_error("database lock is unavailable"))?;
    db.connection().execute(
        "INSERT INTO studio3d_projects(id,name,created_at,updated_at) VALUES(?1,?2,?3,?3)",
        params![id, name, now],
    )?;
    Ok(Studio3DProject {
        id,
        name: name.to_owned(),
        created_at: now.clone(),
        updated_at: now,
        scene_count: 0,
    })
}

#[tauri::command]
pub(crate) fn list_studio3d_projects(
    state: State<'_, AppState>,
) -> Result<Vec<Studio3DProject>, AppError> {
    let db = state
        .database
        .lock()
        .map_err(|_| studio_error("database lock is unavailable"))?;
    let mut query = db.connection().prepare(
        "SELECT p.id,p.name,p.created_at,p.updated_at,(SELECT COUNT(*) FROM studio3d_scenes s WHERE s.project_id=p.id) FROM studio3d_projects p ORDER BY p.updated_at DESC,p.id",
    )?;
    let rows = query.query_map([], |row| {
        Ok(Studio3DProject {
            id: row.get(0)?,
            name: row.get(1)?,
            created_at: row.get(2)?,
            updated_at: row.get(3)?,
            scene_count: row.get(4)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(AppError::from)
}

#[tauri::command]
pub(crate) fn list_studio3d_scenes(
    project_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<Studio3DSceneSummary>, AppError> {
    Uuid::parse_str(&project_id).map_err(|_| studio_error("invalid project id"))?;
    let db = state
        .database
        .lock()
        .map_err(|_| studio_error("database lock is unavailable"))?;
    let mut query = db.connection().prepare("SELECT id,project_id,title,prompt,mode,engine,content_sha256,revision,created_at,updated_at FROM studio3d_scenes WHERE project_id=?1 ORDER BY updated_at DESC,id")?;
    let rows = query.query_map(params![project_id], summary_from_row)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(AppError::from)
}

#[tauri::command]
pub(crate) fn save_studio3d_scene(
    project_id: String,
    scene_id: Option<String>,
    title: String,
    prompt: String,
    mode: String,
    engine: String,
    scene_json: Value,
    glb_base64: String,
    state: State<'_, AppState>,
) -> Result<Studio3DSceneSummary, AppError> {
    Uuid::parse_str(&project_id).map_err(|_| studio_error("invalid project id"))?;
    if let Some(id) = &scene_id {
        Uuid::parse_str(id).map_err(|_| studio_error("invalid scene id"))?;
    }
    let title = title.trim();
    if title.is_empty() || title.chars().count() > 160 || title.chars().any(char::is_control) {
        return Err(studio_error(
            "Scene title must contain 1 to 160 printable characters.",
        ));
    }
    if prompt.chars().count() > 8_000
        || engine.chars().count() > 120
        || engine.chars().any(char::is_control)
        || !matches!(mode.as_str(), "object" | "architecture" | "imported")
    {
        return Err(studio_error("Scene prompt, engine or mode is invalid."));
    }
    let scene_json = serde_json::to_string(&scene_json)
        .map_err(|_| studio_error("scene metadata is invalid"))?;
    if scene_json.len() > MAX_SCENE_JSON_BYTES
        || !serde_json::from_str::<Value>(&scene_json)
            .map(|value| value.is_object())
            .unwrap_or(false)
    {
        return Err(studio_error(
            "Scene metadata must be an object no larger than 2 MB.",
        ));
    }
    let bytes = decode_glb(&glb_base64)?;
    let digest = format!("{:x}", Sha256::digest(&bytes));
    let now = Utc::now().to_rfc3339();
    let supplied_id = scene_id.is_some();
    let id = scene_id.unwrap_or_else(|| Uuid::new_v4().to_string());
    let db = state
        .database
        .lock()
        .map_err(|_| studio_error("database lock is unavailable"))?;
    let connection = db.connection();
    let tx = connection.unchecked_transaction()?;
    let project_exists: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM studio3d_projects WHERE id=?1)",
        params![project_id],
        |row| row.get(0),
    )?;
    if !project_exists {
        return Err(studio_error("project not found"));
    }
    let existing: Option<(i64, String, String)> = tx
        .query_row(
            "SELECT revision,project_id,created_at FROM studio3d_scenes WHERE id=?1",
            params![id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let revision = match existing.as_ref() {
        Some((revision, owner, _)) if owner == &project_id => revision + 1,
        Some(_) => return Err(studio_error("scene belongs to a different project")),
        None if supplied_id => return Err(studio_error("scene not found")),
        None => 1,
    };
    let created_at = existing
        .as_ref()
        .map(|(_, _, created)| created.clone())
        .unwrap_or_else(|| now.clone());
    if existing.is_some() {
        tx.execute("UPDATE studio3d_scenes SET title=?1,prompt=?2,mode=?3,engine=?4,scene_json=?5,glb_blob=?6,content_sha256=?7,revision=?8,updated_at=?9 WHERE id=?10", params![title,prompt,mode,engine,scene_json,bytes,digest,revision,now,id])?;
    } else {
        tx.execute("INSERT INTO studio3d_scenes(id,project_id,title,prompt,mode,engine,scene_json,glb_blob,content_sha256,revision,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,1,?10,?10)", params![id,project_id,title,prompt,mode,engine,scene_json,bytes,digest,now])?;
    }
    tx.execute("INSERT INTO studio3d_scene_revisions(scene_id,revision,scene_json,glb_blob,content_sha256,created_at) VALUES(?1,?2,?3,?4,?5,?6)", params![id,revision,scene_json,bytes,digest,now])?;
    tx.execute(
        "UPDATE studio3d_projects SET updated_at=?1 WHERE id=?2",
        params![now, project_id],
    )?;
    tx.commit()?;
    Ok(Studio3DSceneSummary {
        id,
        project_id,
        title: title.to_owned(),
        prompt,
        mode,
        engine,
        content_sha256: digest,
        revision,
        created_at,
        updated_at: now,
    })
}

#[tauri::command]
pub(crate) fn load_studio3d_scene(
    scene_id: String,
    state: State<'_, AppState>,
) -> Result<Studio3DScene, AppError> {
    Uuid::parse_str(&scene_id).map_err(|_| studio_error("invalid scene id"))?;
    let db = state
        .database
        .lock()
        .map_err(|_| studio_error("database lock is unavailable"))?;
    let row = db.connection().query_row("SELECT id,project_id,title,prompt,mode,engine,content_sha256,revision,created_at,updated_at,scene_json,glb_blob FROM studio3d_scenes WHERE id=?1", params![scene_id], |row| {
        let summary = summary_from_row(row)?;
        let json: String = row.get(10)?;
        let bytes: Vec<u8> = row.get(11)?;
        Ok((summary,json,bytes))
    }).optional()?.ok_or_else(|| studio_error("scene not found"))?;
    let (summary, json, bytes) = row;
    if format!("{:x}", Sha256::digest(&bytes)) != summary.content_sha256 {
        return Err(studio_error("saved GLB checksum does not match"));
    }
    decode_glb(&STANDARD.encode(&bytes))?;
    let scene_json =
        serde_json::from_str(&json).map_err(|_| studio_error("saved scene metadata is invalid"))?;
    Ok(Studio3DScene {
        summary,
        scene_json,
        glb_base64: STANDARD.encode(bytes),
    })
}

#[tauri::command]
pub(crate) fn delete_studio3d_scene(
    project_id: String,
    scene_id: String,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    Uuid::parse_str(&project_id).map_err(|_| studio_error("invalid project id"))?;
    Uuid::parse_str(&scene_id).map_err(|_| studio_error("invalid scene id"))?;
    let db = state
        .database
        .lock()
        .map_err(|_| studio_error("database lock is unavailable"))?;
    if db.connection().execute(
        "DELETE FROM studio3d_scenes WHERE id=?1 AND project_id=?2",
        params![scene_id, project_id],
    )? == 0
    {
        return Err(studio_error("scene not found in this project"));
    }
    Ok(())
}

#[tauri::command]
pub(crate) fn duplicate_studio3d_scene(
    project_id: String,
    scene_id: String,
    title: String,
    state: State<'_, AppState>,
) -> Result<Studio3DSceneSummary, AppError> {
    Uuid::parse_str(&project_id).map_err(|_| studio_error("invalid project id"))?;
    Uuid::parse_str(&scene_id).map_err(|_| studio_error("invalid scene id"))?;
    let title = title.trim();
    if title.is_empty() || title.chars().count() > 160 || title.chars().any(char::is_control) {
        return Err(studio_error(
            "duplicate title must contain 1 to 160 printable characters",
        ));
    }
    let now = Utc::now().to_rfc3339();
    let id = Uuid::new_v4().to_string();
    let db = state
        .database
        .lock()
        .map_err(|_| studio_error("database lock is unavailable"))?;
    let tx = db.connection().unchecked_transaction()?;
    let source = tx.query_row("SELECT prompt,mode,engine,scene_json,glb_blob,content_sha256 FROM studio3d_scenes WHERE id=?1 AND project_id=?2", params![scene_id,project_id], |row| Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?,row.get::<_,String>(3)?,row.get::<_,Vec<u8>>(4)?,row.get::<_,String>(5)?))).optional()?.ok_or_else(|| studio_error("source scene not found in this project"))?;
    if format!("{:x}", Sha256::digest(&source.4)) != source.5 {
        return Err(studio_error("source scene checksum does not match"));
    }
    decode_glb(&STANDARD.encode(&source.4))?;
    tx.execute("INSERT INTO studio3d_scenes(id,project_id,title,prompt,mode,engine,scene_json,glb_blob,content_sha256,revision,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,1,?10,?10)", params![id,project_id,title,source.0,source.1,source.2,source.3,source.4,source.5,now])?;
    tx.execute("INSERT INTO studio3d_scene_revisions(scene_id,revision,scene_json,glb_blob,content_sha256,created_at) VALUES(?1,1,?2,?3,?4,?5)", params![id,source.3,source.4,source.5,now])?;
    tx.execute(
        "UPDATE studio3d_projects SET updated_at=?1 WHERE id=?2",
        params![now, project_id],
    )?;
    tx.commit()?;
    Ok(Studio3DSceneSummary {
        id,
        project_id,
        title: title.to_owned(),
        prompt: source.0,
        mode: source.1,
        engine: source.2,
        content_sha256: source.5,
        revision: 1,
        created_at: now.clone(),
        updated_at: now,
    })
}

#[tauri::command]
pub(crate) fn restore_studio3d_scene_revision(
    scene_id: String,
    revision: i64,
    state: State<'_, AppState>,
) -> Result<Studio3DSceneSummary, AppError> {
    Uuid::parse_str(&scene_id).map_err(|_| studio_error("invalid scene id"))?;
    if revision < 1 {
        return Err(studio_error("revision number must be positive"));
    }
    let now = Utc::now().to_rfc3339();
    let db = state
        .database
        .lock()
        .map_err(|_| studio_error("database lock is unavailable"))?;
    let tx = db.connection().unchecked_transaction()?;
    let current = tx.query_row("SELECT project_id,title,prompt,mode,engine,revision,created_at FROM studio3d_scenes WHERE id=?1", params![scene_id], |row| Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?,row.get::<_,String>(3)?,row.get::<_,String>(4)?,row.get::<_,i64>(5)?,row.get::<_,String>(6)?))).optional()?.ok_or_else(|| studio_error("scene not found"))?;
    let previous = tx.query_row("SELECT scene_json,glb_blob,content_sha256 FROM studio3d_scene_revisions WHERE scene_id=?1 AND revision=?2", params![scene_id,revision], |row| Ok((row.get::<_,String>(0)?,row.get::<_,Vec<u8>>(1)?,row.get::<_,String>(2)?))).optional()?.ok_or_else(|| studio_error("revision not found"))?;
    if format!("{:x}", Sha256::digest(&previous.1)) != previous.2 {
        return Err(studio_error("revision checksum does not match"));
    }
    decode_glb(&STANDARD.encode(&previous.1))?;
    let _: Value = serde_json::from_str(&previous.0)
        .map_err(|_| studio_error("revision scene metadata is invalid"))?;
    let next_revision = current.5 + 1;
    tx.execute("UPDATE studio3d_scenes SET scene_json=?1,glb_blob=?2,content_sha256=?3,revision=?4,updated_at=?5 WHERE id=?6", params![previous.0,previous.1,previous.2,next_revision,now,scene_id])?;
    tx.execute("INSERT INTO studio3d_scene_revisions(scene_id,revision,scene_json,glb_blob,content_sha256,created_at) VALUES(?1,?2,?3,?4,?5,?6)", params![scene_id,next_revision,previous.0,previous.1,previous.2,now])?;
    tx.execute(
        "UPDATE studio3d_projects SET updated_at=?1 WHERE id=?2",
        params![now, current.0],
    )?;
    tx.commit()?;
    Ok(Studio3DSceneSummary {
        id: scene_id,
        project_id: current.0,
        title: current.1,
        prompt: current.2,
        mode: current.3,
        engine: current.4,
        content_sha256: previous.2,
        revision: next_revision,
        created_at: current.6,
        updated_at: now,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cloud_failure_diagnostics_preserve_known_gateway_metadata_without_claiming_application() {
        let error = "Cloud/Studio: HTTP 403: capability denied; upstream request ID: gw-123";
        let diagnostic = cloud_edit_diagnostic(
            "openmindai-super",
            "client-abc",
            cloud_provider_request_id(error).as_deref(),
            cloud_http_status(error),
            None,
            cloud_error_code(error),
            error,
        );
        assert!(diagnostic.contains("model=openmindai-super"));
        assert!(diagnostic.contains("client_request_id=client-abc"));
        assert!(diagnostic.contains("provider_request_id=gw-123"));
        assert!(diagnostic.contains("gateway_status=403"));
        assert!(diagnostic.contains("error_code=HTTP_403"));
        assert!(diagnostic.contains("edit_applied=false"));
        assert!(diagnostic.contains("usage=unknown"));
    }

    #[test]
    fn cloud_failure_diagnostics_mark_transport_outcome_unknown_and_never_invent_status() {
        let error = "Cloud/Studio: Cloud API is unreachable (request timed out)";
        let diagnostic = cloud_edit_diagnostic(
            "m",
            "c",
            None,
            cloud_http_status(error),
            None,
            cloud_error_code(error),
            error,
        );
        assert!(diagnostic.contains("gateway_status=unknown"));
        assert!(diagnostic.contains("error_code=TIMEOUT_OUTCOME_UNKNOWN"));
        assert!(diagnostic.contains("provider_request_id=unavailable"));
        assert!(diagnostic.contains("edit_applied=false"));
    }

    #[test]
    fn cloud_diagnostics_cover_auth_rate_limit_truncation_and_schema_rejection() {
        for (status, expected) in [(401, "HTTP_401"), (403, "HTTP_403"), (429, "HTTP_429")] {
            let detail = format!("Cloud/Studio: HTTP {status}: rejected");
            assert_eq!(cloud_http_status(&detail), Some(status));
            assert_eq!(cloud_error_code(&detail), expected);
        }

        let truncated = r#"{"schemaVersion":1,"objects":["#;
        let parse_error = parse_scene_spec_response(truncated).unwrap_err();
        let diagnostic = cloud_edit_diagnostic(
            "model-x",
            "client-x",
            Some("provider-x"),
            Some(200),
            Some("stop"),
            "INVALID_JSON",
            &parse_error.to_string(),
        );
        assert!(diagnostic.contains("gateway_status=200"));
        assert!(diagnostic.contains("finish_reason=stop"));
        assert!(diagnostic.contains("error_code=INVALID_JSON"));
        assert!(diagnostic.contains("edit_applied=false"));
        let truncated_diagnostic = cloud_edit_diagnostic(
            "model-x",
            "client-x",
            None,
            Some(200),
            Some("length"),
            "TRUNCATED_COMPLETION",
            &parse_error.to_string(),
        );
        assert!(truncated_diagnostic.contains("error_code=TRUNCATED_COMPLETION"));

        let plan = serde_json::json!({"schemaVersion":1,"objects":[{
            "name":"Wrong identity","primitive":"box","dimensions":[1,1,1],
            "position":[0,0,0],"rotation":[0,0,0],"color":[0.5,0.5,0.5],
            "metallic":0,"roughness":0.5,"parent":null
        }]});
        let patch_error = validate_single_object_ai_patch(&plan, "Cube", "edit").unwrap_err();
        let rejected = cloud_edit_diagnostic(
            "model-x",
            "client-y",
            None,
            Some(200),
            Some("stop"),
            "PATCH_REJECTED",
            &patch_error.to_string(),
        );
        assert!(rejected.contains("gateway_status=200"));
        assert!(rejected.contains("error_code=PATCH_REJECTED"));
        assert!(rejected.contains("edit_applied=false"));
    }

    #[test]
    fn ai_scene_edit_accepts_only_one_schema_valid_patch_and_preserves_identity() {
        let plan = serde_json::json!({"schemaVersion":1,"objects":[{
            "name":"Sofa","primitive":"box","dimensions":[2.2,0.8,0.9],
            "position":[1.0,0.4,-1.0],"rotation":[0.0,0.0,0.0],
            "color":[0.2,0.3,0.4],"metallic":0.0,"roughness":0.8,"parent":null
        }]});
        let parsed = validate_scene_spec(plan.clone()).unwrap();
        let patch = validate_single_object_ai_patch(&parsed, "Sofa", "edit").unwrap();
        assert_eq!(patch["name"], "Sofa");
        assert_eq!(patch["dimensions"][0], 2.2);
        assert!(validate_single_object_ai_patch(&parsed, "Chair", "edit").is_err());
        assert!(validate_single_object_ai_patch(&parsed, "Sofa", "add").is_err());

        let mut many = plan;
        many["objects"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({}));
        assert!(validate_single_object_ai_patch(&many, "Sofa", "edit").is_err());
    }

    fn external_gltf_fixture(dir: &std::path::Path) -> PathBuf {
        let binary = [0u8, 0, 0, 0, 0, 0, 0, 0];
        fs::write(dir.join("scene.bin"), binary).unwrap();
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            2,
            2,
            image::Rgba([30, 140, 220, 255]),
        ))
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
        fs::write(dir.join("albedo.png"), png.into_inner()).unwrap();
        let gltf = serde_json::json!({
            "asset":{"version":"2.0","generator":"OpenMindAI external integration fixture"},
            "scene":0,"scenes":[{"nodes":[0]}],
            "nodes":[{"name":"RobotRoot","children":[1,2],"translation":[1,2,3]},
                      {"name":"Body","mesh":0}, {"name":"Camera","mesh":0,"translation":[0,1,0],"extensions":{"KHR_lights_punctual":{"light":0}}}],
            "meshes":[{"name":"RobotParts","primitives":[{"attributes":{"POSITION":0,"NORMAL":1,"TEXCOORD_0":2},"indices":3,"material":0}]}],
            "materials":[{"name":"Paint","pbrMetallicRoughness":{"baseColorTexture":{"index":0},"metallicFactor":0.4,"roughnessFactor":0.3}}],
            "textures":[{"source":0}],"images":[{"uri":"albedo.png"}],
            "animations":[{"name":"Camera move","samplers":[{"input":4,"output":5,"interpolation":"LINEAR"}],"channels":[{"sampler":0,"target":{"node":2,"path":"translation"}}]}],
            "extensionsUsed":["KHR_lights_punctual"],"extensions":{"KHR_lights_punctual":{"lights":[{"type":"point","intensity":2,"color":[1,0.8,0.6]}]}},
            "buffers":[{"uri":"scene.bin","byteLength":8}],
            "bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":8}],
            "accessors":[{"bufferView":0,"componentType":5126,"count":1,"type":"VEC3"}],
        });
        let source = dir.join("scene.gltf");
        fs::write(&source, serde_json::to_vec(&gltf).unwrap()).unwrap();
        source
    }

    fn triangle_glb() -> Vec<u8> {
        // Minimal valid GLB 2.0; the checked bytes exercise the same header and checksum gate as saves.
        let json = br#"{"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[]}]}"#;
        let padded = (json.len() + 3) & !3;
        let total = 12 + 8 + padded;
        let mut bytes = vec![0u8; total];
        bytes[0..4].copy_from_slice(&0x4654_6c67u32.to_le_bytes());
        bytes[4..8].copy_from_slice(&2u32.to_le_bytes());
        bytes[8..12].copy_from_slice(&(total as u32).to_le_bytes());
        bytes[12..16].copy_from_slice(&(padded as u32).to_le_bytes());
        bytes[16..20].copy_from_slice(&0x4e4f_534au32.to_le_bytes());
        bytes[20..20 + json.len()].copy_from_slice(json);
        bytes[20 + json.len()..20 + padded].fill(b' ');
        bytes
    }

    fn generated_triangle_glb() -> Vec<u8> {
        let document = serde_json::json!({
            "asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],
            "nodes":[{"mesh":0}],"meshes":[{"primitives":[{"attributes":{"POSITION":0},"indices":1,"mode":4}]}],
            "buffers":[{"byteLength":48}],
            "bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":36,"target":34962},{"buffer":0,"byteOffset":36,"byteLength":12,"target":34963}],
            "accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3"},{"bufferView":1,"componentType":5125,"count":3,"type":"SCALAR"}]
        });
        let mut json = serde_json::to_vec(&document).unwrap();
        while json.len() % 4 != 0 {
            json.push(b' ');
        }
        let mut binary = Vec::new();
        for value in [0f32, 0., 0., 1., 0., 0., 0., 1., 0.] {
            binary.extend_from_slice(&value.to_le_bytes());
        }
        for index in [0u32, 1, 2] {
            binary.extend_from_slice(&index.to_le_bytes());
        }
        let total = 12 + 8 + json.len() + 8 + binary.len();
        let mut bytes = Vec::with_capacity(total);
        bytes.extend_from_slice(&0x4654_6c67u32.to_le_bytes());
        bytes.extend_from_slice(&2u32.to_le_bytes());
        bytes.extend_from_slice(&(total as u32).to_le_bytes());
        bytes.extend_from_slice(&(json.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&0x4e4f_534au32.to_le_bytes());
        bytes.extend_from_slice(&json);
        bytes.extend_from_slice(&(binary.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&0x004e_4942u32.to_le_bytes());
        bytes.extend_from_slice(&binary);
        bytes
    }

    #[test]
    fn scene_spec_schema_rejects_cycles_and_out_of_range_dimensions() {
        let valid = serde_json::json!({"schemaVersion":1,"objects":[
            {"name":"Body","primitive":"box","dimensions":[2,1,3],"position":[0,0,0],"rotation":[0,0,0],"color":[0.2,0.4,0.8],"metallic":0.2,"roughness":0.6,"parent":null},
            {"name":"Wheel","primitive":"cylinder","dimensions":[0.4,0.4,0.2],"position":[0,-0.5,0],"rotation":[1.57,0,0],"color":[0.1,0.1,0.1],"metallic":0.1,"roughness":0.8,"parent":"Body"}
        ]});
        assert!(validate_scene_spec(valid.clone()).is_ok());
        assert!(parse_scene_spec_response(&valid.to_string()).is_ok());
        assert!(parse_scene_spec_response(&format!("```json\n{valid}\n```")).is_err());
        let mut aliased_null = valid.clone();
        aliased_null["objects"][0]["parent"] = Value::from("null");
        let normalized_null = normalize_scene_spec(aliased_null).unwrap();
        assert_eq!(normalized_null["objects"][0]["parent"], Value::Null);
        assert!(validate_scene_spec(normalized_null).is_ok());
        let mut wrong_version = valid.clone();
        wrong_version["schemaVersion"] = Value::from(2);
        assert!(validate_scene_spec(wrong_version).is_err());
        let mut oversized = valid.clone();
        oversized["objects"][0]["dimensions"][0] = Value::from(100.0);
        assert!(validate_scene_spec(oversized).is_err());
        let mut cyclic = valid;
        cyclic["objects"][0]["parent"] = Value::from("Wheel");
        assert!(validate_scene_spec(cyclic).is_err());
    }

    #[test]
    fn architectural_layout_requires_all_rooms_and_rejects_overlap_or_bad_bounds() {
        let mut layout = serde_json::json!({
            "schemaVersion":2,"footprint":[12,10],"storyHeight":3.2,
            "rooms":[
                {"name":"Living Room","floor":"ground","x":-3.0,"z":-2.5,"width":5.8,"depth":4.8},
                {"name":"Dining Room","floor":"ground","x":2.0,"z":-2.5,"width":4.0,"depth":4.8},
                {"name":"Kitchen","floor":"ground","x":-3.0,"z":2.5,"width":5.8,"depth":4.8},
                {"name":"Ground Bathroom","floor":"ground","x":2.0,"z":2.5,"width":4.0,"depth":4.8},
                {"name":"Bedroom 1","floor":"upper","x":-4.0,"z":-2.5,"width":3.5,"depth":4.8},
                {"name":"Bedroom 2","floor":"upper","x":0.0,"z":-2.5,"width":4.0,"depth":4.8},
                {"name":"Bedroom 3","floor":"upper","x":4.0,"z":-2.5,"width":3.8,"depth":4.8},
                {"name":"Upper Bathroom","floor":"upper","x":-4.0,"z":2.5,"width":3.5,"depth":2.5},
                {"name":"Stair Landing","floor":"upper","x":0.0,"z":2.5,"width":4.0,"depth":1.2}
            ]
        });
        assert!(parse_architecture_spec_response(&layout.to_string()).is_ok());
        layout["rooms"][1]["x"] = serde_json::json!(-3.0);
        let repaired = parse_architecture_spec_response(&layout.to_string()).unwrap();
        assert!(repaired["layoutAdjustments"]
            .as_array()
            .unwrap()
            .iter()
            .any(|name| name == "Dining Room"));
        layout["rooms"][1]["x"] = serde_json::json!(2.0);
        layout["rooms"][0]["width"] = serde_json::json!(13.0);
        assert!(parse_architecture_spec_response(&layout.to_string()).is_err());
    }

    #[test]
    fn generated_glb_must_contain_complete_indexed_triangle_geometry() {
        let glb = generated_triangle_glb();
        assert!(validate_generated_mesh_glb(&glb).is_ok());
        assert!(validate_generated_mesh_glb(&triangle_glb()).is_err());
        let mut truncated = glb;
        truncated.pop();
        assert!(validate_generated_mesh_glb(&truncated).is_err());
    }

    #[tokio::test]
    async fn native_scheduler_enforces_its_concurrency_limit() {
        let first = studio3d_scheduler().acquire().await.unwrap();
        let mut second = tokio::spawn(async { studio3d_scheduler().acquire().await.is_ok() });
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), &mut second)
                .await
                .is_err()
        );
        drop(first);
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(1), second)
                .await
                .unwrap()
                .unwrap()
        );
    }

    #[test]
    fn project_scene_and_revision_schema_is_durable_and_asset_payload_is_validated() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("studio.db");
        let db = crate::database::Database::open(path.clone()).unwrap();
        let tables: i64 = db.connection().query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN ('studio3d_projects','studio3d_scenes','studio3d_scene_revisions','studio3d_jobs')", [], |row| row.get(0)).unwrap();
        assert_eq!(tables, 4);
        let has_request = db
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('studio3d_jobs') WHERE name='request_json'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();
        assert_eq!(has_request, 1);
        let has_object_count = db.connection().query_row("SELECT COUNT(*) FROM pragma_table_info('studio3d_jobs') WHERE name='output_object_count'", [], |row| row.get::<_,i64>(0)).unwrap();
        assert_eq!(has_object_count, 1);
        let bytes = triangle_glb();
        assert_eq!(decode_glb(&STANDARD.encode(&bytes)).unwrap(), bytes);
        assert!(decode_glb(&STANDARD.encode(b"not a glb file")).is_err());
        let project_id = Uuid::new_v4().to_string();
        let scene_id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        let digest = format!("{:x}", Sha256::digest(&bytes));
        let scene_metadata = serde_json::json!({"lights":[{"id":"key","type":"point","color":"#ffeecc","intensity":2.5,"position":[1.0,2.0,3.0],"visible":true}]});
        let scene_metadata_json = scene_metadata.to_string();
        db.connection().execute("INSERT INTO studio3d_projects(id,name,created_at,updated_at) VALUES(?1,'Fixture',?2,?2)", params![project_id,now]).unwrap();
        db.connection().execute("INSERT INTO studio3d_scenes(id,project_id,title,prompt,mode,engine,scene_json,glb_blob,content_sha256,revision,created_at,updated_at) VALUES(?1,?2,'Triangle','fixture prompt','object','test',?3,?4,?5,1,?6,?6)", params![scene_id,project_id,scene_metadata_json,bytes,digest,now]).unwrap();
        db.connection().execute("INSERT INTO studio3d_scene_revisions(scene_id,revision,scene_json,glb_blob,content_sha256,created_at) VALUES(?1,1,?2,?3,?4,?5)", params![scene_id,scene_metadata_json,bytes,digest,now]).unwrap();
        let job_id = Uuid::new_v4().to_string();
        let job_digest = format!("{:x}", Sha256::digest(b"fixture input"));
        db.connection().execute("INSERT INTO studio3d_jobs(id,project_id,generation_mode,inference_route,model_id,engine_id,input_asset_ids_json,current_stage,status,input_sha256,recovery_json,created_at,updated_at) VALUES(?1,?2,'text-to-3d','local','fixture-model','fixture-engine','[]','constructing','running',?3,'{}',?4,?4)", params![job_id,project_id,job_digest,now]).unwrap();
        let completed_job_id = Uuid::new_v4().to_string();
        let mesh_asset_id = Uuid::new_v4().to_string();
        let mesh_bytes = generated_triangle_glb();
        let mesh_digest = format!("{:x}", Sha256::digest(&mesh_bytes));
        db.connection().execute("INSERT INTO studio3d_assets(id,project_id,kind,original_name,media_type,size_bytes,content_sha256,content_blob,created_at) VALUES(?1,?2,'mesh','generated.glb','model/gltf-binary',?3,?4,?5,?6)", params![mesh_asset_id,project_id,mesh_bytes.len() as i64,mesh_digest,mesh_bytes,now]).unwrap();
        db.connection().execute("INSERT INTO studio3d_jobs(id,project_id,generation_mode,inference_route,model_id,engine_id,input_asset_ids_json,current_stage,status,input_sha256,output_asset_id,output_sha256,recovery_json,created_at,updated_at,started_at,completed_at) VALUES(?1,?2,'text-to-3d','local','fixture-model','fixture-engine','[]','published','completed',?3,?4,?5,'{}',?6,?6,?6,?6)", params![completed_job_id,project_id,job_digest,mesh_asset_id,mesh_digest,now]).unwrap();
        drop(db);
        let reopened = crate::database::Database::open(path).unwrap();
        let (stored, hash, revision): (Vec<u8>, String, i64) = reopened
            .connection()
            .query_row(
                "SELECT glb_blob,content_sha256,revision FROM studio3d_scenes WHERE id=?1",
                params![scene_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(stored, bytes);
        assert_eq!(format!("{:x}", Sha256::digest(&stored)), hash);
        assert_eq!(revision, 1);
        let stored_metadata: String = reopened
            .connection()
            .query_row(
                "SELECT scene_json FROM studio3d_scenes WHERE id=?1",
                params![scene_id],
                |row| row.get(0),
            )
            .unwrap();
        let parsed: Value = serde_json::from_str(&stored_metadata).unwrap();
        assert_eq!(parsed["lights"][0]["type"], "point");
        let history_count: i64 = reopened
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM studio3d_scene_revisions WHERE scene_id=?1",
                params![scene_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(history_count, 1);
        let (status, stage, recoverable, stored_input): (String, String, String, String) = reopened.connection().query_row(
            "SELECT status,current_stage,recovery_json,input_sha256 FROM studio3d_jobs WHERE id=?1", params![job_id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)),
        ).unwrap();
        assert_eq!(
            (status.as_str(), stage.as_str()),
            ("interrupted", "interrupted")
        );
        let recovery = serde_json::from_str::<Value>(&recoverable).unwrap();
        assert_eq!(recovery["safeToResume"], false);
        assert_eq!(recovery["restartObserved"], true);
        assert_eq!(stored_input, job_digest);
        let (recovered_object_count, completed_recovery): (i64, String) = reopened
            .connection()
            .query_row(
                "SELECT output_object_count,recovery_json FROM studio3d_jobs WHERE id=?1",
                params![completed_job_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(recovered_object_count, 1);
        assert_eq!(
            serde_json::from_str::<Value>(&completed_recovery).unwrap()["objectCount"],
            1
        );
        reopened
            .connection()
            .execute(
                "UPDATE studio3d_jobs SET status='retry_required' WHERE id=?1",
                params![job_id],
            )
            .unwrap();
        let retry_status: String = reopened
            .connection()
            .query_row(
                "SELECT status FROM studio3d_jobs WHERE id=?1",
                params![job_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(retry_status, "retry_required");
    }

    #[test]
    fn image_import_requires_matching_decodable_format_and_creates_bounded_preview() {
        let image = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            64,
            32,
            image::Rgba([20, 80, 160, 255]),
        ));
        let mut bytes = std::io::Cursor::new(Vec::new());
        image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        let png = bytes.into_inner();
        let (mime, width, height, preview) = image_properties("png", &png).unwrap();
        assert_eq!((mime, width, height), ("image/png", 64, 32));
        assert!(!STANDARD.decode(preview).unwrap().is_empty());
        assert!(image_properties("jpg", &png).is_err());
        assert!(image_properties("png", b"\x89PNG\r\n\x1a\ntruncated").is_err());
        assert!(checked_image_dimensions(20_000, 20_000).is_err());
    }

    #[test]
    fn external_gltf_resources_are_bundled_and_unsafe_or_missing_paths_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let source = external_gltf_fixture(dir.path());
        let (name, bundled) =
            bundle_gltf_resources(source.to_str().unwrap(), dir.path().to_str().unwrap()).unwrap();
        assert_eq!(name, "scene.gltf");
        let document: Value = serde_json::from_slice(&bundled).unwrap();
        assert_eq!(
            document["nodes"][0]["children"].as_array().unwrap().len(),
            2
        );
        assert_eq!(
            document["materials"][0]["pbrMetallicRoughness"]["metallicFactor"],
            0.4
        );
        assert_eq!(document["animations"][0]["name"], "Camera move");
        assert_eq!(
            document["extensions"]["KHR_lights_punctual"]["lights"][0]["type"],
            "point"
        );
        let buffer_uri = document["buffers"][0]["uri"].as_str().unwrap();
        let (_, buffer_data) = buffer_uri
            .strip_prefix("data:")
            .unwrap()
            .split_once(',')
            .unwrap();
        assert_eq!(STANDARD.decode(buffer_data).unwrap(), vec![0; 8]);
        let image_uri = document["images"][0]["uri"].as_str().unwrap();
        let (mime, image_data) = image_uri
            .strip_prefix("data:")
            .unwrap()
            .split_once(';')
            .unwrap();
        assert_eq!(mime, "image/png");
        let encoded = image_data.strip_prefix("base64,").unwrap();
        assert!(image::load_from_memory(&STANDARD.decode(encoded).unwrap()).is_ok());

        let mut malformed: Value = serde_json::from_slice(&fs::read(&source).unwrap()).unwrap();
        malformed["buffers"][0]["uri"] = Value::String("../outside.bin".into());
        fs::write(&source, serde_json::to_vec(&malformed).unwrap()).unwrap();
        assert!(
            bundle_gltf_resources(source.to_str().unwrap(), dir.path().to_str().unwrap()).is_err()
        );
        malformed["buffers"][0]["uri"] = Value::String("missing.bin".into());
        fs::write(&source, serde_json::to_vec(&malformed).unwrap()).unwrap();
        assert!(
            bundle_gltf_resources(source.to_str().unwrap(), dir.path().to_str().unwrap()).is_err()
        );
        malformed["buffers"][0]["uri"] = Value::String("scene.bin?download=1".into());
        fs::write(&source, serde_json::to_vec(&malformed).unwrap()).unwrap();
        assert!(
            bundle_gltf_resources(source.to_str().unwrap(), dir.path().to_str().unwrap()).is_err()
        );
    }

    #[test]
    fn obj_and_stl_fixtures_validate_geometry_and_reject_malformed_inputs() {
        let obj = b"o Triangle\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
        validate_obj(obj).unwrap();
        assert!(validate_obj(b"v NaN 0 0\nf 1 2 3\n").is_err());
        assert!(validate_obj(b"v 0 0 0\n").is_err());

        let stl = b"solid triangle\n facet normal 0 0 1\n outer loop\n vertex 0 0 0\n vertex 1 0 0\n vertex 0 1 0\n endloop\n endfacet\nendsolid triangle\n";
        validate_stl(stl).unwrap();
        assert!(validate_stl(b"solid broken\nendsolid broken\n").is_err());
    }

    #[test]
    fn gltf_bundles_scoped_binary_and_texture_resources_and_rejects_traversal() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("scene.gltf");
        fs::write(root.path().join("mesh.bin"), [0u8, 1, 2, 3]).unwrap();
        let mut texture = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            2,
            2,
            image::Rgba([255, 80, 20, 255]),
        ))
        .write_to(&mut texture, image::ImageFormat::Png)
        .unwrap();
        fs::write(root.path().join("albedo.png"), texture.into_inner()).unwrap();
        fs::write(&source, br#"{"asset":{"version":"2.0"},"buffers":[{"uri":"mesh.bin","byteLength":4}],"images":[{"uri":"albedo.png"}]}"#).unwrap();
        let (name, bundled) =
            bundle_gltf_resources(source.to_str().unwrap(), root.path().to_str().unwrap()).unwrap();
        let parsed: Value = serde_json::from_slice(&bundled).unwrap();
        assert_eq!(name, "scene.gltf");
        assert!(parsed["buffers"][0]["uri"]
            .as_str()
            .unwrap()
            .starts_with("data:application/octet-stream;base64,"));
        assert!(parsed["images"][0]["uri"]
            .as_str()
            .unwrap()
            .starts_with("data:image/png;base64,"));

        fs::write(root.path().join("albedo.png"), b"\x89PNG\r\n\x1a\nfixture").unwrap();
        assert!(
            bundle_gltf_resources(source.to_str().unwrap(), root.path().to_str().unwrap()).is_err()
        );

        fs::write(
            &source,
            br#"{"asset":{"version":"2.0"},"buffers":[{"uri":"../mesh.bin","byteLength":4}]}"#,
        )
        .unwrap();
        assert!(
            bundle_gltf_resources(source.to_str().unwrap(), root.path().to_str().unwrap()).is_err()
        );
    }
}
