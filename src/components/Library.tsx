import { useEffect, useMemo, useState } from "react";
import { Folder } from "lucide-react";
import type { ArtifactKind, LibraryEntry } from "../types";
import { api } from "../api";
import { formatBytes } from "../lib/format";
import { KindIcon } from "./ArtifactCard";

const FILTERS: { id: "all" | ArtifactKind; label: string }[] = [
  { id: "all", label: "All" },
  { id: "text", label: "Text" },
  { id: "markdown", label: "Markdown" },
  { id: "pdf", label: "PDF" },
  { id: "docx", label: "Documents" },
  { id: "code", label: "Code" },
];

export function Library(props: {
  open: boolean;
  onClose: () => void;
  onOpenConversation: (id: string) => void;
}) {
  const [entries, setEntries] = useState<LibraryEntry[]>([]);
  const [filter, setFilter] = useState<"all" | ArtifactKind>("all");
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    if (!props.open) return;
    let cancelled = false;
    setLoading(true);
    api
      .listLibraryEntries()
      .then((items) => {
        if (!cancelled) setEntries(items);
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
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

  const filtered = useMemo(
    () => (filter === "all" ? entries : entries.filter((entry) => entry.kind === filter)),
    [entries, filter],
  );

  if (!props.open) return null;

  return (
    <div className="modal-overlay" role="presentation" onClick={props.onClose}>
      <div
        className="modal-card library-card"
        role="dialog"
        aria-modal="true"
        aria-label="Library"
        onClick={(event) => event.stopPropagation()}
      >
        <h2>Library</h2>
        <div className="library-filters">
          {FILTERS.map((item) => (
            <button
              type="button"
              key={item.id}
              className={filter === item.id ? "library-filter active" : "library-filter"}
              onClick={() => setFilter(item.id)}
            >
              {item.label}
            </button>
          ))}
        </div>
        <div className="library-list">
          {loading ? <p className="muted library-empty">Loading...</p> : null}
          {!loading && filtered.length === 0 ? (
            <p className="muted library-empty">No generated files yet.</p>
          ) : null}
          {filtered.map((entry) => (
            <div className="library-item" key={entry.id}>
              <div className="artifact-icon">
                <KindIcon kind={entry.kind} />
              </div>
              <div className="artifact-body">
                <strong>{entry.name}</strong>
                <span className="muted">
                  {entry.status === "ready" ? formatBytes(entry.sizeBytes) : entry.status}
                  {entry.pageCount ? ` · ${entry.pageCount} page${entry.pageCount === 1 ? "" : "s"}` : ""}
                </span>
                <button
                  type="button"
                  className="library-source"
                  onClick={() => {
                    props.onOpenConversation(entry.conversationId);
                    props.onClose();
                  }}
                >
                  from {entry.conversationTitle}
                </button>
              </div>
              <div className="artifact-actions">
                {entry.status === "ready" ? (
                  <>
                    <button type="button" onClick={() => void api.openArtifact(entry.id)}>
                      Open
                    </button>
                    <button
                      type="button"
                      title="Reveal in folder"
                      onClick={() => void api.revealArtifactInFolder(entry.id)}
                    >
                      <Folder size={15} />
                    </button>
                  </>
                ) : null}
              </div>
            </div>
          ))}
        </div>
        <div className="modal-actions">
          <button type="button" className="ghost-button" onClick={props.onClose}>
            Close
          </button>
        </div>
      </div>
    </div>
  );
}
