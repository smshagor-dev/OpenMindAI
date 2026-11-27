import type { VoiceDraftTarget } from "../lib/voiceDraft";
import { useEffect, useRef, useState } from "react";
import type { RefObject } from "react";
import {
  Code2,
  FileSearch,
  FileText,
  HardDrive,
  Image,
  Music2,
  ShieldCheck,
  Sparkles,
  Video,
  Volume2,
  WandSparkles,
} from "lucide-react";
import type {
  AppPreferences,
  Artifact,
  ArtifactKind,
  ImageRenderProgress,
  ImageGenerationOptions,
  Conversation,
  Message,
  ModelRecord,
  RuntimeInventory,
  WebResearchProgressEvent,
} from "../types";
import type { AttachmentDraft } from "../lib/chat";
import type { PdfExtractionProgress } from "../lib/pdf";
import type { CloudModel } from "../cloudApi";
import type { ExecutionMode } from "../lib/executionRouting";
import { MessageItem } from "./MessageItem";
import { ResearchProgress } from "./ResearchProgress";
import { Composer } from "./Composer";
import type { PreviewTarget } from "./PreviewPanel";

const SUGGESTIONS = [
  {
    id: "code",
    label: "Write or debug code",
    icon: Code2,
    prompt: "Help me write or debug code: ",
  },
  { id: "image", label: "Generate an image", icon: Image, prompt: "Generate an image of: " },
  { id: "video", label: "Generate a video", icon: Video, prompt: "Generate a video of: " },
  {
    id: "voice",
    label: "Generate voice",
    icon: Volume2,
    prompt: "Generate a voice narration for: ",
  },
  {
    id: "sound",
    label: "Generate music or SFX",
    icon: Music2,
    prompt: "Generate music or sound effects: ",
  },
  {
    id: "document",
    label: "Create a document or PDF",
    icon: FileText,
    prompt: "Create a document about: ",
  },
  { id: "file", label: "Analyze a local file", icon: FileSearch, prompt: "Analyze this file: " },
];

interface RealtimeActivity {
  typing: boolean;
  searching: boolean;
  researching: boolean;
  detail: string | null;
}

export function ChatView(props: {
  conversation: Conversation | null;
  messages: Message[];
  prompt: string;
  setPrompt: (value: string) => void;
  createVoiceDraftTarget: () => VoiceDraftTarget;
  imageOptions: ImageGenerationOptions;
  setImageOptions: (options: ImageGenerationOptions) => void;
  attachments: AttachmentDraft[];
  pdfImportProgress?: (PdfExtractionProgress & { fileName: string }) | null;
  onCancelPdfImport?: () => void;
  preferences: AppPreferences | null;
  activity: RealtimeActivity;
  webProgress: WebResearchProgressEvent | null;
  enterToSend: boolean;
  addFiles: (files: FileList | null) => void;
  removeAttachment: (id: string) => void;
  sendMessage: () => void;
  stopGeneration: () => void;
  regenerate: (assistantMessageId: string) => void;
  retry: (assistantMessageId: string) => void;
  editUserMessage: (messageId: string, content: string) => void;
  composerRef: RefObject<HTMLTextAreaElement | null>;
  streaming: boolean;
  submitting: boolean;
  backgroundBusy?: boolean;
  models: ModelRecord[];
  activeModelId: string | null;
  runtime: RuntimeInventory | null;
  modelSwitching: boolean;
  modelSwitchError: string | null;
  onSelectModel: (modelId: string) => void;
  executionMode?: ExecutionMode;
  showCloudModels?: boolean;
  cloudUnavailable?: boolean;
  onConnectCloud?: () => void;
  cloudModels?: CloudModel[];
  activeCloudModelId?: string;
  onSelectCloudModel?: (modelId: string) => void;
  artifactsByMessage: Map<string, Artifact[]>;
  onCreateArtifact: (
    messageId: string,
    kind: ArtifactKind,
    content: string,
    filenameHint?: string,
  ) => void;
  onOpenArtifact: (artifact: Artifact) => void;
  onRevealArtifact: (artifact: Artifact) => void;
  onRetryArtifact: (artifact: Artifact, useCpu?: boolean) => void;
  onCancelImageArtifact: (artifact: Artifact) => void;
  imageProgressByArtifact: Record<string, ImageRenderProgress>;
  onPreview: (target: PreviewTarget) => void;
}) {
  const latestMessageRef = useRef<globalThis.HTMLDivElement | null>(null);
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const stickToBottom = useRef(true);
  const isEmpty = props.messages.length === 0;
  const liveResearch =
    props.webProgress?.conversationId === props.conversation?.id &&
    (props.submitting || props.streaming || props.activity.typing)
      ? props.webProgress
      : null;

  useEffect(() => {
    if (stickToBottom.current) {
      latestMessageRef.current?.scrollIntoView({
        behavior: props.streaming ? "auto" : "smooth",
        block: "end",
      });
    }
  }, [
    props.messages,
    props.streaming,
    props.submitting,
    props.activity.typing,
    props.activity.searching,
    props.activity.researching,
    props.webProgress?.stage,
    props.webProgress?.completed,
  ]);

  const composer = (
    <Composer
      prompt={props.prompt}
      setPrompt={props.setPrompt}
      createVoiceDraftTarget={props.createVoiceDraftTarget}
      imageOptions={props.imageOptions}
      setImageOptions={props.setImageOptions}
      attachments={props.attachments}
      pdfImportProgress={props.pdfImportProgress}
      onCancelPdfImport={props.onCancelPdfImport}
      enterToSend={props.enterToSend}
      streaming={props.streaming}
      submitting={props.submitting}
      backgroundBusy={props.backgroundBusy}
      addFiles={props.addFiles}
      removeAttachment={props.removeAttachment}
      sendMessage={props.sendMessage}
      stopGeneration={props.stopGeneration}
      composerRef={props.composerRef}
      models={props.models}
      activeModelId={props.activeModelId}
      runtime={props.runtime}
      modelSwitching={props.modelSwitching}
      modelSwitchError={props.modelSwitchError}
      onSelectModel={props.onSelectModel}
      executionMode={props.executionMode}
      showCloudModels={props.showCloudModels}
      cloudUnavailable={props.cloudUnavailable}
      onConnectCloud={props.onConnectCloud}
      cloudModels={props.cloudModels}
      activeCloudModelId={props.activeCloudModelId}
      onSelectCloudModel={props.onSelectCloudModel}
      placeholder={isEmpty ? "Ask anything..." : "Message OpenMindAI..."}
      note={
        isEmpty
          ? undefined
          : props.cloudUnavailable
            ? "Cloud Only is waiting for a verified account and an available API model. No local inference will start."
            : props.executionMode === "cloud"
              ? "Running on OpenMindAI Cloud."
              : props.executionMode === "auto"
                ? "Auto: checks factual questions against online sources when available, then uses Cloud or Local."
                : "Running on installed local models on this computer."
      }
    />
  );

  return (
    <>
      <div
        className={isEmpty ? "messages messages-empty" : "messages"}
        ref={scrollRef}
        onScroll={(event) => {
          const node = event.currentTarget;
          const distanceFromBottom = node.scrollHeight - node.scrollTop - node.clientHeight;
          stickToBottom.current = distanceFromBottom < 120;
        }}
      >
        {isEmpty ? (
          <div className="chat-empty-state">
            <div className="chat-empty-inner">
              <div className="chat-empty-hero">
                <div className="chat-empty-brand">
                  <img src="/icon.png" alt="" />
                  <span>OpenMindAI</span>
                </div>
                <h2>What are we building today?</h2>
                <p>
                  Ask for code, documents, media, or project work. OpenMindAI keeps the workspace
                  close and turns your next instruction into action.
                </p>
                <div className="chat-empty-pills" aria-label="OpenMindAI workspace status">
                  <span>
                    <ShieldCheck size={14} /> Local-first
                  </span>
                  <span>
                    <WandSparkles size={14} /> File editing
                  </span>
                  <span>
                    <Sparkles size={14} /> Multimodal
                  </span>
                </div>
              </div>
              {composer}
              <div className="suggestion-list">
                {SUGGESTIONS.map((suggestion) => {
                  const Icon = suggestion.icon;
                  return (
                    <button
                      type="button"
                      className="suggestion-row-item"
                      key={suggestion.id}
                      onClick={() => {
                        props.setPrompt(suggestion.prompt);
                        props.composerRef.current?.focus();
                      }}
                    >
                      <Icon size={15} />
                      <span>{suggestion.label}</span>
                    </button>
                  );
                })}
              </div>
              <p className="chat-empty-footer">
                <HardDrive size={13} /> Local-first workspace - chats, models and artifacts remain
                under the portable root
              </p>
            </div>
          </div>
        ) : (
          props.messages.map((message) => (
            <MessageItem
              key={message.id}
              message={message}
              markdown={props.preferences?.markdownRendering ?? true}
              codeCopyButtons={props.preferences?.codeCopyButtons ?? true}
              canRegenerate={!props.streaming && !props.submitting && !props.backgroundBusy}
              onRegenerate={() => props.regenerate(message.id)}
              onRetry={() => props.retry(message.id)}
              artifacts={props.artifactsByMessage.get(message.id) ?? []}
              onCreateArtifact={(kind, content, filenameHint) =>
                props.onCreateArtifact(message.id, kind, content, filenameHint)
              }
              onOpenArtifact={props.onOpenArtifact}
              onRevealArtifact={props.onRevealArtifact}
              onRetryArtifact={props.onRetryArtifact}
              onCancelImageArtifact={props.onCancelImageArtifact}
              imageProgressByArtifact={props.imageProgressByArtifact}
              onPreview={props.onPreview}
              onEditUser={
                message.role === "user"
                  ? (content) => props.editUserMessage(message.id, content)
                  : undefined
              }
            />
          ))
        )}
        {liveResearch ? <ResearchProgress progress={liveResearch} /> : null}
        {!isEmpty &&
        !liveResearch &&
        (props.submitting ||
          props.streaming ||
          props.activity.typing ||
          props.activity.searching ||
          props.activity.researching) &&
        props.preferences?.typingIndicatorEnabled ? (
          <div className="activity-row">
            <span className="pulse-dot" />
            <span>
              {props.activity.researching ? (
                "Deep research running"
              ) : props.activity.searching ? (
                "Searching sources"
              ) : props.streaming ? (
                "Assistant is typing"
              ) : (
                <ThinkingStages />
              )}
            </span>
            {props.activity.detail ? <small>{props.activity.detail}</small> : null}
          </div>
        ) : null}
        <div className="latest-message-anchor" ref={latestMessageRef} />
      </div>
      {isEmpty ? null : composer}
    </>
  );
}

const THINKING_STAGES = ["Thinking", "Understanding", "Planning", "Writing the reply"];

/**
 * Staged "thinking" indicator shown while a reply is being produced — notably a
 * Cloud request, which returns in one shot with no token streaming. It only
 * fills the real wait: it advances through the stages on a timer and holds on
 * the last one until the response arrives, so it never adds artificial delay.
 */
function ThinkingStages() {
  const [stage, setStage] = useState(0);
  useEffect(() => {
    const timer = window.setInterval(
      () => setStage((prev) => (prev < THINKING_STAGES.length - 1 ? prev + 1 : prev)),
      1100,
    );
    return () => window.clearInterval(timer);
  }, []);
  return (
    <span className="thinking-stages">
      <span className="thinking-stage-label">{THINKING_STAGES[stage]}</span>
      <span className="thinking-ellipsis" aria-hidden="true">
        <i />
        <i />
        <i />
      </span>
    </span>
  );
}
