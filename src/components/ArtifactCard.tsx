import { useEffect, useState } from "react";
import { FileCode, FileText, FileType, Folder, Image, Loader2, RefreshCw, Video, Volume2 } from "lucide-react";
import type { Artifact } from "../types";
import { formatBytes } from "../lib/format";
import { renderMarkdown } from "../lib/markdown";
import { artifactMediaDataUrl } from "../lib/media";

const KIND_LABELS: Record<Artifact["kind"], string> = {
  text: "Text document",
  markdown: "Markdown document",
  code: "Code file",
  pdf: "PDF document",
  docx: "Word document",
  image: "Image",
  audio: "Audio",
  video: "Video",
};

export function KindIcon(props: { kind: Artifact["kind"] }) {
  switch (props.kind) {
    case "pdf":
    case "docx":
      return <FileType size={20} />;
    case "code":
      return <FileCode size={20} />;
    case "image":
      return <Image size={20} />;
    case "audio":
      return <Volume2 size={20} />;
    case "video":
      return <Video size={20} />;
    default:
      return <FileText size={20} />;
  }
}

export function ArtifactCard(props: {
  artifact: Artifact;
  previewContent: string | null;
  onOpen: (artifact: Artifact) => void;
  onReveal: (artifact: Artifact) => void;
  onRetry: (artifact: Artifact) => void;
}) {
  const [previewOpen, setPreviewOpen] = useState(false);
  const [mediaUrl, setMediaUrl] = useState<string | null>(null);
  const [mediaError, setMediaError] = useState<string | null>(null);
  const artifact = props.artifact;

  useEffect(() => {
    let cancelled = false;
    setMediaUrl(null);
    setMediaError(null);
    if (
      artifact.status !== "ready" ||
      (artifact.kind !== "image" && artifact.kind !== "audio")
    ) {
      return () => {
        cancelled = true;
      };
    }

    void artifactMediaDataUrl(artifact.id)
      .then((url) => {
        if (!cancelled) setMediaUrl(url);
      })
      .catch((error) => {
        if (!cancelled) setMediaError(error instanceof Error ? error.message : String(error));
      });
    return () => {
      cancelled = true;
    };
  }, [artifact.id, artifact.kind, artifact.status]);

  return (
    <div className={`artifact-card artifact-${artifact.status}`}>
      <div className="artifact-icon">
        <KindIcon kind={artifact.kind} />
      </div>
      <div className="artifact-body">
        <strong>{artifact.name}</strong>
        <span className="muted">
          {KIND_LABELS[artifact.kind]}
          {artifact.status === "ready" ? ` · ${formatBytes(artifact.sizeBytes)}` : ""}
          {artifact.pageCount ? ` · ${artifact.pageCount} page${artifact.pageCount === 1 ? "" : "s"}` : ""}
        </span>
        {artifact.status === "generating" ? (
          <span className="artifact-status">
            <Loader2 size={13} className="spin" />
            {artifact.kind === "pdf"
              ? "Creating PDF..."
              : artifact.kind === "docx"
                ? "Creating document..."
                : artifact.kind === "image"
                  ? "Generating image..."
                  : artifact.kind === "audio"
                    ? "Generating audio..."
                    : artifact.kind === "video"
                      ? "Generating video..."
                      : "Generating..."}
          </span>
        ) : null}
        {artifact.status === "failed" ? (
          <span className="artifact-status artifact-error">{artifact.error ?? "Generation failed."}</span>
        ) : null}
        {artifact.status === "ready" && artifact.kind === "image" && mediaUrl ? (
          <img className="artifact-inline-image" src={mediaUrl} alt={artifact.name} />
        ) : null}
        {artifact.status === "ready" && artifact.kind === "audio" && mediaUrl ? (
          <audio className="artifact-inline-audio" controls preload="metadata" src={mediaUrl}>
            Your system cannot play this local audio format.
          </audio>
        ) : null}
        {mediaError ? <span className="artifact-status artifact-error">Inline preview unavailable: {mediaError}</span> : null}
      </div>
      <div className="artifact-actions">
        {artifact.status === "ready" && artifact.kind === "markdown" && props.previewContent ? (
          <button type="button" onClick={() => setPreviewOpen(true)}>
            Preview
          </button>
        ) : null}
        {artifact.status === "ready" ? (
          <button type="button" onClick={() => props.onOpen(artifact)}>
            Open
          </button>
        ) : null}
        {artifact.status === "ready" ? (
          <button type="button" title="Reveal in folder" onClick={() => props.onReveal(artifact)}>
            <Folder size={15} />
          </button>
        ) : null}
        {artifact.status === "failed" ? (
          <button type="button" onClick={() => props.onRetry(artifact)}>
            <RefreshCw size={15} /> Retry
          </button>
        ) : null}
      </div>
      {previewOpen ? (
        <div className="modal-overlay" role="presentation" onClick={() => setPreviewOpen(false)}>
          <div
            className="modal-card artifact-preview-card"
            role="dialog"
            aria-modal="true"
            onClick={(event) => event.stopPropagation()}
          >
            <h2>{artifact.name}</h2>
            <div
              className="artifact-preview-body markdown-block"
              dangerouslySetInnerHTML={{ __html: renderMarkdown(props.previewContent ?? "") }}
            />
            <div className="modal-actions">
              <button type="button" className="ghost-button" onClick={() => setPreviewOpen(false)}>
                Close
              </button>
            </div>
          </div>
        </div>
      ) : null}
    </div>
  );
}
