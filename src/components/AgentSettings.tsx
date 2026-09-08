import { useEffect, useMemo, useState } from "react";
import { CheckCircle2, Download, Play, RefreshCw, Server, StopCircle } from "lucide-react";
import { api } from "../api";
import type {
  AppPreferences,
  DownloadStatus,
  HardwareProfile,
  LlamaRuntimeStatus,
  ModelCatalogReport,
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
  startRuntime: () => void;
  stopRuntime: () => void;
}) {
  const [catalog, setCatalog] = useState<ModelCatalogReport | null>(null);
  const [download, setDownload] = useState<DownloadStatus | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [sandbox, setSandbox] = useState<OpenAgentSandboxCapability | null>(null);

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
        <strong>OpenAgent runtime</strong>
        <p className="muted">
          Local API: {props.runtimeStatus?.endpoint ?? "not running"} · Status: {props.runtimeStatus?.state ?? "unknown"}
        </p>
        <p className="muted">
          Backend: {props.runtime?.selected?.manifest.backend ?? "not installed"} · Hardware: {hardwareSummary(props.hardware)}
        </p>
        <div className="button-row">
          <button type="button" onClick={props.startRuntime} title="Start local agent API"><Play size={16} /> Start API</button>
          <button type="button" onClick={props.stopRuntime} title="Stop local agent API"><StopCircle size={16} /> Stop</button>
          <button type="button" onClick={() => void refreshCatalog()} title="Refresh agent setup"><RefreshCw size={16} /> Refresh</button>
          {!props.runtime?.selected ? (
            <button type="button" onClick={() => void api.installRecommendedRuntime().then(() => props.refresh())}>
              <Server size={16} /> Install runtime
            </button>
          ) : null}
        </div>
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
            <option value="isolated_sandbox" disabled={!sandbox?.available}>Strong isolation · no host escape</option>
          </select>
        </label>
        <p className="muted">
          Isolation provider: {sandbox?.provider ?? "none"} · {sandbox?.message ?? "detecting"}.
          {sandbox?.resourceLimits?.length ? ` Limits: ${sandbox.resourceLimits.join(" · ")}.` : ""}
          {sandbox?.processTreeControl ? " Process-tree cleanup enabled." : ""}
          {sandbox?.boundedOutput ? " Output capture is bounded." : ""}
        </p>
      </section>

      <section className="model-catalog-section">
        <h3>NVIDIA Nemotron agent models</h3>
        {agentModels.map((item) => {
          const selected = props.preferences.openagentModelId === item.entry.id;
          const busy = download?.modelId === item.entry.id && ["resolving", "downloading", "verifying"].includes(download.state);
          return (
            <div className="model-download-card" key={item.entry.id}>
              <div>
                <strong>{item.entry.name} {selected ? <span className="model-badge recommended">Active</span> : null}</strong>
                <span>{item.entry.version} · {item.entry.quantization} · {formatBytes(item.entry.sizeBytes)}</span>
                <small>{item.entry.description}</small>
              </div>
              <div className="download-progress"><span>{item.installed ? "Verified locally" : item.compatible ? "Available" : "Hardware not recommended"}</span></div>
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
      {props.models.length === 0 ? <p className="muted">Download and verify a Nemotron model to activate OpenAgent.</p> : null}
    </div>
  );
}

function hardwareSummary(hardware: HardwareProfile | null) {
  if (!hardware) return "detecting";
  if (hardware.backends.cuda) return "NVIDIA CUDA";
  if (hardware.backends.vulkan) return "Vulkan";
  if (hardware.backends.metal) return "Apple Metal";
  return "CPU";
}
