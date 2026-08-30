use std::path::Path;

use crate::{app_error::AppError, portable_root::PortableRootManager};

pub async fn generate_voice(
    root: &PortableRootManager,
    model_path: &Path,
    voices_dir: &Path,
    text: &str,
    output_path: &Path,
) -> Result<(), AppError> {
    let _ = (root, model_path, voices_dir, text, output_path);
    Err(AppError::ArtifactGenerationFailed(
        "Local OpenMindAI Speak is not enabled in the Android/iOS app shell yet. Mobile voice synthesis is planned for the mobile inference phase."
            .to_string(),
    ))
}
