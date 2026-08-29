import { useEffect, useRef, useState } from "react";
import { CheckCheck, Copy, Download, Eye, Pencil, RefreshCw, RotateCcw } from "lucide-react";
import type { Artifact, ArtifactKind, Message } from "../types";
import {
  highlightCode,
  normalizeLanguage,
  renderMarkdown,
  splitMarkdownBlocks,
} from "../lib/markdown";
import { userMessageDisplay } from "../lib/chat";
import { formatTime } from "../lib/format";
import { ArtifactCard } from "./ArtifactCard";
import type { PreviewTarget } from "./PreviewPanel";

const CODE_EXTENSIONS: Record<string, string> = {
  python: "py",
  typescript: "ts",
  javascript: "js",
  rust: "rs",
  json: "json",
  bash: "sh",
  html: "html",
  xml: "xml",
  css: "css",
};

export function MessageItem(props: {
  message: Message;
  markdown: boolean;
  codeCopyButtons: boolean;
  canRegenerate: boolean;
  onRegenerate: () => void;
  onRetry: () => void;
  artifacts: Artifact[];
  onCreateArtifact: (kind: ArtifactKind, content: string, filenameHint?: string) => void;
  onOpenArtifact: (artifact: Artifact) => void;
  onRevealArtifact: (artifact: Artifact) => void;
  onRetryArtifact: (artifact: Artifact) => void;
  onPreview: (target: PreviewTarget) => void;
  onEditUser?: (content: string) => void;
}) {
  const message = props.message;
  const isAssistant = message.role === "assistant";
  const userDisplay = message.role === "user" ? userMessageDisplay(message.content) : null;
  const renderedContent = userDisplay?.displayText || message.content;
  const isThinking = isAssistant && message.status === "streaming" && message.content.trim().length === 0;
  const canSave = isAssistant && message.status === "completed" && message.content.trim().length > 0;
  const showActions = isAssistant && !isThinking && message.status !== "streaming" && message.content.trim().length > 0;
  const canEditUser = Boolean(props.onEditUser && userDisplay?.prompt);
  const [copied, setCopied] = useState(false);
  const copyMessage = async () => {
    await navigator.clipboard.writeText(renderedContent);
    setCopied(true);
    window.setTimeout(() => setCopied(false), 1400);
  };

  return (
    <article className={`message ${message.role}`}>
      {isAssistant ? (
        <div className="assistant-header">
          <img className="assistant-icon" src="/icon.png" alt="" />
          <span className="assistant-model">OpenMindAI</span>
        </div>
      ) : null}

      {isThinking ? (
        <div className="thinking-indicator">
          Thinking...
          <span className="thinking-dots">
            <span />
            <span />
            <span />
          </span>
        </div>
      ) : renderedContent.trim() ? (
        <MessageContent
          content={renderedContent}
          role={message.role}
          markdown={props.markdown}
          codeCopyButtons={props.codeCopyButtons}
          onSaveCode={props.onCreateArtifact}
          onPreview={props.onPreview}
        />
      ) : null}

      {isAssistant && message.status === "cancelled" ? (
        <div className="muted">Generation stopped.</div>
      ) : null}
      {isAssistant && message.status === "failed" ? (
        <div className="muted">Generation failed. You can retry this response.</div>
      ) : null}

      {message.role === "user" ? (
        <div className="message-actions-row message-actions-user">
          <button
            className="message-action-pill"
            title="Copy"
            onClick={() => void copyMessage()}
          >
            <Copy size={14} /> {copied ? "Copied" : "Copy"}
          </button>
          {canEditUser ? (
            <button
              className="message-action-pill"
              title="Edit and resend"
              onClick={() => props.onEditUser?.(userDisplay?.prompt ?? "")}
            >
              <Pencil size={14} /> Edit
            </button>
          ) : null}
          <span className="message-timestamp message-timestamp-user">
            {formatTime(message.createdAt)} <CheckCheck size={13} />
          </span>
        </div>
      ) : null}

      {props.artifacts.length > 0 ? (
        <div className="message-artifacts">
          {props.artifacts.map((artifact) => (
            <ArtifactCard
              key={artifact.id}
              artifact={artifact}
              previewContent={artifact.kind === "markdown" ? message.content : null}
              onOpen={props.onOpenArtifact}
              onReveal={props.onRevealArtifact}
              onRetry={props.onRetryArtifact}
            />
          ))}
        </div>
      ) : null}

      {message.role === "assistant" && message.status === "failed" ? (
        <div className="message-actions-row">
          <button className="message-action-pill" title="Retry" onClick={props.onRetry}>
            <RotateCcw size={14} /> Retry
          </button>
        </div>
      ) : null}

      {showActions ? (
        <div className="message-actions-row">
          <button
            className="message-action-pill"
            title="Copy"
            onClick={() => void copyMessage()}
          >
            <Copy size={14} /> {copied ? "Copied" : "Copy"}
          </button>
          {props.canRegenerate ? (
            <button className="message-action-pill" title="Regenerate" onClick={props.onRegenerate}>
              <RefreshCw size={14} /> Regenerate
            </button>
          ) : null}
          {canSave ? <SaveMenu onSave={(kind) => props.onCreateArtifact(kind, message.content)} /> : null}
          <span className="message-timestamp">{formatTime(message.updatedAt)}</span>
        </div>
      ) : null}
    </article>
  );
}

function SaveMenu(props: { onSave: (kind: ArtifactKind) => void }) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!open) return;
    const onClickOutside = (event: MouseEvent) => {
      if (ref.current && !ref.current.contains(event.target as Node)) setOpen(false);
    };
    window.addEventListener("mousedown", onClickOutside);
    return () => window.removeEventListener("mousedown", onClickOutside);
  }, [open]);

  return (
    <div className="save-menu" ref={ref}>
      <button className="message-action-pill" title="Download" onClick={() => setOpen((value) => !value)}>
        <Download size={14} /> Download
      </button>
      {open ? (
        <div className="save-menu-dropdown" role="menu">
          <button
            role="menuitem"
            onClick={() => {
              props.onSave("text");
              setOpen(false);
            }}
          >
            Save as Text
          </button>
          <button
            role="menuitem"
            onClick={() => {
              props.onSave("markdown");
              setOpen(false);
            }}
          >
            Save as Markdown
          </button>
          <button
            role="menuitem"
            onClick={() => {
              props.onSave("pdf");
              setOpen(false);
            }}
          >
            Export as PDF
          </button>
          <button
            role="menuitem"
            onClick={() => {
              props.onSave("docx");
              setOpen(false);
            }}
          >
            Export as DOCX
          </button>
        </div>
      ) : null}
    </div>
  );
}

function MessageContent(props: {
  content: string;
  role: Message["role"];
  markdown: boolean;
  codeCopyButtons: boolean;
  onSaveCode: (kind: ArtifactKind, content: string, filenameHint?: string) => void;
  onPreview: (target: PreviewTarget) => void;
}) {
  if (props.role !== "assistant" || !props.markdown) {
    return <div className="message-content plain">{props.content}</div>;
  }

  const blocks = splitMarkdownBlocks(props.content);

  return (
    <div className="message-content markdown">
      {blocks.map((block, index) =>
        block.type === "code" ? (
          <CodeBlock
            code={block.content}
            language={block.language}
            showCopy={props.codeCopyButtons}
            onSaveCode={props.onSaveCode}
            onPreview={props.onPreview}
            key={`${block.type}-${index}`}
          />
        ) : (
          <div
            className="markdown-block"
            dangerouslySetInnerHTML={{ __html: renderMarkdown(block.content) }}
            key={`${block.type}-${index}`}
          />
        ),
      )}
    </div>
  );
}

function CodeBlock(props: {
  code: string;
  language: string | null;
  showCopy: boolean;
  onSaveCode: (kind: ArtifactKind, content: string, filenameHint?: string) => void;
  onPreview: (target: PreviewTarget) => void;
}) {
  const language = normalizeLanguage(props.language);
  const highlighted = highlightCode(props.code, language);
  const extension = (language && CODE_EXTENSIONS[language]) || "txt";
  const filename = `snippet.${extension}`;
  const download = () => props.onSaveCode("code", props.code, filename);

  return (
    <figure className="code-card">
      <figcaption>
        <span>{language ?? "text"}</span>
        <div className="code-card-actions">
          <button
            type="button"
            title="Preview"
            onClick={() => props.onPreview({ title: filename, language, code: props.code, onDownload: download })}
          >
            <Eye size={14} />
            Preview
          </button>
          {props.showCopy ? (
            <button type="button" title="Copy code" onClick={() => navigator.clipboard.writeText(props.code)}>
              <Copy size={14} />
              Copy
            </button>
          ) : null}
          <button type="button" title={`Download ${filename}`} onClick={download}>
            <Download size={14} />
            Download
          </button>
        </div>
      </figcaption>
      <pre>
        <code
          className={language ? `language-${language}` : undefined}
          dangerouslySetInnerHTML={{ __html: highlighted }}
        />
      </pre>
    </figure>
  );
}
