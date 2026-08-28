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

replace_once(
    "src/App.tsx",
    '''  const retryArtifact = useCallback(
    (artifact: Artifact) => {
      const source = messages.find((message) => message.id === artifact.messageId);
      if (!source || !artifact.messageId) {
        showError("The original message for this file is no longer available.");
        return;
      }
      void createArtifact(artifact.messageId, artifact.kind, source.content, artifact.name);
    },
    [createArtifact, messages, showError],
  );''',
    '''  const retryArtifact = useCallback(
    (artifact: Artifact) => {
      const source = messages.find((message) => message.id === artifact.messageId);
      if (!source || !artifact.messageId || !activeId) {
        showError("The original message for this file is no longer available.");
        return;
      }
      if (artifact.kind === "image" || artifact.kind === "video" || artifact.kind === "audio") {
        const generationKind = artifact.kind === "audio" ? "voice" : artifact.kind;
        void api
          .createGenerationArtifact(activeId, artifact.messageId, generationKind, source.content)
          .then((next) => {
            setArtifacts((items) => upsertArtifactInList(items, next));
            if (preferences?.openArtifactsAfterGeneration && next.status === "ready") {
              void api.openArtifact(next.id).catch(showError);
            }
          })
          .catch(showError);
        return;
      }
      void createArtifact(artifact.messageId, artifact.kind, source.content, artifact.name);
    },
    [activeId, createArtifact, messages, preferences?.openArtifactsAfterGeneration, showError],
  );''',
)

replace_once(
    "src/lib/chat.ts",
    '''    case "video":
      return "[Mode: Video Creation]\\nCreate a production-quality video generation prompt with subject, scene progression, camera motion, timing, lighting, style, negative prompt, and recommended duration/aspect ratio. Do not claim a video file was generated unless a video generator is connected.";''',
    '''    case "video":
      return "[Mode: Video Creation]\\nWrite only the final positive visual prompt for the local video renderer. Describe subject, environment, action, scene progression, camera motion, lighting, composition, and visual style in natural prose. Do not add headings, markdown, negative prompts, duration, aspect-ratio recommendations, explanations, or claims that rendering already succeeded; the local runtime controls those settings separately.";''',
)

Path("scripts/fix_video_voice_clippy.py").unlink(missing_ok=True)
