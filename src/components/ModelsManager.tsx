import { useEffect, useState } from "react";
import { HardDrive, RefreshCw, StopCircle } from "lucide-react";
import { api } from "../api";
import type { DownloadStatus, LaunchPlan, ModelRecord, PortableRootInfo, RuntimeInventory } from "../types";
import { formatBytes } from "../lib/format";

export function ModelsManager(props: {
  models: ModelRecord[];
  runtime: RuntimeInventory | null;
  root: PortableRootInfo | null;
  refresh: () => void | Promise<void>;
}) {
  const [downloadStatus, setDownloadStatus] = useState<DownloadStatus | null>(null);
  const [launchPlan, setLaunchPlan] = useState<LaunchPlan | null>(null);

  useEffect(() => {
    let cancelled = false;
    const refreshStatus = async () => {
      const status = await api.qwenDownloadStatus();
      if (!cancelled) setDownloadStatus(status);
    };
    void refreshStatus();
    const interval = window.setInterval(() => void refreshStatus(), 1000);
    return () => {
      cancelled = true;
      window.clearInterval(interval);
    };
  }, []);

  const downloadQwen = async () => {
    setLaunchPlan(null);
    setDownloadStatus(await api.downloadQwenModel());
    await props.refresh();
  };
  const cancelQwenDownload = async () => {
    setDownloadStatus(await api.cancelQwenDownload());
  };
  const validateFirstModel = async () => {
    const model = props.models[0];
    if (!model) return;
    await api.validateModel(model.id);
    setLaunchPlan(await api.planModelLaunch(model.id));
  };

  return (
    <>
      <Info label="Model folder" value={props.root?.modelsDir} />
      <div className="model-download-card">
        <div>
          <strong>Qwen3 4B</strong>
          <span>Official Qwen/Qwen3-4B-GGUF · Q4_K_M · {formatBytes(downloadStatus?.totalBytes ?? 2497280256)}</span>
        </div>
        <div className="download-progress">
          <span>{downloadStatus ? downloadStatus.state : "not installed"}</span>
          {downloadStatus?.totalBytes ? (
            <small>
              {formatBytes(downloadStatus.downloadedBytes)} / {formatBytes(downloadStatus.totalBytes)}
              {downloadStatus.percentage != null ? ` · ${downloadStatus.percentage.toFixed(1)}%` : ""}
              {downloadStatus.speedBytesPerSec ? ` · ${formatBytes(downloadStatus.speedBytesPerSec)}/s` : ""}
            </small>
          ) : null}
        </div>
        <div className="button-row">
          <button type="button" onClick={downloadQwen} title="Download Qwen3 4B">
            <HardDrive size={16} />
          </button>
          <button type="button" onClick={cancelQwenDownload} title="Cancel download">
            <StopCircle size={16} />
          </button>
          <button type="button" onClick={validateFirstModel} title="Validate and plan launch" disabled={props.models.length === 0}>
            <RefreshCw size={16} />
          </button>
        </div>
        {downloadStatus?.error ? <p className="muted">{downloadStatus.error}</p> : null}
      </div>
      {props.models.length === 0 ? (
        <p className="muted">No GGUF models discovered under OpenMindAI Root.</p>
      ) : (
        props.models.map((model) => (
          <div className="sub-panel" key={model.id}>
            <Info label="Name" value={model.name} />
            <Info label="Format" value={model.format} />
            <Info label="Quantization" value={model.quantization ?? "Unknown"} />
            <Info label="Size" value={formatBytes(model.sizeBytes)} />
            <Info label="Enabled" value={model.enabled ? "Yes" : "No"} />
            <Info label="State" value={model.state} />
            <Info label="Source" value={model.sourceRepository ?? "Unknown"} />
            <Info label="Verification" value={model.verification ?? "Unknown"} />
            <Info label="Context" value={model.contextLength ? String(model.contextLength) : "Unknown"} />
            <Info label="Path" value={model.path} />
          </div>
        ))
      )}
      <div className="sub-panel">
        <Info label="Runtime selected" value={props.runtime?.selected?.manifest.runtimeName ?? "None"} />
        <Info label="Runtime backend" value={props.runtime?.selected?.manifest.backend} />
        <Info label="llama.cpp device" value={props.runtime?.selected?.deviceOutput?.slice(0, 180)} />
        {launchPlan ? (
          <>
            <Info label="Planned context" value={String(launchPlan.config.contextSize)} />
            <Info label="GPU layers" value={String(launchPlan.config.gpuLayers)} />
            <Info label="CPU threads" value={String(launchPlan.config.threads)} />
            <Info label="Flash attention" value={launchPlan.config.flashAttention ? "On" : "Off"} />
          </>
        ) : null}
      </div>
    </>
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
