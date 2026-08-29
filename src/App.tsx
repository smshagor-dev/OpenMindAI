import { Archive, Check, Pencil, Pin, PinOff, Trash2 } from "lucide-react";
import { listen } from "@tauri-apps/api/event";
import { check as checkForAppUpdate } from "@tauri-apps/plugin-updater";
import { useCallback, useEffect, useMemo, useRef, useState, type SetStateAction } from "react";
import { io, type Socket } from "socket.io-client";
import { api } from "./api";
import type {
  Artifact,
  ArtifactKind,
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
  StreamChunkEvent,
  StreamDoneEvent,
  StreamStartedEvent,
  StorageSummary,
  UserProfile,
} from "./types";
import { Sidebar } from "./components/Sidebar";
import { ChatView } from "./components/ChatView";
import { SettingsDialog } from "./components/SettingsDialog";
import { SetupWizard } from "./components/SetupWizard";
import { ChatSearch } from "./components/ChatSearch";
import { ConfirmDialog } from "./components/ConfirmDialog";
import { ChatModeSwitcher } from "./components/ChatModeSwitcher";
import { Library } from "./components/Library";
import { ModelsManager } from "./components/ModelsManager";
import { TitleBarControls } from "./components/TitleBarControls";
import { StatusBar } from "./components/StatusBar";
import { PreviewPanel, type PreviewTarget } from "./components/PreviewPanel";
import { ToolsWorkspace } from "./components/ToolsWorkspace";
import { ProjectsWorkspace } from "./components/ProjectsWorkspace";
import { notifyUser } from "./lib/notify";
import {
  attachmentMedia,
  buildMessageContent,
  inferChatMode,
  isUntitledConversation,
  mergeStreamingSnapshot,
  readAttachment,
  settledValue,
  titleFromPrompt,
  type AttachmentDraft,
  type ChatMode,
} from "./lib/chat";
import { formatError } from "./lib/format";

type View = "chat" | "settings" | "tools" | "projects";

interface RealtimeActivity {
  typing: boolean;
  searching: boolean;
  researching: boolean;
  detail: string | null;
}

export function App() {
  const [view, setView] = useState<View>("chat");
  const [root, setRoot] = useState<PortableRootInfo | null>(null);
  const [installationStatus, setInstallationStatus] = useState<InstallationStatus | null>(null);
  const [wizardActive, setWizardActive] = useState<boolean | null>(null);
  const [refreshedOnce, setRefreshedOnce] = useState(false);
  const [hardware, setHardware] = useState<HardwareProfile | null>(null);
  const [storage, setStorage] = useState<StorageSummary | null>(null);
  const [models, setModels] = useState<ModelRecord[]>([]);
  const [runtime, setRuntime] = useState<RuntimeInventory | null>(null);
  const [runtimeStatus, setRuntimeStatus] = useState<LlamaRuntimeStatus | null>(null);
  const [performance, setPerformance] = useState<PerformanceProfile | null>(null);
  const [preferences, setPreferences] = useState<AppPreferences | null>(null);
  const [userProfile, setUserProfile] = useState<UserProfile | null>(null);
  const [conversations, setConversations] = useState<Conversation[]>([]);
  const [activeId, setActiveId] = useState<string | null>(null);
  const [messages, setMessages] = useState<Message[]>([]);
  const [artifacts, setArtifacts] = useState<Artifact[]>([]);
  const [prompt, setPrompt] = useState("");
  const [attachments, setAttachments] = useState<AttachmentDraft[]>([]);
  const [editingMessageId, setEditingMessageId] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const [editingTitle, setEditingTitle] = useState(false);
  const [titleDraft, setTitleDraft] = useState("");
  const [activity, setActivity] = useState<RealtimeActivity>({
    typing: false,
    searching: false,
    researching: false,
    detail: null,
  });
  const [error, setError] = useState<string | null>(null);
  const [streamingId, setStreamingId] = useState<string | null>(null);
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false);
  const [sidebarWidth, setSidebarWidth] = useState(() => {
    const stored = Number(window.localStorage.getItem("sidebarWidth"));
    return stored >= 260 && stored <= 440 ? stored : 320;
  });
  const [searchOpen, setSearchOpen] = useState(false);
  const [libraryOpen, setLibraryOpen] = useState(false);
  const [modelsOpen, setModelsOpen] = useState(false);
  const [settingsSection, setSettingsSection] = useState("general");
  const [previewTarget, setPreviewTarget] = useState<PreviewTarget | null>(null);
  const [pendingModelId, setPendingModelId] = useState<string | null>(null);
  const [modelSwitching, setModelSwitching] = useState(false);
  const [modelSwitchError, setModelSwitchError] = useState<string | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<string | null>(null);
  const composerRef = useRef<HTMLTextAreaElement | null>(null);

  const showError = useCallback((caught: unknown) => {
    setError(formatError(caught));
  }, []);

  const refreshApp = useCallback(async () => {
    const results = await Promise.allSettled([
      api.root(),
      api.conversations(),
      api.hardware(),
      api.storage(),
      api.models(),
      api.runtimeInventory(),
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
      setActiveId((current) =>
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
  }, [showError]);

  useEffect(() => {
    void refreshApp();
  }, [refreshApp]);

  useEffect(() => {
    if (!refreshedOnce || installationStatus === null || wizardActive !== null) return;
    const modelReady = models.some((model) => model.state === "ready" || model.state === "loaded");
    const runtimeReady = runtime?.selected != null;
    setWizardActive(installationStatus.setupRequired || !modelReady || !runtimeReady);
  }, [refreshedOnce, installationStatus, models, runtime, wizardActive]);

  const updateCheckStartedRef = useRef(false);
  useEffect(() => {
    if (!preferences || updateCheckStartedRef.current) return;
    updateCheckStartedRef.current = true;
    if (!preferences.autoCheckAppUpdates) return;
    void checkForAppUpdate()
      .then(async (update) => {
        if (!update) return;
        try {
          await notifyUser(
            "OpenMindAI update available",
            `Version ${update.version} is ready to download.`,
          );
          if (preferences.autoDownloadAppUpdates) {
            await update.downloadAndInstall();
            await notifyUser(
              "OpenMindAI is ready to update",
              "Restart OpenMindAI to finish installing the update.",
            );
          }
        } finally {
          await update.close();
        }
      })
      .catch(() => undefined);
  }, [preferences]);

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
    if (preferences.theme !== "system") {
      document.documentElement.dataset.theme = preferences.theme;
      return;
    }
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    const apply = () => {
      document.documentElement.dataset.theme = media.matches ? "dark" : "light";
    };
    apply();
    media.addEventListener("change", apply);
    return () => media.removeEventListener("change", apply);
  }, [preferences]);

  useEffect(() => {
    if (!preferences?.socketRealtimeEnabled || !preferences.socketUrl.trim()) {
      setActivity({ typing: false, searching: false, researching: false, detail: null });
      return;
    }

    const socket: Socket = io(preferences.socketUrl, {
      transports: ["websocket"],
      reconnectionAttempts: 3,
    });

    socket.on("typing:start", (detail?: string) =>
      setActivity((current) => ({ ...current, typing: true, detail: detail ?? current.detail })),
    );
    socket.on("typing:stop", () =>
      setActivity((current) => ({ ...current, typing: false, detail: current.detail })),
    );
    socket.on("search:start", (detail?: string) =>
      setActivity((current) => ({ ...current, searching: true, detail: detail ?? "Searching..." })),
    );
    socket.on("search:stop", () =>
      setActivity((current) => ({ ...current, searching: false, detail: current.detail })),
    );
    socket.on("research:start", (detail?: string) =>
      setActivity((current) => ({
        ...current,
        researching: true,
        detail: detail ?? "Researching...",
      })),
    );
    socket.on("research:stop", () =>
      setActivity((current) => ({ ...current, researching: false, detail: current.detail })),
    );

    return () => {
      socket.disconnect();
    };
  }, [preferences?.socketRealtimeEnabled, preferences?.socketUrl]);

  useEffect(() => {
    if (!activeId) {
      setMessages([]);
      setArtifacts([]);
      setEditingMessageId(null);
      return;
    }
    api
      .messages(activeId)
      .then((items) => setMessages(items.filter((message) => message.role !== "system")))
      .catch(showError);
    api.listArtifacts(activeId).then(setArtifacts).catch(showError);
  }, [activeId, showError]);

  useEffect(() => {
    const unlisteners = [
      listen<StreamStartedEvent>("inference:started", (event) => {
        setMessages((items) => [
          ...items.filter(
            (message) =>
              message.id !== event.payload.user.id &&
              message.id !== event.payload.assistant.id &&
              !(
                message.id.startsWith("local-") &&
                message.conversationId === event.payload.conversationId &&
                message.content === event.payload.user.content
              ),
          ),
          event.payload.user,
          event.payload.assistant,
        ]);
        setStreamingId(event.payload.assistant.id);
        setSubmitting(false);
        setActivity((current) => ({ ...current, typing: true }));
        setConversations((items) =>
          items.map((item) =>
            item.id === event.payload.conversationId
              ? { ...item, activeModelId: event.payload.assistant.modelId }
              : item,
          ),
        );
      }),
      listen<StreamChunkEvent>("inference:chunk", (event) => {
        setMessages((items) =>
          items.map((message) =>
            message.id === event.payload.messageId && message.status === "streaming"
              ? { ...message, content: message.content + event.payload.chunk, status: "streaming" }
              : message,
          ),
        );
      }),
      listen<StreamDoneEvent>("inference:done", (event) => {
        setMessages((items) =>
          items.map((message) =>
            message.id === event.payload.messageId
              ? { ...message, status: event.payload.status }
              : message,
          ),
        );
        setStreamingId((current) => (current === event.payload.messageId ? null : current));
        setSubmitting(false);
        setActivity((current) => ({ ...current, typing: false, detail: null }));
      }),
      listen<Artifact>("artifact:started", (event) =>
        setArtifacts((items) => upsertArtifactInList(items, event.payload)),
      ),
      listen<Artifact>("artifact:done", (event) =>
        setArtifacts((items) => upsertArtifactInList(items, event.payload)),
      ),
    ];
    return () => {
      void Promise.all(unlisteners).then((items) => items.forEach((unlisten) => unlisten()));
    };
  }, []);

  useEffect(() => {
    if (!activeId || !streamingId) return;
    const interval = window.setInterval(() => {
      api
        .messages(activeId)
        .then((items) => {
          const visible = items.filter((message) => message.role !== "system");
          setMessages((current) => mergeStreamingSnapshot(current, visible));
          const streamedMessage = visible.find((message) => message.id === streamingId);
          if (streamedMessage && streamedMessage.status !== "streaming") {
            setStreamingId(null);
          }
        })
        .catch(showError);
    }, 1500);
    return () => window.clearInterval(interval);
  }, [activeId, showError, streamingId]);

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
  }, []);

  const sendMessage = useCallback(async () => {
    const content = prompt.trim();
    if ((!content && attachments.length === 0) || streamingId || submitting) return;

    const draftPrompt = prompt;
    const draftAttachments = attachments.slice();
    const editTargetId = editingMessageId;
    const inferredMode = inferChatMode(content, attachments);
    const messageContent = buildMessageContent(content, attachments, inferredMode);
    const inferenceMedia = attachmentMedia(attachments);
    let conversationId = activeId;
    let conversationForTitle =
      conversations.find((conversation) => conversation.id === conversationId) ?? null;
    let optimisticUser: Message | null = null;
    let editedBranchChanged = false;

    setSubmitting(true);
    try {
      if (!conversationId) {
        const conversation = await api.createConversation();
        conversationId = conversation.id;
        conversationForTitle = conversation;
        setConversations((items) => [conversation, ...items]);
        setActiveId(conversation.id);

        if (pendingModelId) {
          setModelSwitching(true);
          try {
            await api.activateModel(conversationId, pendingModelId);
            setConversations((items) =>
              items.map((item) =>
                item.id === conversationId ? { ...item, activeModelId: pendingModelId } : item,
              ),
            );
          } catch (caught) {
            setModelSwitchError(formatError(caught));
          } finally {
            setModelSwitching(false);
            setPendingModelId(null);
          }
        }
      }

      if (editTargetId) {
        const targetIndex = messages.findIndex((message) => message.id === editTargetId);
        if (targetIndex < 0) {
          throw new Error("The message being edited is no longer available. Reload the chat and try again.");
        }
        const branch = messages.slice(targetIndex);
        editedBranchChanged = branch.length > 0;
        for (const message of branch.slice().reverse()) {
          await api.deleteMessage(message.id);
        }
        setMessages((items) => {
          const index = items.findIndex((message) => message.id === editTargetId);
          return index >= 0 ? items.slice(0, index) : items;
        });
        setEditingMessageId(null);
      }

      setPrompt("");
      setAttachments([]);
      optimisticUser = {
        id: `local-${crypto.randomUUID()}`,
        conversationId,
        role: "user",
        content: messageContent,
        status: "completed",
        modelId: null,
        createdAt: new Date().toISOString(),
        updatedAt: new Date().toISOString(),
      };
      setMessages((items) => [...items, optimisticUser as Message]);

      if (preferences?.autoGenerateTitles ?? true) {
        const generatedTitle = titleFromPrompt(
          content || attachments[0]?.name || "New conversation",
        );
        if (
          conversationForTitle &&
          isUntitledConversation(conversationForTitle.title) &&
          generatedTitle
        ) {
          await api.renameConversation(conversationId, generatedTitle);
          setConversations((items) =>
            items.map((conversation) =>
              conversation.id === conversationId
                ? { ...conversation, title: generatedTitle }
                : conversation,
            ),
          );
        }
      }

      const assistant = await api.sendChatMessage(
        conversationId,
        messageContent,
        inferredMode,
        inferenceMedia,
      );
      const generationKind = generationKindForMode(inferredMode);
      if (generationKind) {
        const generationPrompt = assistant.content.trim() || content;
        const artifact = await api.createGenerationArtifact(
          conversationId,
          assistant.id,
          generationKind,
          generationPrompt,
        );
        setArtifacts((items) => upsertArtifactInList(items, artifact));
        if (preferences?.openArtifactsAfterGeneration && artifact.status === "ready") {
          void api.openArtifact(artifact.id).catch(showError);
        }
      }
      setStreamingId(null);
      await refreshMessages(conversationId, setMessages, showError);
      await refreshApp();
    } catch (caught) {
      if (optimisticUser) {
        setMessages((items) =>
          items
            .filter((message) => message.id !== optimisticUser?.id)
            .map((message) =>
              message.conversationId === conversationId && message.status === "streaming"
                ? { ...message, status: "failed" }
                : message,
            ),
        );
      }
      setStreamingId(null);
      setPrompt(draftPrompt);
      setAttachments(draftAttachments);
      setEditingMessageId(editedBranchChanged ? null : editTargetId);
      showError(caught);
      if (conversationId) {
        await refreshMessages(conversationId, setMessages, showError);
      }
    } finally {
      setSubmitting(false);
    }
  }, [
    activeId,
    attachments,
    conversations,
    editingMessageId,
    messages,
    pendingModelId,
    preferences?.autoGenerateTitles,
    preferences?.openArtifactsAfterGeneration,
    prompt,
    refreshApp,
    showError,
    streamingId,
    submitting,
  ]);

  const stopGeneration = useCallback(async () => {
    if (!streamingId || !activeId) return;
    try {
      await api.cancelGeneration(activeId);
    } catch (caught) {
      showError(caught);
    }
  }, [activeId, showError, streamingId]);

  const editUserMessage = useCallback((messageId: string, content: string) => {
    setEditingMessageId(messageId);
    setPrompt(content);
    setAttachments([]);
    setView("chat");
    window.setTimeout(() => {
      const node = composerRef.current;
      if (!node) return;
      node.focus();
      node.setSelectionRange(node.value.length, node.value.length);
    }, 0);
  }, []);

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
  }, []);

  const regenerate = useCallback(
    async (assistantMessageId: string) => {
      if (!activeId || streamingId || submitting) return;
      const targetIndex = messages.findIndex((message) => message.id === assistantMessageId);
      const user =
        targetIndex > 0
          ? messages
              .slice(0, targetIndex)
              .reverse()
              .find((message) => message.role === "user")
          : null;
      const mode = user ? persistedChatMode(user.content) : "chat";
      setSubmitting(true);
      setMessages((items) => items.filter((message) => message.id !== assistantMessageId));
      try {
        await api.regenerateMessage(activeId, assistantMessageId, mode);
        setStreamingId(null);
        await refreshMessages(activeId, setMessages, showError);
        await refreshApp();
      } catch (caught) {
        showError(caught);
        await refreshMessages(activeId, setMessages, showError);
      } finally {
        setSubmitting(false);
      }
    },
    [activeId, messages, refreshApp, showError, streamingId, submitting],
  );

  const selectModel = useCallback(
    async (modelId: string) => {
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
    [activeId],
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
    const results = await Promise.allSettled(Array.from(files).map(readAttachment));
    const loaded = results.flatMap((result) => (result.status === "fulfilled" ? [result.value] : []));
    const failures = results.flatMap((result) =>
      result.status === "rejected" ? [formatError(result.reason)] : [],
    );

    const accepted: AttachmentDraft[] = [];
    let imageCount = attachments.filter((item) => item.kind === "image").length;
    for (const attachment of loaded) {
      if (attachments.length + accepted.length >= 8) {
        failures.push("A chat can contain at most 8 attachments at once.");
        continue;
      }
      if (attachment.kind === "image" && imageCount >= 4) {
        failures.push("Local vision supports at most 4 images in one request.");
        continue;
      }
      accepted.push(attachment);
      if (attachment.kind === "image") imageCount += 1;
    }

    if (accepted.length > 0) {
      setAttachments((items) => [...items, ...accepted]);
      setEditingMessageId(null);
      setView("chat");
      composerRef.current?.focus();
    }
    if (failures.length > 0) {
      showError(new Error(Array.from(new Set(failures)).join(" ")));
    }
  }

  async function archiveConversation(id: string) {
    await api.archiveConversation(id);
    setConversations((items) => items.filter((item) => item.id !== id));
    if (activeId === id) {
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
    if (activeId === id) {
      setActiveId(null);
      setMessages([]);
      setEditingMessageId(null);
      setEditingTitle(false);
      setTitleDraft("");
    }
    setDeleteTarget(null);
  }

  async function duplicateConversation(conversation: Conversation) {
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
    setActiveId(created.id);
    setEditingMessageId(null);
    setView("chat");
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

  async function startRuntime() {
    setRuntimeStatus(await api.startRuntime());
    setRuntime(await api.runtimeInventory());
  }

  async function stopRuntime() {
    await api.stopRuntime();
    setRuntimeStatus(await api.runtimeStatus());
    setRuntime(await api.runtimeInventory());
  }

  async function updatePreferences(next: AppPreferences) {
    setPreferences(next);
    setPreferences(await api.savePreferences(next));
  }

  async function updateUserProfile(next: UserProfile) {
    setUserProfile(next);
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
    [activeId, preferences?.openArtifactsAfterGeneration, showError],
  );

  const retryArtifact = useCallback(
    (artifact: Artifact) => {
      const source = messages.find((message) => message.id === artifact.messageId);
      if (!source || !artifact.messageId || !activeId) {
        showError("The original message for this file is no longer available.");
        return;
      }
      if (artifact.kind === "image" || artifact.kind === "video" || artifact.kind === "audio") {
        const generationKind = artifact.kind === "audio" ? "voice" : artifact.kind;
        void api
          .createGenerationArtifact(activeId, artifact.messageId, generationKind, source.content)
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
    [activeId, createArtifact, messages, preferences?.openArtifactsAfterGeneration, showError],
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
        onOpenTools={() => setView("tools")}
        onOpenProjects={() => setView("projects")}
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
                    : view === "projects"
                      ? "Projects"
                      : "Settings"}
              </h1>
            )}
          </div>
          <div className="topbar-center">
            <ChatModeSwitcher />
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

        {error ? (
          <button className="error-banner" onClick={() => setError(null)}>
            {error}
          </button>
        ) : null}

        {view === "chat" ? (
          <ChatView
            conversation={activeConversation}
            messages={messages}
            prompt={prompt}
            setPrompt={setPrompt}
            attachments={attachments}
            preferences={preferences}
            activity={activity}
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
            models={models}
            activeModelId={activeModelId}
            runtime={runtime}
            modelSwitching={modelSwitching}
            modelSwitchError={modelSwitchError}
            onSelectModel={selectModel}
            artifactsByMessage={artifactsByMessage}
            onCreateArtifact={createArtifact}
            onOpenArtifact={openArtifactHandler}
            onRevealArtifact={revealArtifactHandler}
            onRetryArtifact={retryArtifact}
            onPreview={setPreviewTarget}
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
        ) : view === "projects" ? (
          <ProjectsWorkspace
            conversations={conversations}
            onOpenConversation={(id) => {
              setView("chat");
              setActiveId(id);
              setEditingMessageId(null);
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
            initialSection={settingsSection}
          />
        )}
        <StatusBar models={models} activeModelId={activeModelId} runtime={runtime} root={root} />
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

async function refreshMessages(
  conversationId: string,
  setMessages: (value: SetStateAction<Message[]>) => void,
  showError: (caught: unknown) => void,
) {
  try {
    const items = await api.messages(conversationId);
    setMessages(items.filter((message) => message.role !== "system"));
  } catch (caught) {
    showError(caught);
  }
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
