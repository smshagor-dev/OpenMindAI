import { useEffect, useRef, useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Brain,
  Cpu,
  Database,
  Download,
  FolderOpen,
  Globe,
  HardDrive,
  Info as InfoIcon,
  Keyboard,
  LoaderCircle,
  MessageSquarePlus,
  Palette,
  Play,
  Plug,
  RefreshCw,
  Server,
  Settings,
  Shield,
  SlidersHorizontal,
  StopCircle,
  Trash2,
  User,
  Wrench,
  Bot,
} from "lucide-react";
import packageJson from "../../package.json";
import type {
  AppPreferences,
  HardwareProfile,
  LlamaRuntimeStatus,
  ModelRecord,
  PerformanceProfile,
  PortableRootInfo,
  RuntimeInventory,
  RuntimeInstallStatus,
  StorageSummary,
  UserProfile,
} from "../types";
import { formatBytes, formatError } from "../lib/format";
import { readImageAsDataUrl } from "../lib/avatar";
import { ModelsManager } from "./ModelsManager";
import { AppsSettings } from "./AppsSettings";
import { MaintenanceCenter } from "./MaintenanceCenter";
import { UpdatesPanel } from "./UpdatesPanel";
import { AgentSettings } from "./AgentSettings";
import { DiffusionRuntimePanel } from "./DiffusionRuntimePanel";

export function SettingsDialog(props: {
  root: PortableRootInfo | null;
  hardware: HardwareProfile | null;
  storage: StorageSummary | null;
  models: ModelRecord[];
  runtime: RuntimeInventory | null;
  runtimeStatus: LlamaRuntimeStatus | null;
  performance: PerformanceProfile | null;
  preferences: AppPreferences | null;
  userProfile: UserProfile | null;
  updatePreferences: (preferences: AppPreferences) => void;
  updateUserProfile: (profile: UserProfile) => void;
  refresh: () => void;
  startRuntime: (action?: "start" | "restart") => void;
  stopRuntime: () => void;
  runtimeAction: "start" | "restart" | "stop" | null;
  runtimeInstallStatus: RuntimeInstallStatus | null;
  runtimeInstalling: boolean;
  runtimeInstallError: string | null;
  installRuntime: () => void;
  cancelRuntimeInstall: () => void;
  initialSection?: string;
}) {
  const [activeSection, setActiveSection] = useState(props.initialSection ?? "general");
  const [preferenceDraft, setPreferenceDraft] = useState<AppPreferences | null>(props.preferences);
  const [profileDraft, setProfileDraft] = useState<UserProfile | null>(props.userProfile);
  const [saveState, setSaveState] = useState<"idle" | "saving" | "saved">("idle");
  const [avatarError, setAvatarError] = useState<string | null>(null);
  const [networkError, setNetworkError] = useState<string | null>(null);
  const [webConnectivity, setWebConnectivity] = useState("Not checked");
  const [providerHealth, setProviderHealth] = useState<{ name: string; status: string }[]>([]);
  const [webCredentialPresence, setWebCredentialPresence] = useState({
    braveConfigured: false,
    googleConfigured: false,
    googleKeyConfigured: false,
    googleEngineConfigured: false,
  });
  const [webCredentialDraft, setWebCredentialDraft] = useState({
    brave: "", googleKey: "", googleEngine: "",
  });
  const [webCredentialMessage, setWebCredentialMessage] = useState("");
  const saveWebCredential = async (field: "brave" | "googleKey" | "googleEngine", value: string) => {
    setWebCredentialMessage("");
    try {
      const result = await invoke<typeof webCredentialPresence>("save_web_provider_credential", {
        field,
        value,
      });
      setWebCredentialPresence(result);
      setWebCredentialDraft((current) => ({ ...current, [field]: "" }));
      setWebCredentialMessage(value ? "Credential saved in the platform secret store." : "Local credential cleared. An environment variable may still supply one.");
      void checkWebConnectivity();
    } catch {
      setWebCredentialMessage("Credential update failed. Nothing was saved.");
    }
  };
  const checkWebConnectivity = async () => {
    setWebConnectivity("Checking internet and search providers...");
    try {
      const result = await invoke<{
        status: string;
        provider: string;
        providers: { name: string; status: string; elapsedMs: number | null }[];
      }>("check_web_connectivity");
      setProviderHealth(result.providers);
      if (result.status === "online") setWebConnectivity(`Online — ${result.provider} reachable`);
      else if (result.status === "restricted") setWebConnectivity("Internet reachable, but search access is restricted");
      else if (result.status === "disabled") setWebConnectivity("Live web features disabled");
      else setWebConnectivity("Offline — local chat is still available");
    } catch {
      setWebConnectivity("Connectivity check failed — search may be unavailable");
    }
  };
  useEffect(() => {
    if (activeSection !== "capabilities") return;
    void invoke<typeof webCredentialPresence>("web_provider_credential_status")
      .then(setWebCredentialPresence)
      .catch(() => setWebCredentialMessage("Could not read the local credential store."));
    void checkWebConnectivity();
    const timer = window.setInterval(() => { void checkWebConnectivity(); }, 60_000);
    return () => window.clearInterval(timer);
  }, [activeSection]);

  const avatarInputRef = useRef<HTMLInputElement | null>(null);

  useEffect(() => {
    setPreferenceDraft(props.preferences);
  }, [props.preferences]);

  useEffect(() => {
    setProfileDraft(props.userProfile);
  }, [props.userProfile]);

  const setPreference = <K extends keyof AppPreferences>(key: K, value: AppPreferences[K]) => {
    setPreferenceDraft((current) => (current ? { ...current, [key]: value } : current));
    setSaveState("idle");
  };
  const setProfile = <K extends keyof UserProfile>(key: K, value: UserProfile[K]) => {
    setProfileDraft((current) => (current ? { ...current, [key]: value } : current));
    setSaveState("idle");
  };
  const uploadAvatar = async (file: File) => {
    setAvatarError(null);
    if (!file.type.startsWith("image/")) {
      setAvatarError("Please choose an image file.");
      return;
    }
    try {
      const dataUrl = await readImageAsDataUrl(file);
      setProfile("avatarDataUrl", dataUrl);
    } catch {
      setAvatarError("Could not read that image. Try a different file.");
    }
  };
  const savePreferences = async () => {
    if (!preferenceDraft) return;
    setSaveState("saving");
    await props.updatePreferences(preferenceDraft);
    setSaveState("saved");
  };
  const saveNetwork = async () => {
    if (!preferenceDraft) return;
    setNetworkError(null);
    if (preferenceDraft.networkProxyMode === "manual" && !preferenceDraft.networkProxyUrl.trim()) {
      setNetworkError("Enter a proxy address, for example 127.0.0.1:8080.");
      return;
    }
    try {
      await savePreferences();
    } catch (error) {
      setSaveState("idle");
      setNetworkError(formatError(error));
    }
  };
  const saveProfile = async () => {
    if (!profileDraft) return;
    setSaveState("saving");
    await props.updateUserProfile(profileDraft);
    setSaveState("saved");
  };
  const sections = [
    { id: "general", label: "General", icon: Settings },
    { id: "personalization", label: "Personalization", icon: User },
    { id: "appearance", label: "Appearance", icon: Palette },
    { id: "chat", label: "Chat", icon: MessageSquarePlus },
    { id: "capabilities", label: "Capabilities", icon: Brain },
    { id: "openagent", label: "Agent Setup", icon: Bot },
    { id: "connectors", label: "Apps", icon: Plug },
    { id: "models", label: "Models", icon: Database },
    { id: "runtime", label: "AI Runtime", icon: Server },
    { id: "hardware", label: "Hardware", icon: Cpu },
    { id: "storage", label: "Storage", icon: HardDrive },
    { id: "maintenance", label: "Maintenance", icon: Wrench },
    { id: "network", label: "Network", icon: Globe },
    { id: "updates", label: "Updates", icon: Download },
    { id: "files", label: "Files & Artifacts", icon: FolderOpen },
    { id: "privacy", label: "Privacy", icon: Shield },
    { id: "shortcuts", label: "Shortcuts", icon: Keyboard },
    { id: "advanced", label: "Advanced", icon: SlidersHorizontal },
    { id: "about", label: "About", icon: InfoIcon },
  ];

  return (
    <div className="settings-panel">
      <nav className="settings-menu">
        {sections.map((section) => {
          const Icon = section.icon;
          return (
            <button
              className={
                activeSection === section.id ? "settings-menu-item active" : "settings-menu-item"
              }
              key={section.id}
              onClick={() => setActiveSection(section.id)}
            >
              <Icon size={17} />
              <span>{section.label}</span>
            </button>
          );
        })}
      </nav>
      <section className="settings-content">
        {activeSection === "general" && preferenceDraft ? (
          <SettingsGroup title="General">
            <SelectRow
              label="Language"
              value={preferenceDraft.language}
              options={["English", "Bangla", "Hindi"]}
              onChange={(value) => setPreference("language", value)}
            />
            <ToggleRow
              label="Compact sidebar"
              checked={preferenceDraft.compactSidebar}
              onChange={(value) => setPreference("compactSidebar", value)}
            />
            <ToggleRow
              label="Confirm before delete"
              checked={preferenceDraft.confirmBeforeDelete}
              onChange={(value) => setPreference("confirmBeforeDelete", value)}
            />
            <SaveBar state={saveState} onSave={savePreferences} />
          </SettingsGroup>
        ) : null}

        {activeSection === "appearance" && preferenceDraft ? (
          <SettingsGroup title="Appearance">
            <SegmentRow
              label="Theme"
              value={preferenceDraft.theme}
              options={["system", "dark", "light"]}
              onChange={(value) => {
                const theme = value as AppPreferences["theme"];
                setPreference("theme", theme);
                if (props.preferences) {
                  void props.updatePreferences({ ...props.preferences, theme });
                }
              }}
            />
            <ToggleRow
              label="Show shortcut hints"
              checked={preferenceDraft.showShortcutHints}
              onChange={(value) => setPreference("showShortcutHints", value)}
            />
            <SaveBar state={saveState} onSave={savePreferences} />
          </SettingsGroup>
        ) : null}

        {activeSection === "personalization" && profileDraft ? (
          <SettingsGroup title="Personalization">
            <div className="avatar-row">
              <span className="avatar-preview">
                {profileDraft.avatarDataUrl ? (
                  <img src={profileDraft.avatarDataUrl} alt="Profile" />
                ) : (
                  (profileDraft.preferredName || profileDraft.fullName || "Local User")
                    .charAt(0)
                    .toUpperCase()
                )}
              </span>
              <div className="avatar-row-actions">
                <input
                  ref={avatarInputRef}
                  type="file"
                  accept="image/*"
                  className="hidden-file-input"
                  onChange={(event) => {
                    const file = event.target.files?.[0];
                    if (file) void uploadAvatar(file);
                    event.currentTarget.value = "";
                  }}
                />
                <button
                  type="button"
                  className="ghost-button"
                  onClick={() => avatarInputRef.current?.click()}
                >
                  Upload photo
                </button>
                {profileDraft.avatarDataUrl ? (
                  <button
                    type="button"
                    className="ghost-button"
                    onClick={() => setProfile("avatarDataUrl", "")}
                  >
                    <Trash2 size={14} /> Remove
                  </button>
                ) : null}
                {avatarError ? <p className="muted avatar-error">{avatarError}</p> : null}
              </div>
            </div>
            <TextRow
              label="Full name"
              value={profileDraft.fullName}
              onChange={(value) => setProfile("fullName", value)}
            />
            <TextRow
              label="Preferred name"
              value={profileDraft.preferredName}
              onChange={(value) => setProfile("preferredName", value)}
            />
            <TextRow
              label="Email"
              value={profileDraft.email}
              onChange={(value) => setProfile("email", value)}
            />
            <TextRow
              label="Occupation"
              value={profileDraft.occupation}
              onChange={(value) => setProfile("occupation", value)}
            />
            <TextRow
              label="Response style"
              value={profileDraft.responseStyle}
              onChange={(value) => setProfile("responseStyle", value)}
            />
            <TextAreaRow
              label="About you"
              value={profileDraft.about}
              onChange={(value) => setProfile("about", value)}
            />
            <TextAreaRow
              label="Custom instructions"
              value={profileDraft.customInstructions}
              onChange={(value) => setProfile("customInstructions", value)}
            />
            <p className="muted">
              This profile is stored locally and added as hidden system context for conversations.
            </p>
            <SaveBar state={saveState} onSave={saveProfile} />
          </SettingsGroup>
        ) : null}

        {activeSection === "chat" && preferenceDraft ? (
          <SettingsGroup title="Chat">
            <ToggleRow
              label="Enter to send"
              checked={preferenceDraft.enterToSend}
              onChange={(value) => setPreference("enterToSend", value)}
            />
            <ToggleRow
              label="Auto-generate titles"
              checked={preferenceDraft.autoGenerateTitles}
              onChange={(value) => setPreference("autoGenerateTitles", value)}
            />
            <ToggleRow
              label="Markdown rendering"
              checked={preferenceDraft.markdownRendering}
              onChange={(value) => setPreference("markdownRendering", value)}
            />
            <ToggleRow
              label="Code copy buttons"
              checked={preferenceDraft.codeCopyButtons}
              onChange={(value) => setPreference("codeCopyButtons", value)}
            />
            <SaveBar state={saveState} onSave={savePreferences} />
          </SettingsGroup>
        ) : null}

        {activeSection === "capabilities" && preferenceDraft ? (
          <SettingsGroup title="Capabilities">
            <ToggleRow
              label="Thinking mode"
              checked={preferenceDraft.thinkingModeEnabled}
              onChange={(value) => setPreference("thinkingModeEnabled", value)}
            />
            <ToggleRow
              label="Web search mode"
              checked={preferenceDraft.webSearchEnabled}
              onChange={(value) => setPreference("webSearchEnabled", value)}
            />
            <ToggleRow
              label="Deep research mode"
              checked={preferenceDraft.deepResearchEnabled}
              onChange={(value) => setPreference("deepResearchEnabled", value)}
            />
            <label className="setting-row">
              <span>Preferred search provider</span>
              <select
                value={preferenceDraft.webSearchPreferredProvider ?? "auto"}
                onChange={(event) => setPreference(
                  "webSearchPreferredProvider",
                  event.target.value as AppPreferences["webSearchPreferredProvider"],
                )}
              >
                <option value="auto">Automatic fallback</option>
                <option value="brave">Brave Search (requires API key)</option>
                <option value="google">Google Custom Search (existing accounts only)</option>
                <option value="duckduckgo">DuckDuckGo</option>
                <option value="bing">Bing</option>
              </select>
            </label>
            <p className="muted">
              Unavailable and rate-limited providers fall back automatically.
              Keys are saved in your operating system&apos;s credential store,
              never in chat history or ordinary app preferences.
            </p>
            {(["brave", "googleKey", "googleEngine"] as const).map((field) => (
              <label className="setting-row" key={field}>
                <span>{field === "brave" ? "Brave API key" : field === "googleKey" ? "Google API key" : "Google Search engine ID"}</span>
                <span>
                  <input
                    type="password"
                    autoComplete="off"
                    value={webCredentialDraft[field]}
                    placeholder={field === "brave"
                      ? (webCredentialPresence.braveConfigured ? "Configured" : "Not configured")
                      : field === "googleKey"
                        ? (webCredentialPresence.googleKeyConfigured ? "Configured" : "Not configured")
                        : (webCredentialPresence.googleEngineConfigured ? "Configured" : "Not configured")}
                    onChange={(event) => setWebCredentialDraft((current) => ({
                      ...current,
                      [field]: event.target.value,
                    }))}
                  />
                  <button type="button" disabled={!webCredentialDraft[field].trim()} onClick={() => void saveWebCredential(field, webCredentialDraft[field])}>Save</button>
                  <button type="button" onClick={() => void saveWebCredential(field, "")}>Clear</button>
                </span>
              </label>
            ))}
            {webCredentialMessage ? <p className="muted" role="status">{webCredentialMessage}</p> : null}
            <div className="settings-control-row">
              <p className="muted" role="status">Web connectivity: {webConnectivity}</p>
              <button type="button" onClick={() => void checkWebConnectivity()}>
                <RefreshCw size={14} aria-hidden="true" /> Check connection
              </button>
            </div>
            {providerHealth.length ? (
              <p className="muted">
                Providers: {providerHealth.map((item) => `${item.name}: ${item.status.replace(/_/g, " ")}`).join(" • ")}
              </p>
            ) : null}
            <p className="muted">
              Web Search tries configured Brave / legacy Google APIs, then DuckDuckGo and
              Bing public search. Deep Research reads multiple public pages and checks
              independent sources. External web requests only run when those modes are enabled.
            </p>
            <ToggleRow
              label="Typing indicators"
              checked={preferenceDraft.typingIndicatorEnabled}
              onChange={(value) => setPreference("typingIndicatorEnabled", value)}
            />
            <ToggleRow
              label="Socket.IO realtime events"
              checked={preferenceDraft.socketRealtimeEnabled}
              onChange={(value) => setPreference("socketRealtimeEnabled", value)}
            />
            <TextRow
              label="Socket.IO URL"
              value={preferenceDraft.socketUrl}
              onChange={(value) => setPreference("socketUrl", value)}
            />
            <p className="muted">
              Without internet, local chat stays available but current facts cannot be verified.
              Brave and existing Google Custom Search credentials can be saved above
              or inherited from runtime environment variables. Bing and DuckDuckGo
              require no key. Health checks make real search requests when enabled.
            </p>
            <SaveBar state={saveState} onSave={savePreferences} />
          </SettingsGroup>
        ) : null}

        {activeSection === "connectors" && (
          <SettingsGroup title="Apps">
            <AppsSettings />
          </SettingsGroup>
        )}

        {activeSection === "openagent" && preferenceDraft ? (
          <SettingsGroup title="Agent Setup">
            <AgentSettings
              hardware={props.hardware}
              models={props.models}
              runtime={props.runtime}
              runtimeStatus={props.runtimeStatus}
              preferences={preferenceDraft}
              onPreferencesChange={(next) => {
                setPreferenceDraft(next);
                void props.updatePreferences(next);
              }}
              refresh={props.refresh}
              startRuntime={props.startRuntime}
              stopRuntime={props.stopRuntime}
              runtimeAction={props.runtimeAction}
            />
          </SettingsGroup>
        ) : null}

        {activeSection === "models" && (
          <SettingsGroup title="Models">
            <ModelsManager
              hardware={props.hardware}
              models={props.models}
              runtime={props.runtime}
              root={props.root}
              refresh={props.refresh}
            />
          </SettingsGroup>
        )}

        {activeSection === "runtime" && (
          <SettingsGroup title="AI Runtime">
            <Info label="Status" value={props.runtimeStatus?.state} />
            <Info
              label="Selected"
              value={props.runtime?.selected?.manifest.runtimeName ?? "None"}
            />
            <Info label="Backend" value={props.runtime?.selected?.manifest.backend} />
            <Info label="Version" value={props.runtime?.selected?.manifest.version} />
            <Info label="Location" value={props.runtime?.selected?.manifest.binaries.server} />
            {!props.runtime?.selected ? (
              <p className="setup-warning">
                No usable llama.cpp runtime was detected. Chat history and Settings remain available.
                OpenMindAI can download, verify, and install an official runtime in its portable storage.
              </p>
            ) : null}
            <div className="button-row">
              <button
                type="button"
                onClick={props.installRuntime}
                disabled={props.runtimeInstalling || props.runtimeAction !== null}
              >
                {props.runtimeInstalling ? <LoaderCircle className="spin" size={16} /> : <Download size={16} />}
                {props.runtimeInstalling ? "Installing runtime…" : "Install / repair runtime"}
              </button>
              {props.runtimeInstalling ? (
                <button type="button" onClick={props.cancelRuntimeInstall}>Cancel download</button>
              ) : null}
            </div>
            {props.runtimeInstalling || props.runtimeInstallStatus ? (
              <div className="setup-progress" role="status" aria-live="polite">
                <progress
                  max={100}
                  value={props.runtimeInstallStatus?.percentage ?? 0}
                  aria-label="Runtime download progress"
                />
                <p className="muted">
                  {props.runtimeInstallStatus?.backend ? `${props.runtimeInstallStatus.backend} · ` : ""}
                  {props.runtimeInstallStatus?.state ?? "resolving"}
                  {props.runtimeInstallStatus?.percentage != null
                    ? ` · ${Math.round(props.runtimeInstallStatus.percentage)}%`
                    : ""}
                </p>
              </div>
            ) : null}
            {props.runtimeInstallError || props.runtimeInstallStatus?.error ? (
              <p className="setup-warning">{props.runtimeInstallError ?? props.runtimeInstallStatus?.error}</p>
            ) : null}
            <div className="button-row">
              <button type="button" onClick={props.refresh} title="Validate">
                <RefreshCw size={16} />
              </button>
              <button
                type="button"
                onClick={() => props.startRuntime("restart")}
                title="Restart Runtime"
                disabled={props.runtimeAction !== null || !props.runtime?.selected || props.runtimeInstalling}
              >
                {props.runtimeAction === "start" || props.runtimeAction === "restart" ? (
                  <LoaderCircle className="spin" size={16} />
                ) : (
                  <Play size={16} />
                )}
              </button>
              <button
                type="button"
                onClick={props.stopRuntime}
                title="Stop Runtime"
                disabled={props.runtimeAction !== null}
              >
                {props.runtimeAction === "stop" ? (
                  <LoaderCircle className="spin" size={16} />
                ) : (
                  <StopCircle size={16} />
                )}
              </button>
            </div>
            <DiffusionRuntimePanel />
          </SettingsGroup>
        )}

        {activeSection === "hardware" && (
          <SettingsGroup title="Hardware">
            <Info label="OS" value={props.hardware?.operatingSystem} />
            <Info label="CPU" value={props.hardware?.cpu.name} />
            <Info
              label="Cores"
              value={
                props.hardware
                  ? `${props.hardware.cpu.physicalCores ?? "Unknown"} physical / ${props.hardware.cpu.logicalThreads} threads`
                  : undefined
              }
            />
            <Info label="RAM" value={formatBytes(props.hardware?.memory.totalBytes)} />
            <Info label="Acceleration" value={backendLabel(props.hardware)} />
            {props.hardware?.gpus.map((gpu) => (
              <div className="sub-panel" key={gpu.id}>
                <Info label="GPU" value={gpu.name} />
                <Info label="Vendor" value={gpu.vendor} />
                <Info label="Dedicated VRAM" value={formatBytes(gpu.dedicatedVramBytes)} />
                <Info label="Shared Memory" value={formatBytes(gpu.sharedMemoryBytes)} />
              </div>
            ))}
          </SettingsGroup>
        )}

        {activeSection === "storage" && (
          <SettingsGroup title="Storage">
            <Info label="OpenMindAI Root" value={props.root?.root} />
            <Info label="Database" value={formatBytes(props.storage?.databaseBytes)} />
            <Info label="Models" value={formatBytes(props.storage?.modelsBytes)} />
            <Info label="Cache" value={formatBytes(props.storage?.cacheBytes)} />
            <Info label="Generated" value={formatBytes(props.storage?.generatedBytes)} />
          </SettingsGroup>
        )}

        {activeSection === "maintenance" && (
          <SettingsGroup title="Maintenance">
            <MaintenanceCenter
              root={props.root}
              hardware={props.hardware}
              storage={props.storage}
              models={props.models}
              runtimeStatus={props.runtimeStatus}
              refresh={props.refresh}
            />
          </SettingsGroup>
        )}

        {activeSection === "files" && preferenceDraft ? (
          <SettingsGroup title="Files & Artifacts">
            <Info
              label="Text & Markdown files"
              value={props.root?.root ? `${props.root.root}\\generated\\files` : undefined}
            />
            <Info
              label="PDF & DOCX exports"
              value={props.root?.root ? `${props.root.root}\\generated\\exports` : undefined}
            />
            <ToggleRow
              label="Open after generation"
              checked={preferenceDraft.openArtifactsAfterGeneration}
              onChange={(value) => setPreference("openArtifactsAfterGeneration", value)}
            />
            <p className="muted">
              Generated files are written under your OpenMindAI Root and stay associated with the
              conversation that created them, so they remain available after restarting the app.
            </p>
            <SaveBar state={saveState} onSave={savePreferences} />
          </SettingsGroup>
        ) : null}

        {activeSection === "privacy" && preferenceDraft ? (
          <SettingsGroup title="Privacy">
            <ToggleRow
              label="Save chat history locally"
              checked={preferenceDraft.saveChatHistory}
              onChange={(value) => setPreference("saveChatHistory", value)}
            />
            <ToggleRow
              label="Telemetry"
              checked={preferenceDraft.telemetryEnabled}
              onChange={(value) => setPreference("telemetryEnabled", value)}
            />
            <Info label="Data location" value={props.root?.root} />
            <SaveBar state={saveState} onSave={savePreferences} />
          </SettingsGroup>
        ) : null}

        {activeSection === "shortcuts" && (
          <SettingsGroup title="Shortcuts">
            <Info label="New chat" value="Ctrl+N" />
            <Info label="Search chats" value="Ctrl+K" />
            <Info label="Settings" value="Ctrl+," />
            <Info label="Send" value="Enter / Ctrl+Enter" />
            <Info label="New line" value="Shift+Enter" />
            <Info label="Stop generation" value="Esc" />
          </SettingsGroup>
        )}

        {activeSection === "advanced" && preferenceDraft ? (
          <SettingsGroup title="Advanced">
            <SelectRow
              label="Performance profile"
              value={preferenceDraft.defaultPerformanceProfile}
              options={["Auto", "Eco", "Balanced", "Performance", "Maximum"]}
              onChange={(value) => setPreference("defaultPerformanceProfile", value)}
            />
            <ToggleRow
              label="Autostart local runtime"
              checked={preferenceDraft.localRuntimeAutostart}
              onChange={(value) => setPreference("localRuntimeAutostart", value)}
            />
            <Info label="Recommended backend" value={props.performance?.recommendedBackend} />
            <Info
              label="RAM budget"
              value={formatBytes(props.performance?.systemMemoryBudgetBytes)}
            />
            <SaveBar state={saveState} onSave={savePreferences} />
          </SettingsGroup>
        ) : null}

        {activeSection === "network" && preferenceDraft ? (
          <SettingsGroup title="Network">
            <label className="setting-row">
              <span>Proxy</span>
              <select
                value={preferenceDraft.networkProxyMode}
                onChange={(event) =>
                  setPreference(
                    "networkProxyMode",
                    event.target.value as AppPreferences["networkProxyMode"],
                  )
                }
              >
                <option value="direct">OpenMindAI download manager (ignore PC proxy)</option>
                <option value="manual">Manual proxy</option>
                <option value="system">Use PC proxy settings</option>
              </select>
            </label>
            {preferenceDraft.networkProxyMode === "manual" ? (
              <TextRow
                label="Proxy address"
                value={preferenceDraft.networkProxyUrl}
                onChange={(value) => setPreference("networkProxyUrl", value)}
              />
            ) : null}
            <p className="muted">
              Model, dataset and runtime downloads (including GitHub releases), app updates and
              connected apps use this setting. Interrupted downloads resume automatically.
              {preferenceDraft.networkProxyMode === "direct"
                ? " OpenMindAI connects on its own and ignores the PC's proxy, so a proxy that blocks GitHub does not stop runtime downloads."
                : null}
              {preferenceDraft.networkProxyMode === "system"
                ? " Uses HTTPS_PROXY / HTTP_PROXY when set, otherwise the Windows proxy settings."
                : null}
              {preferenceDraft.networkProxyMode === "manual"
                ? " Use host:port or http://user:password@host:port."
                : null}
              {" The local AI runtime and other local network addresses are always reached directly."}
            </p>
            {networkError ? <p className="muted avatar-error">{networkError}</p> : null}
            <SaveBar state={saveState} onSave={() => void saveNetwork()} />
          </SettingsGroup>
        ) : null}

        {activeSection === "updates" && preferenceDraft ? (
          <SettingsGroup title="Updates">
            <UpdatesPanel
              preferenceDraft={preferenceDraft}
              setPreference={setPreference}
              saveState={saveState}
              onSave={savePreferences}
            />
          </SettingsGroup>
        ) : null}

        {activeSection === "about" && (
          <SettingsGroup title="About">
            <Info label="Name" value="OpenMindAI" />
            <Info label="Tagline" value="Your AI. Your Models. Your Way." />
            <Info label="Version" value={packageJson.version} />
            <Info label="OpenMindAI Root" value={props.root?.root} />
            <p className="muted">
              Runs locally on your device. No account, no cloud dependency required.
            </p>
          </SettingsGroup>
        )}
      </section>
    </div>
  );
}

export function SaveBar(props: { state: "idle" | "saving" | "saved"; onSave: () => void }) {
  return (
    <div className="save-bar">
      <span>
        {props.state === "saved"
          ? "Saved"
          : props.state === "saving"
            ? "Saving..."
            : "Unsaved changes"}
      </span>
      <button disabled={props.state === "saving"} onClick={props.onSave} type="button">
        Save
      </button>
    </div>
  );
}

function SettingsGroup(props: { title: string; children: ReactNode }) {
  return (
    <div className="settings-group">
      <h2>{props.title}</h2>
      {props.children}
    </div>
  );
}

export function ToggleRow(props: {
  label: string;
  checked: boolean;
  onChange: (value: boolean) => void;
}) {
  return (
    <label className="setting-row">
      <span>{props.label}</span>
      <input
        type="checkbox"
        checked={props.checked}
        onChange={(event) => props.onChange(event.target.checked)}
      />
    </label>
  );
}

export function SelectRow(props: {
  label: string;
  value: string;
  options: string[];
  onChange: (value: string) => void;
}) {
  return (
    <label className="setting-row">
      <span>{props.label}</span>
      <select value={props.value} onChange={(event) => props.onChange(event.target.value)}>
        {props.options.map((option) => (
          <option key={option}>{option}</option>
        ))}
      </select>
    </label>
  );
}

function TextRow(props: { label: string; value: string; onChange: (value: string) => void }) {
  return (
    <label className="setting-row">
      <span>{props.label}</span>
      <input
        type="text"
        value={props.value}
        onChange={(event) => props.onChange(event.target.value)}
      />
    </label>
  );
}

function TextAreaRow(props: { label: string; value: string; onChange: (value: string) => void }) {
  return (
    <label className="setting-row textarea-row">
      <span>{props.label}</span>
      <textarea value={props.value} onChange={(event) => props.onChange(event.target.value)} />
    </label>
  );
}

function SegmentRow(props: {
  label: string;
  value: string;
  options: string[];
  onChange: (value: string) => void;
}) {
  return (
    <div className="setting-row">
      <span>{props.label}</span>
      <div className="segmented">
        {props.options.map((option) => (
          <button
            className={props.value === option ? "active" : ""}
            key={option}
            onClick={() => props.onChange(option)}
            type="button"
          >
            {option}
          </button>
        ))}
      </div>
    </div>
  );
}

function Info(props: { label: string; value?: string | null }) {
  return (
    <div className="info-row">
      <span>{props.label}</span>
      <strong>{props.value ?? "Unavailable"}</strong>
    </div>
  );
}

function backendLabel(profile: HardwareProfile | null) {
  if (!profile) return undefined;
  if (profile.backends.cuda) return "CUDA";
  if (profile.backends.vulkan) return "Vulkan";
  if (profile.backends.sycl) return "SYCL";
  if (profile.backends.hip) return "HIP";
  if (profile.backends.metal) return "Metal";
  return "CPU";
}
