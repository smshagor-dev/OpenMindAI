import { useEffect, useMemo, useRef, useState } from "react";
import { MessageSquare } from "lucide-react";
import { api } from "../api";
import { userMessageDisplay } from "../lib/chat";
import type { Conversation } from "../types";

interface SearchEntry {
  conversation: Conversation;
  content: string;
}

export function ChatSearch(props: {
  open: boolean;
  conversations: Conversation[];
  onSelect: (id: string) => void;
  onClose: () => void;
}) {
  const { open, conversations, onSelect, onClose } = props;
  const [query, setQuery] = useState("");
  const [entries, setEntries] = useState<SearchEntry[]>([]);
  const [loading, setLoading] = useState(false);
  const inputRef = useRef<HTMLInputElement | null>(null);

  useEffect(() => {
    if (!open) return;
    setQuery("");
    window.setTimeout(() => inputRef.current?.focus(), 0);

    let cancelled = false;
    setLoading(true);
    void Promise.allSettled(
      conversations.map(async (conversation) => {
        const messages = await api.messages(conversation.id);
        const searchable = messages
          .filter((message) => message.role !== "system")
          .map((message) =>
            message.role === "user" ? userMessageDisplay(message.content).displayText : message.content,
          )
          .join("\n");
        return { conversation, content: searchable } satisfies SearchEntry;
      }),
    ).then((results) => {
      if (cancelled) return;
      const loaded = results.flatMap((result, index) =>
        result.status === "fulfilled"
          ? [result.value]
          : [{ conversation: conversations[index], content: "" }],
      );
      setEntries(loaded);
      setLoading(false);
    });

    return () => {
      cancelled = true;
    };
  }, [conversations, open]);

  useEffect(() => {
    if (!open) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [open, onClose]);

  const results = useMemo(() => {
    const normalized = query.trim().toLowerCase();
    const source = normalized
      ? entries.filter(
          ({ conversation, content }) =>
            conversation.title.toLowerCase().includes(normalized) ||
            content.toLowerCase().includes(normalized),
        )
      : entries;
    return source.slice(0, 40);
  }, [entries, query]);

  if (!open) return null;

  return (
    <div className="modal-overlay" role="presentation" onClick={onClose}>
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
          placeholder="Search chat titles and messages..."
          value={query}
          onChange={(event) => setQuery(event.target.value)}
        />
        <div className="search-results">
          {loading ? (
            <p className="muted search-empty">Indexing local chat history...</p>
          ) : results.length === 0 ? (
            <p className="muted search-empty">No matching chats.</p>
          ) : (
            results.map(({ conversation }) => (
              <button
                type="button"
                className="search-result"
                key={conversation.id}
                onClick={() => {
                  onSelect(conversation.id);
                  onClose();
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
