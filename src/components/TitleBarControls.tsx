import { Minus, Square, X } from "lucide-react";
import { getCurrentWindow } from "@tauri-apps/api/window";

const isTauri = "__TAURI_INTERNALS__" in window;
const appWindow = isTauri ? getCurrentWindow() : null;

export function TitleBarControls() {
  return (
    <div className="window-controls">
      <button className="window-control-button" title="Minimize" aria-label="Minimize window" onClick={() => void appWindow?.minimize()}>
        <Minus size={16} />
      </button>
      <button
        className="window-control-button"
        title="Maximize / Restore"
        aria-label="Maximize or restore window"
        onClick={() => void appWindow?.toggleMaximize()}
      >
        <Square size={14} />
      </button>
      <button className="window-control-button close" title="Close" aria-label="Close window" onClick={() => void appWindow?.close()}>
        <X size={16} />
      </button>
    </div>
  );
}
