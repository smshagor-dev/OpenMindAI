import { useEffect, useRef, useState } from "react";
import { ChevronDown, Check, Loader2 } from "lucide-react";
import type { ModelRecord, RuntimeInventory } from "../types";
import { formatBytes } from "../lib/format";

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

  const activeModel = props.models.find((model) => model.id === props.activeModelId) ?? null;
  const label = props.switching
    ? "Loading model..."
    : activeModel?.name ?? (props.models[0]?.name ?? "No model installed");

  return (
    <div className="model-selector" ref={containerRef}>
      <button
        type="button"
        className="model-selector-trigger"
        onClick={() => setOpen((value) => !value)}
        disabled={props.models.length === 0}
        aria-haspopup="listbox"
        aria-expanded={open}
      >
        {props.switching ? <Loader2 size={14} className="spin" /> : null}
        <span>{label}</span>
        <ChevronDown size={14} />
      </button>
      {open ? (
        <div className="model-selector-menu" role="listbox">
          {props.models.length === 0 ? (
            <p className="muted model-selector-empty">
              No GGUF models installed yet. Add one from Settings &gt; Models.
            </p>
          ) : (
            props.models.map((model) => {
              const selected =
                model.id === props.activeModelId || (!props.activeModelId && model === props.models[0]);
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
                    <strong>{model.name}</strong>
                    <small>
                      {[model.family, model.quantization, formatBytes(model.sizeBytes)]
                        .filter(Boolean)
                        .join(" · ")}
                    </small>
                  </div>
                  {selected ? <Check size={15} className="model-selector-check" /> : null}
                </button>
              );
            })
          )}
          <div className="model-selector-footer">
            <span>
              Runtime: {props.runtime?.selected?.manifest.runtimeName ?? "Not detected"}
              {props.runtime?.selected ? ` · ${props.runtime.selected.manifest.backend}` : ""}
            </span>
          </div>
        </div>
      ) : null}
      {props.switchError ? <p className="model-selector-error">{props.switchError}</p> : null}
    </div>
  );
}
