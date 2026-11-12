import { useEffect, useRef, useState } from "react";
import { MessageSquare } from "lucide-react";
import { api } from "../api";
import type { Conversation } from "../types";

const PAGE_SIZE = 40;
const SEARCH_DEBOUNCE_MS = 250;

export function ChatSearch(props: {
  open: boolean;
  conversations: Conversation[];
  onSelect: (id: string) => void;
  onClose: () => void;
}) {
  const { open, onSelect, onClose } = props;
  const [query, setQuery] = useState("");
  const [search, setSearch] = useState("");
  const [results, setResults] = useState<Conversation[]>([]);
  const [cursors, setCursors] = useState<(string | null)[]>([null]);
  const [pageIndex, setPageIndex] = useState(0);
  const [nextCursor, setNextCursor] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const inputRef = useRef<HTMLInputElement | null>(null);
  // Increment immediately on edits/navigation, not only when an effect
  // reruns: old replies cannot flash on screen during the debounce interval.
  const requestVersion = useRef(0);

  useEffect(() => {
    if (!open) {
      requestVersion.current += 1;
      return;
    }
    setQuery("");
    setSearch("");
    setResults([]);
    setPageIndex(0);
    setCursors([null]);
    setNextCursor(null);
    setError(null);
    const focus = window.setTimeout(() => inputRef.current?.focus(), 0);
    return () => {
      requestVersion.current += 1;
      window.clearTimeout(focus);
    };
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const timer = window.setTimeout(() => {
      setSearch(query.trim());
      setCursors([null]);
      setPageIndex(0);
    }, SEARCH_DEBOUNCE_MS);
    return () => window.clearTimeout(timer);
  }, [open, query]);

  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    const version = ++requestVersion.current;
    setLoading(true);
    setError(null);
    void api
      .searchConversations(search, cursors[pageIndex] ?? undefined, PAGE_SIZE)
      .then((page) => {
        if (cancelled || version !== requestVersion.current) return;
        setResults(page.conversations);
        setNextCursor(page.nextCursor);
      })
      .catch((reason: unknown) => {
        if (cancelled || version !== requestVersion.current) return;
        setResults([]);
        setNextCursor(null);
        setError(String(reason));
      })
      .finally(() => {
        if (!cancelled && version === requestVersion.current) setLoading(false);
      });
    return () => {
      cancelled = true;
      requestVersion.current += 1;
    };
  }, [open, search, pageIndex, cursors]);

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
          maxLength={256}
          onChange={(event) => {
            requestVersion.current += 1;
            setQuery(event.target.value);
            setResults([]);
            setNextCursor(null);
            setLoading(true);
            setError(null);
          }}
        />
        <div className="search-results">
          {loading ? (
            <p className="muted search-empty" role="status">Searching local chats...</p>
          ) : error ? (
            <p className="search-empty" role="alert">
              Chat search failed: {error}
            </p>
          ) : results.length === 0 ? (
            <p className="muted search-empty">No matching chats.</p>
          ) : (
            results.map((conversation) => (
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
        <div className="search-pagination" aria-label="Chat search pagination">
          <span className="muted">Page {pageIndex + 1}</span>
          <div className="button-row">
            <button
              type="button"
              className="ghost-button"
              disabled={loading || pageIndex === 0}
              onClick={() => {
                requestVersion.current += 1;
                setPageIndex((page) => Math.max(0, page - 1));
              }}
            >
              Previous
            </button>
            <button
              type="button"
              className="ghost-button"
              disabled={loading || !nextCursor}
              onClick={() => {
                if (!nextCursor) return;
                requestVersion.current += 1;
                setCursors((items) => [...items.slice(0, pageIndex + 1), nextCursor]);
                setPageIndex((page) => page + 1);
              }}
            >
              Next
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
