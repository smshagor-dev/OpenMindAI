import { invoke } from "@tauri-apps/api/core";
import type {
  Artifact,
  LlamaRuntimeStatus,
  Message,
  RuntimeInventory,
  RuntimeValidation,
} from "./types";
import type {
  ConnectedProvider,
  GoogleWorkspaceStatus,
  IntegrationStatus,
} from "./lib/connectedActions";
import { createSoundscapeArtifact } from "./lib/media";
import { api as legacyApi } from "./api_legacy";

const isTauri = "__TAURI_INTERNALS__" in window;
const RUNTIME_BACKGROUND_SCAN_DELAY_MS = 30_000;

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

type RuntimeBootstrapSnapshot = {
  inventory: RuntimeInventory;
  status: LlamaRuntimeStatus;
};

let runtimeForegroundStarted = false;
let runtimeScanTimer: number | null = null;
let runtimeInventoryCache: RuntimeInventory | null = null;
let runtimeScanPromise: Promise<RuntimeInventory> | null = null;
let runtimeBootstrapPromise: Promise<RuntimeBootstrapSnapshot> | null = null;

function connectedInvoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri) {
    return Promise.reject(new Error("Connected app actions require the OpenMindAI desktop app."));
  }
  return invoke<T>(command, args);
}

function cachedRuntimeValidation(): RuntimeValidation {
  return {
    manifest: {
      runtimeName: "OpenMindAI Runtime",
      version: "cached",
      platform: navigator.platform || "desktop",
      architecture: "cached",
      backend: "cpu",
      source: "saved installation",
      installedAt: "",
      binaries: {
        server: null,
        cli: null,
        bench: null,
      },
      checksum: null,
      status: "ready",
    },
    serverExists: true,
    cliExists: true,
    benchExists: false,
    versionOutput: null,
    deviceOutput: null,
    usable: true,
    message: "Saved runtime readiness; deep validation deferred until after startup.",
  };
}

function statusFromInventory(inventory: RuntimeInventory): LlamaRuntimeStatus {
  return {
    available: inventory.selected !== null,
    backend: inventory.selected?.manifest.backend ?? null,
    endpoint: null,
    state: inventory.serverState,
    selectedRuntime: inventory.selected,
  };
}

async function runtimeBootstrapSnapshot(): Promise<RuntimeBootstrapSnapshot> {
  if (runtimeBootstrapPromise) return runtimeBootstrapPromise;

  runtimeBootstrapPromise = legacyApi
    .installationStatus()
    .then((installation) => {
      if (installation.setupRequired) {
        const inventory: RuntimeInventory = {
          runtimes: [],
          selected: null,
          serverState: "stopped",
        };
        return {
          inventory,
          status: statusFromInventory(inventory),
        };
      }

      const selected = cachedRuntimeValidation();
      const inventory: RuntimeInventory = {
        runtimes: [selected],
        selected,
        serverState: "stopped",
      };
      return {
        inventory,
        status: statusFromInventory(inventory),
      };
    })
    .catch(() => {
      const inventory: RuntimeInventory = {
        runtimes: [],
        selected: null,
        serverState: "stopped",
      };
      return {
        inventory,
        status: statusFromInventory(inventory),
      };
    });

  return runtimeBootstrapPromise;
}

async function deepRuntimeInventory(): Promise<RuntimeInventory> {
  if (runtimeInventoryCache) return runtimeInventoryCache;
  if (runtimeScanPromise) return runtimeScanPromise;

  runtimeScanPromise = legacyApi
    .runtimeInventory()
    .then((inventory) => {
      runtimeInventoryCache = inventory;
      return inventory;
    })
    .finally(() => {
      runtimeScanPromise = null;
    });

  return runtimeScanPromise;
}

function markRuntimeForegroundStarted() {
  runtimeForegroundStarted = true;
  if (runtimeScanTimer !== null) {
    window.clearTimeout(runtimeScanTimer);
    runtimeScanTimer = null;
  }
}

function invalidateRuntimeSnapshot() {
  runtimeInventoryCache = null;
  runtimeBootstrapPromise = null;
}

function scheduleBackgroundRuntimeScan() {
  if (!isTauri || runtimeScanTimer !== null) return;
  runtimeScanTimer = window.setTimeout(() => {
    runtimeScanTimer = null;
    if (runtimeForegroundStarted || runtimeInventoryCache || runtimeScanPromise) return;

    const run = () => {
      if (runtimeForegroundStarted || runtimeInventoryCache || runtimeScanPromise) return;
      void deepRuntimeInventory().catch(() => undefined);
    };

    if ("requestIdleCallback" in window) {
      window.requestIdleCallback(run, { timeout: 10_000 });
    } else {
      run();
    }
  }, RUNTIME_BACKGROUND_SCAN_DELAY_MS);
}

scheduleBackgroundRuntimeScan();

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
  runtimeInventory: async () => {
    if (runtimeInventoryCache) return runtimeInventoryCache;
    return (await runtimeBootstrapSnapshot()).inventory;
  },
  runtimeStatus: async () => {
    if (runtimeInventoryCache) return statusFromInventory(runtimeInventoryCache);
    return (await runtimeBootstrapSnapshot()).status;
  },
  activateModel: async (conversationId: string, modelId: string) => {
    markRuntimeForegroundStarted();
    const status = await legacyApi.activateModel(conversationId, modelId);
    invalidateRuntimeSnapshot();
    return status;
  },
  startRuntime: async () => {
    markRuntimeForegroundStarted();
    const status = await legacyApi.startRuntime();
    invalidateRuntimeSnapshot();
    return status;
  },
  stopRuntime: async () => {
    markRuntimeForegroundStarted();
    await legacyApi.stopRuntime();
    invalidateRuntimeSnapshot();
  },
  projectAgentStatus,
  sendChatMessage: async (
    conversationId: string,
    content: string,
    mode: string,
    media: VisualMedia[] = [],
  ) => {
    markRuntimeForegroundStarted();
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
    markRuntimeForegroundStarted();
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
