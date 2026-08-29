import type { ModelRecord, PortableRootInfo, RuntimeInventory } from "../types";

const BACKEND_LABELS: Record<string, string> = {
  cpu: "CPU",
  cuda: "CUDA",
  vulkan: "Vulkan",
  sycl: "SYCL",
  hip: "HIP",
  metal: "Metal",
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
  const modeLabel = props.root?.mode === "portableMarker" ? "Portable mode" : "System Active";
  const modelReady = model ? model.state === "ready" || model.state === "loaded" : false;
  const backendReady = Boolean(backend);
  const portableReady = Boolean(props.root);

  return (
    <div className="status-bar">
      <span className="status-segment">
        <span className={modelReady ? "status-dot status-dot-ready" : "status-dot"} />
        {model ? "OpenMindAI" : "No model installed"}
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
