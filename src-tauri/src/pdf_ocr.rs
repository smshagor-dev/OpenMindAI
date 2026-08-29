use serde::{Deserialize, Serialize};
use tauri::State;

use crate::{
    allocate_local_port,
    app_error::AppError,
    inference::InferenceMedia,
    installed_catalog_entry_by_id,
    model_registry::ModelRegistry,
    vision_batch,
    AppState,
    ModelLaunchPlanner,
};

const MAX_PAGES_PER_BATCH: usize = 4;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfOcrPageInput {
    pub page_number: u32,
    pub data_url: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfOcrPageResult {
    pub page_number: u32,
    pub text: String,
}

#[tauri::command]
pub(crate) async fn ocr_pdf_pages(
    pages: Vec<PdfOcrPageInput>,
    state: State<'_, AppState>,
) -> Result<Vec<PdfOcrPageResult>, AppError> {
    if pages.is_empty() || pages.len() > MAX_PAGES_PER_BATCH {
        return Err(AppError::InferenceFailed(format!(
            "PDF OCR accepts between 1 and {MAX_PAGES_PER_BATCH} pages per batch"
        )));
    }

    let lens_status = installed_catalog_entry_by_id(&state, "qwen25-vl-3b-q4km")?
        .ok_or_else(|| {
            AppError::ModelNotFound(
                "OpenMindAI Lens is required to read scanned/image-only PDF pages. Download Lens from Settings > Models first."
                    .to_string(),
            )
        })?;
    let lens_path = lens_status.installed_path.as_deref().ok_or_else(|| {
        AppError::ModelNotFound("OpenMindAI Lens model path is unavailable.".to_string())
    })?;
    let normalized_lens_path = lens_path.replace('\\', "/").to_ascii_lowercase();

    let lens_model = {
        let db = state
            .database
            .lock()
            .map_err(|_| AppError::internal("database lock poisoned"))?;
        ModelRegistry::new(&db, &state.root)
            .discover_gguf_models()?
            .into_iter()
            .find(|model| {
                model.path.replace('\\', "/").to_ascii_lowercase() == normalized_lens_path
                    || model.family.as_deref() == Some("qwen-vl")
            })
            .ok_or_else(|| {
                AppError::ModelNotFound(
                    "OpenMindAI Lens is installed but was not registered as a usable local model. Validate or re-download it from Settings > Models."
                        .to_string(),
                )
            })?
    };

    let hardware = state.hardware.clone();
    let plan = ModelLaunchPlanner::plan(&lens_model, &hardware, allocate_local_port()?);
    let endpoint = {
        let mut runtime = state
            .runtime
            .lock()
            .map_err(|_| AppError::internal("runtime lock poisoned"))?;
        runtime.ensure_model_server(&hardware, &plan.config)?;
        runtime.status(&hardware)?.endpoint.ok_or_else(|| {
            AppError::InferenceServerUnavailable("OpenMindAI Lens runtime endpoint is missing".to_string())
        })?
    };

    let mut results = Vec::with_capacity(pages.len());
    for page in pages {
        if page.page_number == 0 {
            return Err(AppError::InferenceFailed(
                "PDF page numbers must start at 1".to_string(),
            ));
        }
        let media = InferenceMedia {
            kind: "image".to_string(),
            name: format!("PDF page {}", page.page_number),
            mime_type: "image/jpeg".to_string(),
            data_url: page.data_url,
        };
        let prompt = format!(
            "Read PDF page {} completely. Transcribe all visible text in natural reading order. Preserve headings, list items, table rows/cells, equations, labels, and captions when readable. For charts or diagrams, add a short bracketed description of the visible information. Do not summarize or invent missing text. Return page content only, without a preamble.",
            page.page_number
        );
        let text = vision_batch::analyze_image(&state.http, &endpoint, &prompt, &media).await?;
        results.push(PdfOcrPageResult {
            page_number: page.page_number,
            text,
        });
    }
    Ok(results)
}
