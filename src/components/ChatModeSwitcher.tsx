export function ChatModeSwitcher(props: {
  active: "chat" | "work";
  onChat: () => void;
  onWork: () => void;
}) {
  return (
    <div className="chat-mode-switcher" role="tablist" aria-label="Workspace mode">
      <button
        type="button"
        className={`chat-mode-tab${props.active === "chat" ? " active" : ""}`}
        role="tab"
        aria-selected={props.active === "chat"}
        onClick={props.onChat}
      >
        Chat
      </button>
      <button
        type="button"
        className={`chat-mode-tab${props.active === "work" ? " active" : ""}`}
        role="tab"
        aria-selected={props.active === "work"}
        title="Open project work"
        onClick={props.onWork}
      >
        Work
      </button>
    </div>
  );
}
