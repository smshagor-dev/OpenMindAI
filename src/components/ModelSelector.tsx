import { useEffect, useRef, useState } from "react";
import { ChevronDown, Check, Loader2 } from "lucide-react";
import type { ModelRecord, RuntimeInventory } from "../types";

export function ModelSelector(props: {
  models: ModelRecord[];
  activeModelId: string | null;
  runtime: RuntimeInventory | null;
  switching: boolean;
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

  const visibleModels = dedupeModelsByPersonalName(props.models, props.activeModelId);
  const activeModel = props.models.find((model) => model.id === props.activeModelId) ?? null;
  const label = props.switching
    ? "Loading model..."
    : displayModelName(activeModel ?? visibleModels[0] ?? null);

  return (
    <div className="model-selector" ref={containerRef}>
      <button
        type="button"
        className="model-selector-trigger"
        onClick={() => setOpen((value) => !value)}
        disabled={visibleModels.length === 0}
        aria-haspopup="listbox"
        aria-expanded={open}
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

function dedupeModelsByPersonalName(models: ModelRecord[], activeModelId: string | null): ModelRecord[] {
  const bestByName = new Map<string, ModelRecord>();
  for (const model of models) {
    const name = displayModelName(model);
    const current = bestByName.get(name);
    if (!current || modelPreferenceScore(model, activeModelId) > modelPreferenceScore(current, activeModelId)) {
      bestByName.set(name, model);
    }
  }
  return Array.from(bestByName.values()).sort((left, right) =>
    displayModelName(left).localeCompare(displayModelName(right)),
  );
}

function modelPreferenceScore(model: ModelRecord, activeModelId: string | null): number {
  let score = 0;
  if (model.id === activeModelId) score += 1000;
  if (model.enabled) score += 100;
  if (model.state === "loaded") score += 50;
  if (model.state === "ready") score += 40;
  if (model.verification === "verified") score += 30;
  if (model.path.toLowerCase().includes("q4_k_m")) score += 20;
  if (model.path.toLowerCase().includes("q8_0")) score -= 20;
  return score;
}

function displayModelName(model: ModelRecord | null): string {
  if (!model) return "No model installed";
  if (model.name.startsWith("OpenMindAI")) return model.name;

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
    "nvidia/NVIDIA-Nemotron-3-Nano-4B-GGUF": "OpenMindAI Agent Lite",
    "ggml-org/NVIDIA-Nemotron-3-Nano-30B-A3B-GGUF": "OpenMindAI Agent",
    "ggml-org/NVIDIA-Nemotron-3.5-Lightning-30B-A3B-GGUF": "OpenMindAI Agent Lightning",
    "ggml-org/Nemotron-3-Super-120B-GGUF": "OpenMindAI Agent Pro",
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
    return "OpenMindAI Agent Lightning";
  }
  if (normalized.includes("/agent-pro/") || normalized.includes("nemotron-3-super")) return "OpenMindAI Agent Pro";
  if (normalized.includes("/agent-lite/") || normalized.includes("nemotron3-nano-4b")) return "OpenMindAI Agent Lite";
  if (normalized.includes("/agent/") || normalized.includes("nemotron-3-nano-30b")) return "OpenMindAI Agent";
  return null;
}
