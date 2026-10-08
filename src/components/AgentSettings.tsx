import { useEffect, useMemo, useState } from "react";
import { CheckCircle2, Download, LoaderCircle, Play, RefreshCw, Server, ShieldCheck, StopCircle } from "lucide-react";
import { api } from "../api";
import type {
  AppPreferences,
  CodingAgentStatus,
  DownloadStatus,
  HardwareProfile,
  LlamaRuntimeStatus,
  ModelCatalogReport,
  ModelCatalogStatus,
  ModelRecord,
  OpenAgentSandboxCapability,
  RuntimeInventory,
} from "../types";
import { formatBytes } from "../lib/format";

export function AgentSettings(props: {
  hardware: HardwareProfile | null;
  models: ModelRecord[];
  runtime: RuntimeInventory | null;
  runtimeStatus: LlamaRuntimeStatus | null;
  preferences: AppPreferences;
  onPreferencesChange: (preferences: AppPreferences) => void;
  refresh: () => void | Promise<void>;
  startRuntime: (action?: "start" | "restart") => void | Promise<void>;
  stopRuntime: () => void | Promise<void>;
  runtimeAction: "start" | "restart" | "stop" | null;
}) {
  const [catalog, setCatalog] = useState<ModelCatalogReport | null>(null);
  const [download, setDownload] = useState<DownloadStatus | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [sandbox, setSandbox] = useState<OpenAgentSandboxCapability | null>(null);
  const [qualification, setQualification] = useState<string | null>(null);
  const [agentStatus, setAgentStatus] = useState<CodingAgentStatus | null>(null);

  const refreshCatalog = async () => {
    try {
      setCatalog(await api.checkModelUpdates());
      setDownload(await api.modelDownloadStatus());
      setSandbox(await api.openagentSandboxCapability());
      setMessage(null);
    } catch (error) {
      setMessage(error instanceof Error ? error.message : String(error));
    }
  };

  useEffect(() => {
    void refreshCatalog();
  }, []);

  useEffect(() => {
    let cancelled = false;
    const refreshStatus = async () => {
      try {
        const status = await api.modelDownloadStatus();
        if (!cancelled) setDownload(status);
      } catch (error) {
        if (!cancelled) setMessage(error instanceof Error ? error.message : String(error));
      }
    };
    const interval = window.setInterval(() => void refreshStatus(), 1000);
    return () => {
      cancelled = true;
      window.clearInterval(interval);
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    const refreshAgentStatus = async () => {
      try {
        const status = await api.codingAgentStatus();
        if (!cancelled) setAgentStatus(status);
      } catch {
        if (!cancelled) setAgentStatus(null);
      }
    };
    void refreshAgentStatus();
    const interval = window.setInterval(() => void refreshAgentStatus(), 2000);
    return () => {
      cancelled = true;
      window.clearInterval(interval);
    };
  }, [props.preferences]);

  const loadAgent = async () => {
    try {
      setAgentStatus(await api.startCodingAgent());
    } catch (error) {
      setMessage(error instanceof Error ? error.message : String(error));
    }
  };

  const agentModels = useMemo(
    () =>
      (catalog?.entries ?? [])
        .filter((item) => item.entry.family === "nemotron" && item.entry.kind === "agent")
        .sort((left, right) => Number(right.installed) - Number(left.installed)),
    [catalog],
  );

  const save = <K extends keyof AppPreferences>(key: K, value: AppPreferences[K]) => {
    props.onPreferencesChange({ ...props.preferences, [key]: value });
  };

  const install = async (modelId: string) => {
    setMessage(null);
    const item = agentModels.find((candidate) => candidate.entry.id === modelId);
    if (item) {
      setDownload(preparingDownloadStatus(item));
    }
    try {
      setDownload(await api.downloadCatalogModel(modelId));
      await props.refresh();
      await refreshCatalog();
    } catch (error) {
      setMessage(error instanceof Error ? error.message : String(error));
      setDownload(await api.modelDownloadStatus());
    }
  };

  return (
    <div className="agent-settings-stack">
      <section className="sub-panel">
        <div className="agent-runtime-head">
          <strong>OpenAgent runtime</strong>
          <span className={`agent-runtime-status ${runtimeStatusClass(props.runtimeStatus?.state, props.runtimeAction)}`}>
            {props.runtimeAction ? <LoaderCircle className="spin" size={14} /> : <span className="agent-runtime-dot" />}
            {runtimeStatusLabel(props.runtimeStatus?.state, props.runtimeAction)}
          </span>
        </div>
        <p className="muted">
          Local API: {props.runtimeStatus?.endpoint ?? "not running"} · Status: {props.runtimeStatus?.state ?? "unknown"}
        </p>
        <p className="muted">
          Backend: {props.runtime?.selected?.manifest.backend ?? "not installed"} · Hardware: {hardwareSummary(props.hardware)}
        </p>
        <label className="agent-setting-field agent-toggle-field">
          <span>Auto-start agent API</span>
          <input
            type="checkbox"
            checked={props.preferences.localRuntimeAutostart}
            onChange={(event) => save("localRuntimeAutostart", event.target.checked)}
          />
        </label>
        <div className="button-row">
          <button
            type="button"
            onClick={() => void props.startRuntime(isRuntimeRunning(props.runtimeStatus?.state) ? "restart" : "start")}
            disabled={props.runtimeAction !== null || !props.runtime?.selected}
            title={isRuntimeRunning(props.runtimeStatus?.state) ? "Restart local agent API" : "Start local agent API"}
          >
            {props.runtimeAction === "start" || props.runtimeAction === "restart" ? (
              <LoaderCircle className="spin" size={16} />
            ) : (
              <Play size={16} />
            )}
            {isRuntimeRunning(props.runtimeStatus?.state) ? "Restart API" : "Start API"}
          </button>
          <button
            type="button"
            onClick={() => void props.stopRuntime()}
            disabled={props.runtimeAction !== null || !isRuntimeRunning(props.runtimeStatus?.state)}
            title="Stop local agent API"
          >
            {props.runtimeAction === "stop" ? <LoaderCircle className="spin" size={16} /> : <StopCircle size={16} />}
            Stop
          </button>
          <button type="button" onClick={() => void refreshCatalog()} title="Refresh agent setup"><RefreshCw size={16} /> Refresh</button>
          {!props.runtime?.selected ? (
            <button type="button" onClick={() => void api.installRecommendedRuntime().then(() => props.refresh())}>
              <Server size={16} /> Install runtime
            </button>
          ) : null}
        </div>
      </section>

      <section className="sub-panel">
        <div className="agent-runtime-head">
          <strong>Coding agent</strong>
          <span className={`agent-runtime-status ${agentStatusClass(agentStatus?.state)}`}>
            {agentStatus?.state === "starting" || agentStatus?.state === "loadingModel" ? (
              <LoaderCircle className="spin" size={14} />
            ) : (
              <span className="agent-runtime-dot" />
            )}
            {agentStatusLabel(agentStatus)}
          </span>
        </div>
        {agentStatus?.model ? (
          <p className="muted">
            {agentStatus.model.name} · {agentStatus.model.displayName}
            {agentStatus.model.quantization ? ` · ${agentStatus.model.quantization}` : ""}
          </p>
        ) : null}
        {agentStatus?.model && agentStatus.effectiveContext ? (
          <p className="muted">
            Context {agentStatus.configuredContext} · {agentStatus.parallelWorkers} parallel{" "}
            {agentStatus.parallelWorkers === 1 ? "worker" : "workers"} · {agentStatus.effectiveContext} tokens per request
            {agentStatus.effectiveContextSource === "estimate" ? " (estimated until loaded)" : ""} ·{" "}
            {agentStatus.gpuLayers ? `GPU layers ${agentStatus.gpuLayers}` : "CPU only"}
          </p>
        ) : null}
        {agentStatus?.runtimePlacement ? (
          <p className="muted">
            {agentStatus.runtimePlacement === "dedicated" ? "Dedicated runtime" : "Shared runtime with Core"}:{" "}
            {agentStatus.placementReason}
            {agentStatus.runtimePlacement === "shared" && agentStatus.sharedRuntimeModel
              ? ` ${agentStatus.sharedRuntimeModel} is loaded now; the agent loads on the next coding request.`
              : ""}
          </p>
        ) : null}
        {agentStatus?.state === "disabled" || agentStatus?.state === "notInstalled" ? (
          <p className="model-selector-error">{agentStatus.message}</p>
        ) : null}
        {agentStatus?.state === "error" && agentStatus.error ? (
          <p className="model-selector-error">{agentStatus.error}</p>
        ) : null}
        <label className="agent-setting-field">
          <span>Agent runtime mode</span>
          <select
            value={props.preferences.openagentRuntimeMode ?? "automatic"}
            onChange={(event) => save("openagentRuntimeMode", event.target.value as AppPreferences["openagentRuntimeMode"])}
          >
            <option value="automatic">Automatic</option>
            <option value="shared">Shared runtime</option>
            <option value="dedicated">Dedicated runtime</option>
          </select>
        </label>
        {agentStatus?.state === "stopped" || agentStatus?.state === "error" ? (
          <div className="button-row">
            <button type="button" onClick={() => void loadAgent()} title="Load the coding agent model now">
              <Play size={16} /> Load agent
            </button>
          </div>
        ) : null}
      </section>

      <section className="sub-panel">
        <strong>Execution policy</strong>
        <label className="agent-setting-field">
          <span>Approval mode</span>
          <select value={props.preferences.openagentApprovalMode} onChange={(event) => save("openagentApprovalMode", event.target.value as AppPreferences["openagentApprovalMode"])}>
            <option value="risk_based">Risk based</option>
            <option value="always_ask">Always ask</option>
            <option value="trusted_workspace">Trusted workspace</option>
          </select>
        </label>
        <label className="agent-setting-field">
          <span>Sandbox</span>
          <select value={props.preferences.openagentSandboxMode} onChange={(event) => save("openagentSandboxMode", event.target.value as AppPreferences["openagentSandboxMode"])}>
            <option value="attached_workspace">Attached workspace boundary</option>
            <option value="isolated_sandbox">Strong isolation · no host escape{sandbox?.available ? "" : " (provider unavailable)"}</option>
          </select>
        </label>
        <p className="muted">
          Isolation provider: {sandbox?.provider ?? "none"} · {sandbox?.message ?? "detecting"}.
          {sandbox?.resourceLimits?.length ? ` Limits: ${sandbox.resourceLimits.join(" · ")}.` : ""}
          {sandbox?.processTreeControl ? " Process-tree cleanup enabled." : ""}
          {sandbox?.boundedOutput ? " Output capture is bounded." : ""}
        </p>
      </section>

      <section className="sub-panel">
        <strong>Coding Workspace controls</strong>
        <label className="agent-setting-field agent-toggle-field"><span>Enabled</span><input type="checkbox" checked={props.preferences.codingEnabled} onChange={(event) => save("codingEnabled", event.target.checked)} /></label>
        <label className="agent-setting-field"><span>Autonomy</span><select value={props.preferences.codingAutonomy} onChange={(event) => save("codingAutonomy", event.target.value as AppPreferences["codingAutonomy"])}><option value="bounded">Bounded execution</option><option value="review_first">Review first</option></select></label>
        <p className="muted">OpenAgent can keep working continuously. Runs no longer stop from token or minute budgets.</p>
        <label className="agent-setting-field"><span>Parallel read workers</span><input type="number" min={1} max={4} value={props.preferences.codingMaxParallelWorkers} onChange={(event) => save("codingMaxParallelWorkers", Number(event.target.value))} /></label>
        <label className="agent-setting-field"><span>CI repair limit</span><input type="number" min={1} max={5} value={props.preferences.codingCiRepairLimit} onChange={(event) => save("codingCiRepairLimit", Number(event.target.value))} /></label>
        <label className="agent-setting-field"><span>Context size</span><input type="number" min={4096} max={131072} step={4096} value={props.preferences.codingContextSize} onChange={(event) => save("codingContextSize", Number(event.target.value))} /></label>
        <label className="agent-setting-field"><span>GPU layers (-1 auto)</span><input type="number" min={-1} max={999} value={props.preferences.codingGpuLayers} onChange={(event) => save("codingGpuLayers", Number(event.target.value))} /></label>
        <label className="agent-setting-field"><span>Sandbox network</span><select value={props.preferences.codingNetworkEnabled ? "on" : "off"} onChange={(event) => save("codingNetworkEnabled", event.target.value === "on")}><option value="off">Off · default</option><option value="on" disabled>On · reserved for explicit future policy</option></select></label>
        <div className="button-row"><button type="button" onClick={() => void api.runCodingQualification().then((report) => setQualification(report.passed ? `Qualification passed · ${report.checks.length} checks` : `Qualification failed · ${report.checks.filter((item) => !item.passed).map((item) => item.id).join(", ")}`)).catch((error) => setQualification(String(error)))}><ShieldCheck size={16} /> Run qualification</button></div>
        {qualification ? <p className="muted">{qualification}</p> : null}
      </section>

      <section className="model-catalog-section">
        <h3>OpenAgent - Your personal Agent</h3>
        {agentModels.map((item) => {
          const selected = props.preferences.openagentModelId === item.entry.id;
          const itemStatus = download?.modelId === item.entry.id ? download : null;
          const busy = itemStatus !== null && isActiveDownload(itemStatus);
          return (
            <div className="model-download-card" key={item.entry.id}>
              <div>
                <strong>{item.entry.name} {selected ? <span className="model-badge recommended">Active</span> : null}</strong>
                <span>{formatBytes(item.entry.sizeBytes)} · {deviceSupportLabel(item)}</span>
                <small>{item.entry.description}</small>
              </div>
              <div className="download-progress">
                <span>{downloadProgressLabel(item, itemStatus)}</span>
                {itemStatus?.totalBytes ? (
                  <small>
                    {formatBytes(itemStatus.downloadedBytes)} / {formatBytes(itemStatus.totalBytes)}
                    {itemStatus.percentage != null ? ` Â· ${itemStatus.percentage.toFixed(1)}%` : ""}
                    {itemStatus.speedBytesPerSec ? ` Â· ${formatBytes(itemStatus.speedBytesPerSec)}/s` : ""}
                  </small>
                ) : null}
                {itemStatus?.error ? <small>{itemStatus.error}</small> : null}
              </div>
              <div className="button-row">
                {item.installed ? (
                  <button type="button" disabled={selected} onClick={() => save("openagentModelId", item.entry.id)}>
                    <CheckCircle2 size={16} /> {selected ? "Active" : "Use for OpenAgent"}
                  </button>
                ) : (
                  <button type="button" disabled={!item.downloadSupported || busy} onClick={() => void install(item.entry.id)}>
                    <Download size={16} /> {busy ? "Installing…" : "Download"}
                  </button>
                )}
              </div>
            </div>
          );
        })}
      </section>
      {message ? <p className="model-selector-error">{message}</p> : null}
      {props.models.length === 0 ? <p className="muted">Download and verify an OpenAgent model to activate your personal agent.</p> : null}
    </div>
  );
}

function agentStatusClass(state: CodingAgentStatus["state"] | undefined) {
  if (state === "starting" || state === "loadingModel") return "agent-runtime-status-busy";
  if (state === "ready") return "agent-runtime-status-ready";
  return "agent-runtime-status-stopped";
}

function agentStatusLabel(status: CodingAgentStatus | null) {
  switch (status?.state) {
    case "disabled":
      return "Disabled";
    case "notInstalled":
      return "Not installed";
    case "starting":
      return "Starting";
    case "loadingModel":
      return status.elapsedMs ? `Loading model · ${Math.round(status.elapsedMs / 1000)}s` : "Loading model";
    case "ready":
      return "Ready";
    case "error":
      return "Error";
    case "stopped":
      return "Stopped";
    default:
      return "Unknown";
  }
}

function isRuntimeRunning(state: LlamaRuntimeStatus["state"] | undefined) {
  return state === "ready" || state === "running" || state === "loadingModel";
}

function runtimeStatusClass(
  state: LlamaRuntimeStatus["state"] | undefined,
  action: "start" | "restart" | "stop" | null,
) {
  if (action || state === "starting" || state === "stopping" || state === "loadingModel") {
    return "agent-runtime-status-busy";
  }
  if (isRuntimeRunning(state)) return "agent-runtime-status-ready";
  return "agent-runtime-status-stopped";
}

function runtimeStatusLabel(
  state: LlamaRuntimeStatus["state"] | undefined,
  action: "start" | "restart" | "stop" | null,
) {
  if (action === "start") return "Starting";
  if (action === "restart") return "Restarting";
  if (action === "stop") return "Stopping";
  if (state === "loadingModel") return "Loading model";
  if (isRuntimeRunning(state)) return "Running";
  if (state === "failed") return "Failed";
  return "Stopped";
}

function hardwareSummary(hardware: HardwareProfile | null) {
  if (!hardware) return "detecting";
  if (hardware.backends.cuda) return "NVIDIA CUDA";
  if (hardware.backends.vulkan) return "Vulkan";
  if (hardware.backends.metal) return "Apple Metal";
  return "CPU";
}

function deviceSupportLabel(item: { installed: boolean; compatible: boolean; downloadSupported: boolean }) {
  if (item.installed) return "Device supported";
  if (!item.downloadSupported) {
    return item.compatible ? "Device supported - manual access" : "Device not recommended";
  }
  return item.compatible ? "Device supported" : "Device not recommended";
}

function isActiveDownload(status: DownloadStatus) {
  return (
    status.state === "resolving" ||
    status.state === "downloading" ||
    status.state === "verifying"
  );
}

function preparingDownloadStatus(item: ModelCatalogStatus): DownloadStatus {
  return {
    modelId: item.entry.id,
    name: item.entry.name,
    state: "resolving",
    repo: item.entry.repo,
    quantization: item.entry.quantization,
    filename: null,
    downloadedBytes: 0,
    totalBytes: item.entry.sizeBytes,
    percentage: 0,
    speedBytesPerSec: null,
    destination: null,
    error: null,
  };
}

function downloadProgressLabel(item: ModelCatalogStatus, status: DownloadStatus | null) {
  if (status?.state === "resolving") return "Preparing download";
  if (status?.state === "downloading") return "Downloading";
  if (status?.state === "verifying") return "Verifying";
  if (status?.state === "completed" || item.installed) return "Verified locally";
  if (status?.state === "pausedInterrupted") return "Paused";
  if (status?.state === "failed") return "Download failed";
  if (status?.state === "cancelled") return "Cancelled";
  return item.compatible ? "Available" : "Hardware not recommended";
}
