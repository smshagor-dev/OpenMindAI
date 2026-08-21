import { useEffect, useRef, useState } from "react";
import {
  FileText,
  FileType,
  Hash,
  Image,
  Mic,
  Paperclip,
  Plus,
  Send,
  Square,
  Video,
  Volume2,
  X,
} from "lucide-react";
import type { ReactNode, RefObject } from "react";
import type { ModelRecord, RuntimeInventory } from "../types";
import { formatBytes } from "../lib/format";
import type { AttachmentDraft } from "../lib/chat";
import { api } from "../api";
import { ModelSelector } from "./ModelSelector";

export function Composer(props: {
  prompt: string;
  setPrompt: (value: string) => void;
  attachments: AttachmentDraft[];
  enterToSend: boolean;
  streaming: boolean;
  addFiles: (files: FileList | null) => void;
  removeAttachment: (id: string) => void;
  sendMessage: () => void;
  stopGeneration: () => void;
  composerRef: RefObject<HTMLTextAreaElement>;
  models: ModelRecord[];
  activeModelId: string | null;
  runtime: RuntimeInventory | null;
  modelSwitching: boolean;
  modelSwitchError: string | null;
  onSelectModel: (modelId: string) => void;
  placeholder?: string;
  note?: string;
}) {
  const textareaRef = props.composerRef;
  const fileInputRef = useRef<HTMLInputElement | null>(null);

  useEffect(() => {
    const node = textareaRef.current;
    if (!node) return;
    node.style.height = "auto";
    node.style.height = `${Math.min(node.scrollHeight, 180)}px`;
  }, [props.prompt, textareaRef]);

  const [micNote, setMicNote] = useState(false);

  const insertTemplate = (template: string) => {
    props.setPrompt(template);
    window.setTimeout(() => {
      const node = textareaRef.current;
      if (!node) return;
      node.focus();
      node.setSelectionRange(node.value.length, node.value.length);
    }, 0);
  };

  return (
    <form
      className="composer"
      onSubmit={(event) => {
        event.preventDefault();
        void props.sendMessage();
      }}
    >
      <div className="composer-box">
        {props.attachments.length ? (
          <div className="attachment-tray">
            {props.attachments.map((attachment) => (
              <span className="attachment-chip" key={attachment.id}>
                {attachment.kind === "image" ? (
                  <Image size={14} />
                ) : attachment.kind === "pdf" ? (
                  <FileText size={14} />
                ) : (
                  <Paperclip size={14} />
                )}
                <span>{attachment.name}</span>
                <small>{attachment.kind}</small>
                <small>{formatBytes(attachment.size)}</small>
                <button type="button" title="Remove attachment" onClick={() => props.removeAttachment(attachment.id)}>
                  <X size={13} />
                </button>
              </span>
            ))}
          </div>
        ) : null}
        <div className="composer-row">
          <ComposerTools
            onAttachFiles={() => fileInputRef.current?.click()}
            onCreateDocument={() => insertTemplate("Create a Word document about: ")}
            onCreatePdf={() => insertTemplate("Create a PDF document about: ")}
            onCreateMarkdown={() => insertTemplate("Create a Markdown document about: ")}
            onGenerateImage={() => insertTemplate("Generate an image of: ")}
            onGenerateVideo={() => insertTemplate("Generate a video of: ")}
            onGenerateVoice={() => insertTemplate("Generate a voice narration for: ")}
          />
          <input
            ref={fileInputRef}
            multiple
            type="file"
            className="hidden-file-input"
            onChange={(event) => {
              void props.addFiles(event.target.files);
              event.currentTarget.value = "";
            }}
          />
          <textarea
            ref={textareaRef}
            value={props.prompt}
            onChange={(event) => props.setPrompt(event.target.value)}
            onKeyDown={(event) => {
              if (props.enterToSend && event.key === "Enter" && !event.shiftKey) {
                event.preventDefault();
                void props.sendMessage();
              }
            }}
            placeholder={props.placeholder ?? "Ask anything..."}
            rows={1}
          />
          <div className="composer-trailing">
            <ModelSelector
              models={props.models}
              activeModelId={props.activeModelId}
              runtime={props.runtime}
              switching={props.modelSwitching}
              switchError={props.modelSwitchError}
              onSelect={props.onSelectModel}
            />
            <button
              type="button"
              title="Voice input"
              className="icon-button"
              onClick={() => setMicNote((value) => !value)}
            >
              <Mic size={18} />
            </button>
            {props.streaming ? (
              <button type="button" title="Stop" onClick={props.stopGeneration}>
                <Square size={18} />
              </button>
            ) : (
              <button type="submit" title="Send" className="composer-send">
                <Send size={18} />
              </button>
            )}
          </div>
        </div>
      </div>
      {micNote ? (
        <p className="composer-note">Voice input is not available yet. It needs a local speech-to-text runtime.</p>
      ) : null}
      {props.note ? <p className="composer-note">{props.note}</p> : null}
    </form>
  );
}

function ComposerTools(props: {
  onAttachFiles: () => void;
  onCreateDocument: () => void;
  onCreatePdf: () => void;
  onCreateMarkdown: () => void;
  onGenerateImage: () => void;
  onGenerateVideo: () => void;
  onGenerateVoice: () => void;
}) {
  const [open, setOpen] = useState(false);
  const [generationModels, setGenerationModels] = useState({
    image: false,
    video: false,
    voice: false,
  });
  const ref = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    let cancelled = false;
    void api
      .checkModelUpdates()
      .then((catalog) => {
        if (cancelled) return;
        setGenerationModels({
          image: catalog.entries.some((item) => item.installed && item.entry.kind === "image"),
          video: catalog.entries.some((item) => item.installed && item.entry.kind === "video"),
          voice: catalog.entries.some((item) => item.installed && item.entry.kind === "text-to-speech"),
        });
      })
      .catch(() => {
        if (!cancelled) setGenerationModels({ image: false, video: false, voice: false });
      });
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    if (!open) return;
    const onClickOutside = (event: MouseEvent) => {
      if (ref.current && !ref.current.contains(event.target as Node)) {
        setOpen(false);
      }
    };
    window.addEventListener("mousedown", onClickOutside);
    return () => window.removeEventListener("mousedown", onClickOutside);
  }, [open]);

  return (
    <div className="composer-tools" ref={ref}>
      <button
        type="button"
        className="icon-button"
        title="Tools"
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((value) => !value)}
      >
        <Plus size={18} />
      </button>
      {open ? (
        <div className="composer-tools-menu" role="menu">
          <button
            type="button"
            role="menuitem"
            onClick={() => {
              props.onAttachFiles();
              setOpen(false);
            }}
          >
            <span className="tool-menu-icon">
              <Paperclip size={16} />
            </span>
            <span className="tool-menu-text">
              <strong>Attach file</strong>
              <small>Upload from your computer</small>
            </span>
          </button>
          <button
            type="button"
            role="menuitem"
            onClick={() => {
              props.onCreateDocument();
              setOpen(false);
            }}
          >
            <span className="tool-menu-icon">
              <FileText size={16} />
            </span>
            <span className="tool-menu-text">
              <strong>Create Word document</strong>
              <small>Generate a .docx from your request</small>
            </span>
          </button>
          <button
            type="button"
            role="menuitem"
            onClick={() => {
              props.onCreatePdf();
              setOpen(false);
            }}
          >
            <span className="tool-menu-icon">
              <FileType size={16} />
            </span>
            <span className="tool-menu-text">
              <strong>Create PDF</strong>
              <small>Generate a formatted PDF</small>
            </span>
          </button>
          <button
            type="button"
            role="menuitem"
            onClick={() => {
              props.onCreateMarkdown();
              setOpen(false);
            }}
          >
            <span className="tool-menu-icon">
              <Hash size={16} />
            </span>
            <span className="tool-menu-text">
              <strong>Create Markdown</strong>
              <small>Generate a .md document</small>
            </span>
          </button>
          <GenerationTool
            enabled={generationModels.image}
            icon={<Image size={16} />}
            title="Generate image"
            description="Create with OpenMindAI Canvas"
            onClick={() => {
              props.onGenerateImage();
              setOpen(false);
            }}
          />
          <GenerationTool
            enabled={generationModels.video}
            icon={<Video size={16} />}
            title="Generate video"
            description="Create with OpenMindAI Motion"
            onClick={() => {
              props.onGenerateVideo();
              setOpen(false);
            }}
          />
          <GenerationTool
            enabled={generationModels.voice}
            icon={<Volume2 size={16} />}
            title="Generate voice"
            description="Create with OpenMindAI Speak"
            onClick={() => {
              props.onGenerateVoice();
              setOpen(false);
            }}
          />
        </div>
      ) : null}
    </div>
  );
}

function GenerationTool(props: {
  enabled: boolean;
  icon: ReactNode;
  title: string;
  description: string;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      role="menuitem"
      className={props.enabled ? "" : "composer-tools-disabled"}
      onClick={props.enabled ? props.onClick : undefined}
    >
      <span className="tool-menu-icon">{props.icon}</span>
      <span className="tool-menu-text">
        <strong>
          {props.title}
          {!props.enabled ? <small className="tool-menu-badge tool-menu-badge-required">Model download required</small> : null}
        </strong>
        <small>{props.enabled ? props.description : "Download the model from Settings > Models"}</small>
      </span>
    </button>
  );
}
