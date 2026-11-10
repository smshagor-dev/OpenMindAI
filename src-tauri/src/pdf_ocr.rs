use serde::{Deserialize, Serialize};
use tauri::State;

use crate::{
    allocate_local_port,
    app_error::AppError,
    inference::InferenceMedia,
    model_catalog,
    model_download::{validate_gguf_header, validate_installed_dependencies},
    model_registry::ModelRegistry,
    vision_batch, AppState, ModelLaunchPlanner,
};

#[path = "pdf_ocr_selection.rs"]
mod selection;

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

    let entry = model_catalog::entry_by_id("qwen25-vl-3b-q4km")?;
    let paths = selection::lens_paths(state.root.root(), &entry)?;
    validate_gguf_header(&paths.checkpoint, &state.root)?;
    if !validate_installed_dependencies(&state.root, &entry, true)? {
        return Err(AppError::ModelInvalid("OpenMindAI Lens projector integrity check failed. Validate or re-download Lens from Settings > Models.".into()));
    }
    let lens_model = {
        let db = state
            .database
            .lock()
            .map_err(|_| AppError::internal("database lock poisoned"))?;
        let models = ModelRegistry::new(&db, &state.root).discover_gguf_models()?;
        selection::select_lens_model(state.root.root(), &paths.checkpoint, models)?
    };

    let hardware = crate::hardware::HardwareProfiler::for_inference(&state.hardware);
    let mut plan = ModelLaunchPlanner::plan(&lens_model, &hardware, allocate_local_port()?);
    plan.config.mmproj_path = Some(paths.projector.to_string_lossy().into_owned());
    // Held while the pages are read.
    let (endpoint, _lens_lease) = {
        let (status, lease) =
            crate::runtime::ensure_model_ready(&state.runtime, &hardware, &plan.config)?;
        let endpoint = status.endpoint.ok_or_else(|| {
            AppError::InferenceServerUnavailable(
                "OpenMindAI Lens runtime endpoint is missing".to_string(),
            )
        })?;
        (endpoint, lease)
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
