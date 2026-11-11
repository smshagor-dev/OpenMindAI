import { useEffect, useRef, useState } from "react";
import { ChevronDown, Check, Loader2 } from "lucide-react";
import type { ModelRecord, RuntimeInventory } from "../types";
import { formatBytes } from "../lib/format";

export function ModelSelector(props: {
  models: ModelRecord[];
  activeModelId: string | null;
  runtime: RuntimeInventory | null;
  switching: boolean;
  disabled?: boolean;
  switchError: string | null;
  onSelect: (modelId: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const containerRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!open) return;
    const onClickOutside = (event: MouseEvent) => {
      if (containerRef.current && !containerRef.current.contains(event.target as Node)) {
        setOpen(false);
      }
    };
    window.addEventListener("mousedown", onClickOutside);
    return () => window.removeEventListener("mousedown", onClickOutside);
  }, [open]);

  const visibleModels = modelsById(props.models.filter((model) => model.enabled));
  const activeModel = props.models.find((model) => model.id === props.activeModelId) ?? null;
  const label = props.switching
    ? "Loading model..."
    : activeModel && !activeModel.enabled
      ? `${displayModelName(activeModel)} (unavailable)`
      : selectedModelLabel(activeModel ?? visibleModels[0] ?? null, visibleModels);

  return (
    <div className="model-selector" ref={containerRef}>
      <button
        type="button"
        className="model-selector-trigger"
        onClick={() => setOpen((value) => !value)}
        disabled={visibleModels.length === 0 || props.disabled}
        aria-haspopup="listbox"
        aria-expanded={open}
        title={label}
      >
        {props.switching ? <Loader2 size={14} className="spin" /> : null}
        <span>{label}</span>
        <ChevronDown size={14} />
      </button>
      {open ? (
        <div className="model-selector-menu" role="listbox">
          {visibleModels.length === 0 ? (
            <p className="muted model-selector-empty">
              No OpenMindAI model installed yet. Add one from Settings &gt; Models.
            </p>
          ) : (
            visibleModels.map((model) => {
              const selected =
                model.id === props.activeModelId || (!props.activeModelId && model === visibleModels[0]);
              return (
                <button
                  type="button"
                  role="option"
                  disabled={props.disabled}
                  aria-selected={selected}
                  key={model.id}
                  className="model-selector-item"
                  onClick={() => {
                    props.onSelect(model.id);
                    setOpen(false);
                  }}
                >
                  <div className="model-selector-item-text">
                    <strong>{displayModelName(model)}</strong>
                    <small title={`${model.path} (${model.id})`}>{optionDetails(model, visibleModels)}</small>
                  </div>
                  {selected ? <Check size={15} className="model-selector-check" /> : null}
                </button>
              );
            })
          )}
        </div>
      ) : null}
      {props.switchError ? <p className="model-selector-error">{props.switchError}</p> : null}
    </div>
  );
}

function modelsById(models: ModelRecord[]): ModelRecord[] {
  return Array.from(new Map(models.map((model) => [model.id, model])).values()).sort((left, right) =>
    displayModelName(left).localeCompare(displayModelName(right)) ||
    modelDetails(left).localeCompare(modelDetails(right)) || left.id.localeCompare(right.id),
  );
}

function modelDetails(model: ModelRecord): string {
  const filename = model.path.split(/[\\/]/).pop() || model.id;
  return [model.quantization, formatBytes(model.sizeBytes), filename].filter(Boolean).join(" · ");
}

function optionDetails(model: ModelRecord, models: ModelRecord[]): string {
  const details = modelDetails(model);
  const ambiguous = models.some((other) => other.id !== model.id &&
    displayModelName(other) === displayModelName(model) && modelDetails(other) === details);
  return ambiguous ? `${model.id} · ${details}` : details;
}

function selectedModelLabel(model: ModelRecord | null, models: ModelRecord[]): string {
  const name = displayModelName(model);
  if (!model || !models.some((other) => other.id !== model.id && displayModelName(other) === name)) return name;
  return `${name} · ${optionDetails(model, models)}`;
}

function displayModelName(model: ModelRecord | null): string {
  if (!model) return "No model installed";
  if (model.name.startsWith("OpenMindAI") || model.name.startsWith("OpenAgent")) return model.name;

  const repoName = modelNameByRepo(model.sourceRepository);
  if (repoName) return repoName;

  const pathName = modelNameByPath(model.path);
  if (pathName) return pathName;

  return "OpenMindAI Model";
}

function modelNameByRepo(repo: string | null): string | null {
  const names: Record<string, string> = {
    "Qwen/Qwen3-4B-GGUF": "OpenMindAI Core",
    "Qwen/Qwen3-8B-GGUF": "OpenMindAI Titan",
    "ggml-org/Qwen2.5-VL-3B-Instruct-GGUF": "OpenMindAI Lens",
    "ggml-org/gpt-oss-20b-GGUF": "OpenMindAI Forge",
    "ggml-org/gpt-oss-120b-GGUF": "OpenMindAI Forge Max",
    "ggml-org/gemma-4-E2B-it-GGUF": "OpenMindAI Flash",
    "ggml-org/gemma-4-E4B-it-GGUF": "OpenMindAI Flash Plus",
    "ggml-org/gemma-4-12B-it-GGUF": "OpenMindAI Vision",
    "ggml-org/gemma-4-26B-A4B-it-GGUF": "OpenMindAI Vision Pro",
    "ggml-org/gemma-4-31B-it-GGUF": "OpenMindAI Vision Max",
    "nvidia/NVIDIA-Nemotron-3-Nano-4B-GGUF": "OpenAgent Lite",
    "ggml-org/NVIDIA-Nemotron-3-Nano-30B-A3B-GGUF": "OpenAgent",
    "ggml-org/NVIDIA-Nemotron-3.5-Lightning-30B-A3B-GGUF": "OpenAgent Lightning",
    "ggml-org/Nemotron-3-Super-120B-GGUF": "OpenAgent Pro",
  };
  return repo ? names[repo] ?? null : null;
}

function modelNameByPath(path: string): string | null {
  const normalized = path.replace(/\\/g, "/").toLowerCase();
  if (normalized.includes("/qwen3-4b/")) return "OpenMindAI Core";
  if (normalized.includes("/qwen3-8b/")) return "OpenMindAI Titan";
  if (normalized.includes("/qwen2.5-vl-3b/")) return "OpenMindAI Lens";
  if (normalized.includes("/forge-max/") || normalized.includes("gpt-oss-120b")) return "OpenMindAI Forge Max";
  if (normalized.includes("/forge/") || normalized.includes("gpt-oss-20b")) return "OpenMindAI Forge";
  if (normalized.includes("/flash-plus/") || normalized.includes("gemma-4-e4b")) return "OpenMindAI Flash Plus";
  if (normalized.includes("/flash/") || normalized.includes("gemma-4-e2b")) return "OpenMindAI Flash";
  if (normalized.includes("/vision-max/") || normalized.includes("gemma-4-31b")) return "OpenMindAI Vision Max";
  if (normalized.includes("/vision-pro/") || normalized.includes("gemma-4-26b")) return "OpenMindAI Vision Pro";
  if (normalized.includes("/vision/") || normalized.includes("gemma-4-12b")) return "OpenMindAI Vision";
  if (normalized.includes("/agent-lightning/") || normalized.includes("nemotron-3.5-lightning")) {
    return "OpenAgent Lightning";
  }
  if (normalized.includes("/agent-pro/") || normalized.includes("nemotron-3-super")) return "OpenAgent Pro";
  if (normalized.includes("/agent-lite/") || normalized.includes("nemotron3-nano-4b")) return "OpenAgent Lite";
  if (normalized.includes("/agent/") || normalized.includes("nemotron-3-nano-30b")) return "OpenAgent";
  return null;
}
