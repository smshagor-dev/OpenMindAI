import { useState } from "react";
import { ConnectedWorkspace } from "./ConnectedWorkspace";

export function ChatModeSwitcher() {
  const [workOpen, setWorkOpen] = useState(false);

  return (
    <>
      <div className="chat-mode-switcher" role="tablist" aria-label="Workspace mode">
        <button
          type="button"
          className={`chat-mode-tab${workOpen ? "" : " active"}`}
          role="tab"
          aria-selected={!workOpen}
          onClick={() => setWorkOpen(false)}
        >
          Chat
        </button>
        <button
          type="button"
          className={`chat-mode-tab${workOpen ? " active" : ""}`}
          role="tab"
          aria-selected={workOpen}
          title="Google Workspace and GitHub actions"
          onClick={() => setWorkOpen(true)}
        >
          Work
        </button>
      </div>
      {workOpen ? <ConnectedWorkspace onClose={() => setWorkOpen(false)} /> : null}
    </>
  );
}
