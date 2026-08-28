from pathlib import Path


def replace_once(path: str, old: str, new: str) -> None:
    file = Path(path)
    text = file.read_text(encoding="utf-8")
    if old not in text:
        raise SystemExit(f"anchor not found in {path}: {old[:160]!r}")
    file.write_text(text.replace(old, new, 1), encoding="utf-8")


replace_once(
    "src-tauri/src/diffusion_runtime.rs",
    '''pub async fn generate_video(
    root: &PortableRootManager,
    client: &Client,
    hardware: &HardwareProfile,
    diffusion_model_path: &Path,
    vae_path: &Path,
    text_encoder_path: &Path,
    prompt: &str,
    output_path: &Path,
) -> Result<(), AppError> {
    let prompt = prompt.trim();''',
    '''pub(crate) struct VideoGenerationRequest<'a> {
    pub diffusion_model_path: &'a Path,
    pub vae_path: &'a Path,
    pub text_encoder_path: &'a Path,
    pub prompt: &'a str,
    pub output_path: &'a Path,
}

pub async fn generate_video(
    root: &PortableRootManager,
    client: &Client,
    hardware: &HardwareProfile,
    request: VideoGenerationRequest<'_>,
) -> Result<(), AppError> {
    let VideoGenerationRequest {
        diffusion_model_path,
        vae_path,
        text_encoder_path,
        prompt,
        output_path,
    } = request;
    let prompt = prompt.trim();''',
)

replace_once(
    "src-tauri/src/lib.rs",
    '''            diffusion_runtime::generate_video(
                &state.root,
                &state.http,
                &hardware,
                &model_path,
                &vae_path,
                &text_encoder_path,
                prompt,
                path,
            )
            .await''',
    '''            diffusion_runtime::generate_video(
                &state.root,
                &state.http,
                &hardware,
                diffusion_runtime::VideoGenerationRequest {
                    diffusion_model_path: &model_path,
                    vae_path: &vae_path,
                    text_encoder_path: &text_encoder_path,
                    prompt,
                    output_path: path,
                },
            )
            .await''',
)

Path("scripts/fix_video_voice_clippy.py").unlink(missing_ok=True)
