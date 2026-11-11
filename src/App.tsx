import { Archive, Check, Pencil, Pin, PinOff, Trash2 } from "lucide-react";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore, type SetStateAction } from "react";
import { io, type Socket } from "socket.io-client";
import { api } from "./api";
import type {
  Artifact,
  ArtifactKind,
  ImageGenerationOptions,
  ImageRenderProgress,
  Conversation,
  AppPreferences,
  HardwareProfile,
  InstallationStatus,
  LlamaRuntimeStatus,
  Message,
  ModelRecord,
  PerformanceProfile,
  PortableRootInfo,
  Project,
  RuntimeInventory,
  RuntimeInstallStatus,
  StreamChunkEvent,
  StreamDoneEvent,
  StreamStartedEvent,
  WebResearchProgressEvent,
  StorageSummary,
  UserProfile,
} from "./types";
import { Sidebar } from "./components/Sidebar";
import { ChatView } from "./components/ChatView";
import { SettingsDialog } from "./components/SettingsDialog";
import { ChatHistoryNotice } from "./components/ChatHistoryNotice";
import { SetupWizard } from "./components/SetupWizard";
import { ChatSearch } from "./components/ChatSearch";
import { ConfirmDialog } from "./components/ConfirmDialog";
import { ChatModeSwitcher } from "./components/ChatModeSwitcher";
import { Library } from "./components/Library";
import { ModelsManager } from "./components/ModelsManager";
import { DatasetsManager } from "./components/DatasetsManager";
import { TitleBarControls } from "./components/TitleBarControls";
import { StatusBar } from "./components/StatusBar";
import { PreviewPanel, type PreviewTarget } from "./components/PreviewPanel";
import { ToolsWorkspace } from "./components/ToolsWorkspace";
import { ProjectsWorkspace } from "./components/ProjectsWorkspace";
import { WorkWorkspace } from "./components/WorkWorkspace";
import { StudioHub } from "./components/StudioHub";
import { cloud, hostedChat, submitQueuedScene, type CloudConnection, type CloudModel, type ExecutionMode, type StudioSection } from "./cloudApi";
import { cloudProbeDelayMs, offlineSourceNotice, requestsLocalOnly, runWithFallback, selectExecutionRoute, shouldShowCloudModels } from "./lib/executionRouting";
import { notifyUser } from "./lib/notify";
import {
  attachmentKind,
  attachmentMedia,
  buildMessageContent,
  inferChatMode,
  isUntitledConversation,
  readAttachment,
  settledValue,
  titleFromPrompt,
  type AttachmentDraft,
  type ChatMode,
} from "./lib/chat";
import { formatError } from "./lib/format";
import { imageRendererPrompt, persistDirectImageRequest, plainImageRequest } from "./lib/imageGeneration";
import { ConversationState } from "./lib/conversationState";

type View = "chat" | "work" | "settings" | "tools" | "projects" | StudioSection;

type RuntimeAction = "start" | "restart" | "stop";

function optimisticRuntimeStatus(
  current: LlamaRuntimeStatus | null,
  state: LlamaRuntimeStatus["state"],
): LlamaRuntimeStatus {
  return {
    available: state !== "stopped" && state !== "failed",
    backend: current?.backend ?? current?.selectedRuntime?.manifest.backend ?? null,
    endpoint: state === "stopped" || state === "failed" ? null : (current?.endpoint ?? null),
    state,
    selectedRuntime: current?.selectedRuntime ?? null,
  };
}

export function App() {
  const [chatState] = useState(() => new ConversationState());
  const chatSnapshot = useSyncExternalStore(chatState.subscribe, chatState.getSnapshot, chatState.getSnapshot);
  const { activeId, messages, artifacts } = chatSnapshot;
  const setActiveId = chatState.selectConversation;
  const chatScope = useMemo(() => chatState.scope(activeId), [chatState, activeId]);
  const { setMessages, setArtifacts } = chatScope;
  const activeRun = chatState.run(activeId);
  const streamingId = activeRun.streamingId;
  const submitting = activeRun.request !== null && streamingId === null;
  const activity = activeRun.activity;
  const webProgress = activeRun.webProgress;
  const backgroundBusy = chatState.anyBusy() && !chatState.isBusy(activeId);
  const backgroundRuns = [...chatSnapshot.runs.keys()].filter((id) => id !== activeId && chatState.isBusy(id));
  const copyrightYear = new Date().getFullYear();
  const [view, setView] = useState<View>("chat");
  const [executionMode, setExecutionMode] = useState<ExecutionMode>("auto");
  const [internetAvailable, setInternetAvailable] = useState(() => window.navigator.onLine);
  const [routeNotice, setRouteNotice] = useState<string | null>(null);
  const [studioSubmission, setStudioSubmission] = useState<string | null>(null);
  const [studioOutboxReady, setStudioOutboxReady] = useState(false);
  const [cloudConnection, setCloudConnection] = useState<CloudConnection | null>(null);
  const [cloudModels, setCloudModels] = useState<CloudModel[]>([]);
  const [cloudModel, setCloudModel] = useState("");
  const showCloudApiModels = shouldShowCloudModels(executionMode, internetAvailable,
    Boolean(cloudConnection?.connected), cloudModels.length);
  useEffect(() => {
    if (!("__TAURI_INTERNALS__" in window)) return;
    let active = true;
    void cloud.outboxRecover().then(() => {
      if (active) setStudioOutboxReady(true);
    }).catch(() => undefined);
    return () => { active = false; };
  }, []);
  useEffect(() => {
    if (!studioOutboxReady || !internetAvailable || !cloudConnection?.connected || executionMode === "local") return;
    let active = true;
    void (async () => {
      // Only a saved, never-submitted job is dispatched automatically.
      // Interrupted/uncertain jobs stay in review state, not silently replayed.
      const pending = (await cloud.outboxList()).filter((item) =>
        item.status === "queued" && !requestsLocalOnly(item.payload.prompt));
      if (!pending.length) return;
      await cloud.studioEngines(); // read-only connectivity and capability preflight
      for (const item of pending) {
        if (!active) return;
        try {
          const accepted = await submitQueuedScene(item, "cloud");
          if (accepted && active) setRouteNotice("Saved offline 3D request accepted by Cloud.");
        } catch {
          if (active) setRouteNotice("A queued 3D submission needs review. Open 3D Studio to check its status.");
          return;
        }
      }
    })().catch(() => undefined);
    return () => { active = false; };
  }, [studioOutboxReady, internetAvailable, cloudConnection?.connected, executionMode]);
  useEffect(() => {
    let alive = true;
    let timer: number | null = null;
    let failures = 0;
    const checkCloud = async () => {
      let healthy = false;
      try {
        if (!window.navigator.onLine) {
          if (alive) {
            setCloudConnection((current) => current ? { ...current, connected: false } : current);
            setCloudModels([]);
            setCloudModel("");
          }
        } else {
          const status = await cloud.status();
          if (!alive) return;
          setCloudConnection(status);
          if (!status.connected) { setCloudModels([]); setCloudModel(""); }
          if (status.connected) {
            const available = await cloud.models();
            if (!alive) return;
            setCloudModels(available);
            setCloudModel((selected) => available.some((entry) => entry.id === selected)
              ? selected : available[0]?.id ?? "");
          }
          healthy = status.connected || status.message == null;
        }
      } catch {
        if (alive) {
          setCloudConnection((current) => current ? { ...current, connected: false } : current);
          setCloudModels([]);
          setCloudModel("");
        }
      } finally {
        if (alive) {
          failures = healthy ? 0 : failures + 1;
          // Like Mobile: 5s → 10s → 20s → 40s → 60s after failure.
          // Health checks only read session status and model catalog, never replay POSTs.
          timer = window.setTimeout(() => void checkCloud(),
            cloudProbeDelayMs(healthy, failures, 0.9 + Math.random() * 0.2));
        }
      }
    };
    void checkCloud();
    return () => {
      alive = false;
      if (timer !== null) window.clearTimeout(timer);
    };
  }, []);
  useEffect(() => {
    let active = true;
    const reconnect = () => {
      setInternetAvailable(true);
      void cloud.status().then((status) => {
        if (!active) return;
        setCloudConnection(status);
        if (!status.connected) { setCloudModels([]); setCloudModel(""); }
        if (status.connected) void cloud.models().then((available) => {
          if (!active) return;
          setCloudModels(available);
          setCloudModel((current) => available.some((item) => item.id === current) ? current : available[0]?.id ?? "");
        }).catch(() => undefined);
      }).catch(() => undefined);
    };
    const disconnect = () => setInternetAvailable(false);
    window.addEventListener("online", reconnect);
    window.addEventListener("offline", disconnect);
    return () => {
      active = false;
      window.removeEventListener("online", reconnect);
      window.removeEventListener("offline", disconnect);
    };
  }, []);
  const connectedCloud = (connection: CloudConnection) => {
    setCloudConnection(connection);
    setCloudModels([]);
    setCloudModel("");
    void cloud.models().then((models) => {
      setCloudModels(models);
      setCloudModel((selected) => models.some((model) => model.id === selected) ? selected : models[0]?.id ?? "");
    }).catch(() => { setCloudModels([]); setCloudModel(""); });
  };
  const [root, setRoot] = useState<PortableRootInfo | null>(null);
  const [installationStatus, setInstallationStatus] = useState<InstallationStatus | null>(null);
  const [wizardActive, setWizardActive] = useState<boolean | null>(null);
  const [refreshedOnce, setRefreshedOnce] = useState(false);
  const [hardware, setHardware] = useState<HardwareProfile | null>(null);
  const [storage, setStorage] = useState<StorageSummary | null>(null);
  const [models, setModels] = useState<ModelRecord[]>([]);
  const [runtime, setRuntime] = useState<RuntimeInventory | null>(null);
  const [runtimeStatus, setRuntimeStatus] = useState<LlamaRuntimeStatus | null>(null);
  const [runtimeInstallStatus, setRuntimeInstallStatus] = useState<RuntimeInstallStatus | null>(null);
  const [runtimeInstallError, setRuntimeInstallError] = useState<string | null>(null);
  const [runtimeInstalling, setRuntimeInstalling] = useState(false);
  const [performance, setPerformance] = useState<PerformanceProfile | null>(null);
  const [preferences, setPreferences] = useState<AppPreferences | null>(null);
  const [userProfile, setUserProfile] = useState<UserProfile | null>(null);
  const [conversations, setConversations] = useState<Conversation[]>([]);
  const [imageOptions, setImageOptions] = useState<ImageGenerationOptions>({
    negativePrompt: "",
    seed: null,
    steps: null,
  });
  const [imageProgressByArtifact, setImageProgressByArtifact] = useState<Record<string, ImageRenderProgress>>({});
  const [prompt, setPromptState] = useState("");
  const [attachments, setAttachmentsState] = useState<AttachmentDraft[]>([]);
  const attachmentItems = useRef<AttachmentDraft[]>([]);
  const attachmentClearVersion = useRef(0);
  const attachmentQueue = useRef<Promise<void>>(Promise.resolve());
  const draftRevision = useRef(0);
  const latestPrompt = useRef("");
  const discardedDraft = useRef(0);
  const setPrompt = useCallback((value: string) => {
    draftRevision.current += 1;
    if (!value.trim()) discardedDraft.current += 1;
    latestPrompt.current = value;
    setPromptState(value);
  }, []);
  const createVoiceDraftTarget = useCallback(() => {
    const view = chatState.captureView();
    const discard = discardedDraft.current;
    const isCurrent = () => chatState.isCurrent(view) && discardedDraft.current === discard;
    return {
      isCurrent,
      append: (text: string) => {
        const transcript = text.trim();
        if (!transcript || !isCurrent()) return false;
        const current = latestPrompt.current.trimEnd();
        setPrompt(current.trim() ? `${current} ${transcript}` : transcript);
        return true;
      },
    };
  }, [chatState, setPrompt]);
  const setAttachments = useCallback((change: SetStateAction<AttachmentDraft[]>) => {
    const next = typeof change === "function" ? change(attachmentItems.current) : change;
    if (typeof change !== "function" && next.length === 0) attachmentClearVersion.current += 1;
    attachmentItems.current = next;
    draftRevision.current += 1;
    setAttachmentsState(next);
  }, []);
  const [editingMessageId, setEditingMessageId] = useState<string | null>(null);
  const [editingTitle, setEditingTitle] = useState(false);
  const [titleDraft, setTitleDraft] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false);
  const [sidebarWidth, setSidebarWidth] = useState(() => {
    const stored = Number(window.localStorage.getItem("sidebarWidth"));
    return stored >= 260 && stored <= 440 ? stored : 320;
  });
  const [searchOpen, setSearchOpen] = useState(false);
  const [libraryOpen, setLibraryOpen] = useState(false);
  const [modelsOpen, setModelsOpen] = useState(false);
  const [datasetsOpen, setDatasetsOpen] = useState(false);
  const [settingsSection, setSettingsSection] = useState("general");
  const [previewTarget, setPreviewTarget] = useState<PreviewTarget | null>(null);
  const [pendingModelId, setPendingModelId] = useState<string | null>(null);
  const [modelSwitching, setModelSwitching] = useState(false);
  const [modelSwitchError, setModelSwitchError] = useState<string | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<string | null>(null);
  const [workThreadId, setWorkThreadId] = useState<string | null>(null);
  const [runtimeAction, setRuntimeAction] = useState<RuntimeAction | null>(null);
  const composerRef = useRef<HTMLTextAreaElement | null>(null);
  const runtimeAutostartAttemptedRef = useRef(false);
  const runtimeAutoInstallAttemptedRef = useRef(false);
  const runtimeInstallInFlightRef = useRef(false);

  const showError = useCallback((caught: unknown) => {
    setError(formatError(caught));
  }, []);

  const refreshMessages = useCallback(async (conversationId: string) => {
    try {
      await chatState.loadMessages(conversationId, api.messages);
    } catch (caught) {
      chatState.scope(conversationId).setError(formatError(caught));
    }
  }, [chatState]);

  const refreshApp = useCallback(async () => {
    const initialView = chatState.captureView();
    const results = await Promise.allSettled([
      api.root(),
      api.conversations(),
      api.hardware(),
      api.storage(),
      api.models(),
      api.refreshRuntimeInventory(),
      api.runtimeStatus(),
      api.performance(),
      api.preferences(),
      api.userProfile(),
      api.installationStatus(),
    ]);

    const firstError = results.find((result) => result.status === "rejected");
    if (firstError?.status === "rejected") showError(firstError.reason);

    const rootInfo = settledValue<PortableRootInfo>(results[0]);
    const conversationList = settledValue<Conversation[]>(results[1]);
    const hardwareInfo = settledValue<HardwareProfile>(results[2]);
    const storageInfo = settledValue<StorageSummary>(results[3]);
    const modelList = settledValue<ModelRecord[]>(results[4]);
    const runtimeInfo = settledValue<RuntimeInventory>(results[5]);
    const runtimeStatusInfo = settledValue<LlamaRuntimeStatus>(results[6]);
    const performanceInfo = settledValue<PerformanceProfile>(results[7]);
    const preferencesInfo = settledValue<AppPreferences>(results[8]);
    const userProfileInfo = settledValue<UserProfile>(results[9]);
    const installationStatusInfo = settledValue<InstallationStatus>(results[10]);

    if (rootInfo) setRoot(rootInfo);
    if (conversationList) {
      setConversations(conversationList);
      if (chatState.isCurrent(initialView)) setActiveId((current) =>
        current && conversationList.some((conversation) => conversation.id === current)
          ? current
          : null,
      );
    }
    if (hardwareInfo) setHardware(hardwareInfo);
    if (storageInfo) setStorage(storageInfo);
    if (modelList) setModels(modelList);
    if (runtimeInfo) setRuntime(runtimeInfo);
    if (runtimeStatusInfo) setRuntimeStatus(runtimeStatusInfo);
    if (performanceInfo) setPerformance(performanceInfo);
    if (preferencesInfo) setPreferences(preferencesInfo);
    if (userProfileInfo) setUserProfile(userProfileInfo);
    if (installationStatusInfo) setInstallationStatus(installationStatusInfo);
    setRefreshedOnce(true);
  }, [showError, chatState, setActiveId]);

  useEffect(() => {
    void refreshApp();
  }, [refreshApp]);

  useEffect(() => {
    if (!runtimeInstalling) return;
    const refreshProgress = () => {
      void api.runtimeInstallStatus().then(setRuntimeInstallStatus).catch(() => undefined);
    };
    refreshProgress();
    const timer = window.setInterval(refreshProgress, 750);
    return () => window.clearInterval(timer);
  }, [runtimeInstalling]);

  useEffect(() => {
    if (!refreshedOnce || !installationStatus || installationStatus.setupRequired ||
        runtime === null || runtime.selected !== null || runtimeAutoInstallAttemptedRef.current) return;
    runtimeAutoInstallAttemptedRef.current = true;
    void installRuntime();
    // One automatic attempt per application launch; manual retry remains available.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [refreshedOnce, installationStatus, runtime]);

  useEffect(() => {
    if (!refreshedOnce || installationStatus === null || wizardActive !== null) return;
    // Only first-time root/profile configuration is modal. Runtime repair
    // must not hide existing chats or prevent access to Settings.
    setWizardActive(installationStatus.setupRequired);
  }, [refreshedOnce, installationStatus, wizardActive]);

  useEffect(() => {
    if (!preferences?.localRuntimeAutostart) {
      runtimeAutostartAttemptedRef.current = false;
      return;
    }
    if (runtimeAutostartAttemptedRef.current || !refreshedOnce || !runtime?.selected || runtimeAction) return;
    const state = runtimeStatus?.state ?? "stopped";
    if (state !== "stopped" && state !== "failed") {
      runtimeAutostartAttemptedRef.current = true;
      return;
    }
    runtimeAutostartAttemptedRef.current = true;
    void startRuntime("start");
    // startRuntime is intentionally omitted so autostart stays one-shot per runtime state.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    preferences?.localRuntimeAutostart,
    refreshedOnce,
    runtime?.selected,
    runtimeAction,
    runtimeStatus?.state,
  ]);

  const modelUpdateCheckStartedRef = useRef(false);
  useEffect(() => {
    if (!preferences || modelUpdateCheckStartedRef.current) return;
    modelUpdateCheckStartedRef.current = true;
    if (!preferences.notifyModelUpdates) return;
    void api
      .checkModelUpdates()
      .then(async (report) => {
        const updated = report.entries.filter((status) => status.updateAvailable);
        if (updated.length === 0) return;
        const names = updated.map((status) => status.entry.name).join(", ");
        await notifyUser("New AI model available", `An update is available for: ${names}.`);
      })
      .catch(() => undefined);
  }, [preferences]);

  useEffect(() => {
    document.documentElement.dataset.compactSidebar = String(preferences?.compactSidebar ?? false);
    if (!preferences) return;

    try {
      window.localStorage.setItem("openmindai:theme", preferences.theme);
    } catch {
      // A storage restriction must not prevent selecting a theme.
    }

    const apply = (theme: "dark" | "light") => {
      document.documentElement.dataset.theme = theme;
      document
        .querySelector('meta[name="theme-color"]')
        ?.setAttribute("content", theme === "light" ? "#f1f1f1" : "#090909");
    };

    if (preferences.theme !== "system") {
      apply(preferences.theme);
      return;
    }

    const media = window.matchMedia("(prefers-color-scheme: dark)");
    const onSystemThemeChange = () => apply(media.matches ? "dark" : "light");
    onSystemThemeChange();
    media.addEventListener("change", onSystemThemeChange);
    return () => media.removeEventListener("change", onSystemThemeChange);
  }, [preferences]);

  useEffect(() => {
    if (!preferences?.socketRealtimeEnabled || !preferences.socketUrl.trim()) return;
    const socket: Socket = io(preferences.socketUrl, { transports: ["websocket"], reconnectionAttempts: 3 });
    const owners = new Set<string>();
    const activityEvent = (field: "typing" | "searching" | "researching", enabled: boolean) =>
      (payload?: { conversationId?: string; detail?: string }) => {
        // Untagged legacy broadcasts cannot safely identify their destination chat.
        if (!payload || typeof payload.conversationId !== "string") return;
        owners.add(payload.conversationId);
        chatState.scope(payload.conversationId).setActivity((current) => ({
          ...current, [field]: enabled, detail: typeof payload.detail === "string" ? payload.detail : current.detail,
        }));
      };
    socket.on("typing:start", activityEvent("typing", true));
    socket.on("typing:stop", activityEvent("typing", false));
    socket.on("search:start", activityEvent("searching", true));
    socket.on("search:stop", activityEvent("searching", false));
    socket.on("research:start", activityEvent("researching", true));
    socket.on("research:stop", activityEvent("researching", false));
    return () => {
      socket.disconnect();
      for (const owner of owners) if (!chatState.isBusy(owner)) {
        chatState.scope(owner).setActivity({ typing: false, searching: false, researching: false, detail: null });
      }
    };
  }, [preferences?.socketRealtimeEnabled, preferences?.socketUrl, chatState]);

  useEffect(() => {
    if (!activeId) { setEditingMessageId(null); return; }
    void refreshMessages(activeId);
    void chatState.loadArtifacts(activeId, api.listArtifacts)
      .catch((caught) => chatState.scope(activeId).setError(formatError(caught)));
  }, [activeId, chatState, refreshMessages]);

  useEffect(() => {
    const unlisteners = [
      listen<StreamStartedEvent>("inference:started", (event) => {
        if (!chatState.startStream(event.payload)) return;
        if (event.payload.assistant.conversationId !== event.payload.conversationId) return;
        setConversations((items) => items.map((item) => item.id === event.payload.conversationId
          ? { ...item, activeModelId: event.payload.assistant.modelId } : item));
      }),
      listen<{ conversationId: string; modelName: string; replacing: string | null }>("runtime:model-loading", (event) => {
        const { conversationId, modelName, replacing } = event.payload;
        chatState.scope(conversationId).setActivity((current) => ({ ...current,
          detail: `Loading ${modelName}…${replacing ? ` (switching from ${replacing})` : ""} This can take a few minutes.`,
        }));
      }),
      listen<WebResearchProgressEvent>("web-research:progress", (event) => {
        chatState.scope(event.payload.conversationId).setWebProgress(event.payload);
      }),
      listen<StreamChunkEvent>("inference:chunk", (event) => chatState.streamChunk(event.payload)),
      listen<StreamDoneEvent>("inference:done", (event) => {
        chatState.finishStream(event.payload);
        if (chatState.run(event.payload.conversationId).request === null) void refreshMessages(event.payload.conversationId);
      }),
      listen<Artifact>("artifact:started", (event) => {
        chatState.scope(event.payload.conversationId).setArtifacts((items) => upsertArtifactInList(items, event.payload));
      }),
      listen<Artifact>("artifact:done", (event) => {
        chatState.scope(event.payload.conversationId).setArtifacts((items) => upsertArtifactInList(items, event.payload));
        setImageProgressByArtifact((current) => {
          if (!(event.payload.id in current)) return current;
          const next = { ...current }; delete next[event.payload.id]; return next;
        });
      }),
      listen<ImageRenderProgress>("image:progress", (event) => {
        setImageProgressByArtifact((current) => ({ ...current, [event.payload.artifactId]: event.payload }));
      }),
    ];
    return () => { void Promise.all(unlisteners).then((items) => items.forEach((unlisten) => unlisten())); };
  }, [chatState, refreshMessages]);

  const newConversation = useCallback(() => {
    setActiveId(null);
    setMessages([]);
    setPrompt("");
    setAttachments([]);
    setEditingMessageId(null);
    setView("chat");
    setEditingTitle(false);
    setTitleDraft("");
    setPendingModelId(null);
    setModelSwitchError(null);
    window.setTimeout(() => composerRef.current?.focus(), 0);
  }, [setActiveId, setAttachments, setMessages, setPrompt]);

  const sendMessage = useCallback(async () => {
    const content = prompt.trim();
    if ((!content && attachments.length === 0) || chatState.anyBusy()) return;
    const draftPrompt = prompt;
    const draftAttachments = attachments.slice();
    const originalDraftRevision = draftRevision.current;
    let clearedDraftRevision: number | null = null;
    let requestView = chatState.captureView();
    const editTargetId = editingMessageId;
    const inferredMode = inferChatMode(content, attachments);
    const messageContent = buildMessageContent(content, attachments, inferredMode);
    const inferenceMedia = attachmentMedia(attachments);
    let conversationId = activeId;
    let conversationForTitle = conversations.find((conversation) => conversation.id === conversationId) ?? null;
    let optimisticUser: Message | null = null;
    let editForkCreated = false;
    const lease = chatState.beginRequest(conversationId);
    if (!lease) return;
    let requestScope = chatState.scope(conversationId);
    try {
      if (editTargetId) {
        if (!conversationId) {
          throw new Error("Open the original conversation before editing a message.");
        }
        const sourceConversationId = conversationId;
        const fork = await api.forkConversationForEdit(conversationId, editTargetId);
        conversationId = fork.conversation.id;
        conversationForTitle = fork.conversation;
        editForkCreated = true;
        chatState.assignRequest(lease, conversationId);
        requestScope = chatState.scope(conversationId);
        setConversations((items) => [fork.conversation, ...items]);
        if (chatState.isCurrent(requestView)) {
          setActiveId(conversationId);
          requestView = chatState.captureView();
          setWorkThreadId((current) => current === sourceConversationId ? fork.conversation.id : current);
          requestScope.setMessages(fork.messages);
          requestScope.setArtifacts([]);
        }
      } else if (!conversationId) {
        const conversation = await api.createConversation();
        conversationId = conversation.id;
        conversationForTitle = conversation;
        chatState.assignRequest(lease, conversationId);
        requestScope = chatState.scope(conversationId);
        setConversations((items) => [conversation, ...items]);
        if (chatState.isCurrent(requestView)) {
          setActiveId(conversationId);
          requestView = chatState.captureView();
        }
        if (pendingModelId) {
          if (chatState.isCurrent(requestView)) setModelSwitching(true);
          try {
            await api.activateModel(conversationId, pendingModelId);
            setConversations((items) => items.map((item) => item.id === conversationId ? { ...item, activeModelId: pendingModelId } : item));
          } catch (caught) {
            if (chatState.isCurrent(requestView)) setModelSwitchError(formatError(caught));
          } finally {
            setModelSwitching(false);
            if (chatState.isCurrent(requestView)) setPendingModelId(null);
          }
        }
      }
      if (chatState.isCurrent(requestView) && draftRevision.current === originalDraftRevision) {
        setPrompt(""); setAttachments([]); setEditingMessageId(null);
        clearedDraftRevision = draftRevision.current;
      }
      optimisticUser = {
        id: `local-${crypto.randomUUID()}`, conversationId, role: "user", content: messageContent,
        status: "completed", modelId: null, createdAt: new Date().toISOString(), updatedAt: new Date().toISOString(),
      };
      requestScope.setMessages((items) => [...items, optimisticUser as Message]);
      if (preferences?.autoGenerateTitles ?? true) {
        const generatedTitle = titleFromPrompt(content || attachments[0]?.name || "New conversation");
        if (conversationForTitle && isUntitledConversation(conversationForTitle.title) && generatedTitle) {
          await api.renameConversation(conversationId, generatedTitle);
          setConversations((items) => items.map((conversation) => conversation.id === conversationId ? { ...conversation, title: generatedTitle } : conversation));
        }
      }
      // Unimplemented Cloud media types still work using their existing local engine.
      const cloudChatCapable = inferenceMedia.length === 0 &&
        ["chat", "thinking", "search", "research"].includes(inferredMode) && Boolean(cloudModel);
      const route = selectExecutionRoute({
        preference: cloudChatCapable ? executionMode : "local",
        task: inferredMode === "search" ? "search" : inferredMode === "research" ? "research" : "chat",
        online: internetAvailable,
        cloudConnected: Boolean(cloudConnection?.connected),
        cloudModelReady: Boolean(cloudModel),
        forceLocal: requestsLocalOnly(content),
      });
      const localOffline = (!internetAvailable || requestsLocalOnly(content)) &&
        ["search", "research"].includes(inferredMode);
      const localContent = localOffline
        ? offlineSourceNotice(buildMessageContent(content, attachments, "chat"))
        : messageContent;
      const localMode = localOffline ? "chat" : inferredMode;
      const requestConversationId = conversationId;
      if (!requestConversationId) throw new Error("Conversation was not created.");
      const response = await runWithFallback(
        route,
        () => hostedChat(requestConversationId, messageContent, cloudModel, inferredMode),
        () => inferredMode === "image"
          ? persistDirectImageRequest(requestConversationId, localContent, api)
          : api.sendChatMessage(requestConversationId, localContent, localMode, inferenceMedia),
        (reason) => {
          setRouteNotice(reason);
          setCloudConnection((current) => current ? { ...current, connected: false } : current);
        },
      );
      const assistant = response.result;
      if (route.offline) setRouteNotice("Offline: processed locally. Live web sources cannot be verified.");
      else if (!response.fallback && route.target === "local" && executionMode !== "local")
        setRouteNotice(cloudChatCapable ? route.reason
          : "This media operation uses the local processor; an equivalent Cloud API is not available.");
      requestScope.setMessages((items) => items.filter((message) => message.id !== optimisticUser?.id));
      const generationKind = generationKindForMode(inferredMode);
      if (generationKind) {
        const generationPrompt = generationKind === "image" ? plainImageRequest(messageContent) : assistant.content.trim() || content;
        const artifact = await api.createGenerationArtifact(conversationId, assistant.id, generationKind, generationPrompt, false, generationKind === "image" ? imageOptions : undefined);
        requestScope.setArtifacts((items) => upsertArtifactInList(items, artifact));
        if (preferences?.openArtifactsAfterGeneration && artifact.status === "ready") void api.openArtifact(artifact.id).catch(showError);
      }
      await refreshMessages(conversationId);
      setConversations(await api.conversations());
    } catch (caught) {
      if (optimisticUser) requestScope.setMessages((items) => items.filter((message) => message.id !== optimisticUser?.id)
        .map((message) => message.status === "streaming" ? { ...message, status: "failed" } : message));
      if (chatState.isCurrent(requestView) && draftRevision.current === (clearedDraftRevision ?? originalDraftRevision)) {
        setPrompt(draftPrompt); setAttachments(draftAttachments);
        setEditingMessageId(editForkCreated ? null : editTargetId);
      }
      requestScope.setError(formatError(caught));
      if (conversationId) await refreshMessages(conversationId);
    } finally {
      chatState.finishRequest(lease);
    }
  }, [activeId, attachments, chatState, conversations, editingMessageId, imageOptions, pendingModelId,
    executionMode, cloudModel, internetAvailable, cloudConnection?.connected, preferences?.autoGenerateTitles, preferences?.openArtifactsAfterGeneration, prompt, refreshMessages, setActiveId, setAttachments, setPrompt, showError]);

  const sendWorkTask = useCallback(async (conversationId: string, content: string) => {
    const task = content.trim();
    if (!task) return;
    const lease = chatState.beginRequest(conversationId);
    if (!lease) return;
    const requestScope = chatState.scope(conversationId);
    const inferredMode = inferChatMode(task, []);
    const messageContent = buildMessageContent(task, [], inferredMode);
    setPrompt(""); setAttachments([]); setEditingMessageId(null);
    setActiveId(conversationId); setWorkThreadId(conversationId);
    try {
      const generatedTitle = titleFromPrompt(task);
      if (generatedTitle) {
        await api.renameConversation(conversationId, generatedTitle);
        setConversations((items) => items.map((conversation) => conversation.id === conversationId ? { ...conversation, title: generatedTitle } : conversation));
      }
      const availableForCloud = ["chat", "thinking", "search", "research"].includes(inferredMode) && Boolean(cloudModel);
      const route = selectExecutionRoute({
        preference: availableForCloud ? executionMode : "local",
        task: inferredMode === "search" ? "search" : inferredMode === "research" ? "research" : "chat",
        online: internetAvailable,
        cloudConnected: Boolean(cloudConnection?.connected),
        cloudModelReady: Boolean(cloudModel),
        forceLocal: requestsLocalOnly(task),
      });
      await runWithFallback(
        route,
        () => hostedChat(conversationId, messageContent, cloudModel, inferredMode),
        () => api.sendChatMessage(conversationId,
          (!internetAvailable || requestsLocalOnly(task)) && ["search", "research"].includes(inferredMode)
            ? offlineSourceNotice(buildMessageContent(task, [], "chat")) : messageContent,
          (!internetAvailable || requestsLocalOnly(task)) && ["search", "research"].includes(inferredMode)
            ? "chat" : inferredMode, []),
        (reason) => {
          setRouteNotice(reason);
          setCloudConnection((current) => current ? { ...current, connected: false } : current);
        },
      );
      await refreshMessages(conversationId);
      setConversations(await api.conversations());
    } catch (caught) {
      requestScope.setError(formatError(caught));
      await refreshMessages(conversationId);
    } finally { chatState.finishRequest(lease); }
  }, [chatState, refreshMessages, setActiveId, setAttachments, setPrompt, executionMode, cloudModel, internetAvailable, cloudConnection?.connected]);

  const cancelConversationGeneration = useCallback(async (conversationId: string) => {
    try { await chatState.cancelGeneration(conversationId, api.cancelGeneration); }
    catch (caught) { chatState.scope(conversationId).setError(formatError(caught)); }
  }, [chatState]);
  const stopGeneration = useCallback(async () => {
    const owner = chatState.getSnapshot().activeId;
    try { await chatState.cancelActiveGeneration(api.cancelGeneration); }
    catch (caught) { chatState.scope(owner).setError(formatError(caught)); }
  }, [chatState]);

  const editUserMessage = useCallback((messageId: string, content: string) => {
    setEditingMessageId(messageId);
    setPrompt(content);
    setAttachments([]);
    setView((current) => (current === "work" ? "work" : "chat"));
    window.setTimeout(() => {
      const node = composerRef.current;
      if (!node) return;
      node.focus();
      node.setSelectionRange(node.value.length, node.value.length);
    }, 0);
  }, [setAttachments, setPrompt]);

  // Studio submissions are actually sent to the local engine, not left as
  // drafts that require a second click. The existing composer failure recovery
  // still retains the complete request if the local runtime is unavailable.
  useEffect(() => {
    if (studioSubmission === null || view !== "chat") return;
    if (prompt !== studioSubmission) {
      setStudioSubmission(null);
      return;
    }
    if (chatState.anyBusy()) return;
    setStudioSubmission(null);
    void sendMessage();
  }, [studioSubmission, view, prompt, sendMessage, chatState]);
  const submitStudioLocally = useCallback((content: string) => {
    setAttachments([]);
    setEditingMessageId(null);
    setPrompt(content);
    setView("chat");
    setStudioSubmission(content);
  }, [setAttachments, setPrompt]);
  const startToolPrompt = useCallback((content: string) => {
    setEditingMessageId(null);
    setPrompt(content);
    setView("chat");
    window.setTimeout(() => {
      const node = composerRef.current;
      if (!node) return;
      node.focus();
      node.setSelectionRange(node.value.length, node.value.length);
    }, 0);
  }, [setPrompt]);

  const regenerate = useCallback(async (assistantMessageId: string) => {
    if (!activeId) return;
    const targetIndex = messages.findIndex((message) => message.id === assistantMessageId);
    const user = targetIndex > 0 ? messages.slice(0, targetIndex).reverse().find((message) => message.role === "user") : null;
    const mode = user ? persistedChatMode(user.content) : "chat";
    const lease = chatState.beginRequest(activeId);
    if (!lease) return;
    const requestScope = chatState.scope(activeId);
    try {
      const cloudCapable = ["chat", "thinking", "search", "research"].includes(mode) && Boolean(cloudModel);
      const route = selectExecutionRoute({
        preference: cloudCapable ? executionMode : "local",
        task: mode === "search" ? "search" : mode === "research" ? "research" : "chat",
        online: internetAvailable,
        cloudConnected: Boolean(cloudConnection?.connected),
        cloudModelReady: Boolean(cloudModel),
        forceLocal: user ? requestsLocalOnly(user.content) : false,
      });
      await runWithFallback(route,
        async () => {
          if (!user) throw new Error("The original user message is unavailable.");
          return hostedChat(activeId, user.content, cloudModel, mode, assistantMessageId);
        },
        () => api.regenerateMessage(activeId, assistantMessageId,
          (!internetAvailable || (user && requestsLocalOnly(user.content))) &&
            ["search", "research"].includes(mode) ? "chat" : mode),
        (reason) => {
          setRouteNotice(reason);
          setCloudConnection((current) => current ? { ...current, connected: false } : current);
        },
      );
      await refreshMessages(activeId);
      await refreshApp();
    } catch (caught) {
      requestScope.setError(formatError(caught));
      await refreshMessages(activeId);
    } finally { chatState.finishRequest(lease); }
  }, [activeId, chatState, messages, refreshApp, refreshMessages, executionMode, cloudModel, internetAvailable, cloudConnection?.connected]);

  const selectModel = useCallback(
    async (modelId: string) => {
      if (chatState.anyBusy()) return;
      setModelSwitchError(null);
      if (!activeId) {
        setPendingModelId(modelId);
        return;
      }
      setModelSwitching(true);
      try {
        await api.activateModel(activeId, modelId);
        setConversations((items) =>
          items.map((item) => (item.id === activeId ? { ...item, activeModelId: modelId } : item)),
        );
      } catch (caught) {
        setModelSwitchError(formatError(caught));
      } finally {
        setModelSwitching(false);
      }
    },
    [activeId, chatState],
  );

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement | null;
      const isTyping = target?.tagName === "TEXTAREA" || target?.tagName === "INPUT";

      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "n") {
        event.preventDefault();
        void newConversation();
      }

      if ((event.ctrlKey || event.metaKey) && event.key === "Enter") {
        event.preventDefault();
        void sendMessage();
      }

      if (event.key === "Escape" && streamingId) {
        event.preventDefault();
        void stopGeneration();
      }

      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "k") {
        event.preventDefault();
        setSearchOpen(true);
      }

      if ((event.ctrlKey || event.metaKey) && event.key === ",") {
        event.preventDefault();
        setView("settings");
      }

      if (!isTyping && event.key === "?") {
        event.preventDefault();
        setError(
          "Shortcuts: Ctrl+N new chat, Ctrl+K search chats, Ctrl+, settings, Ctrl+Enter send, Esc stop.",
        );
      }
    };

    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [newConversation, sendMessage, stopGeneration, streamingId]);

  async function addFiles(files: FileList | null) {
    if (!files) return;
    const selection = Array.from(files);
    const view = chatState.captureView();
    const clearVersion = attachmentClearVersion.current;
    const isCurrent = () => chatState.isCurrent(view) && attachmentClearVersion.current === clearVersion;
    const ingest = async () => {
      const failures = new Set<string>();
      let added = false;
      for (const file of selection) {
        if (!isCurrent()) return;
        const items = attachmentItems.current;
        if (items.length >= 8) {
          failures.add("A chat can contain at most 8 attachments at once.");
          continue;
        }
        const kind = attachmentKind(file);
        if (kind === "binary") {
          failures.add("This file type cannot be read by local chat. Attach text/code, PDF, image, audio, or video files.");
          continue;
        }
        if (kind === "image" && items.filter(item => item.kind === "image").length >= 4) {
          failures.add("Local vision supports at most 4 images in one request.");
          continue;
        }
        try {
          const attachment = await readAttachment(file);
          if (!isCurrent()) return;
          // Re-check after decoding: files may have been removed/restored meanwhile.
          const current = attachmentItems.current;
          if (current.length >= 8) {
            failures.add("A chat can contain at most 8 attachments at once.");
          } else if (attachment.kind === "image" && current.filter(item => item.kind === "image").length >= 4) {
            failures.add("Local vision supports at most 4 images in one request.");
          } else {
            setAttachments(items => [...items, attachment]);
            added = true;
          }
        } catch (error) {
          if (!isCurrent()) return;
          failures.add(formatError(error));
        }
      }
      if (!isCurrent()) return;
      if (added) {
        setEditingMessageId(null);
        setView("chat");
        composerRef.current?.focus();
      }
      if (failures.size) showError(new Error([...failures].join(" ")));
    };
    const pending = attachmentQueue.current.then(ingest, ingest);
    attachmentQueue.current = pending.catch(() => {});
    await pending;
  }

  async function archiveConversation(id: string) {
    await api.archiveConversation(id);
    setConversations((items) => items.filter((item) => item.id !== id));
    if (chatState.getSnapshot().activeId === id) {
      setActiveId(null);
      setMessages([]);
      setEditingMessageId(null);
      setEditingTitle(false);
      setTitleDraft("");
    }
  }

  function requestDeleteConversation(id: string) {
    if (preferences?.confirmBeforeDelete ?? true) {
      setDeleteTarget(id);
      return;
    }
    void performDelete(id);
  }

  async function performDelete(id: string) {
    await api.deleteConversation(id);
    setConversations((items) => items.filter((item) => item.id !== id));
    if (chatState.getSnapshot().activeId === id) {
      setActiveId(null);
      setMessages([]);
      setEditingMessageId(null);
      setEditingTitle(false);
      setTitleDraft("");
    }
    setDeleteTarget(null);
  }

  async function duplicateConversation(conversation: Conversation) {
    const initialView = chatState.captureView();
    const created = await api.createConversation(`${conversation.title} (copy)`);
    const original = await api.messages(conversation.id);
    for (const message of original) {
      if (message.role === "system") continue;
      if (message.role === "user") {
        await api.addUserMessage(created.id, message.content);
      } else if (message.role === "assistant") {
        const assistant = await api.createStreamingAssistantMessage(created.id);
        if (message.content) await api.appendMessageChunk(assistant.id, message.content);
        await api.completeMessage(
          assistant.id,
          message.status === "streaming" ? "completed" : message.status,
        );
      }
    }
    await refreshApp();
    if (chatState.isCurrent(initialView)) {
      setActiveId(created.id);
      setEditingMessageId(null);
      setView("chat");
    }
  }

  async function createProjectConversation(project: Project) {
    const title = `${project.name}: New chat`;
    const conversation = await api.createConversation(title);
    setConversations((items) => [conversation, ...items]);
    return conversation;
  }

  async function renameActiveConversation() {
    if (!activeConversation) return;
    const nextTitle = titleDraft.trim();
    if (!nextTitle || nextTitle === activeConversation.title) {
      setEditingTitle(false);
      setTitleDraft("");
      return;
    }
    await api.renameConversation(activeConversation.id, nextTitle);
    setConversations((items) =>
      items.map((conversation) =>
        conversation.id === activeConversation.id
          ? { ...conversation, title: nextTitle }
          : conversation,
      ),
    );
    setEditingTitle(false);
    setTitleDraft("");
  }

  async function togglePin(conversation: Conversation) {
    await api.pinConversation(conversation.id, !conversation.pinned);
    await refreshApp();
  }

  async function installRuntime() {
    if (runtimeInstallInFlightRef.current) return;
    runtimeInstallInFlightRef.current = true;
    setRuntimeInstalling(true);
    setRuntimeInstallError(null);
    setRuntimeInstallStatus(null);
    try {
      setRuntimeInstallStatus(await api.installRecommendedRuntime());
      await refreshApp();
    } catch (caught) {
      const message = formatError(caught);
      if (!message.toLowerCase().includes("cancel")) setRuntimeInstallError(message);
      void api.runtimeInstallStatus().then(setRuntimeInstallStatus).catch(() => undefined);
    } finally {
      runtimeInstallInFlightRef.current = false;
      setRuntimeInstalling(false);
    }
  }

  async function cancelRuntimeInstall() {
    try {
      setRuntimeInstallStatus(await api.cancelRuntimeInstall());
    } catch (caught) {
      setRuntimeInstallError(formatError(caught));
    }
  }

  async function startRuntime(action: RuntimeAction = "restart") {
    if (runtimeAction) return;
    setRuntimeAction(action);
    setRuntimeStatus((current) => optimisticRuntimeStatus(current, action === "stop" ? "stopping" : "starting"));
    try {
      setRuntimeStatus(await api.startRuntime());
      setRuntime(await api.runtimeInventory());
    } catch (caught) {
      showError(caught);
      setRuntimeStatus((current) => optimisticRuntimeStatus(current, "failed"));
    } finally {
      setRuntimeAction(null);
    }
  }

  async function stopRuntime() {
    if (runtimeAction) return;
    setRuntimeAction("stop");
    setRuntimeStatus((current) => optimisticRuntimeStatus(current, "stopping"));
    try {
      await api.stopRuntime();
      setRuntimeStatus(await api.runtimeStatus());
      setRuntime(await api.runtimeInventory());
    } catch (caught) {
      showError(caught);
      setRuntimeStatus((current) => optimisticRuntimeStatus(current, "failed"));
    } finally {
      setRuntimeAction(null);
    }
  }

  async function updatePreferences(next: AppPreferences) {
    const saved = await api.savePreferences(next);
    setPreferences(saved);
    setPerformance(await api.performance());
  }

  async function updateUserProfile(next: UserProfile) {
    setUserProfile(await api.saveUserProfile(next));
  }

  const createArtifact = useCallback(
    async (messageId: string, kind: ArtifactKind, content: string, filenameHint?: string) => {
      if (!activeId) return;
      try {
        const artifact =
          kind === "pdf" || kind === "docx"
            ? await api.createDocumentArtifact(
                activeId,
                messageId,
                kind,
                filenameHint ?? null,
                content,
                null,
              )
            : await api.createTextArtifact(
                activeId,
                messageId,
                kind,
                filenameHint ?? null,
                content,
              );
        setArtifacts((items) => upsertArtifactInList(items, artifact));
        if (preferences?.openArtifactsAfterGeneration && artifact.status === "ready") {
          void api.openArtifact(artifact.id).catch(showError);
        }
      } catch (caught) {
        showError(caught);
      }
    },
    [activeId, preferences?.openArtifactsAfterGeneration, setArtifacts, showError],
  );

  const retryArtifact = useCallback(
    (artifact: Artifact, useCpu = false) => {
      const source = messages.find((message) => message.id === artifact.messageId);
      if (!source || !artifact.messageId || !activeId) {
        showError("The original message for this file is no longer available.");
        return;
      }
      if (artifact.kind === "image" || artifact.kind === "video" || artifact.kind === "audio") {
        const generationKind = artifact.kind === "audio" ? "voice" : artifact.kind;
        const sourceIndex = messages.findIndex((message) => message.id === source.id);
        const precedingUser =
          sourceIndex > 0
            ? messages
                .slice(0, sourceIndex)
                .reverse()
                .find((message) => message.role === "user")
            : null;
        const generationPrompt =
          generationKind === "image" && precedingUser
            ? imageRendererPrompt(source.content, precedingUser.content)
            : source.content;
        void api
          .createGenerationArtifact(
            activeId,
            artifact.messageId,
            generationKind,
            generationPrompt,
            useCpu,
            generationKind === "image" ? imageOptions : undefined,
          )
          .then((next) => {
            setArtifacts((items) => upsertArtifactInList(items, next));
            if (preferences?.openArtifactsAfterGeneration && next.status === "ready") {
              void api.openArtifact(next.id).catch(showError);
            }
          })
          .catch(showError);
        return;
      }
      void createArtifact(artifact.messageId, artifact.kind, source.content, artifact.name);
    },
    [activeId, createArtifact, imageOptions, messages, preferences?.openArtifactsAfterGeneration, setArtifacts, showError],
  );

  const cancelImageArtifact = useCallback(
    (artifact: Artifact) => {
      void api.cancelImageGeneration(artifact.id).then((cancelled) => {
        if (!cancelled) showError("Image generation has already finished.");
      }).catch(showError);
    },
    [showError],
  );

  const openArtifactHandler = useCallback(
    (artifact: Artifact) => {
      void api.openArtifact(artifact.id).catch(showError);
    },
    [showError],
  );

  const revealArtifactHandler = useCallback(
    (artifact: Artifact) => {
      void api.revealArtifactInFolder(artifact.id).catch(showError);
    },
    [showError],
  );

  const artifactsByMessage = useMemo(() => {
    const map = new Map<string, Artifact[]>();
    for (const artifact of artifacts) {
      if (!artifact.messageId) continue;
      const existing = map.get(artifact.messageId) ?? [];
      existing.push(artifact);
      map.set(artifact.messageId, existing);
    }
    return map;
  }, [artifacts]);

  const activeConversation = useMemo(
    () => conversations.find((conversation) => conversation.id === activeId) ?? null,
    [activeId, conversations],
  );
  const activeModelId = activeConversation?.activeModelId ?? pendingModelId;

  if (wizardActive) {
    return (
      <SetupWizard
        mode={installationStatus?.setupRequired ? "full" : "preparing"}
        hardware={hardware}
        models={models}
        runtime={runtime}
        root={root}
        pending={installationStatus?.pending}
        refresh={refreshApp}
        onDismiss={() => setWizardActive(false)}
      />
    );
  }
  if (wizardActive === null) return <div className="app-loading" aria-hidden="true" />;

  return (
    <main className="app-shell">
      <Sidebar
        collapsed={sidebarCollapsed}
        onToggleCollapsed={() => setSidebarCollapsed((value) => !value)}
        conversations={conversations}
        activeId={activeId}
        view={view}
        userProfile={userProfile}
        width={sidebarWidth}
        onWidthChange={(width) => {
          setSidebarWidth(width);
          window.localStorage.setItem("sidebarWidth", String(width));
        }}
        onNewChat={newConversation}
        onOpenSearch={() => setSearchOpen(true)}
        onOpenConversation={(id) => {
          setView("chat");
          setActiveId(id);
          setEditingMessageId(null);
          setEditingTitle(false);
          setTitleDraft("");
          setPendingModelId(null);
        }}
        onRename={(conversation) => {
          setView("chat");
          setActiveId(conversation.id);
          setEditingMessageId(null);
          setTitleDraft(conversation.title);
          setEditingTitle(true);
        }}
        onTogglePin={togglePin}
        onArchive={archiveConversation}
        onDelete={requestDeleteConversation}
        onDuplicate={duplicateConversation}
        onOpenLibrary={() => setLibraryOpen(true)}
        onOpenModels={() => setModelsOpen(true)}
        onOpenDatasets={() => setDatasetsOpen(true)}
        onOpenTools={() => setView("tools")}
        onOpenProjects={() => setView("projects")}
        onOpenStudio={(section) => setView(section)}
        onOpenSettings={(section) => {
          setSettingsSection(section ?? "general");
          setView("settings");
        }}
      />

      <section className="workspace">
        <header className="topbar" data-tauri-drag-region="">
          <div>
            {view === "chat" && !activeConversation ? null : view === "chat" &&
              activeConversation &&
              editingTitle ? (
              <form
                className="title-editor"
                onSubmit={(event) => {
                  event.preventDefault();
                  void renameActiveConversation();
                }}
              >
                <input
                  autoFocus
                  value={titleDraft}
                  onChange={(event) => setTitleDraft(event.target.value)}
                  onKeyDown={(event) => {
                    if (event.key === "Escape") {
                      setEditingTitle(false);
                      setTitleDraft("");
                    }
                  }}
                />
                <button title="Save title" type="submit">
                  <Check size={16} />
                </button>
              </form>
            ) : (
              <h1>
                {view === "chat"
                  ? activeConversation?.title
                  : view === "tools"
                    ? "Tools"
                    : view === "work"
                      ? "Work"
                      : view === "projects"
                        ? "Projects"
                        : view === "research-studio"
                        ? "Research Studio"
                        : view === "3d-studio"
                          ? "3D Studio"
                          : view === "web-search"
                            ? "Web Search"
                            : "Settings"}
              </h1>
            )}
          </div>
          <div className="topbar-center">
            <ChatModeSwitcher
              active={view === "work" ? "work" : "chat"}
              onChat={() => {
                setWorkThreadId(null);
                setView("chat");
              }}
              onWork={() => {
                setWorkThreadId(null);
                setActiveId(null);
                setMessages([]);
                setPrompt("");
                setAttachments([]);
                setView("work");
              }}
            />
            <div className="execution-mode-switch" role="group" aria-label="AI execution mode">
              <button type="button" className={executionMode === "auto" ? "active" : ""} aria-pressed={executionMode === "auto"} onClick={() => setExecutionMode("auto")} title="Automatically route to Cloud when connected or Local when offline">Auto</button>
              <button type="button" className={executionMode === "local" ? "active" : ""} aria-pressed={executionMode === "local"} onClick={() => setExecutionMode("local")} title="Use this computer for local inference">
                Local
              </button>
              <button type="button" className={executionMode === "cloud" ? "active" : ""} aria-pressed={executionMode === "cloud"} onClick={() => setExecutionMode("cloud")} title={cloudConnection?.connected ? "Connected to OpenMindAI Cloud" : "Sign in from Research Studio to use Cloud"}>
                <span className={internetAvailable && cloudConnection?.connected ? "studio-dot good" : "studio-dot"} /> Cloud
              </button>
            </div>
            {view === "work" && !workThreadId && showCloudApiModels ? (
              <select className="cloud-model-select" aria-label="Cloud API model" value={cloudModel} onChange={(event) => setCloudModel(event.target.value)}>
                {cloudModels.map((model) => <option key={model.id} value={model.id}>{model.name || model.id}</option>)}
              </select>
            ) : null}
            <span className="mode-connection-status" role="status">{internetAvailable ? "Online" : "Offline"}</span>
          </div>
          <div className="topbar-actions">
            {view === "chat" && activeConversation ? (
              <>
                <button
                  title="Rename"
                  onClick={() => {
                    setTitleDraft(activeConversation.title);
                    setEditingTitle(true);
                  }}
                >
                  <Pencil size={17} />
                </button>
                <button
                  title={activeConversation.pinned ? "Unpin" : "Pin"}
                  onClick={() => togglePin(activeConversation)}
                >
                  {activeConversation.pinned ? <PinOff size={17} /> : <Pin size={17} />}
                </button>
                <button title="Archive" onClick={() => archiveConversation(activeConversation.id)}>
                  <Archive size={17} />
                </button>
                <button
                  title="Delete"
                  onClick={() => requestDeleteConversation(activeConversation.id)}
                >
                  <Trash2 size={17} />
                </button>
              </>
            ) : null}
            <TitleBarControls />
          </div>
        </header>

        {editingMessageId && (view === "chat" || view === "work") ? (
          <div className="chat-edit-notice" role="status">
            <span>
              Editing opens a new chat with the earlier text history. Original messages,
              attachments and generated files stay in the original chat. Reattach files needed for this edit.
            </span>
            <button type="button" disabled={submitting} onClick={() => {
              setEditingMessageId(null);
              setPrompt("");
              setAttachments([]);
            }}>Cancel edit</button>
          </div>
        ) : null}

        {preferences?.chatHistoryNoticeRequired ? (
          <ChatHistoryNotice
            onAcknowledge={async () => {
              setPreferences(await api.acknowledgeChatHistoryNotice());
            }}
            onOpenPrivacy={() => { setSettingsSection("privacy"); setView("settings"); }}
          />
        ) : null}

        {error || activeRun.error ? (
          <button className="error-banner" onClick={() => { setError(null); chatScope.setError(null); }}>
            {error ?? activeRun.error}
          </button>
        ) : null}

        {backgroundRuns.map((owner) => (
          <div className="background-generation-notice" key={owner ?? "new-chat"} role="status">
            <span>Generation continues in {conversations.find((item) => item.id === owner)?.title ?? "another chat"}.</span>
            {owner ? <button type="button" onClick={() => { setWorkThreadId(null); setView("chat"); setActiveId(owner); }}>Open chat</button> : null}
            {owner && chatState.run(owner).streamingId ? <button type="button" onClick={() => void cancelConversationGeneration(owner)}>Stop this generation</button> : null}
          </div>
        ))}

        {runtime && !runtime.selected ? (
          <div className="runtime-recovery-banner" role="status" aria-live="polite">
            <div>
              <strong>Local AI engine is missing</strong>
              <span>
                {runtimeInstalling
                  ? `Installing with OpenMindAI Download Manager · ${runtimeInstallStatus?.state ?? "resolving"} ${runtimeInstallStatus?.percentage != null ? `(${Math.round(runtimeInstallStatus.percentage)}%)` : ""}`
                  : "Your chats are accessible. Install llama.cpp to run the local model."}
              </span>
              {runtimeInstallError ? <span className="runtime-recovery-error">{runtimeInstallError}</span> : null}
            </div>
            <div className="runtime-recovery-actions">
              {runtimeInstalling ? (
                <button type="button" onClick={() => void cancelRuntimeInstall()}>Cancel</button>
              ) : (
                <button type="button" onClick={() => void installRuntime()}>Retry install</button>
              )}
              <button type="button" onClick={() => { setSettingsSection("runtime"); setView("settings"); }}>
                Runtime settings
              </button>
            </div>
          </div>
        ) : null}

        {routeNotice ? (
          <div className="routing-notice" role="status">
            <span>{routeNotice}</span>
            <button type="button" onClick={() => setRouteNotice(null)} aria-label="Dismiss routing notice">Dismiss</button>
          </div>
        ) : null}
        {view === "chat" ? (
          <ChatView
            conversation={activeConversation}
            messages={messages}
            prompt={prompt}
            setPrompt={setPrompt}
            createVoiceDraftTarget={createVoiceDraftTarget}
            imageOptions={imageOptions}
            setImageOptions={setImageOptions}
            attachments={attachments}
            preferences={preferences}
            activity={activity}
            webProgress={webProgress?.conversationId === activeId ? webProgress : null}
            enterToSend={preferences?.enterToSend ?? true}
            addFiles={addFiles}
            removeAttachment={(id) =>
              setAttachments((items) => items.filter((item) => item.id !== id))
            }
            sendMessage={sendMessage}
            stopGeneration={stopGeneration}
            regenerate={regenerate}
            retry={regenerate}
            editUserMessage={editUserMessage}
            composerRef={composerRef}
            streaming={Boolean(streamingId)}
            submitting={submitting}
            backgroundBusy={backgroundBusy}
            models={models}
            activeModelId={activeModelId}
            runtime={runtime}
            modelSwitching={modelSwitching}
            modelSwitchError={modelSwitchError}
            onSelectModel={selectModel}
            showCloudModels={showCloudApiModels}
            cloudModels={showCloudApiModels ? cloudModels : []}
            activeCloudModelId={showCloudApiModels ? cloudModel : ""}
            onSelectCloudModel={setCloudModel}
            artifactsByMessage={artifactsByMessage}
            onCreateArtifact={createArtifact}
            onOpenArtifact={openArtifactHandler}
            onRevealArtifact={revealArtifactHandler}
            onRetryArtifact={retryArtifact}
            onCancelImageArtifact={cancelImageArtifact}
            imageProgressByArtifact={imageProgressByArtifact}
            onPreview={setPreviewTarget}
          />
        ) : view === "research-studio" || view === "3d-studio" || view === "web-search" ? (
          <StudioHub
            section={view}
            mode={executionMode}
            internetAvailable={internetAvailable}
            connection={cloudConnection}
            onConnected={connectedCloud}
            onDisconnect={() => { setCloudConnection({ connected: false, user: null, message: null }); setCloudModels([]); setCloudModel(""); }}
            onLocalPrompt={submitStudioLocally}
          />
        ) : view === "tools" ? (
          <ToolsWorkspace
            models={models}
            runtime={runtime}
            storage={storage}
            onStartPrompt={startToolPrompt}
            onAttachFiles={(files) => {
              void addFiles(files);
              setView("chat");
            }}
            onOpenLibrary={() => setLibraryOpen(true)}
            onOpenModels={() => setModelsOpen(true)}
            onOpenSettings={(section) => {
              setSettingsSection(section ?? "general");
              setView("settings");
            }}
          />
        ) : view === "work" ? (
          workThreadId ? (
            <ChatView
              conversation={activeConversation}
              messages={messages}
              prompt={prompt}
              setPrompt={setPrompt}
              createVoiceDraftTarget={createVoiceDraftTarget}
              imageOptions={imageOptions}
              setImageOptions={setImageOptions}
              attachments={attachments}
              preferences={preferences}
              activity={activity}
              webProgress={webProgress?.conversationId === activeId ? webProgress : null}
              enterToSend={preferences?.enterToSend ?? true}
              addFiles={addFiles}
              removeAttachment={(id) =>
                setAttachments((items) => items.filter((item) => item.id !== id))
              }
              sendMessage={sendMessage}
              stopGeneration={stopGeneration}
              regenerate={regenerate}
              retry={regenerate}
              editUserMessage={editUserMessage}
              composerRef={composerRef}
              streaming={Boolean(streamingId)}
              submitting={submitting}
              backgroundBusy={backgroundBusy}
              models={models}
              activeModelId={activeModelId}
              runtime={runtime}
              modelSwitching={modelSwitching}
              modelSwitchError={modelSwitchError}
              onSelectModel={selectModel}
              showCloudModels={showCloudApiModels}
              cloudModels={showCloudApiModels ? cloudModels : []}
              activeCloudModelId={showCloudApiModels ? cloudModel : ""}
              onSelectCloudModel={setCloudModel}
              artifactsByMessage={artifactsByMessage}
              onCreateArtifact={createArtifact}
              onOpenArtifact={openArtifactHandler}
              onRevealArtifact={revealArtifactHandler}
              onRetryArtifact={retryArtifact}
              onCancelImageArtifact={cancelImageArtifact}
              imageProgressByArtifact={imageProgressByArtifact}
              onPreview={setPreviewTarget}
            />
          ) : (
            <WorkWorkspace
              conversations={conversations}
              onOpenConversation={(id, draft) => {
                setView("work");
                setWorkThreadId(id);
                setActiveId(id);
                setEditingMessageId(null);
                setAttachments([]);
                setPrompt("");
                if (draft?.trim()) {
                  void sendWorkTask(id, draft.trim());
                }
              }}
              onCreateProjectChat={createProjectConversation}
            />
          )
        ) : view === "projects" ? (
          <ProjectsWorkspace
            conversations={conversations}
            onOpenConversation={(id, draft) => {
              setView("chat");
              setActiveId(id);
              setEditingMessageId(null);
              setAttachments([]);
              if (draft?.trim()) {
                setPrompt(draft.trim());
                window.setTimeout(() => {
                  const node = composerRef.current;
                  if (!node) return;
                  node.focus();
                  node.setSelectionRange(node.value.length, node.value.length);
                }, 0);
              }
            }}
            onCreateProjectChat={createProjectConversation}
          />
        ) : (
          <SettingsDialog
            root={root}
            hardware={hardware}
            storage={storage}
            models={models}
            runtime={runtime}
            runtimeStatus={runtimeStatus}
            performance={performance}
            preferences={preferences}
            userProfile={userProfile}
            updatePreferences={updatePreferences}
            updateUserProfile={updateUserProfile}
            refresh={refreshApp}
            startRuntime={startRuntime}
            stopRuntime={stopRuntime}
            runtimeAction={runtimeAction}
            runtimeInstallStatus={runtimeInstallStatus}
            runtimeInstalling={runtimeInstalling}
            runtimeInstallError={runtimeInstallError}
            installRuntime={() => void installRuntime()}
            cancelRuntimeInstall={() => void cancelRuntimeInstall()}
            initialSection={settingsSection}
          />
        )}
        <footer className="workspace-footer">
          <p className="workspace-copyright">
            &copy; {copyrightYear}{" "}
            <a
              href="https://openmindai.org"
              onClick={(event) => {
                event.preventDefault();
                void api.openExternalUrl("https://openmindai.org");
              }}
            >
              OpenMindAI
            </a>
            . Designed and developed by{" "}
            <a
              href="https://smshagor.com"
              onClick={(event) => {
                event.preventDefault();
                void api.openExternalUrl("https://smshagor.com");
              }}
            >
              Md Shahanur Islam Shagor
            </a>
            . All rights reserved.
          </p>
          {view !== "work" ? (
            <StatusBar
              models={models}
              activeModelId={activeModelId}
              runtime={runtime}
              root={root}
            />
          ) : null}
        </footer>
      </section>

      <PreviewPanel target={previewTarget} onClose={() => setPreviewTarget(null)} />

      <ChatSearch
        open={searchOpen}
        conversations={conversations}
        onSelect={(id) => {
          setView("chat");
          setActiveId(id);
          setEditingMessageId(null);
        }}
        onClose={() => setSearchOpen(false)}
      />
      <Library
        open={libraryOpen}
        onClose={() => setLibraryOpen(false)}
        onOpenConversation={(id) => {
          setView("chat");
          setActiveId(id);
          setEditingMessageId(null);
        }}
      />
      {modelsOpen ? (
        <div className="modal-overlay" role="presentation" onClick={() => setModelsOpen(false)}>
          <div
            className="modal-card models-modal-card"
            role="dialog"
            aria-modal="true"
            aria-label="Models"
            onClick={(event) => event.stopPropagation()}
          >
            <h2>Models</h2>
            <ModelsManager
              hardware={hardware}
              models={models}
              runtime={runtime}
              root={root}
              refresh={refreshApp}
            />
            <div className="modal-actions">
              <button type="button" className="ghost-button" onClick={() => setModelsOpen(false)}>
                Close
              </button>
            </div>
          </div>
        </div>
      ) : null}
      {datasetsOpen ? (
        <div className="modal-overlay" role="presentation" onClick={() => setDatasetsOpen(false)}>
          <div
            className="modal-card models-modal-card datasets-modal-card"
            role="dialog"
            aria-modal="true"
            aria-label="Datasets"
            onClick={(event) => event.stopPropagation()}
          >
            <h2>Datasets</h2>
            <DatasetsManager />
            <div className="modal-actions">
              <button
                type="button"
                className="ghost-button"
                onClick={() => setDatasetsOpen(false)}
              >
                Close
              </button>
            </div>
          </div>
        </div>
      ) : null}
      <ConfirmDialog
        open={deleteTarget !== null}
        title="Delete this conversation?"
        description="This can't be undone."
        confirmLabel="Delete"
        danger
        onConfirm={() => deleteTarget && void performDelete(deleteTarget)}
        onCancel={() => setDeleteTarget(null)}
      />
    </main>
  );
}



function upsertArtifactInList(items: Artifact[], next: Artifact) {
  const index = items.findIndex((item) => item.id === next.id);
  if (index === -1) return [...items, next];
  const copy = items.slice();
  copy[index] = next;
  return copy;
}

function generationKindForMode(mode: ChatMode): "image" | "video" | "voice" | null {
  if (mode === "image" || mode === "video" || mode === "voice") return mode;
  return null;
}

function persistedChatMode(content: string): ChatMode {
  if (content.startsWith("[Mode: Web Search]")) return "search";
  if (content.startsWith("[Mode: Deep Research]")) return "research";
  if (content.startsWith("[Mode: Think carefully before answering.")) return "thinking";
  if (content.startsWith("[Mode: Document Writer]")) return "document";
  if (content.startsWith("[Mode: PDF]")) return "pdf";
  if (content.startsWith("[Mode: Image Creation]")) return "image";
  if (content.startsWith("[Mode: Video Creation]")) return "video";
  if (content.startsWith("[Mode: Voice Creation]")) return "voice";
  if (content.startsWith("[Mode: Image/Vision Review]")) return "vision";
  return "chat";
}
