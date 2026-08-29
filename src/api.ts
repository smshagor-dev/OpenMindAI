import { invoke } from "@tauri-apps/api/core";
import type { Artifact, Message } from "./types";
import { createSoundscapeArtifact } from "./lib/media";
import { api as legacyApi } from "./api_legacy";

const isTauri = "__TAURI_INTERNALS__" in window;

type VisualMedia = {
  kind: "image";
  name: string;
  mimeType: "image/png" | "image/jpeg";
  dataUrl: string;
};

async function messageUsesSoundscape(conversationId: string, messageId: string | null) {
  if (!messageId) return false;
  const history = await legacyApi.messages(conversationId);
  const targetIndex = history.findIndex((message) => message.id === messageId);
  if (targetIndex < 0) return false;
  const target = history[targetIndex];
  if (target.role === "user") {
    return target.content.startsWith("[Mode: Music/SFX Creation]");
  }
  const source = history
    .slice(0, targetIndex)
    .reverse()
    .find((message) => message.role === "user");
  return source?.content.startsWith("[Mode: Music/SFX Creation]") ?? false;
}

export const api = {
  ...legacyApi,
  sendChatMessage: async (
    conversationId: string,
    content: string,
    mode: string,
    media: VisualMedia[] = [],
  ) => {
    const assistant =
      mode === "vision" && media.length > 0 && isTauri
        ? await invoke<Message>("send_multimodal_chat_message", {
            conversationId,
            content,
            mode,
            media,
          })
        : await legacyApi.sendChatMessage(conversationId, content, mode, media);

    if (mode === "sound" && isTauri) {
      await createSoundscapeArtifact(
        conversationId,
        assistant.id,
        assistant.content.trim() || content,
      );
    }
    return assistant;
  },
  regenerateMessage: async (
    conversationId: string,
    assistantMessageId: string,
    mode: string,
  ): Promise<Message> => {
    let resolvedMode = mode;
    const history = await legacyApi.messages(conversationId);
    const targetIndex = history.findIndex((message) => message.id === assistantMessageId);
    const source =
      targetIndex > 0
        ? history
            .slice(0, targetIndex)
            .reverse()
            .find((message) => message.role === "user")
        : null;

    if (source?.content.startsWith("[Mode: Music/SFX Creation]")) resolvedMode = "sound";
    const isVisualTurn = Boolean(
      source?.content.startsWith("[Mode: Multimodal Vision Review]") ||
        source?.content.startsWith("[Mode: Image/Vision Review]"),
    );
    if (isVisualTurn) resolvedMode = "vision";

    const assistant =
      isVisualTurn && isTauri
        ? await invoke<Message>("regenerate_multimodal_message", {
            conversationId,
            assistantMessageId,
          })
        : await legacyApi.regenerateMessage(
            conversationId,
            assistantMessageId,
            resolvedMode,
          );

    if (resolvedMode === "sound" && isTauri) {
      await createSoundscapeArtifact(
        conversationId,
        assistant.id,
        assistant.content.trim(),
      );
    }
    return assistant;
  },
  createGenerationArtifact: async (
    conversationId: string,
    messageId: string | null,
    kind: "image" | "video" | "voice",
    prompt: string,
  ): Promise<Artifact> => {
    if (kind === "voice" && isTauri && (await messageUsesSoundscape(conversationId, messageId))) {
      return createSoundscapeArtifact(conversationId, messageId, prompt);
    }
    return legacyApi.createGenerationArtifact(conversationId, messageId, kind, prompt);
  },
};
