#[path = "../../src-tauri/src/app_error.rs"]
pub mod app_error;
#[path = "../../src-tauri/src/coding_patch.rs"]
pub mod coding_patch;

// Expose the real crate-private recovery entry point to this standalone harness.
pub fn recover(root: &std::path::Path) -> Result<usize, app_error::AppError> {
    coding_patch::recover_pending_transactions(root)
}
