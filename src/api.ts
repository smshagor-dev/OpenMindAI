import { invoke } from "@tauri-apps/api/core";
import type { Artifact, Message } from "./types";
import type {
  ConnectedProvider,
  GoogleWorkspaceStatus,
  IntegrationStatus,
} from "./lib/connectedActions";
import { createSoundscapeArtifact } from "./lib/media";
import { api as legacyApi } from "./api_legacy";

const isTauri = "__TAURI_INTERNALS__" in window;

type VisualMedia = {
  kind: "image";
  name: string;
  mimeType: "image/png" | "image/jpeg";
  dataUrl: string;
};

type EcosystemProvider = Exclude<ConnectedProvider, "google" | "github">;

type ProjectAgentStatus = {
  available: boolean;
  projectId: string | null;
  projectName: string | null;
  fullPcAccess: boolean;
  terminalEnabled: boolean;
  attachedRoots: number;
};

function connectedInvoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri) {
    return Promise.reject(new Error("Connected app actions require the OpenMindAI desktop app."));
  }
  return invoke<T>(command, args);
}

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

async function projectAgentStatus(conversationId: string): Promise<ProjectAgentStatus | null> {
  if (!isTauri) return null;
  try {
    return await invoke<ProjectAgentStatus>("project_agent_status_for_conversation", {
      conversationId,
    });
  } catch {
    return null;
  }
}

async function shouldUseProjectAgent(conversationId: string, mode: string) {
  if (!isTauri || (mode !== "chat" && mode !== "thinking")) return false;
  const status = await projectAgentStatus(conversationId);
  return status?.available ?? false;
}

export const api = {
  ...legacyApi,
  projectAgentStatus,
  sendChatMessage: async (
    conversationId: string,
    content: string,
    mode: string,
    media: VisualMedia[] = [],
  ) => {
    let assistant: Message;
    if (await shouldUseProjectAgent(conversationId, mode)) {
      assistant = await invoke<Message>("send_project_agent_message", {
        conversationId,
        content,
      });
    } else if (mode === "vision" && media.length > 0 && isTauri) {
      assistant = await invoke<Message>("send_multimodal_chat_message", {
        conversationId,
        content,
        mode,
        media,
      });
    } else {
      assistant = await legacyApi.sendChatMessage(conversationId, content, mode, media);
    }

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
        source?.content.startsWith("[Mode: Image/Vision Review]") ||
        source?.content.includes("[Image attached:") ||
        source?.content.includes("[PDF processed locally:") ||
        source?.content.includes("[Video processed locally:"),
    );
    if (isVisualTurn) resolvedMode = "vision";

    let assistant: Message;
    if (
      !isVisualTurn &&
      (resolvedMode === "chat" || resolvedMode === "thinking") &&
      (await shouldUseProjectAgent(conversationId, resolvedMode))
    ) {
      assistant = await invoke<Message>("regenerate_project_agent_message", {
        conversationId,
        assistantMessageId,
      });
    } else if (isVisualTurn && isTauri) {
      assistant = await invoke<Message>("regenerate_multimodal_message", {
        conversationId,
        assistantMessageId,
      });
    } else {
      assistant = await legacyApi.regenerateMessage(
        conversationId,
        assistantMessageId,
        resolvedMode,
      );
    }

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
  googleWorkspaceStatus: () =>
    connectedInvoke<GoogleWorkspaceStatus>("google_workspace_status"),
  connectGoogleWorkspace: () =>
    connectedInvoke<GoogleWorkspaceStatus>("connect_google_workspace"),
  disconnectGoogleWorkspace: () =>
    connectedInvoke<void>("disconnect_google_workspace"),
  executeGoogleWorkspaceAction: (
    action: string,
    params: Record<string, unknown>,
    approved = false,
  ) =>
    connectedInvoke<unknown>("execute_google_workspace_action", {
      action,
      params,
      approved,
    }),
  executeGithubWorkspaceAction: (
    action: string,
    params: Record<string, unknown>,
    approved = false,
  ) =>
    connectedInvoke<unknown>("execute_github_workspace_action", {
      action,
      params,
      approved,
    }),
  integrationStatus: (provider: EcosystemProvider) =>
    connectedInvoke<IntegrationStatus>("integration_status", { provider }),
  saveIntegrationConfig: (
    provider: EcosystemProvider,
    config: Record<string, unknown>,
    secret?: string,
  ) =>
    connectedInvoke<IntegrationStatus>("save_integration_config", {
      provider,
      config,
      secret: secret ?? null,
    }),
  clearIntegrationConfig: (provider: EcosystemProvider) =>
    connectedInvoke<void>("clear_integration_config", { provider }),
  connectIntegration: (provider: EcosystemProvider, token?: string) =>
    connectedInvoke<IntegrationStatus>("connect_integration", {
      provider,
      token: token?.trim() ? token.trim() : null,
    }),
  disconnectIntegration: (provider: EcosystemProvider) =>
    connectedInvoke<void>("disconnect_integration", { provider }),
  executeIntegrationAction: (
    provider: EcosystemProvider,
    action: string,
    params: Record<string, unknown>,
    approved = false,
  ) =>
    connectedInvoke<unknown>("execute_integration_action", {
      provider,
      action,
      params,
      approved,
    }),
};
