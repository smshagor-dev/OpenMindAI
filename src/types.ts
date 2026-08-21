export interface PortableRootInfo {
  root: string;
  mode: "developmentEnv" | "savedInstallation" | "portableMarker" | "developmentFallback";
  databasePath: string;
  modelsDir: string;
  logsDir: string;
  writable: boolean;
}

export type SetupState =
  | "storageChosen"
  | "profileChosen"
  | "rootCreated"
  | "runtimeReady"
  | "modelReady"
  | "completed";

export interface PendingSetup {
  root: string;
  profileName: string;
  state: SetupState;
}

export interface InstallationStatus {
  setupRequired: boolean;
  reason: string | null;
  root: string | null;
  pending: PendingSetup | null;
}

export interface StorageLocationCheck {
  path: string;
  writable: boolean;
  availableBytes: number | null;
  recommendedBytes: number;
}

export interface Conversation {
  id: string;
  title: string;
  createdAt: string;
  updatedAt: string;
  archivedAt: string | null;
  pinned: boolean;
  activeModelId: string | null;
}

export interface Message {
  id: string;
  conversationId: string;
  role: "system" | "user" | "assistant" | "tool";
  content: string;
  status: "pending" | "streaming" | "completed" | "cancelled" | "failed";
  modelId: string | null;
  createdAt: string;
  updatedAt: string;
}

export interface HardwareProfile {
  operatingSystem: string;
  architecture: string;
  cpu: {
    name: string;
    physicalCores: number | null;
    logicalThreads: number;
  };
  memory: {
    totalBytes: number;
    availableBytes: number;
  };
  gpus: GpuInfo[];
  primaryGpu: string | null;
  recommendedInferenceGpu: string | null;
  backends: {
    cpu: boolean;
    cuda: boolean;
    vulkan: boolean;
    sycl: boolean;
    hip: boolean;
    metal: boolean;
  };
}

export interface GpuInfo {
  id: string;
  name: string;
  vendor: "nvidia" | "amd" | "intel" | "microsoftSoftware" | "unknown";
  vendorId: number | null;
  deviceId: number | null;
  subsystemId: number | null;
  revision: number | null;
  dedicatedVramBytes: number | null;
  dedicatedSystemMemoryBytes: number | null;
  sharedMemoryBytes: number | null;
  luid: string | null;
  isDiscrete: boolean;
  isIntegrated: boolean;
  isSoftware: boolean;
  availableBackends: BackendKind[];
  recommendedBackend: BackendKind;
}

export type BackendKind = "cpu" | "cuda" | "vulkan" | "sycl" | "hip" | "metal";

export interface RuntimeValidation {
  manifest: {
    runtimeName: string;
    version: string;
    platform: string;
    architecture: string;
    backend: BackendKind;
    source: string;
    installedAt: string;
    binaries: {
      server: string | null;
      cli: string | null;
      bench: string | null;
    };
    checksum: string | null;
    status: "available" | "validating" | "ready" | "invalid" | "missing" | "disabled";
  };
  serverExists: boolean;
  cliExists: boolean;
  benchExists: boolean;
  versionOutput: string | null;
  deviceOutput: string | null;
  usable: boolean;
  message: string;
}

export interface RuntimeInventory {
  runtimes: RuntimeValidation[];
  selected: RuntimeValidation | null;
  serverState: "stopped" | "starting" | "ready" | "loadingModel" | "running" | "stopping" | "failed";
}

export interface LlamaRuntimeStatus {
  available: boolean;
  backend: BackendKind | null;
  endpoint: string | null;
  state: RuntimeInventory["serverState"];
  selectedRuntime: RuntimeValidation | null;
}

export interface PerformanceProfile {
  mode: "auto" | "eco" | "balanced" | "performance" | "maximum";
  recommendedBackend: BackendKind;
  cpuThreads: number;
  systemMemoryBudgetBytes: number;
  vramBudgetBytes: number | null;
  mmap: boolean;
  flashAttention: boolean;
}

export interface ModelRecord {
  id: string;
  name: string;
  family: string | null;
  path: string;
  format: string;
  quantization: string | null;
  sizeBytes: number;
  capabilities: string;
  contextLength: number | null;
  preferredBackend: string | null;
  enabled: boolean;
  sourceRepository: string | null;
  verification: string | null;
  state:
    | "notInstalled"
    | "downloading"
    | "installed"
    | "validating"
    | "ready"
    | "loading"
    | "loaded"
    | "unloading"
    | "failed";
  createdAt: string;
  updatedAt: string;
}

export interface DiagnosticCheck {
  id: string;
  label: string;
  status: "ok" | "warning" | "error";
  detail: string | null;
}

export interface DiagnosticReport {
  checks: DiagnosticCheck[];
}

export interface RepairSummary {
  actions: string[];
}

export interface BackupInfo {
  name: string;
  sizeBytes: number;
  createdAt: string;
}

export interface RuntimeInstallStatus {
  state:
    | "idle"
    | "resolving"
    | "downloading"
    | "verifying"
    | "extracting"
    | "completed"
    | "failed"
    | "cancelled";
  backend: BackendKind | null;
  version: string | null;
  downloadedBytes: number;
  totalBytes: number | null;
  percentage: number | null;
  speedBytesPerSec: number | null;
  error: string | null;
}

export interface DownloadStatus {
  modelId: string;
  name: string;
  state:
    | "queued"
    | "resolving"
    | "downloading"
    | "pausedInterrupted"
    | "verifying"
    | "completed"
    | "failed"
    | "cancelled";
  repo: string;
  quantization: string;
  filename: string | null;
  downloadedBytes: number;
  totalBytes: number | null;
  percentage: number | null;
  speedBytesPerSec: number | null;
  destination: string | null;
  error: string | null;
}

export interface LaunchPlan {
  config: {
    modelPath: string;
    backend: BackendKind;
    device: string | null;
    gpuLayers: number;
    contextSize: number;
    threads: number;
    batchSize: number;
    ubatchSize: number;
    flashAttention: boolean;
    mmap: boolean;
    mlock: boolean;
    parallelism: number;
    host: string;
    port: number;
  };
  estimatedModelBytes: number;
  estimatedContextBytes: number;
  dedicatedVramBudgetBytes: number | null;
  notes: string[];
}

export interface StreamChunkEvent {
  conversationId: string;
  messageId: string;
  chunk: string;
}

export interface StreamStartedEvent {
  conversationId: string;
  user: Message;
  assistant: Message;
  routedModelName: string;
  routingReason: string;
}

export interface StreamDoneEvent {
  conversationId: string;
  messageId: string;
  status: Message["status"];
}

export type ArtifactKind = "text" | "markdown" | "code" | "pdf" | "docx" | "image" | "audio" | "video";
export type ArtifactStatus = "generating" | "ready" | "failed";

export interface Artifact {
  id: string;
  conversationId: string;
  messageId: string | null;
  name: string;
  path: string;
  mimeType: string;
  kind: ArtifactKind;
  sizeBytes: number;
  pageCount: number | null;
  status: ArtifactStatus;
  error: string | null;
  createdAt: string;
  updatedAt: string;
}

export interface LibraryEntry {
  id: string;
  conversationId: string;
  conversationTitle: string;
  messageId: string | null;
  name: string;
  path: string;
  mimeType: string;
  kind: ArtifactKind;
  sizeBytes: number;
  pageCount: number | null;
  status: ArtifactStatus;
  error: string | null;
  createdAt: string;
  updatedAt: string;
}

export interface ProjectFile {
  id: string;
  projectId: string;
  name: string;
  sizeBytes: number;
  mimeType: string | null;
  contentText: string | null;
  status: "ready" | "tracked" | "skipped" | "failed";
  error: string | null;
  addedAt: string;
}

export interface Project {
  id: string;
  name: string;
  instructions: string;
  createdAt: string;
  updatedAt: string;
  conversationIds: string[];
  files: ProjectFile[];
}

export interface GithubAccount {
  login: string;
  name: string | null;
  avatarUrl: string | null;
  htmlUrl: string;
}

export interface GithubRepo {
  id: number;
  name: string;
  fullName: string;
  private: boolean;
  stargazersCount: number;
  htmlUrl: string;
  description: string | null;
  updatedAt: string | null;
}

export interface GithubIssue {
  id: number;
  number: number;
  title: string;
  state: string;
  htmlUrl: string;
  isPullRequest: boolean;
}

export interface GoogleCredentialsStatus {
  clientId: string;
  hasSecret: boolean;
}

export interface StorageSummary {
  root: string;
  modelsBytes: number;
  databaseBytes: number;
  cacheBytes: number;
  generatedBytes: number;
  availableBytes: number | null;
}

export interface CacheClearResult {
  bytesFreed: number;
}

export type ThemeMode = "system" | "dark" | "light";

export interface AppPreferences {
  theme: ThemeMode;
  language: string;
  compactSidebar: boolean;
  enterToSend: boolean;
  showShortcutHints: boolean;
  autoGenerateTitles: boolean;
  codeCopyButtons: boolean;
  markdownRendering: boolean;
  defaultPerformanceProfile: string;
  telemetryEnabled: boolean;
  saveChatHistory: boolean;
  localRuntimeAutostart: boolean;
  confirmBeforeDelete: boolean;
  webSearchEnabled: boolean;
  deepResearchEnabled: boolean;
  thinkingModeEnabled: boolean;
  typingIndicatorEnabled: boolean;
  socketRealtimeEnabled: boolean;
  socketUrl: string;
  openArtifactsAfterGeneration: boolean;
  autoCheckAppUpdates: boolean;
  autoDownloadAppUpdates: boolean;
  notifyModelUpdates: boolean;
  autoDownloadModelUpdates: boolean;
  updateChannel: string;
}

export interface ModelCatalogEntry {
  id: string;
  name: string;
  version: string;
  family: string;
  kind: string;
  runtime: string;
  repo: string;
  quantization: string;
  required: boolean;
  capabilities: string[];
  sizeBytes: number;
  minRamBytes: number;
  minVramBytes: number | null;
  license: string;
  description: string;
  download: {
    strategy: "singleFile";
    filenamePattern: string;
    destinationDir: string;
    format: string;
  } | null;
}

export interface ModelCatalogStatus {
  entry: ModelCatalogEntry;
  installed: boolean;
  compatible: boolean;
  downloadSupported: boolean;
  installedPath: string | null;
  updateAvailable: boolean;
}

export interface ModelCatalogReport {
  entries: ModelCatalogStatus[];
}

export interface UserProfile {
  fullName: string;
  email: string;
  about: string;
  occupation: string;
  preferredName: string;
  responseStyle: string;
  customInstructions: string;
  avatarDataUrl: string;
}
