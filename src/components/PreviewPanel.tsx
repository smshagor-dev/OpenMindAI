import { useEffect, useState } from "react";
import { Code2, Copy, Download, Eye, X } from "lucide-react";
import { highlightCode, normalizeLanguage } from "../lib/markdown";

export interface PreviewTarget {
  title: string;
  language: string | null;
  code: string;
  onDownload: () => void;
}

export function PreviewPanel(props: { target: PreviewTarget | null; onClose: () => void }) {
  const language = normalizeLanguage(props.target?.language ?? null);
  const isHtml = language === "html";
  const [mode, setMode] = useState<"preview" | "code">(isHtml ? "preview" : "code");

  useEffect(() => {
    setMode(isHtml ? "preview" : "code");
  }, [props.target, isHtml]);

  useEffect(() => {
    if (!props.target) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        props.onClose();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [props]);

  if (!props.target) return null;
  const target = props.target;

  return (
    <aside className="preview-panel" role="complementary" aria-label="Preview">
      <div className="preview-panel-header">
        <span className="preview-panel-title">{target.title}</span>
        <div className="preview-panel-actions">
          {isHtml ? (
            <div className="preview-panel-tabs">
              <button
                type="button"
                className={mode === "preview" ? "preview-tab active" : "preview-tab"}
                onClick={() => setMode("preview")}
              >
                <Eye size={13} /> Preview
              </button>
              <button
                type="button"
                className={mode === "code" ? "preview-tab active" : "preview-tab"}
                onClick={() => setMode("code")}
              >
                <Code2 size={13} /> Code
              </button>
            </div>
          ) : null}
          <button
            type="button"
            className="icon-button"
            title="Copy code"
            onClick={() => navigator.clipboard.writeText(target.code)}
          >
            <Copy size={15} />
          </button>
          <button type="button" className="icon-button" title={`Download ${target.title}`} onClick={target.onDownload}>
            <Download size={15} />
          </button>
          <button type="button" className="icon-button" title="Close" onClick={props.onClose}>
            <X size={16} />
          </button>
        </div>
      </div>
      <div className="preview-panel-body">
        {isHtml && mode === "preview" ? (
          <iframe title={target.title} className="preview-frame" sandbox="allow-scripts" srcDoc={target.code} />
        ) : (
          <pre className="preview-code">
            <code
              className={language ? `language-${language}` : undefined}
              dangerouslySetInnerHTML={{ __html: highlightCode(target.code, language) }}
            />
          </pre>
        )}
      </div>
    </aside>
  );
}
