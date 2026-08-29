import type { Message } from "./types";
import { createSoundscapeArtifact } from "./lib/media";
import { api as legacyApi } from "./api_legacy";

const isTauri = "__TAURI_INTERNALS__" in window;

export const api = {
  ...legacyApi,
  sendChatMessage: async (
    conversationId: string,
    content: string,
    mode: string,
    media: Array<{
      kind: "image";
      name: string;
      mimeType: "image/png" | "image/jpeg";
      dataUrl: string;
    }> = [],
  ) => {
    const assistant = await legacyApi.sendChatMessage(conversationId, content, mode, media);
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
    if (resolvedMode === "chat") {
      const history = await legacyApi.messages(conversationId);
      const targetIndex = history.findIndex((message) => message.id === assistantMessageId);
      const source = targetIndex > 0
        ? history.slice(0, targetIndex).reverse().find((message) => message.role === "user")
        : null;
      if (source?.content.startsWith("[Mode: Music/SFX Creation]")) resolvedMode = "sound";
    }
    const assistant = await legacyApi.regenerateMessage(
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
