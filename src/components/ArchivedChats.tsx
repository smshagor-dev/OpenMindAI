import { useEffect, useRef, useState } from "react";
import { ArchiveRestore } from "lucide-react";
import { api } from "../api";
import type { Conversation } from "../types";

// The backend enforces an upper bound of 200 rows. A future cursor-backed
// archive endpoint is needed for collections exceeding that limit.
const ARCHIVE_LIMIT = 200;

export function ArchivedChats(props: {
  open: boolean;
  onClose: () => void;
  onRestore: (conversation: Conversation) => Promise<void>;
}) {
  const { open, onClose, onRestore } = props;
  const [rows, setRows] = useState<Conversation[]>([]);
  const [filter, setFilter] = useState("");
  const [loading, setLoading] = useState(false);
  const [restoringId, setRestoringId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const version = useRef(0);

  useEffect(() => {
    if (!open) {
      version.current += 1;
      return;
    }
    const ticket = ++version.current;
    setRows([]);
    setFilter("");
    setError(null);
    setLoading(true);
    void api.listArchivedConversations(ARCHIVE_LIMIT)
      .then((conversations) => {
        if (ticket === version.current) setRows(conversations);
      })
      .catch((reason: unknown) => {
        if (ticket === version.current) setError(String(reason));
      })
      .finally(() => {
        if (ticket === version.current) setLoading(false);
      });
    return () => { version.current += 1; };
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !restoringId) onClose();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [open, onClose, restoringId]);

  if (!open) return null;
  const visible = rows.filter((item) =>
    item.title.toLocaleLowerCase().includes(filter.trim().toLocaleLowerCase())
  );

  const restore = async (conversation: Conversation) => {
    if (restoringId) return;
    setRestoringId(conversation.id);
    setError(null);
    try {
      await onRestore(conversation);
      setRows((items) => items.filter((item) => item.id !== conversation.id));
    } catch (reason) {
      setError(String(reason));
    } finally {
      setRestoringId(null);
    }
  };

  return (
    <div className="modal-overlay" role="presentation" onClick={() => { if (!restoringId) onClose(); }}>
      <div
        className="modal-card search-card"
        role="dialog"
        aria-modal="true"
        aria-label="Archived chats"
        onClick={(event) => event.stopPropagation()}
      >
        <h2>Archived chats</h2>
        <p className="muted">Archive hides a chat without deleting its messages or files. Restore to return it to your chat list.</p>
        <input
          className="search-input"
          aria-label="Filter archived chat titles"
          placeholder="Filter archived chat titles..."
          value={filter}
          onChange={(event) => setFilter(event.target.value)}
        />
        <div className="search-results">
          {loading ? <p className="muted search-empty" role="status">Loading archived chats...</p>
          : visible.length === 0 ? <p className="muted search-empty">No archived chats in this view.</p>
          : visible.map((item) => (
            <div className="search-result" key={item.id}>
              <span>{item.title}</span>
              <button
                type="button"
                className="ghost-button"
                disabled={restoringId !== null}
                onClick={() => void restore(item)}
                aria-label={`Restore ${item.title}`}
              >
                <ArchiveRestore size={15} /> Restore
              </button>
            </div>
          ))}
        </div>
        {rows.length >= ARCHIVE_LIMIT ? (
          <p className="muted">Showing the newest {ARCHIVE_LIMIT} archived chats. Older items require archive pagination.</p>
        ) : null}
        {error ? <p role="alert">{error}</p> : null}
        <div className="modal-actions">
          <button type="button" className="ghost-button" disabled={restoringId !== null} onClick={onClose}>
            Close
          </button>
        </div>
      </div>
    </div>
  );
}
