import { invoke } from "@tauri-apps/api/core";
import type { Message } from "./types";
import { createSoundscapeArtifact } from "./lib/media";
import { api as legacyApi } from "./api_legacy";

const isTauri = "__TAURI_INTERNALS__" in window;

type VisualMedia = {
  kind: "image";
  name: string;
  mimeType: "image/png" | "image/jpeg";
  dataUrl: string;
};

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
};
