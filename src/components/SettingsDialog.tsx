import { useEffect, useRef, useState, type ReactNode } from "react";
import {
  Brain,
  Cpu,
  Database,
  Download,
  FolderOpen,
  HardDrive,
  Info as InfoIcon,
  Keyboard,
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
  StorageSummary,
  UserProfile,
} from "../types";
import { formatBytes } from "../lib/format";
import { readImageAsDataUrl } from "../lib/avatar";
import { ModelsManager } from "./ModelsManager";
import { Connectors } from "./Connectors";
import { MaintenanceCenter } from "./MaintenanceCenter";
import { UpdatesPanel } from "./UpdatesPanel";

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
  startRuntime: () => void;
  stopRuntime: () => void;
  initialSection?: string;
}) {
  const [activeSection, setActiveSection] = useState(props.initialSection ?? "general");
  const [preferenceDraft, setPreferenceDraft] = useState<AppPreferences | null>(props.preferences);
  const [profileDraft, setProfileDraft] = useState<UserProfile | null>(props.userProfile);
  const [saveState, setSaveState] = useState<"idle" | "saving" | "saved">("idle");
  const [avatarError, setAvatarError] = useState<string | null>(null);
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
    { id: "connectors", label: "Connectors", icon: Plug },
    { id: "models", label: "Models", icon: Database },
    { id: "runtime", label: "AI Runtime", icon: Server },
    { id: "hardware", label: "Hardware", icon: Cpu },
    { id: "storage", label: "Storage", icon: HardDrive },
    { id: "maintenance", label: "Maintenance", icon: Wrench },
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
              className={activeSection === section.id ? "settings-menu-item active" : "settings-menu-item"}
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
            <SelectRow label="Language" value={preferenceDraft.language} options={["English", "Bangla", "Hindi"]} onChange={(value) => setPreference("language", value)} />
            <ToggleRow label="Compact sidebar" checked={preferenceDraft.compactSidebar} onChange={(value) => setPreference("compactSidebar", value)} />
            <ToggleRow label="Confirm before delete" checked={preferenceDraft.confirmBeforeDelete} onChange={(value) => setPreference("confirmBeforeDelete", value)} />
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
            <ToggleRow label="Show shortcut hints" checked={preferenceDraft.showShortcutHints} onChange={(value) => setPreference("showShortcutHints", value)} />
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
                  (profileDraft.preferredName || profileDraft.fullName || "Local User").charAt(0).toUpperCase()
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
                <button type="button" className="ghost-button" onClick={() => avatarInputRef.current?.click()}>
                  Upload photo
                </button>
                {profileDraft.avatarDataUrl ? (
                  <button type="button" className="ghost-button" onClick={() => setProfile("avatarDataUrl", "")}>
                    <Trash2 size={14} /> Remove
                  </button>
                ) : null}
                {avatarError ? <p className="muted avatar-error">{avatarError}</p> : null}
              </div>
            </div>
            <TextRow label="Full name" value={profileDraft.fullName} onChange={(value) => setProfile("fullName", value)} />
            <TextRow label="Preferred name" value={profileDraft.preferredName} onChange={(value) => setProfile("preferredName", value)} />
            <TextRow label="Email" value={profileDraft.email} onChange={(value) => setProfile("email", value)} />
            <TextRow label="Occupation" value={profileDraft.occupation} onChange={(value) => setProfile("occupation", value)} />
            <TextRow label="Response style" value={profileDraft.responseStyle} onChange={(value) => setProfile("responseStyle", value)} />
            <TextAreaRow label="About you" value={profileDraft.about} onChange={(value) => setProfile("about", value)} />
            <TextAreaRow label="Custom instructions" value={profileDraft.customInstructions} onChange={(value) => setProfile("customInstructions", value)} />
            <p className="muted">This profile is stored locally and added as hidden system context for conversations.</p>
            <SaveBar state={saveState} onSave={saveProfile} />
          </SettingsGroup>
        ) : null}

        {activeSection === "chat" && preferenceDraft ? (
          <SettingsGroup title="Chat">
            <ToggleRow label="Enter to send" checked={preferenceDraft.enterToSend} onChange={(value) => setPreference("enterToSend", value)} />
            <ToggleRow label="Auto-generate titles" checked={preferenceDraft.autoGenerateTitles} onChange={(value) => setPreference("autoGenerateTitles", value)} />
            <ToggleRow label="Markdown rendering" checked={preferenceDraft.markdownRendering} onChange={(value) => setPreference("markdownRendering", value)} />
            <ToggleRow label="Code copy buttons" checked={preferenceDraft.codeCopyButtons} onChange={(value) => setPreference("codeCopyButtons", value)} />
            <SaveBar state={saveState} onSave={savePreferences} />
          </SettingsGroup>
        ) : null}

        {activeSection === "capabilities" && preferenceDraft ? (
          <SettingsGroup title="Capabilities">
            <ToggleRow label="Thinking mode" checked={preferenceDraft.thinkingModeEnabled} onChange={(value) => setPreference("thinkingModeEnabled", value)} />
            <ToggleRow label="Web search mode" checked={preferenceDraft.webSearchEnabled} onChange={(value) => setPreference("webSearchEnabled", value)} />
            <ToggleRow label="Deep research mode" checked={preferenceDraft.deepResearchEnabled} onChange={(value) => setPreference("deepResearchEnabled", value)} />
            <ToggleRow label="Typing indicators" checked={preferenceDraft.typingIndicatorEnabled} onChange={(value) => setPreference("typingIndicatorEnabled", value)} />
            <ToggleRow label="Socket.IO realtime events" checked={preferenceDraft.socketRealtimeEnabled} onChange={(value) => setPreference("socketRealtimeEnabled", value)} />
            <TextRow label="Socket.IO URL" value={preferenceDraft.socketUrl} onChange={(value) => setPreference("socketUrl", value)} />
            <p className="muted">Search, research, document, PDF, image prompt, and vision review modes are available in Chat. Local-only modes disclose when a dedicated live web, PDF, image, or vision runtime is not connected.</p>
            <SaveBar state={saveState} onSave={savePreferences} />
          </SettingsGroup>
        ) : null}

        {activeSection === "connectors" && (
          <SettingsGroup title="Connectors">
            <Connectors />
          </SettingsGroup>
        )}

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
            <Info label="Selected" value={props.runtime?.selected?.manifest.runtimeName ?? "None"} />
            <Info label="Backend" value={props.runtime?.selected?.manifest.backend} />
            <Info label="Version" value={props.runtime?.selected?.manifest.version} />
            <Info label="Location" value={props.runtime?.selected?.manifest.binaries.server} />
            <div className="button-row">
              <button type="button" onClick={props.refresh} title="Validate"><RefreshCw size={16} /></button>
              <button type="button" onClick={props.startRuntime} title="Restart Runtime"><Play size={16} /></button>
              <button type="button" onClick={props.stopRuntime} title="Stop Runtime"><StopCircle size={16} /></button>
            </div>
          </SettingsGroup>
        )}

        {activeSection === "hardware" && (
          <SettingsGroup title="Hardware">
            <Info label="OS" value={props.hardware?.operatingSystem} />
            <Info label="CPU" value={props.hardware?.cpu.name} />
            <Info label="Cores" value={props.hardware ? `${props.hardware.cpu.physicalCores ?? "Unknown"} physical / ${props.hardware.cpu.logicalThreads} threads` : undefined} />
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
            <Info label="Text & Markdown files" value={props.root?.root ? `${props.root.root}\\generated\\files` : undefined} />
            <Info label="PDF & DOCX exports" value={props.root?.root ? `${props.root.root}\\generated\\exports` : undefined} />
            <ToggleRow
              label="Open after generation"
              checked={preferenceDraft.openArtifactsAfterGeneration}
              onChange={(value) => setPreference("openArtifactsAfterGeneration", value)}
            />
            <p className="muted">
              Generated files are written under your OpenMindAI Root and stay associated with the conversation
              that created them, so they remain available after restarting the app.
            </p>
            <SaveBar state={saveState} onSave={savePreferences} />
          </SettingsGroup>
        ) : null}

        {activeSection === "privacy" && preferenceDraft ? (
          <SettingsGroup title="Privacy">
            <ToggleRow label="Save chat history locally" checked={preferenceDraft.saveChatHistory} onChange={(value) => setPreference("saveChatHistory", value)} />
            <ToggleRow label="Telemetry" checked={preferenceDraft.telemetryEnabled} onChange={(value) => setPreference("telemetryEnabled", value)} />
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
            <SelectRow label="Performance profile" value={preferenceDraft.defaultPerformanceProfile} options={["Auto", "Eco", "Balanced", "Performance", "Maximum"]} onChange={(value) => setPreference("defaultPerformanceProfile", value)} />
            <ToggleRow label="Autostart local runtime" checked={preferenceDraft.localRuntimeAutostart} onChange={(value) => setPreference("localRuntimeAutostart", value)} />
            <Info label="Recommended backend" value={props.performance?.recommendedBackend} />
            <Info label="RAM budget" value={formatBytes(props.performance?.systemMemoryBudgetBytes)} />
            <SaveBar state={saveState} onSave={savePreferences} />
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
            <p className="muted">Runs locally on your device. No account, no cloud dependency required.</p>
          </SettingsGroup>
        )}
      </section>
    </div>
  );
}

export function SaveBar(props: { state: "idle" | "saving" | "saved"; onSave: () => void }) {
  return (
    <div className="save-bar">
      <span>{props.state === "saved" ? "Saved" : props.state === "saving" ? "Saving..." : "Unsaved changes"}</span>
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

export function ToggleRow(props: { label: string; checked: boolean; onChange: (value: boolean) => void }) {
  return (
    <label className="setting-row">
      <span>{props.label}</span>
      <input type="checkbox" checked={props.checked} onChange={(event) => props.onChange(event.target.checked)} />
    </label>
  );
}

export function SelectRow(props: { label: string; value: string; options: string[]; onChange: (value: string) => void }) {
  return (
    <label className="setting-row">
      <span>{props.label}</span>
      <select value={props.value} onChange={(event) => props.onChange(event.target.value)}>
        {props.options.map((option) => <option key={option}>{option}</option>)}
      </select>
    </label>
  );
}

function TextRow(props: { label: string; value: string; onChange: (value: string) => void }) {
  return (
    <label className="setting-row">
      <span>{props.label}</span>
      <input type="text" value={props.value} onChange={(event) => props.onChange(event.target.value)} />
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

function SegmentRow(props: { label: string; value: string; options: string[]; onChange: (value: string) => void }) {
  return (
    <div className="setting-row">
      <span>{props.label}</span>
      <div className="segmented">
        {props.options.map((option) => (
          <button className={props.value === option ? "active" : ""} key={option} onClick={() => props.onChange(option)} type="button">
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
