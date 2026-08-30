import { BrainCircuit, ChevronRight, Cpu, Search, Sparkles } from "lucide-react";
import type { Conversation, ModelRecord, RuntimeInventory } from "../types";

export function MobileHome(props: {
  conversations: Conversation[];
  models: ModelRecord[];
  activeModelId: string | null;
  runtime: RuntimeInventory | null;
  onOpenConversation: (id: string) => void;
  onOpenModels: () => void;
  onOpenSearch: () => void;
}) {
  const activeModel =
    props.models.find((model) => model.id === props.activeModelId) ??
    props.models.find((model) => model.state === "loaded") ??
    props.models.find((model) => model.state === "ready") ??
    props.models.find((model) => model.enabled) ??
    null;
  const recent = props.conversations.filter((item) => !item.archivedAt).slice(0, 5);
  const ready = Boolean(activeModel) && props.runtime?.selected != null;

  return (
    <section className="mobile-home" aria-label="OpenMindAI home">
      <div className="mobile-home-heading">
        <div>
          <span className="mobile-home-kicker">OPENMINDAI</span>
          <h2>Your local AI workspace</h2>
        </div>
        <button
          type="button"
          className="mobile-home-search"
          aria-label="Search conversations"
          onClick={props.onOpenSearch}
        >
          <Search size={19} />
        </button>
      </div>

      <button type="button" className="mobile-home-card model" onClick={props.onOpenModels}>
        <span className="mobile-home-card-icon model-icon">
          <Cpu size={19} />
        </span>
        <span className="mobile-home-card-copy">
          <small>Local Model</small>
          <strong>{activeModel?.name ?? "Choose a local model"}</strong>
          <span>
            {activeModel
              ? `${activeModel.quantization ?? activeModel.format} · Local`
              : "Download a compatible model to begin"}
          </span>
        </span>
        <span className={ready ? "mobile-ready-pill" : "mobile-ready-pill pending"}>
          <i /> {ready ? "Ready" : "Setup"}
        </span>
      </button>

      <div className="mobile-home-card thinking">
        <span className="mobile-home-card-icon thinking-icon">
          <BrainCircuit size={19} />
        </span>
        <span className="mobile-home-card-copy">
          <small>Thinking Mode</small>
          <strong>Enhanced reasoning available</strong>
          <span>Runs through your selected local model</span>
        </span>
        <Sparkles className="mobile-thinking-spark" size={17} />
      </div>

      <div className="mobile-recent-heading">
        <span>Recent Conversations</span>
        <button type="button" onClick={props.onOpenSearch}>Search</button>
      </div>

      <div className="mobile-recent-list">
        {recent.length > 0 ? (
          recent.map((conversation) => (
            <button
              type="button"
              className="mobile-recent-row"
              key={conversation.id}
              onClick={() => props.onOpenConversation(conversation.id)}
            >
              <span className="mobile-recent-copy">
                <strong>{conversation.title}</strong>
                <small>{relativeTime(conversation.updatedAt)}</small>
              </span>
              <ChevronRight size={17} />
            </button>
          ))
        ) : (
          <div className="mobile-recent-empty">
            <Sparkles size={18} />
            <span>Your conversations will appear here.</span>
          </div>
        )}
      </div>
    </section>
  );
}

function relativeTime(value: string) {
  const timestamp = new Date(value).getTime();
  if (!Number.isFinite(timestamp)) return "Recently";
  const delta = Math.max(0, Date.now() - timestamp);
  const minutes = Math.floor(delta / 60_000);
  if (minutes < 1) return "Just now";
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  if (days < 7) return `${days}d ago`;
  return new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric" }).format(
    new Date(timestamp),
  );
}
