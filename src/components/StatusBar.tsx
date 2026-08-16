import type { ModelRecord, PortableRootInfo, RuntimeInventory } from "../types";

const BACKEND_LABELS: Record<string, string> = {
  cpu: "CPU",
  cuda: "CUDA",
  vulkan: "Vulkan",
  sycl: "SYCL",
  hip: "HIP",
  metal: "Metal",
};

const STATE_LABELS: Record<ModelRecord["state"], string> = {
  notInstalled: "not installed",
  downloading: "downloading",
  installed: "installed",
  validating: "validating",
  ready: "ready",
  loading: "loading",
  loaded: "loaded",
  unloading: "unloading",
  failed: "failed",
};

export function StatusBar(props: {
  models: ModelRecord[];
  activeModelId: string | null;
  runtime: RuntimeInventory | null;
  root: PortableRootInfo | null;
}) {
  const model = props.models.find((item) => item.id === props.activeModelId) ?? props.models[0] ?? null;
  const backend = props.runtime?.selected?.manifest.backend;
  const backendLabel = backend ? (BACKEND_LABELS[backend] ?? backend) : null;
  const modeLabel = props.root?.mode === "portableMarker" ? "Portable mode" : "Local Active";
  const modelReady = model ? model.state === "ready" || model.state === "loaded" : false;
  const backendReady = Boolean(backend);
  const portableReady = props.root?.mode === "portableMarker";

  return (
    <div className="status-bar">
      <span className="status-segment">
        <span className={modelReady ? "status-dot status-dot-ready" : "status-dot"} />
        {model ? `${model.name} ${STATE_LABELS[model.state]}` : "No model installed"}
      </span>
      {backendLabel ? (
        <span className="status-segment">
          <span className={backendReady ? "status-dot status-dot-ready" : "status-dot"} />
          {backendLabel}
        </span>
      ) : null}
      <span className="status-segment">
        <span className={portableReady ? "status-dot status-dot-ready" : "status-dot"} />
        {modeLabel}
      </span>
    </div>
  );
}
