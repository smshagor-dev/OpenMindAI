import { useEffect, useMemo, useRef, useState } from "react";
import { MessageSquare } from "lucide-react";
import type { Conversation } from "../types";

export function ChatSearch(props: {
  open: boolean;
  conversations: Conversation[];
  onSelect: (id: string) => void;
  onClose: () => void;
}) {
  const [query, setQuery] = useState("");
  const inputRef = useRef<HTMLInputElement | null>(null);

  useEffect(() => {
    if (props.open) {
      setQuery("");
      window.setTimeout(() => inputRef.current?.focus(), 0);
    }
  }, [props.open]);

  useEffect(() => {
    if (!props.open) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        props.onClose();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [props]);

  const results = useMemo(() => {
    const normalized = query.trim().toLowerCase();
    const source = normalized
      ? props.conversations.filter((conversation) => conversation.title.toLowerCase().includes(normalized))
      : props.conversations;
    return source.slice(0, 40);
  }, [props.conversations, query]);

  if (!props.open) return null;

  return (
    <div className="modal-overlay" role="presentation" onClick={props.onClose}>
      <div
        className="modal-card search-card"
        role="dialog"
        aria-modal="true"
        aria-label="Search chats"
        onClick={(event) => event.stopPropagation()}
      >
        <input
          ref={inputRef}
          className="search-input"
          placeholder="Search chats..."
          value={query}
          onChange={(event) => setQuery(event.target.value)}
        />
        <div className="search-results">
          {results.length === 0 ? (
            <p className="muted search-empty">No matching chats.</p>
          ) : (
            results.map((conversation) => (
              <button
                type="button"
                className="search-result"
                key={conversation.id}
                onClick={() => {
                  props.onSelect(conversation.id);
                  props.onClose();
                }}
              >
                <MessageSquare size={15} />
                <span>{conversation.title}</span>
              </button>
            ))
          )}
        </div>
      </div>
    </div>
  );
}
