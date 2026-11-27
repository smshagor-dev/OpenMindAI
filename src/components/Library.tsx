import { useEffect, useRef, useState } from "react";
import { Folder } from "lucide-react";
import type { ArtifactKind } from "../types";
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
  { id: "image", label: "Images" },
  { id: "audio", label: "Audio" },
  { id: "video", label: "Videos" },
];

const PAGE_SIZE = 50;
type LibraryPage = Awaited<ReturnType<typeof api.listLibraryEntries>>;

export function Library(props: {
  open: boolean;
  onClose: () => void;
  onOpenConversation: (id: string) => void;
}) {
  const [filter, setFilter] = useState<"all" | ArtifactKind>("all");
  const [searchInput, setSearchInput] = useState("");
  const [search, setSearch] = useState("");
  const [cursors, setCursors] = useState<(string | null)[]>([null]);
  const [pageIndex, setPageIndex] = useState(0);
  const [pageState, setPageState] = useState<{ key: string; page: LibraryPage } | null>(null);
  const [errorState, setErrorState] = useState<{ key: string; message: string } | null>(null);
  const searchTimer = useRef<number | null>(null);
  const queryKey = JSON.stringify([props.open, filter, search, cursors[pageIndex] ?? null, pageIndex]);
  const currentPage = pageState?.key === queryKey ? pageState.page : null;
  const entries = currentPage?.entries ?? [];
  const nextCursor = currentPage?.nextCursor ?? null;
  const totalCount = currentPage?.totalCount ?? 0;
  const error = errorState?.key === queryKey ? errorState.message : null;
  const loading = props.open && currentPage === null && error === null;

  useEffect(() => {
    return () => {
      if (searchTimer.current !== null) window.clearTimeout(searchTimer.current);
    };
  }, []);

  const updateSearchInput = (value: string) => {
    setSearchInput(value);
    if (searchTimer.current !== null) window.clearTimeout(searchTimer.current);
    searchTimer.current = window.setTimeout(() => {
      setSearch(value.trim());
      setCursors([null]);
      setPageIndex(0);
      searchTimer.current = null;
    }, 250);
  };

  useEffect(() => {
    if (!props.open) return;
    let cancelled = false;
    void api
      .listLibraryEntries({
        kind: filter === "all" ? undefined : filter,
        search: search || undefined,
        cursor: cursors[pageIndex] ?? undefined,
        limit: PAGE_SIZE,
      })
      .then((page) => {
        if (cancelled) return;
        setPageState({ key: queryKey, page });
      })
      .catch((reason: unknown) => {
        if (cancelled) return;
        setErrorState({ key: queryKey, message: String(reason) });
      });
    return () => {
      cancelled = true;
    };
  }, [props.open, filter, search, pageIndex, cursors, queryKey]);

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
              onClick={() => {
                setFilter(item.id);
                setPageIndex(0);
                setCursors([null]);
              }}
            >
              {item.label}
            </button>
          ))}
        </div>
        <label className="library-search">
          <span>Search generated files or source chats</span>
          <input
            type="search"
            value={searchInput}
            maxLength={256}
            placeholder="Search all saved files..."
            onChange={(event) => updateSearchInput(event.target.value)}
          />
        </label>
        <div className="library-list">
          {loading ? <p className="muted library-empty">Loading...</p> : null}
          {error ? <p className="library-empty" role="alert">Library could not load: {error}</p> : null}
          {!loading && !error && entries.length === 0 ? (
            <p className="muted library-empty">No generated files match your filters.</p>
          ) : null}
          {entries.map((entry) => (
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
        <div className="library-pagination" aria-label="Library pagination">
          <span className="muted">
            {totalCount} matching file{totalCount === 1 ? "" : "s"} · Page {pageIndex + 1}
          </span>
          <div className="button-row">
            <button
              type="button"
              className="ghost-button"
              disabled={loading || pageIndex === 0}
              onClick={() => setPageIndex((page) => Math.max(0, page - 1))}
            >
              Previous
            </button>
            <button
              type="button"
              className="ghost-button"
              disabled={loading || !nextCursor}
              onClick={() => {
                if (!nextCursor) return;
                setCursors((items) => [...items.slice(0, pageIndex + 1), nextCursor]);
                setPageIndex((page) => page + 1);
              }}
            >
              Next
            </button>
          </div>
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
