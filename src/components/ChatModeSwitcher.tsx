import { useState } from "react";
import { ConnectedAgentPanel } from "./ConnectedAgentPanel";

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
          title="Google Workspace and GitHub connected work"
          onClick={() => setWorkOpen(true)}
        >
          Work
        </button>
      </div>
      {workOpen ? <ConnectedAgentPanel onClose={() => setWorkOpen(false)} /> : null}
    </>
  );
}
