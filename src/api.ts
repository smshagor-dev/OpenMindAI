import { invoke } from "@tauri-apps/api/core";
import type {
  Artifact,
  ArtifactKind,
  Conversation,
  AppPreferences,
  DownloadStatus,
  GithubAccount,
  GithubIssue,
  GithubRepo,
  GoogleCredentialsStatus,
  HardwareProfile,
  InstallationStatus,
  SetupState,
  StorageLocationCheck,
  RuntimeInstallStatus,
  DiagnosticReport,
  RepairSummary,
  BackupInfo,
  CacheClearResult,
  ModelCatalogReport,
  LaunchPlan,
  LibraryEntry,
  LlamaRuntimeStatus,
  Message,
  ModelRecord,
  PerformanceProfile,
  PortableRootInfo,
  Project,
  ProjectFile,
  RuntimeInventory,
  StorageSummary,
  UserProfile,
} from "./types";

const isTauri = "__TAURI_INTERNALS__" in window;
const CHAT_FAILURE_RECOVERY_CLOCK_SKEW_MS = 250;

interface CachedMessageMeta {
  conversationId: string;
  role: Message["role"];
  order: number;
}

const cachedMessageMeta = new Map<string, CachedMessageMeta>();
const pendingMessageDeletes = new Set<string>();
let pendingDeleteTimer: number | null = null;
let pendingDeleteFlush: Promise<void> | null = null;

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri) {
    return browserFallback<T>(command, args);
  }

  const startedAt = Date.now();
  try {
    return await invoke<T>(command, args);
  } catch (error) {
    const conversationId = args?.conversationId;
    if (
      typeof conversationId === "string" &&
      (command === "send_chat_message" || command === "regenerate_message")
    ) {
      await recoverRecentFailedGeneration(conversationId, startedAt).catch(() => undefined);
    }
    throw error;
  }
}

async function recoverRecentFailedGeneration(conversationId: string, startedAt: number) {
  const messages = await invoke<Message[]>("list_messages", { conversationId });
  const cutoff = startedAt - CHAT_FAILURE_RECOVERY_CLOCK_SKEW_MS;
  const failedAttempt = messages
    .slice()
    .reverse()
    .find((message) => {
      if (message.role !== "assistant") return false;
      if (message.status !== "streaming" && message.status !== "pending") return false;
      const createdAt = Date.parse(message.createdAt);
      return Number.isFinite(createdAt) && createdAt >= cutoff;
    });

  if (!failedAttempt) return;
  await invoke<void>("complete_message", {
    messageId: failedAttempt.id,
    status: "failed",
  });
}

function rememberMessages(conversationId: string, messages: Message[]) {
  messages.forEach((message, order) => {
    cachedMessageMeta.set(message.id, {
      conversationId,
      role: message.role,
      order,
    });
  });
}

function queueMessageDelete(messageId: string) {
  pendingMessageDeletes.add(messageId);
  if (pendingDeleteTimer === null) {
    pendingDeleteTimer = window.setTimeout(() => {
      pendingDeleteTimer = null;
      void flushPendingMessageDeletes().catch((error) => {
        console.error("Failed to flush queued message deletion", error);
      });
    }, 0);
  }
  // Edit/resend currently awaits every delete call. Resolve immediately so
  // all branch IDs can be collected in one event-loop turn; send/regenerate
  // explicitly flush the queue before creating any replacement message.
  return Promise.resolve();
}

async function flushPendingMessageDeletes(): Promise<void> {
  if (pendingDeleteTimer !== null) {
    window.clearTimeout(pendingDeleteTimer);
    pendingDeleteTimer = null;
  }
  if (pendingDeleteFlush) {
    await pendingDeleteFlush;
  }
  if (pendingMessageDeletes.size === 0) return;

  const ids = Array.from(pendingMessageDeletes);
  pendingMessageDeletes.clear();
  const work = async () => {
    const byConversation = new Map<string, Array<{ id: string; meta: CachedMessageMeta }>>();
    const unknownIds: string[] = [];

    for (const id of ids) {
      const meta = cachedMessageMeta.get(id);
      if (!meta) {
        unknownIds.push(id);
        continue;
      }
      const group = byConversation.get(meta.conversationId) ?? [];
      group.push({ id, meta });
      byConversation.set(meta.conversationId, group);
    }

    for (const group of byConversation.values()) {
      const userTurns = group
        .filter((entry) => entry.meta.role === "user")
        .sort((left, right) => left.meta.order - right.meta.order);
      if (userTurns.length > 0) {
        // The repository deletes this user turn plus every later non-system
        // turn atomically. Choosing the earliest queued user collapses the UI's
        // reverse delete loop into one durable branch truncation.
        await call<void>("delete_message", { messageId: userTurns[0].id });
        continue;
      }
      for (const entry of group) {
        await call<void>("delete_message", { messageId: entry.id });
      }
    }

    for (const id of unknownIds) {
      await call<void>("delete_message", { messageId: id });
    }
  };

  const current = work();
  pendingDeleteFlush = current;
  try {
    await current;
  } finally {
    if (pendingDeleteFlush === current) pendingDeleteFlush = null;
  }

  if (pendingMessageDeletes.size > 0) {
    await flushPendingMessageDeletes();
  }
}

export const api = {
  root: () => call<PortableRootInfo>("get_portable_root"),
  installationStatus: () => call<InstallationStatus>("installation_status"),
  completeSetup: (root: string, profileName: string) =>
    call<PortableRootInfo>("complete_setup", { root, profileName }),
  saveSetupProgress: (root: string, profileName: string, state: SetupState) =>
    call<void>("save_setup_progress", { root, profileName, state }),
  markRuntimeReady: () => call<void>("mark_runtime_ready"),
  markModelReady: () => call<void>("mark_model_ready"),
  checkStorageLocation: (path: string) =>
    call<StorageLocationCheck>("check_storage_location", { path }),
  conversations: () => call<Conversation[]>("list_conversations"),
  createConversation: (title?: string) => call<Conversation>("create_conversation", { title }),
  renameConversation: (id: string, title: string) =>
    call<void>("rename_conversation", { id, title }),
  pinConversation: (id: string, pinned: boolean) =>
    call<void>("set_conversation_pinned", { id, pinned }),
  setConversationModel: (id: string, modelId: string) =>
    call<Conversation>("set_conversation_model", { id, modelId }),
  activateModel: (conversationId: string, modelId: string) =>
    call<LlamaRuntimeStatus>("activate_model", { conversationId, modelId }),
  archiveConversation: (id: string) => call<void>("archive_conversation", { id }),
  deleteConversation: async (id: string) => {
    await flushPendingMessageDeletes();
    return call<void>("delete_conversation", { id });
  },
  messages: async (conversationId: string) => {
    const messages = await call<Message[]>("list_messages", { conversationId });
    rememberMessages(conversationId, messages);
    return messages;
  },
  addUserMessage: (conversationId: string, content: string) =>
    call<Message>("add_user_message", { conversationId, content }),
  createStreamingAssistantMessage: (conversationId: string) =>
    call<Message>("create_streaming_assistant_message", { conversationId }),
  appendMessageChunk: (messageId: string, chunk: string) =>
    call<void>("append_message_chunk", { messageId, chunk }),
  completeMessage: (messageId: string, status: Message["status"]) =>
    call<void>("complete_message", { messageId, status }),
  deleteMessage: (messageId: string) => queueMessageDelete(messageId),
  projects: () => call<Project[]>("list_projects"),
  createProject: (name: string) => call<Project>("create_project", { name }),
  updateProject: (projectId: string, name: string, instructions: string) =>
    call<Project>("update_project", { projectId, name, instructions }),
  deleteProject: (projectId: string) => call<void>("delete_project", { projectId }),
  linkProjectConversation: (projectId: string, conversationId: string) =>
    call<void>("link_project_conversation", { projectId, conversationId }),
  unlinkProjectConversation: (projectId: string, conversationId: string) =>
    call<void>("unlink_project_conversation", { projectId, conversationId }),
  addProjectFile: (
    projectId: string,
    name: string,
    sizeBytes: number,
    mimeType: string | null,
    contentText: string | null,
    status: ProjectFile["status"],
    error: string | null,
  ) =>
    call<ProjectFile>("add_project_file", {
      input: {
        projectId,
        name,
        sizeBytes,
        mimeType,
        contentText,
        status,
        error,
      },
    }),
  deleteProjectFile: (projectId: string, fileId: string) =>
    call<void>("delete_project_file", { projectId, fileId }),
  hardware: () => call<HardwareProfile>("detect_hardware"),
  performance: () => call<PerformanceProfile>("get_performance_profile"),
  models: () => call<ModelRecord[]>("discover_models"),
  qwenDownloadStatus: () => call<DownloadStatus>("get_qwen_download_status"),
  modelDownloadStatus: () => call<DownloadStatus>("get_model_download_status"),
  downloadQwenModel: () => call<DownloadStatus>("download_qwen_model"),
  downloadCatalogModel: (modelId: string) =>
    call<DownloadStatus>("download_catalog_model", { modelId }),
  cancelQwenDownload: () => call<DownloadStatus>("cancel_qwen_download"),
  cancelModelDownload: () => call<DownloadStatus>("cancel_model_download"),
  pauseModelDownload: () => call<DownloadStatus>("pause_model_download"),
  deleteCatalogModel: (modelId: string) => call<void>("delete_catalog_model", { modelId }),
  validateModel: (modelId: string) => call<ModelRecord>("validate_model", { modelId }),
  planModelLaunch: (modelId: string) => call<LaunchPlan>("plan_model_launch", { modelId }),
  storage: () => call<StorageSummary>("get_storage_summary"),
  clearCache: () => call<CacheClearResult>("clear_cache"),
  runDiagnostics: () => call<DiagnosticReport>("run_diagnostics"),
  repairInstallation: () => call<RepairSummary>("repair_installation"),
  backupDatabase: () => call<BackupInfo>("backup_database"),
  listBackups: () => call<BackupInfo[]>("list_backups"),
  openMaintenanceFolder: (folder: "logs" | "backups") =>
    call<void>("open_maintenance_folder", { folder }),
  readRecentLogs: () => call<string>("read_recent_logs"),
  checkModelUpdates: () => call<ModelCatalogReport>("check_model_updates"),
  runtimeStatus: () => call<LlamaRuntimeStatus>("get_llama_runtime_status"),
  runtimeInventory: () => call<RuntimeInventory>("get_llama_runtime_inventory"),
  runtimeInstallStatus: () => call<RuntimeInstallStatus>("get_runtime_install_status"),
  installRecommendedRuntime: () => call<RuntimeInstallStatus>("install_recommended_runtime"),
  cancelRuntimeInstall: () => call<RuntimeInstallStatus>("cancel_runtime_install"),
  startRuntime: () => call<LlamaRuntimeStatus>("start_llama_runtime"),
  stopRuntime: () => call<void>("stop_llama_runtime"),
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
    await flushPendingMessageDeletes();
    return call<Message>("send_chat_message", { conversationId, content, mode, media });
  },
  regenerateMessage: async (conversationId: string, assistantMessageId: string, mode: string) => {
    await flushPendingMessageDeletes();
    return call<Message>("regenerate_message", { conversationId, assistantMessageId, mode });
  },
  cancelGeneration: (conversationId: string) => call<void>("cancel_generation", { conversationId }),
  preferences: () => call<AppPreferences>("get_app_preferences"),
  savePreferences: (preferences: AppPreferences) =>
    call<AppPreferences>("save_app_preferences", { preferences }),
  userProfile: () => call<UserProfile>("get_user_profile"),
  saveUserProfile: (profile: UserProfile) => call<UserProfile>("save_user_profile", { profile }),
  githubAccount: () => call<GithubAccount | null>("get_github_account"),
  saveGithubToken: (token: string) => call<GithubAccount>("save_github_token", { token }),
  disconnectGithub: () => call<void>("disconnect_github"),
  listGithubRepos: () => call<GithubRepo[]>("list_github_repos"),
  listGithubIssues: (repoFullName: string) =>
    call<GithubIssue[]>("list_github_issues", { repoFullName }),
  googleCredentials: () => call<GoogleCredentialsStatus | null>("get_google_credentials"),
  saveGoogleCredentials: (clientId: string, clientSecret: string) =>
    call<GoogleCredentialsStatus>("save_google_credentials", { clientId, clientSecret }),
  clearGoogleCredentials: () => call<void>("clear_google_credentials"),
  createTextArtifact: (
    conversationId: string,
    messageId: string | null,
    kind: ArtifactKind,
    filename: string | null,
    content: string,
  ) =>
    call<Artifact>("create_text_artifact", { conversationId, messageId, kind, filename, content }),
  createDocumentArtifact: (
    conversationId: string,
    messageId: string | null,
    kind: ArtifactKind,
    filename: string | null,
    content: string,
    title: string | null,
  ) =>
    call<Artifact>("create_document_artifact", {
      conversationId,
      messageId,
      kind,
      filename,
      content,
      title,
    }),
  createGenerationArtifact: (
    conversationId: string,
    messageId: string | null,
    kind: "image" | "video" | "voice",
    prompt: string,
  ) => call<Artifact>("create_generation_artifact", { conversationId, messageId, kind, prompt }),
  listArtifacts: (conversationId: string) => call<Artifact[]>("list_artifacts", { conversationId }),
  listLibraryEntries: () => call<LibraryEntry[]>("list_library_entries"),
  openArtifact: (artifactId: string) => call<void>("open_artifact", { artifactId }),
  openExternalUrl: (url: string) => call<void>("open_external_url", { url }),
  revealArtifactInFolder: (artifactId: string) =>
    call<void>("reveal_artifact_in_folder", { artifactId }),
};

function browserFallback<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  const now = new Date().toISOString();
  if (command === "get_portable_root") {
    return Promise.resolve({
      root: "Development preview",
      mode: "developmentFallback",
      databasePath: "Tauri command unavailable in browser preview",
      modelsDir: "Development preview",
      logsDir: "Development preview",
      writable: false,
    } as T);
  }
  if (command === "installation_status") {
    return Promise.resolve({
      setupRequired: false,
      reason: "Tauri command unavailable in browser preview",
      root: null,
      pending: null,
    } as T);
  }
  if (command === "complete_setup") {
    return Promise.reject(new Error("Setup requires the desktop app."));
  }
  if (
    command === "save_setup_progress" ||
    command === "mark_runtime_ready" ||
    command === "mark_model_ready"
  ) {
    return Promise.resolve(undefined as T);
  }
  if (command === "check_storage_location") {
    return Promise.resolve({
      path: (args?.path as string) ?? "",
      writable: false,
      availableBytes: null,
      recommendedBytes: 6 * 1024 * 1024 * 1024,
    } as T);
  }
  if (command === "list_conversations") return Promise.resolve([] as T);
  if (command === "create_conversation") {
    return Promise.resolve({
      id: crypto.randomUUID(),
      title: (args?.title as string) || "New conversation",
      createdAt: now,
      updatedAt: now,
      archivedAt: null,
      pinned: false,
      activeModelId: null,
    } as T);
  }
  if (command === "list_messages") return Promise.resolve([] as T);
  if (command === "set_conversation_model") {
    return Promise.resolve({
      id: (args?.id as string) ?? crypto.randomUUID(),
      title: "New conversation",
      createdAt: now,
      updatedAt: now,
      archivedAt: null,
      pinned: false,
      activeModelId: (args?.modelId as string) ?? null,
    } as T);
  }
  if (command === "delete_message") return Promise.resolve(undefined as T);
  if (command === "list_projects") return Promise.resolve([] as T);
  if (command === "create_project" || command === "update_project") {
    return Promise.resolve({
      id: (args?.projectId as string) || crypto.randomUUID(),
      name: (args?.name as string) || "New project",
      instructions: (args?.instructions as string) || "",
      createdAt: now,
      updatedAt: now,
      conversationIds: [],
      files: [],
    } as T);
  }
  if (
    command === "delete_project" ||
    command === "link_project_conversation" ||
    command === "unlink_project_conversation" ||
    command === "delete_project_file"
  ) {
    return Promise.resolve(undefined as T);
  }
  if (command === "add_project_file") {
    const input = (args?.input as Record<string, unknown> | undefined) ?? {};
    return Promise.resolve({
      id: crypto.randomUUID(),
      projectId: (input.projectId as string) || "",
      name: (input.name as string) || "file",
      sizeBytes: (input.sizeBytes as number) || 0,
      mimeType: (input.mimeType as string) || null,
      contentText: (input.contentText as string) || null,
      status: (input.status as ProjectFile["status"]) || "tracked",
      error: (input.error as string) || null,
      addedAt: now,
    } as T);
  }
  if (
    command === "create_text_artifact" ||
    command === "create_document_artifact" ||
    command === "create_generation_artifact"
  ) {
    const kind = (args?.kind as string) ?? "text";
    const content = (args?.content as string) ?? "";
    const prompt = (args?.prompt as string) ?? "";
    const artifactKind = kind === "voice" ? "audio" : kind;
    return Promise.resolve({
      id: crypto.randomUUID(),
      conversationId: (args?.conversationId as string) ?? "",
      messageId: (args?.messageId as string) ?? null,
      name: (args?.filename as string) || `artifact.${artifactKind}`,
      path: `generated/files/artifact.${artifactKind}`,
      mimeType: "text/plain",
      kind: artifactKind,
      sizeBytes: content.length || prompt.length,
      pageCount: null,
      status: command === "create_generation_artifact" && kind !== "image" ? "failed" : "ready",
      error:
        command === "create_generation_artifact" && kind !== "image"
          ? "Local media runtime connector is not available in browser preview."
          : null,
      createdAt: now,
      updatedAt: now,
    } as T);
  }
  if (command === "list_artifacts" || command === "list_library_entries")
    return Promise.resolve([] as T);
  if (command === "open_artifact" || command === "reveal_artifact_in_folder") {
    return Promise.resolve(undefined as T);
  }
  if (command === "open_external_url") {
    window.open((args?.url as string) ?? "", "_blank", "noopener,noreferrer");
    return Promise.resolve(undefined as T);
  }
  if (command === "regenerate_message") {
    return Promise.resolve({
      id: crypto.randomUUID(),
      conversationId: (args?.conversationId as string) ?? "",
      role: "assistant",
      content: "",
      status: "streaming",
      modelId: null,
      createdAt: now,
      updatedAt: now,
    } as T);
  }
  if (command === "activate_model") {
    return Promise.resolve({
      available: false,
      backend: null,
      endpoint: null,
      state: "stopped",
      selectedRuntime: null,
    } as T);
  }
  if (command === "detect_hardware") {
    return Promise.resolve({
      os: navigator.platform,
      operatingSystem: navigator.platform,
      architecture: "browser-preview",
      cpu: {
        name: "Unavailable outside Tauri",
        physicalCores: null,
        logicalThreads: navigator.hardwareConcurrency,
      },
      memory: { totalBytes: 0, availableBytes: 0 },
      gpus: [],
      primaryGpu: null,
      recommendedInferenceGpu: null,
      backends: { cpu: true, cuda: false, vulkan: false, sycl: false, hip: false, metal: false },
    } as T);
  }
  if (command === "get_performance_profile") {
    return Promise.resolve({
      mode: "auto",
      recommendedBackend: "cpu",
      cpuThreads: 1,
      systemMemoryBudgetBytes: 0,
      vramBudgetBytes: null,
      mmap: true,
      flashAttention: false,
    } as T);
  }
  if (command === "discover_models") return Promise.resolve([] as T);
  if (
    command === "get_qwen_download_status" ||
    command === "get_model_download_status" ||
    command === "download_qwen_model" ||
    command === "download_catalog_model" ||
    command === "cancel_qwen_download" ||
    command === "cancel_model_download" ||
    command === "pause_model_download"
  ) {
    return Promise.resolve({
      modelId: (args?.modelId as string) ?? "qwen3-4b-q4km",
      name: "OpenMindAI Core",
      state: "queued",
      repo: "Qwen/Qwen3-4B-GGUF",
      quantization: "Q4_K_M",
      filename: "Qwen3-4B-Q4_K_M.gguf",
      downloadedBytes: 0,
      totalBytes: 2497280256,
      percentage: 0,
      speedBytesPerSec: null,
      destination: null,
      error: null,
    } as T);
  }
  if (command === "get_llama_runtime_inventory") {
    return Promise.resolve({ runtimes: [], selected: null, serverState: "stopped" } as T);
  }
  if (
    command === "get_runtime_install_status" ||
    command === "install_recommended_runtime" ||
    command === "cancel_runtime_install"
  ) {
    return Promise.resolve({
      state: "idle",
      backend: null,
      version: null,
      downloadedBytes: 0,
      totalBytes: null,
      percentage: null,
      speedBytesPerSec: null,
      error:
        command === "install_recommended_runtime"
          ? "Runtime install requires the desktop app."
          : null,
    } as T);
  }
  if (command === "get_llama_runtime_status" || command === "start_llama_runtime") {
    return Promise.resolve({
      available: false,
      backend: null,
      endpoint: null,
      state: "stopped",
      selectedRuntime: null,
    } as T);
  }
  if (command === "get_storage_summary") {
    return Promise.resolve({
      root: "Development preview",
      modelsBytes: 0,
      databaseBytes: 0,
      cacheBytes: 0,
      generatedBytes: 0,
      availableBytes: null,
    } as T);
  }
  if (command === "clear_cache") {
    return Promise.resolve({ bytesFreed: 0 } as T);
  }
  if (command === "delete_catalog_model") {
    return Promise.resolve(undefined as T);
  }
  if (command === "run_diagnostics") {
    return Promise.resolve({ checks: [] } as T);
  }
  if (command === "repair_installation") {
    return Promise.reject(new Error("Repair requires the desktop app."));
  }
  if (command === "backup_database") {
    return Promise.reject(new Error("Backup requires the desktop app."));
  }
  if (command === "list_backups") {
    return Promise.resolve([] as T);
  }
  if (command === "open_maintenance_folder") {
    return Promise.resolve(undefined as T);
  }
  if (command === "read_recent_logs") {
    return Promise.resolve("" as T);
  }
  if (command === "check_model_updates") {
    return Promise.resolve({
      entries: [
        {
          entry: {
            id: "qwen3-4b-q4km",
            name: "OpenMindAI Core",
            version: "1",
            family: "qwen",
            kind: "chat",
            runtime: "llama.cpp",
            repo: "Qwen/Qwen3-4B-GGUF",
            quantization: "Q4_K_M",
            required: true,
            capabilities: ["chat", "reasoning", "code"],
            sizeBytes: 2497280256,
            minRamBytes: 8589934592,
            minVramBytes: null,
            license: "apache-2.0",
            description: "Balanced everyday local brain for writing, coding, and reasoning.",
            download: {
              strategy: "singleFile",
              filenamePattern: "*.gguf",
              destinationDir: "models/llm/qwen/qwen3-4b",
              format: "gguf",
            },
          },
          installed: false,
          compatible: true,
          downloadSupported: true,
          installedPath: null,
          updateAvailable: false,
        },
      ],
    } as T);
  }
  if (command === "get_app_preferences" || command === "save_app_preferences") {
    return Promise.resolve({
      theme: "light",
      language: "English",
      compactSidebar: false,
      enterToSend: true,
      showShortcutHints: true,
      autoGenerateTitles: true,
      codeCopyButtons: true,
      markdownRendering: true,
      defaultPerformanceProfile: "Auto",
      telemetryEnabled: false,
      saveChatHistory: true,
      localRuntimeAutostart: false,
      confirmBeforeDelete: true,
      webSearchEnabled: true,
      deepResearchEnabled: true,
      thinkingModeEnabled: true,
      typingIndicatorEnabled: true,
      socketRealtimeEnabled: false,
      socketUrl: "http://127.0.0.1:3001",
      openArtifactsAfterGeneration: false,
      autoCheckAppUpdates: true,
      autoDownloadAppUpdates: false,
      notifyModelUpdates: true,
      autoDownloadModelUpdates: false,
      updateChannel: "Stable",
    } as T);
  }
  if (command === "get_user_profile") {
    return Promise.resolve({
      fullName: "",
      email: "",
      about: "",
      occupation: "",
      preferredName: "",
      responseStyle: "",
      customInstructions: "",
      avatarDataUrl: "",
    } as T);
  }
  if (command === "save_user_profile") {
    return Promise.resolve((args?.profile as T) ?? ({} as T));
  }
  if (command === "get_github_account") return Promise.resolve(null as T);
  if (command === "save_github_token") {
    return Promise.reject(new Error("GitHub connections require the desktop app."));
  }
  if (command === "disconnect_github") return Promise.resolve(undefined as T);
  if (command === "list_github_repos" || command === "list_github_issues")
    return Promise.resolve([] as T);
  if (command === "get_google_credentials") return Promise.resolve(null as T);
  if (command === "save_google_credentials") {
    return Promise.resolve({
      clientId: (args?.clientId as string) ?? "",
      hasSecret: Boolean(args?.clientSecret),
    } as T);
  }
  if (command === "clear_google_credentials") return Promise.resolve(undefined as T);
  return Promise.resolve(undefined as T);
}
