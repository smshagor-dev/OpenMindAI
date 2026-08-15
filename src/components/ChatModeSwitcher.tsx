export function ChatModeSwitcher() {
  return (
    <div className="chat-mode-switcher" role="tablist" aria-label="Chat mode">
      <button type="button" className="chat-mode-tab active" role="tab" aria-selected="true">
        Chat
      </button>
      <button
        type="button"
        className="chat-mode-tab"
        role="tab"
        aria-selected="false"
        disabled
        title="Work mode — coming soon"
      >
        Work
      </button>
    </div>
  );
}
