import { useEffect, useRef, useState } from "react";
import {
  FileText,
  FileType,
  Hash,
  Image,
  Loader2,
  Mic,
  MicOff,
  Music2,
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
import { transcribeAudioBlob } from "../lib/media";
import { api } from "../api";
import { ModelSelector } from "./ModelSelector";

export function Composer(props: {
  prompt: string;
  setPrompt: (value: string) => void;
  attachments: AttachmentDraft[];
  enterToSend: boolean;
  streaming: boolean;
  submitting: boolean;
  addFiles: (files: FileList | null) => void | Promise<void>;
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
  const recorderRef = useRef<MediaRecorder | null>(null);
  const streamRef = useRef<MediaStream | null>(null);
  const chunksRef = useRef<Blob[]>([]);
  const audioPreviewUrlsRef = useRef(new Map<string, string>());
  const attachmentsRef = useRef(props.attachments);
  attachmentsRef.current = props.attachments;
  const [recording, setRecording] = useState(false);
  const [transcribing, setTranscribing] = useState(false);
  const [micError, setMicError] = useState<string | null>(null);
  const canSend = props.prompt.trim().length > 0 || props.attachments.length > 0;

  useEffect(() => {
    const node = textareaRef.current;
    if (!node) return;
    node.style.height = "auto";
    node.style.height = `${Math.min(node.scrollHeight, 180)}px`;
  }, [props.prompt, textareaRef]);

  useEffect(
    () => () => {
      if (recorderRef.current?.state === "recording") recorderRef.current.stop();
      streamRef.current?.getTracks().forEach((track) => track.stop());
      for (const url of audioPreviewUrlsRef.current.values()) URL.revokeObjectURL(url);
      audioPreviewUrlsRef.current.clear();
    },
    [],
  );

  useEffect(() => {
    pruneAudioPreviews(audioPreviewUrlsRef.current, props.attachments);
  }, [props.attachments]);

  const insertTemplate = (template: string) => {
    props.setPrompt(template);
    window.setTimeout(() => {
      const node = textareaRef.current;
      if (!node) return;
      node.focus();
      node.setSelectionRange(node.value.length, node.value.length);
    }, 0);
  };

  const stopMicrophone = () => {
    const recorder = recorderRef.current;
    if (recorder?.state === "recording") recorder.stop();
  };

  const toggleMicrophone = async () => {
    if (recording) {
      stopMicrophone();
      return;
    }
    if (transcribing || props.streaming || props.submitting) return;
    setMicError(null);
    if (!navigator.mediaDevices?.getUserMedia || typeof MediaRecorder === "undefined") {
      setMicError("Microphone recording is not available in this WebView.");
      return;
    }

    try {
      const stream = await navigator.mediaDevices.getUserMedia({
        audio: {
          channelCount: 1,
          echoCancellation: true,
          noiseSuppression: true,
          autoGainControl: true,
        },
      });
      streamRef.current = stream;
      chunksRef.current = [];
      const preferred = ["audio/webm;codecs=opus", "audio/webm", "audio/mp4"].find((mimeType) =>
        MediaRecorder.isTypeSupported(mimeType),
      );
      const recorder = preferred
        ? new MediaRecorder(stream, { mimeType: preferred })
        : new MediaRecorder(stream);
      recorderRef.current = recorder;
      recorder.ondataavailable = (event) => {
        if (event.data.size > 0) chunksRef.current.push(event.data);
      };
      recorder.onerror = () => {
        setMicError("Microphone recording failed.");
        setRecording(false);
        stream.getTracks().forEach((track) => track.stop());
      };
      recorder.onstop = () => {
        setRecording(false);
        stream.getTracks().forEach((track) => track.stop());
        streamRef.current = null;
        recorderRef.current = null;
        const chunks = chunksRef.current.slice();
        chunksRef.current = [];
        if (chunks.length === 0) return;
        const blob = new Blob(chunks, {
          type: recorder.mimeType || chunks[0].type || "audio/webm",
        });
        setTranscribing(true);
        void transcribeAudioBlob(blob, "microphone")
          .then((result) => {
            const transcript = result.text.trim();
            if (!transcript) return;
            const current = props.prompt.trim();
            props.setPrompt(current ? `${props.prompt.trimEnd()} ${transcript}` : transcript);
            window.setTimeout(() => textareaRef.current?.focus(), 0);
          })
          .catch((error) => {
            setMicError(error instanceof Error ? error.message : String(error));
          })
          .finally(() => setTranscribing(false));
      };
      recorder.start(500);
      setRecording(true);
    } catch (error) {
      setMicError(
        error instanceof Error
          ? `Microphone unavailable: ${error.message}`
          : "Microphone permission was not granted.",
      );
    }
  };

  const addSelectedFiles = async (files: FileList | null) => {
    if (!files) return;
    for (const file of Array.from(files)) {
      if (!isAudioPreviewFile(file)) continue;
      const key = attachmentPreviewKey(file);
      if (!audioPreviewUrlsRef.current.has(key)) {
        audioPreviewUrlsRef.current.set(key, URL.createObjectURL(file));
      }
    }
    try {
      await props.addFiles(files);
    } finally {
      window.setTimeout(() => {
        pruneAudioPreviews(audioPreviewUrlsRef.current, attachmentsRef.current);
      }, 0);
    }
  };

  const removeAttachment = (attachment: AttachmentDraft) => {
    const key = attachmentPreviewKey(attachment);
    const previewUrl = audioPreviewUrlsRef.current.get(key);
    if (previewUrl) {
      URL.revokeObjectURL(previewUrl);
      audioPreviewUrlsRef.current.delete(key);
    }
    props.removeAttachment(attachment.id);
  };

  return (
    <form
      className="composer"
      onSubmit={(event) => {
        event.preventDefault();
        if (!canSend || props.streaming || props.submitting || recording || transcribing) return;
        void props.sendMessage();
      }}
    >
      <div className="composer-box">
        {props.attachments.length ? (
          <div className="attachment-tray">
            {props.attachments.map((attachment) => {
              const audioPreview =
                attachment.kind === "audio"
                  ? audioPreviewUrlsRef.current.get(attachmentPreviewKey(attachment)) ?? null
                  : null;
              return (
                <span className="attachment-chip attachment-chip-media" key={attachment.id}>
                  {attachment.kind === "image" && attachment.mediaDataUrl ? (
                    <img
                      className="attachment-inline-image"
                      src={attachment.mediaDataUrl}
                      alt={attachment.name}
                    />
                  ) : attachment.kind === "image" ? (
                    <Image size={14} />
                  ) : attachment.kind === "pdf" ? (
                    <FileText size={14} />
                  ) : attachment.kind === "audio" ? (
                    <Volume2 size={14} />
                  ) : attachment.kind === "video" ? (
                    <Video size={14} />
                  ) : (
                    <Paperclip size={14} />
                  )}
                  <span>{attachment.name}</span>
                  <small>{attachment.kind}</small>
                  <small>{formatBytes(attachment.size)}</small>
                  {audioPreview ? (
                    <audio
                      className="attachment-inline-audio"
                      controls
                      preload="metadata"
                      src={audioPreview}
                    >
                      Your system cannot play this local audio format.
                    </audio>
                  ) : null}
                  <button
                    type="button"
                    title="Remove attachment"
                    onClick={() => removeAttachment(attachment)}
                  >
                    <X size={13} />
                  </button>
                </span>
              );
            })}
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
            onGenerateSound={() => insertTemplate("Generate music or sound effects: ")}
          />
          <input
            ref={fileInputRef}
            multiple
            type="file"
            accept="text/*,.md,.txt,.json,.csv,.ts,.tsx,.js,.jsx,.rs,.py,.html,.css,.sql,.toml,.yaml,.yml,application/pdf,image/png,image/jpeg,image/webp,audio/*,.wav,.mp3,.m4a,.aac,.flac,.ogg,.opus,video/*,.mp4,.webm,.mov,.m4v"
            className="hidden-file-input"
            onChange={(event) => {
              const files = event.currentTarget.files;
              event.currentTarget.value = "";
              void addSelectedFiles(files);
            }}
          />
          <textarea
            ref={textareaRef}
            value={props.prompt}
            onChange={(event) => props.setPrompt(event.target.value)}
            onKeyDown={(event) => {
              if (
                props.enterToSend &&
                event.key === "Enter" &&
                !event.shiftKey &&
                !props.streaming &&
                !props.submitting &&
                !recording &&
                !transcribing &&
                canSend
              ) {
                event.preventDefault();
                void props.sendMessage();
              }
            }}
            placeholder={
              recording
                ? "Listening..."
                : transcribing
                  ? "Transcribing locally..."
                  : (props.placeholder ?? "Ask anything...")
            }
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
              className={recording ? "icon-button mic-active" : "icon-button"}
              title={
                recording ? "Stop recording" : transcribing ? "Transcribing locally" : "Voice input"
              }
              aria-pressed={recording}
              disabled={transcribing || props.streaming || props.submitting}
              onClick={() => void toggleMicrophone()}
            >
              {transcribing ? (
                <Loader2 size={18} className="spin" />
              ) : recording ? (
                <MicOff size={18} />
              ) : (
                <Mic size={18} />
              )}
            </button>
            {props.streaming ? (
              <button type="button" title="Stop" onClick={props.stopGeneration}>
                <Square size={18} />
              </button>
            ) : (
              <button
                type="submit"
                title={props.submitting ? "Preparing request" : "Send"}
                className="composer-send"
                disabled={!canSend || props.submitting || recording || transcribing}
              >
                <Send size={18} />
              </button>
            )}
          </div>
        </div>
      </div>
      {micError ? (
        <p className="composer-note composer-note-error">{micError}</p>
      ) : props.note ? (
        <p className="composer-note">{props.note}</p>
      ) : null}
    </form>
  );
}

function attachmentPreviewKey(file: Pick<File, "name" | "size" | "type"> | AttachmentDraft) {
  return `${file.name}\u0000${file.size}\u0000${file.type}`;
}

function isAudioPreviewFile(file: File) {
  return (
    file.type.toLowerCase().startsWith("audio/") ||
    /\.(wav|mp3|m4a|aac|flac|ogg|opus)$/i.test(file.name)
  );
}

function pruneAudioPreviews(
  previews: Map<string, string>,
  attachments: AttachmentDraft[],
) {
  const active = new Set(
    attachments
      .filter((attachment) => attachment.kind === "audio")
      .map((attachment) => attachmentPreviewKey(attachment)),
  );
  for (const [key, url] of previews) {
    if (active.has(key)) continue;
    URL.revokeObjectURL(url);
    previews.delete(key);
  }
}

function ComposerTools(props: {
  onAttachFiles: () => void;
  onCreateDocument: () => void;
  onCreatePdf: () => void;
  onCreateMarkdown: () => void;
  onGenerateImage: () => void;
  onGenerateVideo: () => void;
  onGenerateVoice: () => void;
  onGenerateSound: () => void;
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
          voice: catalog.entries.some(
            (item) => item.installed && item.entry.kind === "text-to-speech",
          ),
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
              <small>Text, PDF, image, audio, or video</small>
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
          <GenerationTool
            enabled
            icon={<Music2 size={16} />}
            title="Generate music / SFX"
            description="Create offline with OpenMindAI Soundscape"
            onClick={() => {
              props.onGenerateSound();
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
          {!props.enabled ? (
            <small className="tool-menu-badge tool-menu-badge-required">Model download required</small>
          ) : null}
        </strong>
        <small>{props.enabled ? props.description : "Download the model from Settings > Models"}</small>
      </span>
    </button>
  );
}
