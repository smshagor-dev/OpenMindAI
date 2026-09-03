import { useEffect, useMemo, useState } from "react";
import { CheckCircle2, Download, PauseCircle, RefreshCw, StopCircle, Trash2 } from "lucide-react";
import { api } from "../api";
import type {
  DownloadStatus,
  HardwareProfile,
  LaunchPlan,
  ModelCatalogReport,
  ModelCatalogStatus,
  ModelRecord,
  PortableRootInfo,
  RuntimeInventory,
} from "../types";
import { formatBytes } from "../lib/format";
import { ConfirmDialog } from "./ConfirmDialog";

export function ModelsManager(props: {
  hardware: HardwareProfile | null;
  models: ModelRecord[];
  runtime: RuntimeInventory | null;
  root: PortableRootInfo | null;
  refresh: () => void | Promise<void>;
  variant?: "settings" | "setup";
  showRootInfo?: boolean;
  showRuntimeInfo?: boolean;
  allowDelete?: boolean;
}) {
  const [downloadStatus, setDownloadStatus] = useState<DownloadStatus | null>(null);
  const [catalog, setCatalog] = useState<ModelCatalogReport | null>(null);
  const [launchPlan, setLaunchPlan] = useState<LaunchPlan | null>(null);
  const [catalogError, setCatalogError] = useState<string | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<ModelCatalogStatus | null>(null);

  const refreshCatalog = async () => {
    try {
      setCatalog(await api.checkModelUpdates());
      setCatalogError(null);
    } catch (error) {
      setCatalogError(error instanceof Error ? error.message : String(error));
    }
  };

  useEffect(() => {
    let cancelled = false;
    const refreshStatus = async () => {
      const status = await api.modelDownloadStatus();
      if (!cancelled) setDownloadStatus(status);
    };
    void refreshStatus();
    void refreshCatalog();
    const interval = window.setInterval(() => void refreshStatus(), 1000);
    return () => {
      cancelled = true;
      window.clearInterval(interval);
    };
  }, []);

  const recommendedIds = useMemo(
    () => recommendedModelIds(catalog?.entries ?? [], props.hardware),
    [catalog, props.hardware],
  );

  const groupedCatalog = useMemo(() => {
    const groups = new Map<string, ModelCatalogStatus[]>();
    for (const item of catalog?.entries ?? []) {
      const label = collectionLabel(item.entry.kind);
      groups.set(label, [...(groups.get(label) ?? []), item]);
    }
    return Array.from(groups.entries()).map(([label, entries]) => [
      label,
      [...entries].sort((left, right) => {
        const leftScore = modelSortScore(left, recommendedIds);
        const rightScore = modelSortScore(right, recommendedIds);
        return rightScore - leftScore || left.entry.name.localeCompare(right.entry.name);
      }),
    ] as const);
  }, [catalog, recommendedIds]);

  const primaryRecommendation = useMemo(() => {
    const entries = catalog?.entries ?? [];
    return (
      entries.find(
        (item) =>
          recommendedIds.has(item.entry.id) &&
          item.entry.kind === "chat" &&
          (item.downloadSupported || item.installed),
      ) ??
      entries.find(
        (item) => recommendedIds.has(item.entry.id) && (item.downloadSupported || item.installed),
      ) ??
      null
    );
  }, [catalog, recommendedIds]);

  const downloadedCount = catalog?.entries.filter((item) => item.installed).length ?? 0;
  const recommendedCount = catalog?.entries.filter((item) => recommendedIds.has(item.entry.id)).length ?? 0;
  const activeDownload = catalog?.entries.find((item) => item.entry.id === downloadStatus?.modelId) ?? null;

  const downloadModel = async (modelId: string) => {
    setLaunchPlan(null);
    try {
      setDownloadStatus(await api.downloadCatalogModel(modelId));
      await props.refresh();
      await refreshCatalog();
    } catch {
      setDownloadStatus(await api.modelDownloadStatus());
    }
  };

  const cancelDownload = async () => {
    setDownloadStatus(await api.cancelModelDownload());
  };

  const pauseDownload = async () => {
    setDownloadStatus(await api.pauseModelDownload());
  };

  const validateFirstModel = async () => {
    const model = props.models[0];
    if (!model) return;
    await api.validateModel(model.id);
    setLaunchPlan(await api.planModelLaunch(model.id));
  };

  const deleteModel = async () => {
    if (!deleteTarget) return;
    await api.deleteCatalogModel(deleteTarget.entry.id);
    setDeleteTarget(null);
    await props.refresh();
    await refreshCatalog();
  };

  return (
    <>
      {props.showRootInfo === false ? null : <Info label="Model folder" value={props.root?.modelsDir} />}
      {catalogError ? <p className="model-selector-error">{catalogError}</p> : null}
      <ModelCatalogOverview
        variant={props.variant ?? "settings"}
        hardware={props.hardware}
        primary={primaryRecommendation}
        downloadStatus={downloadStatus}
        activeDownloadName={activeDownload?.entry.name ?? null}
        downloadedCount={downloadedCount}
        recommendedCount={recommendedCount}
        onDownload={primaryRecommendation ? () => downloadModel(primaryRecommendation.entry.id) : undefined}
        onPause={pauseDownload}
        onCancel={cancelDownload}
      />

      {groupedCatalog.map(([group, entries]) => (
        <section className="model-catalog-section" key={group}>
          <h3>{group}</h3>
          {entries.map((item) => (
            <CatalogModelCard
              key={item.entry.id}
              item={item}
              status={downloadStatus?.modelId === item.entry.id ? downloadStatus : null}
              recommended={recommendedIds.has(item.entry.id)}
              onDownload={() => downloadModel(item.entry.id)}
              onPause={pauseDownload}
              onCancel={cancelDownload}
              onDelete={props.allowDelete === false ? undefined : () => setDeleteTarget(item)}
            />
          ))}
        </section>
      ))}

      {props.showRuntimeInfo === false ? null : (
        <div className="sub-panel">
          <Info label="Runtime selected" value={props.runtime?.selected?.manifest.runtimeName ?? "None"} />
          <Info label="Runtime backend" value={props.runtime?.selected?.manifest.backend} />
          <Info label="llama.cpp device" value={props.runtime?.selected?.deviceOutput?.slice(0, 180)} />
          <div className="button-row">
            <button
              type="button"
              onClick={validateFirstModel}
              title="Validate and plan first chat model"
              disabled={props.models.length === 0}
            >
              <RefreshCw size={16} />
            </button>
          </div>
          {launchPlan ? (
            <>
              <Info label="Planned context" value={String(launchPlan.config.contextSize)} />
              <Info label="GPU layers" value={String(launchPlan.config.gpuLayers)} />
              <Info label="CPU threads" value={String(launchPlan.config.threads)} />
              <Info label="Flash attention" value={launchPlan.config.flashAttention ? "On" : "Off"} />
            </>
          ) : null}
        </div>
      )}

      <ConfirmDialog
        open={deleteTarget !== null}
        title={`Delete ${deleteTarget?.entry.name ?? "model"}?`}
        description="This will permanently delete all downloaded data for this model from your OpenMindAI Root. You will need to download it again before using it."
        confirmLabel="Delete model"
        danger
        onConfirm={() => void deleteModel()}
        onCancel={() => setDeleteTarget(null)}
      />
    </>
  );
}

function ModelCatalogOverview(props: {
  variant: "settings" | "setup";
  hardware: HardwareProfile | null;
  primary: ModelCatalogStatus | null;
  downloadStatus: DownloadStatus | null;
  activeDownloadName: string | null;
  downloadedCount: number;
  recommendedCount: number;
  onDownload?: () => void | Promise<void>;
  onPause: () => void | Promise<void>;
  onCancel: () => void | Promise<void>;
}) {
  const primaryStatus = props.downloadStatus?.modelId === props.primary?.entry.id ? props.downloadStatus : null;
  const busy =
    props.downloadStatus?.state === "resolving" ||
    props.downloadStatus?.state === "downloading" ||
    props.downloadStatus?.state === "verifying";
  const title = props.variant === "setup" ? "Start with the best local model" : "Model library";
  const hardwareLabel = props.hardware?.backends.cuda
    ? "NVIDIA CUDA ready"
    : props.hardware?.recommendedInferenceGpu
      ? `${props.hardware.recommendedInferenceGpu} ready`
      : "CPU ready";

  return (
    <section className="model-catalog-overview">
      <div className="model-catalog-overview-main">
        <span className="model-catalog-eyebrow">{hardwareLabel}</span>
        <strong>{props.primary?.entry.name ?? title}</strong>
        <p>
          {props.primary
            ? `${props.primary.entry.name} is recommended for this computer.`
            : "OpenMindAI will recommend the best model after hardware detection finishes."}
        </p>
        <div className="model-catalog-stats">
          <span>{props.recommendedCount} recommended</span>
          <span>{props.downloadedCount} downloaded</span>
          {props.activeDownloadName && props.downloadStatus ? (
            <span>
              {props.activeDownloadName}
              {props.downloadStatus.speedBytesPerSec
                ? ` · ${formatBytes(props.downloadStatus.speedBytesPerSec)}/s`
                : ""}
            </span>
          ) : null}
        </div>
      </div>
      <div className="button-row model-catalog-overview-actions">
        {props.primary?.installed ? (
          <button type="button" title={`${props.primary.entry.name} verified`} disabled>
            <CheckCircle2 size={16} />
          </button>
        ) : primaryStatus && busy ? (
          <>
            <button type="button" onClick={props.onPause} title="Pause download">
              <PauseCircle size={16} />
            </button>
            <button type="button" onClick={props.onCancel} title="Cancel download">
              <StopCircle size={16} />
            </button>
          </>
        ) : props.primary ? (
          <button
            type="button"
            className="primary-button"
            onClick={props.onDownload}
            title={`Download ${props.primary.entry.name}`}
            disabled={!props.primary.downloadSupported || busy}
          >
            <Download size={16} />
          </button>
        ) : null}
      </div>
    </section>
  );
}

function CatalogModelCard(props: {
  item: ModelCatalogStatus;
  status: DownloadStatus | null;
  recommended: boolean;
  onDownload: () => void | Promise<void>;
  onPause: () => void | Promise<void>;
  onCancel: () => void | Promise<void>;
  onDelete?: () => void;
}) {
  const entry = props.item.entry;
  const busy =
    props.status?.state === "resolving" ||
    props.status?.state === "downloading" ||
    props.status?.state === "verifying";
  const paused = props.status?.state === "pausedInterrupted";
  const availabilityBadge = props.item.installed
    ? "Downloaded"
    : props.item.downloadSupported
      ? "Download"
      : "Manual access";

  return (
    <div className="model-download-card">
      <div>
        <strong>
          {entry.name}
          <span className={props.item.installed ? "model-badge downloaded" : "model-badge"}>
            {availabilityBadge}
          </span>
          {props.recommended ? <span className="model-badge recommended">Recommended</span> : null}
          {props.item.installed ? <CheckCircle2 size={15} /> : null}
        </strong>
        <span>
          OpenMindAI local package · {formatBytes(entry.sizeBytes)} · {licenseLabel(entry.license)}
        </span>
        <small>{entry.capabilities.map(capabilityLabel).join(" · ")}</small>
        <small>{entry.description}</small>
      </div>
      <div className="download-progress">
        <span>{catalogAvailabilityLabel(props.item)}</span>
        {props.status?.totalBytes ? (
          <small>
            {formatBytes(props.status.downloadedBytes)} / {formatBytes(props.status.totalBytes)}
            {props.status.percentage != null ? ` · ${props.status.percentage.toFixed(1)}%` : ""}
            {props.status.speedBytesPerSec ? ` · ${formatBytes(props.status.speedBytesPerSec)}/s` : ""}
          </small>
        ) : props.item.installed ? null : (
          <small>{systemFitLabel(props.item)}</small>
        )}
      </div>
      <div className="button-row">
        {props.item.installed ? (
          <>
            <button type="button" title={`${entry.name} verified`} disabled>
              <CheckCircle2 size={16} />
            </button>
            {props.onDelete ? (
              <button
                type="button"
                className="danger-button"
                onClick={props.onDelete}
                title={`Delete ${entry.name}`}
              >
                <Trash2 size={16} />
              </button>
            ) : null}
          </>
        ) : busy ? (
          <>
            <button type="button" onClick={props.onPause} title="Pause download">
              <PauseCircle size={16} />
            </button>
            <button type="button" onClick={props.onCancel} title="Cancel download">
              <StopCircle size={16} />
            </button>
          </>
        ) : (
          <button
            type="button"
            onClick={props.onDownload}
            title={
              props.item.downloadSupported
                ? `${paused ? "Resume" : "Download"} ${entry.name}`
                : `${entry.name} requires manual upstream license/access setup`
            }
            disabled={!props.item.downloadSupported}
          >
            <Download size={16} />
          </button>
        )}
      </div>
      {props.status?.error ? <p className="muted">{props.status.error}</p> : null}
    </div>
  );
}

function collectionLabel(kind: string) {
  const labels: Record<string, string> = {
    chat: "OpenMindAI Core",
    reasoning: "OpenMindAI Reasoning",
    agent: "OpenMindAI Agent",
    vision: "OpenMindAI Vision",
    "speech-to-text": "OpenMindAI Hear",
    "text-to-speech": "OpenMindAI Speak",
    image: "OpenMindAI Canvas",
    audio: "OpenMindAI Soundscape",
    video: "OpenMindAI Motion",
  };
  return labels[kind] ?? kind;
}

function recommendedModelIds(entries: ModelCatalogStatus[], hardware: HardwareProfile | null) {
  const ids = new Set<string>();
  const compatible = entries.filter(
    (entry) => entry.compatible && (entry.downloadSupported || entry.installed),
  );
  const totalRam = hardware?.memory.totalBytes ?? 0;
  const maxVram = Math.max(
    0,
    ...(hardware?.gpus ?? []).map((gpu) => gpu.dedicatedVramBytes ?? 0),
  );
  const hasNvidia = Boolean(hardware?.backends.cuda);
  const hasLargeMemory = totalRam >= 32 * 1024 * 1024 * 1024 || maxVram >= 12 * 1024 * 1024 * 1024;

  addFirstAvailable(ids, compatible, chatPreferenceOrder(totalRam, maxVram));
  addFirstAvailable(ids, compatible, reasoningPreferenceOrder(totalRam, maxVram));
  addFirstAvailable(ids, compatible, visionPreferenceOrder(totalRam, maxVram));
  addFirstAvailable(ids, compatible, agentPreferenceOrder(totalRam, maxVram));
  addPreferred(ids, compatible, "speech-to-text", "whisper-large-v3-turbo-q5");
  addPreferred(ids, compatible, "text-to-speech", "kokoro-82m-onnx");
  addPreferred(ids, compatible, "image", hasNvidia && hasLargeMemory ? "flux1-schnell" : "sdxl-base-1");
  addPreferred(ids, compatible, "video", "wan21-t2v-13b");

  return ids;
}

function chatPreferenceOrder(totalRam: number, maxVram: number) {
  const gib = 1024 * 1024 * 1024;
  if (totalRam >= 32 * gib || maxVram >= 16 * gib) {
    return [
      "mistral-small31-24b-q4km",
      "qwen3-8b-q4km",
      "qwen3-4b-q4km",
      "qwen3-17b-q4km",
      "qwen3-06b-q4",
    ];
  }
  if (totalRam >= 16 * gib || maxVram >= 8 * gib) {
    return ["qwen3-8b-q4km", "qwen3-4b-q4km", "qwen3-17b-q4km", "qwen3-06b-q4"];
  }
  if (totalRam >= 8 * gib) {
    return ["qwen3-4b-q4km", "phi4-mini-q4km", "qwen3-17b-q4km", "qwen3-06b-q4"];
  }
  if (totalRam >= 6 * gib) {
    return ["qwen3-17b-q4km", "qwen3-06b-q4"];
  }
  return ["qwen3-06b-q4", "qwen3-17b-q4km"];
}

function reasoningPreferenceOrder(totalRam: number, maxVram: number) {
  const gib = 1024 * 1024 * 1024;
  const memoryClass = Math.max(totalRam, maxVram);
  if (memoryClass >= 96 * gib) {
    return ["gpt-oss-120b-mxfp4", "gpt-oss-20b-mxfp4", "deepseek-r1-7b-q4km", "deepseek-r1-15b-q4km"];
  }
  if (memoryClass >= 32 * gib) {
    return ["gpt-oss-20b-mxfp4", "deepseek-r1-7b-q4km", "deepseek-r1-15b-q4km"];
  }
  if (memoryClass >= 16 * gib) {
    return ["deepseek-r1-7b-q4km", "gpt-oss-20b-mxfp4", "deepseek-r1-15b-q4km"];
  }
  return ["deepseek-r1-15b-q4km", "deepseek-r1-7b-q4km"];
}

function visionPreferenceOrder(totalRam: number, maxVram: number) {
  const gib = 1024 * 1024 * 1024;
  const memoryClass = Math.max(totalRam, maxVram);
  if (memoryClass >= 32 * gib) {
    return ["gemma4-31b-q4", "gemma4-26b-a4b-q4", "gemma4-12b-q4", "gemma4-e4b-q4", "gemma4-e2b-q4", "qwen25-vl-3b-q4km"];
  }
  if (memoryClass >= 24 * gib) {
    return ["gemma4-26b-a4b-q4", "gemma4-12b-q4", "gemma4-e4b-q4", "gemma4-e2b-q4", "qwen25-vl-3b-q4km"];
  }
  if (memoryClass >= 16 * gib) {
    return ["gemma4-12b-q4", "gemma4-e4b-q4", "gemma4-e2b-q4", "qwen25-vl-3b-q4km"];
  }
  if (memoryClass >= 12 * gib) {
    return ["gemma4-e4b-q4", "gemma4-e2b-q4", "qwen25-vl-3b-q4km"];
  }
  return ["gemma4-e2b-q4", "qwen25-vl-3b-q4km"];
}

function agentPreferenceOrder(totalRam: number, maxVram: number) {
  const gib = 1024 * 1024 * 1024;
  const memoryClass = Math.max(totalRam, maxVram);
  if (memoryClass >= 96 * gib) {
    return ["nemotron3-super-120b-q4k", "nemotron35-lightning-30b-a3b-q4", "nemotron3-nano-30b-a3b-q4km", "nemotron3-nano-4b-q4km"];
  }
  if (memoryClass >= 32 * gib) {
    return ["nemotron35-lightning-30b-a3b-q4", "nemotron3-nano-30b-a3b-q4km", "nemotron3-nano-4b-q4km"];
  }
  return ["nemotron3-nano-4b-q4km"];
}

function addFirstAvailable(ids: Set<string>, entries: ModelCatalogStatus[], preferredIds: string[]) {
  const preferred = preferredIds
    .map((id) => entries.find((entry) => entry.entry.id === id))
    .find((entry): entry is ModelCatalogStatus => Boolean(entry));
  if (preferred) ids.add(preferred.entry.id);
}

function addPreferred(
  ids: Set<string>,
  entries: ModelCatalogStatus[],
  kind: string,
  preferredId: string,
) {
  const preferred = entries.find((entry) => entry.entry.id === preferredId);
  if (preferred) {
    ids.add(preferred.entry.id);
    return;
  }
  const fallback = entries.find((entry) => entry.entry.kind === kind);
  if (fallback) ids.add(fallback.entry.id);
}

function modelSortScore(item: ModelCatalogStatus, recommendedIds: Set<string>) {
  let score = 0;
  if (item.installed) score += 100;
  if (recommendedIds.has(item.entry.id)) score += 50;
  if (item.compatible) score += 10;
  if (item.downloadSupported) score += 5;
  return score;
}

function catalogAvailabilityLabel(item: ModelCatalogStatus) {
  if (item.installed) return "Ready";
  if (!item.downloadSupported) return "Manual license/access";
  if (item.compatible) return "Available";
  return "Needs stronger hardware";
}

function systemFitLabel(item: ModelCatalogStatus) {
  if (item.installed) return "Ready";
  if (!item.downloadSupported) {
    return item.compatible
      ? `Compatible · manual upstream access · ${licenseLabel(item.entry.license)}`
      : `Not recommended for this PC · manual upstream access`;
  }
  if (!item.compatible) return "Not recommended for this PC";
  return "Ready for this PC";
}

function capabilityLabel(capability: string) {
  const labels: Record<string, string> = {
    chat: "Chat",
    reasoning: "Reasoning",
    code: "Code",
    math: "Math",
    agent: "Agent",
    "tool-use": "Tools",
    "long-context": "Long context",
    vision: "Vision",
    ocr: "OCR",
    "image-review": "Image review",
    transcription: "Transcription",
    translation: "Translation",
    audio: "Audio",
    tts: "Voice output",
    voice: "Voice",
    "text-to-image": "Image generation",
    image: "Image",
    "text-to-audio": "Audio generation",
    music: "Music",
    sound: "Sound",
    "text-to-video": "Video generation",
    video: "Video",
  };
  return labels[capability] ?? capability;
}

function licenseLabel(license: string) {
  const labels: Record<string, string> = {
    "apache-2.0": "Apache 2.0",
    mit: "MIT",
    gemma: "Upstream terms",
    "llama-3.2-community": "Community terms",
    "nvidia-nemotron-open-model-license": "Open Model License",
    "openmdw-1.1": "OpenMDW 1.1",
    "openrail++": "OpenRAIL++",
    "stability-ai-nc": "Community license",
  };
  return labels[license.toLowerCase()] ?? license;
}

function Info(props: { label: string; value?: string | null }) {
  return (
    <div className="info-row">
      <span>{props.label}</span>
      <strong>{props.value ?? "Unavailable"}</strong>
    </div>
  );
}
