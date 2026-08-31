import { invoke } from "@tauri-apps/api/core";
import type { Artifact, LlamaRuntimeStatus, Message, RuntimeInventory } from "./types";
import type {
  ConnectedProvider,
  GoogleWorkspaceStatus,
  IntegrationStatus,
} from "./lib/connectedActions";
import { createSoundscapeArtifact } from "./lib/media";
import { getPlatformCapabilities, type PlatformCapabilities } from "./lib/platform";
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

export type MobileModelRecommendation = {
  supported: boolean;
  tier: "nano" | "swift" | "core";
  modelId: string;
  name: string;
  repository: string;
  quantization: string;
  sizeBytes: number;
  totalRamBytes: number;
  installed: boolean;
  installedModelPath: string | null;
  reason: string;
};

export type MobileVisionStatus = {
  supported: boolean;
  installed: boolean;
  modelId: string;
  modelName: string;
  reason: string;
};

function connectedInvoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri) {
    return Promise.reject(new Error("Connected app actions require the OpenMindAI native app."));
  }
  return invoke<T>(command, args);
}

async function isMobileNativeTarget() {
  if (!isTauri) return false;
  const capabilities = await getPlatformCapabilities();
  return capabilities.target === "android" || capabilities.target === "ios";
}

function embeddedMobileRuntime(capabilities: PlatformCapabilities): RuntimeInventory {
  const selected: NonNullable<RuntimeInventory["selected"]> = {
    manifest: {
      runtimeName: "OpenMindAI Embedded llama.cpp",
      version: "native",
      platform: capabilities.target,
      architecture: "mobile",
      backend: capabilities.target === "ios" ? "metal" : "cpu",
      source: "embedded",
      installedAt: "bundled",
      binaries: { server: null, cli: null, bench: null },
      checksum: null,
      status: "ready",
    },
    serverExists: false,
    cliExists: false,
    benchExists: false,
    versionOutput: "Embedded llama.cpp mobile runtime",
    deviceOutput:
      "Native in-process mobile inference with task-aware text and multimodal routing.",
    usable: true,
    message: "Embedded on-device runtime ready",
  };

  return {
    runtimes: [selected],
    selected,
    serverState: "ready",
  };
}

async function mobileAwareRuntimeInventory(): Promise<RuntimeInventory> {
  const capabilities = await getPlatformCapabilities();
  if (capabilities.mobile && capabilities.mobileModelRuntimeReady) {
    return embeddedMobileRuntime(capabilities);
  }
  return legacyApi.runtimeInventory();
}

async function mobileAwareRuntimeStatus(): Promise<LlamaRuntimeStatus> {
  const capabilities = await getPlatformCapabilities();
  if (capabilities.mobile && capabilities.mobileModelRuntimeReady) {
    const inventory = embeddedMobileRuntime(capabilities);
    return {
      available: true,
      backend: inventory.selected?.manifest.backend ?? null,
      endpoint: null,
      state: "ready",
      selectedRuntime: inventory.selected,
    };
  }
  return legacyApi.runtimeStatus();
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

function mobileTextMode(mode: string) {
  if (mode === "document" || mode === "pdf") return "chat";
  return mode;
}

async function sendMobileLocalChat(
  conversationId: string,
  content: string,
  mode: string,
): Promise<Message> {
  const resolvedMode = mobileTextMode(mode);
  if (resolvedMode !== "chat" && resolvedMode !== "thinking") {
    throw new Error(
      `Mobile local ${mode} needs a specialized route. Install/enable the matching capability or use a connected provider.`,
    );
  }
  return invoke<Message>("mobile_send_chat_message", {
    conversationId,
    content,
    mode: resolvedMode,
  });
}

async function regenerateMobileLocalChat(
  conversationId: string,
  assistantMessageId: string,
  mode: string,
): Promise<Message> {
  const resolvedMode = mobileTextMode(mode);
  if (resolvedMode !== "chat" && resolvedMode !== "thinking") {
    throw new Error(`Mobile local regeneration for ${mode} needs a specialized route.`);
  }
  return invoke<Message>("mobile_regenerate_message", {
    conversationId,
    assistantMessageId,
    mode: resolvedMode,
  });
}

export const api = {
  ...legacyApi,
  runtimeInventory: mobileAwareRuntimeInventory,
  runtimeStatus: mobileAwareRuntimeStatus,
  startRuntime: mobileAwareRuntimeStatus,
  stopRuntime: mobileAwareRuntimeStatus,
  activateModel: async (conversationId: string, modelId: string) => {
    if (await isMobileNativeTarget()) {
      await legacyApi.setConversationModel(conversationId, modelId);
      return mobileAwareRuntimeStatus();
    }
    return legacyApi.activateModel(conversationId, modelId);
  },
  projectAgentStatus,
  mobileModelRecommendation: () =>
    connectedInvoke<MobileModelRecommendation>("mobile_model_recommendation"),
  mobileVisionStatus: () => connectedInvoke<MobileVisionStatus>("mobile_vision_status"),
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
    } else if (await isMobileNativeTarget()) {
      if (media.length > 0 || mode === "vision") {
        if (media.length === 0) {
          throw new Error("Attach at least one image, PDF page, or video frame for Vision mode.");
        }
        assistant = await invoke<Message>("mobile_send_vision_message", {
          conversationId,
          content,
          media,
        });
      } else {
        assistant = await sendMobileLocalChat(conversationId, content, mode);
      }
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
    } else if (await isMobileNativeTarget()) {
      if (isVisualTurn) {
        assistant = await invoke<Message>("mobile_regenerate_vision_message", {
          conversationId,
          assistantMessageId,
        });
      } else {
        assistant = await regenerateMobileLocalChat(
          conversationId,
          assistantMessageId,
          resolvedMode,
        );
      }
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
