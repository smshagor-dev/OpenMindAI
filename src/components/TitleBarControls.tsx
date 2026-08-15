import { Minus, Square, X } from "lucide-react";
import { getCurrentWindow } from "@tauri-apps/api/window";

const isTauri = "__TAURI_INTERNALS__" in window;
const appWindow = isTauri ? getCurrentWindow() : null;

export function TitleBarControls() {
  return (
    <div className="window-controls">
      <button className="window-control-button" title="Minimize" onClick={() => void appWindow?.minimize()}>
        <Minus size={14} />
      </button>
      <button
        className="window-control-button"
        title="Maximize / Restore"
        onClick={() => void appWindow?.toggleMaximize()}
      >
        <Square size={12} />
      </button>
      <button className="window-control-button close" title="Close" onClick={() => void appWindow?.close()}>
        <X size={15} />
      </button>
    </div>
  );
}
