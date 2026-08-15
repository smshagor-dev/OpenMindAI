import { useEffect, useRef, useState } from "react";
import type { RefObject } from "react";
import { Code2, FileSearch, FileText, HardDrive, Image } from "lucide-react";
import type {
  AppPreferences,
  Artifact,
  ArtifactKind,
  Conversation,
  Message,
  ModelRecord,
  RuntimeInventory,
} from "../types";
import type { AttachmentDraft } from "../lib/chat";
import { MessageItem } from "./MessageItem";
import { Composer } from "./Composer";
import type { PreviewTarget } from "./PreviewPanel";

const SUGGESTIONS = [
  { id: "code", label: "Write or debug code", icon: Code2, prompt: "Help me write or debug code: " },
  { id: "image", label: "Generate an image", icon: Image, prompt: null },
  { id: "document", label: "Create a document or PDF", icon: FileText, prompt: "Create a document about: " },
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
  attachments: AttachmentDraft[];
  preferences: AppPreferences | null;
  activity: RealtimeActivity;
  enterToSend: boolean;
  addFiles: (files: FileList | null) => void;
  removeAttachment: (id: string) => void;
  sendMessage: () => void;
  stopGeneration: () => void;
  regenerate: (assistantMessageId: string) => void;
  retry: (assistantMessageId: string) => void;
  composerRef: RefObject<HTMLTextAreaElement>;
  streaming: boolean;
  models: ModelRecord[];
  activeModelId: string | null;
  runtime: RuntimeInventory | null;
  modelSwitching: boolean;
  modelSwitchError: string | null;
  onSelectModel: (modelId: string) => void;
  artifactsByMessage: Map<string, Artifact[]>;
  onCreateArtifact: (messageId: string, kind: ArtifactKind, content: string, filenameHint?: string) => void;
  onOpenArtifact: (artifact: Artifact) => void;
  onRevealArtifact: (artifact: Artifact) => void;
  onRetryArtifact: (artifact: Artifact) => void;
  onPreview: (target: PreviewTarget) => void;
}) {
  const latestMessageRef = useRef<globalThis.HTMLDivElement | null>(null);
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const stickToBottom = useRef(true);
  const isEmpty = props.messages.length === 0;
  const [imageNote, setImageNote] = useState(false);

  useEffect(() => {
    if (stickToBottom.current) {
      latestMessageRef.current?.scrollIntoView({ behavior: "smooth", block: "end" });
    }
  }, [props.messages, props.streaming, props.activity.typing, props.activity.searching, props.activity.researching]);

  const composer = (
    <Composer
      prompt={props.prompt}
      setPrompt={props.setPrompt}
      attachments={props.attachments}
      enterToSend={props.enterToSend}
      streaming={props.streaming}
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
      placeholder={isEmpty ? "Ask anything..." : "Message OpenMindAI..."}
      note={isEmpty ? undefined : "Responses are generated locally on your machine."}
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
              <h2>Where should we begin?</h2>
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
                        if (suggestion.prompt === null) {
                          setImageNote((value) => !value);
                          return;
                        }
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
              {imageNote ? (
                <p className="chat-empty-note">
                  Local image generation isn&rsquo;t installed yet — it needs its own downloaded model and runtime.
                </p>
              ) : null}
              <p className="chat-empty-footer">
                <HardDrive size={13} /> Local-first workspace · chats, models and artifacts remain under the
                portable root
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
              canRegenerate={!props.streaming}
              onRegenerate={() => props.regenerate(message.id)}
              onRetry={() => props.retry(message.id)}
              artifacts={props.artifactsByMessage.get(message.id) ?? []}
              onCreateArtifact={(kind, content, filenameHint) =>
                props.onCreateArtifact(message.id, kind, content, filenameHint)
              }
              onOpenArtifact={props.onOpenArtifact}
              onRevealArtifact={props.onRevealArtifact}
              onRetryArtifact={props.onRetryArtifact}
              onPreview={props.onPreview}
            />
          ))
        )}
        {!isEmpty &&
        (props.streaming || props.activity.typing || props.activity.searching || props.activity.researching) &&
        props.preferences?.typingIndicatorEnabled ? (
          <div className="activity-row">
            <span className="pulse-dot" />
            <span>
              {props.activity.researching
                ? "Deep research running"
                : props.activity.searching
                  ? "Searching sources"
                  : props.activity.typing || props.streaming
                    ? "Assistant is typing"
                    : "Working"}
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
