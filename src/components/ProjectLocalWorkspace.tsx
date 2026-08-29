import { open } from "@tauri-apps/plugin-dialog";
import {
  ArrowLeft,
  CheckCircle2,
  ChevronRight,
  FileCode2,
  FilePlus2,
  Folder,
  FolderOpen,
  FolderPlus,
  HardDrive,
  LoaderCircle,
  LockKeyhole,
  PencilLine,
  Play,
  RefreshCw,
  Save,
  ShieldCheck,
  TerminalSquare,
  Trash2,
  Unplug,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import {
  localWorkspaceApi,
  type ProjectLocalAccessStatus,
  type ProjectWorkspaceRoot,
  type TerminalCommandResult,
  type WorkspaceEntry,
} from "../lib/localWorkspace";
import { formatBytes, formatError } from "../lib/format";
import { ConfirmDialog } from "./ConfirmDialog";
import "../projects-local-workspace.css";

export function ProjectLocalWorkspace(props: { projectId: string; projectName: string }) {
  const [status, setStatus] = useState<ProjectLocalAccessStatus | null>(null);
  const [activeRootId, setActiveRootId] = useState<string | null>(null);
  const [currentPath, setCurrentPath] = useState("");
  const [locationInput, setLocationInput] = useState("");
  const [entries, setEntries] = useState<WorkspaceEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [editorPath, setEditorPath] = useState<string | null>(null);
  const [editorContent, setEditorContent] = useState("");
  const [editorOriginal, setEditorOriginal] = useState("");
  const [newItemKind, setNewItemKind] = useState<"file" | "directory" | null>(null);
  const [newItemName, setNewItemName] = useState("");
  const [renameTarget, setRenameTarget] = useState<WorkspaceEntry | null>(null);
  const [renameValue, setRenameValue] = useState("");
  const [deleteTarget, setDeleteTarget] = useState<WorkspaceEntry | null>(null);
  const [detachTarget, setDetachTarget] = useState<ProjectWorkspaceRoot | null>(null);
  const [confirmFullAccess, setConfirmFullAccess] = useState(false);
  const [terminalCommand, setTerminalCommand] = useState("");
  const [terminalCwd, setTerminalCwd] = useState("");
  const [terminalHistory, setTerminalHistory] = useState<TerminalCommandResult[]>([]);

  const activeRoot = useMemo(
    () => status?.roots.find((root) => root.id === activeRootId) ?? status?.roots[0] ?? null,
    [activeRootId, status],
  );
  const editorDirty = editorPath !== null && editorContent !== editorOriginal;

  const loadStatus = useCallback(async () => {
    const next = await localWorkspaceApi.status(props.projectId);
    setStatus(next);
    setActiveRootId((current) => {
      if (current && next.roots.some((root) => root.id === current)) return current;
      return next.roots[0]?.id ?? null;
    });
    return next;
  }, [props.projectId]);

  const loadDirectory = useCallback(
    async (rootId: string | null, path: string) => {
      if (!rootId && !isAbsoluteLike(path)) {
        setEntries([]);
        return;
      }
      const next = await localWorkspaceApi.listDirectory(props.projectId, rootId, path);
      setEntries(next);
      setCurrentPath(path);
      setLocationInput(path || activeRoot?.path || "");
    },
    [activeRoot?.path, props.projectId],
  );

  useEffect(() => {
    let alive = true;
    setLoading(true);
    setError(null);
    void loadStatus()
      .then((next) => {
        if (!alive) return;
        const root = next.roots[0];
        if (root) {
          setTerminalCwd(root.path);
          return localWorkspaceApi.listDirectory(props.projectId, root.id, "").then((items) => {
            if (alive) setEntries(items);
          });
        }
        setEntries([]);
      })
      .catch((caught) => {
        if (alive) setError(formatError(caught));
      })
      .finally(() => {
        if (alive) setLoading(false);
      });
    return () => {
      alive = false;
    };
  }, [loadStatus, props.projectId]);

  useEffect(() => {
    if (!activeRoot) {
      setCurrentPath("");
      setLocationInput("");
      setEntries([]);
      return;
    }
    setCurrentPath("");
    setLocationInput(activeRoot.path);
    setTerminalCwd(activeRoot.path);
    setEditorPath(null);
    setEditorContent("");
    setEditorOriginal("");
    void loadDirectory(activeRoot.id, "").catch((caught) => setError(formatError(caught)));
  }, [activeRoot?.id]); // eslint-disable-line react-hooks/exhaustive-deps

  const run = async <T,>(key: string, action: () => Promise<T>): Promise<T | undefined> => {
    if (busy) return undefined;
    setBusy(key);
    setError(null);
    try {
      return await action();
    } catch (caught) {
      setError(formatError(caught));
      return undefined;
    } finally {
      setBusy(null);
    }
  };

  const attachFolder = async () => {
    const selected = await open({
      directory: true,
      multiple: false,
      title: `Attach a local folder to ${props.projectName}`,
    });
    if (!selected || Array.isArray(selected)) return;
    const next = await run("attach", () => localWorkspaceApi.attachFolder(props.projectId, selected));
    if (!next) return;
    setStatus(next);
    const root =
      next.roots.find((item) => sameDisplayPath(item.path, selected)) ??
      next.roots[next.roots.length - 1];
    if (root) setActiveRootId(root.id);
  };

  const detachFolder = async () => {
    if (!detachTarget) return;
    const next = await run("detach", () =>
      localWorkspaceApi.detachFolder(props.projectId, detachTarget.id),
    );
    if (!next) return;
    setStatus(next);
    setDetachTarget(null);
    setEditorPath(null);
  };

  const enableFullAccess = async () => {
    const next = await run("full-access", () =>
      localWorkspaceApi.setFullAccess(props.projectId, true, true),
    );
    if (!next) return;
    setStatus(next);
    setConfirmFullAccess(false);
  };

  const disableFullAccess = async () => {
    const next = await run("full-access", () =>
      localWorkspaceApi.setFullAccess(props.projectId, false, false),
    );
    if (next) setStatus(next);
  };

  const refreshDirectory = async () => {
    if (!activeRoot) return;
    await run("refresh", () => loadDirectory(activeRoot.id, currentPath));
  };

  const openEntry = async (entry: WorkspaceEntry) => {
    if (!activeRoot) return;
    if (entry.kind === "directory") {
      if (editorDirty && !window.confirm("Discard unsaved editor changes and open this folder?")) return;
      setEditorPath(null);
      await run("navigate", () => loadDirectory(activeRoot.id, entry.relativePath));
      return;
    }
    if (entry.kind !== "file") return;
    if (editorDirty && !window.confirm("Discard unsaved editor changes and open another file?")) return;
    const file = await run("read-file", () =>
      localWorkspaceApi.readFile(props.projectId, activeRoot.id, entry.relativePath),
    );
    if (!file) return;
    setEditorPath(entry.relativePath);
    setEditorContent(file.content);
    setEditorOriginal(file.content);
  };

  const saveEditor = async () => {
    if (!activeRoot || !editorPath || !editorDirty) return;
    const saved = await run("save-file", () =>
      localWorkspaceApi.writeFile(
        props.projectId,
        activeRoot.id,
        editorPath,
        editorContent,
        true,
      ),
    );
    if (!saved) return;
    setEditorOriginal(editorContent);
    await loadDirectory(activeRoot.id, currentPath).catch(() => undefined);
  };

  const createItem = async () => {
    if (!activeRoot || !newItemKind) return;
    const name = newItemName.trim();
    if (!name) return;
    const path = joinWorkspacePath(currentPath, name);
    const created = await run(`create-${newItemKind}`, () =>
      newItemKind === "directory"
        ? localWorkspaceApi.createDirectory(props.projectId, activeRoot.id, path, true)
        : localWorkspaceApi.writeFile(props.projectId, activeRoot.id, path, "", true),
    );
    if (!created) return;
    setNewItemKind(null);
    setNewItemName("");
    await loadDirectory(activeRoot.id, currentPath).catch(() => undefined);
    if (newItemKind === "file") {
      setEditorPath(path);
      setEditorContent("");
      setEditorOriginal("");
    }
  };

  const renameItem = async () => {
    if (!activeRoot || !renameTarget) return;
    const nextName = renameValue.trim();
    if (!nextName) return;
    const parent = parentWorkspacePath(renameTarget.relativePath);
    const targetPath = joinWorkspacePath(parent, nextName);
    const moved = await run("rename", () =>
      localWorkspaceApi.movePath(
        props.projectId,
        activeRoot.id,
        renameTarget.relativePath,
        targetPath,
        true,
      ),
    );
    if (!moved) return;
    if (editorPath === renameTarget.relativePath) setEditorPath(targetPath);
    setRenameTarget(null);
    setRenameValue("");
    await loadDirectory(activeRoot.id, currentPath).catch(() => undefined);
  };

  const deleteItem = async () => {
    if (!activeRoot || !deleteTarget) return;
    const removed = await run("delete-path", async () => {
      await localWorkspaceApi.deletePath(
        props.projectId,
        activeRoot.id,
        deleteTarget.relativePath,
        true,
      );
      return true;
    });
    if (!removed) return;
    if (editorPath === deleteTarget.relativePath) {
      setEditorPath(null);
      setEditorContent("");
      setEditorOriginal("");
    }
    setDeleteTarget(null);
    await loadDirectory(activeRoot.id, currentPath).catch(() => undefined);
  };

  const goToLocation = async () => {
    if (!activeRoot) return;
    const raw = locationInput.trim();
    if (!raw) return;
    let path = raw;
    if (!status?.fullPcAccess && sameDisplayPath(raw, activeRoot.path)) path = "";
    await run("go", () => loadDirectory(activeRoot.id, path));
  };

  const goUp = async () => {
    if (!activeRoot) return;
    if (!currentPath) return;
    const parent = parentWorkspacePath(currentPath);
    await run("navigate", () => loadDirectory(activeRoot.id, parent));
  };

  const runTerminal = async () => {
    if (!activeRoot || !status?.terminalEnabled) return;
    const command = terminalCommand.trim();
    if (!command) return;
    const result = await run("terminal", () =>
      localWorkspaceApi.runTerminal(
        props.projectId,
        activeRoot.id,
        terminalCwd || activeRoot.path,
        command,
        true,
      ),
    );
    if (!result) return;
    setTerminalHistory((items) => [...items.slice(-29), result]);
    setTerminalCwd(result.cwd);
    setTerminalCommand("");
  };

  return (
    <section className="project-card local-workspace-card">
      <div className="local-workspace-heading">
        <div>
          <span className="tools-eyebrow">Local machine</span>
          <h3><HardDrive size={16} /> Local Workspace</h3>
          <p>Attach real PC folders. OpenMindAI can browse and edit them directly; terminal access is a separate explicit grant.</p>
        </div>
        <div className="local-workspace-heading-actions">
          <button type="button" className="ghost-button" disabled={busy !== null} onClick={() => void attachFolder()}>
            {busy === "attach" ? <LoaderCircle className="spin" size={15} /> : <FolderPlus size={15} />}
            Attach folder
          </button>
          {status?.fullPcAccess ? (
            <button type="button" className="local-access-enabled" disabled={busy !== null} onClick={() => void disableFullAccess()}>
              <ShieldCheck size={15} /> Full PC access on
            </button>
          ) : (
            <button type="button" className="local-access-button" disabled={busy !== null} onClick={() => setConfirmFullAccess(true)}>
              <LockKeyhole size={15} /> Enable Full PC + Terminal
            </button>
          )}
        </div>
      </div>

      {error ? <button type="button" className="error-banner local-workspace-error" onClick={() => setError(null)}>{error}</button> : null}

      {loading ? (
        <div className="local-workspace-loading"><LoaderCircle className="spin" size={20} /> Opening local workspace…</div>
      ) : status && status.roots.length > 0 ? (
        <>
          <div className="local-root-tabs" role="tablist" aria-label="Attached project folders">
            {status.roots.map((root) => (
              <button
                type="button"
                role="tab"
                aria-selected={root.id === activeRoot?.id}
                className={root.id === activeRoot?.id ? "local-root-tab active" : "local-root-tab"}
                key={root.id}
                onClick={() => setActiveRootId(root.id)}
              >
                <FolderOpen size={15} />
                <span><strong>{root.label}</strong><small>{root.exists ? (root.writable ? "Read / write" : "Read only") : "Unavailable"}</small></span>
              </button>
            ))}
            {activeRoot ? (
              <button type="button" className="local-root-detach" title="Detach folder" onClick={() => setDetachTarget(activeRoot)}>
                <Unplug size={14} />
              </button>
            ) : null}
          </div>

          {activeRoot ? (
            <div className="local-workspace-grid">
              <div className="local-file-panel">
                <div className="local-browser-toolbar">
                  <button type="button" title="Parent folder" disabled={!currentPath || busy !== null} onClick={() => void goUp()}>
                    <ArrowLeft size={15} />
                  </button>
                  <div className="local-location-box">
                    <Folder size={14} />
                    <input
                      value={locationInput}
                      onChange={(event) => setLocationInput(event.target.value)}
                      onKeyDown={(event) => {
                        if (event.key === "Enter") void goToLocation();
                      }}
                      aria-label="Local workspace path"
                      title={status.fullPcAccess ? "Relative or absolute path" : "Attached folder path"}
                    />
                  </div>
                  <button type="button" title="Go to path" disabled={busy !== null} onClick={() => void goToLocation()}>
                    <ChevronRight size={15} />
                  </button>
                  <button type="button" title="Refresh" disabled={busy !== null} onClick={() => void refreshDirectory()}>
                    {busy === "refresh" ? <LoaderCircle className="spin" size={15} /> : <RefreshCw size={15} />}
                  </button>
                </div>

                <div className="local-browser-actions">
                  <button type="button" onClick={() => { setNewItemKind("file"); setNewItemName(""); }}><FilePlus2 size={14} /> New file</button>
                  <button type="button" onClick={() => { setNewItemKind("directory"); setNewItemName(""); }}><FolderPlus size={14} /> New folder</button>
                  {status.fullPcAccess ? <span><ShieldCheck size={13} /> Absolute paths unlocked</span> : <span><LockKeyhole size={13} /> Folder scoped</span>}
                </div>

                {newItemKind ? (
                  <div className="local-inline-form">
                    {newItemKind === "file" ? <FileCode2 size={15} /> : <Folder size={15} />}
                    <input
                      autoFocus
                      value={newItemName}
                      onChange={(event) => setNewItemName(event.target.value)}
                      onKeyDown={(event) => {
                        if (event.key === "Enter") void createItem();
                        if (event.key === "Escape") setNewItemKind(null);
                      }}
                      placeholder={newItemKind === "file" ? "filename.ts" : "folder-name"}
                    />
                    <button type="button" disabled={!newItemName.trim() || busy !== null} onClick={() => void createItem()}>Create</button>
                    <button type="button" onClick={() => setNewItemKind(null)}>Cancel</button>
                  </div>
                ) : null}

                <div className="local-entry-list">
                  {busy === "navigate" || busy === "go" ? (
                    <div className="local-entry-empty"><LoaderCircle className="spin" size={18} /> Reading folder…</div>
                  ) : entries.length ? entries.map((entry) => (
                    <div className="local-entry-row" key={`${entry.kind}-${entry.path}`}>
                      <button type="button" className="local-entry-open" onClick={() => void openEntry(entry)}>
                        {entry.kind === "directory" ? <Folder size={16} /> : <FileCode2 size={16} />}
                        <span><strong>{entry.name}</strong><small>{entry.kind === "file" && entry.sizeBytes !== null ? formatBytes(entry.sizeBytes) : entry.kind}</small></span>
                      </button>
                      <button
                        type="button"
                        title="Rename"
                        onClick={() => {
                          setRenameTarget(entry);
                          setRenameValue(entry.name);
                        }}
                      >
                        <PencilLine size={14} />
                      </button>
                      <button type="button" title="Delete" className="local-entry-delete" onClick={() => setDeleteTarget(entry)}>
                        <Trash2 size={14} />
                      </button>
                    </div>
                  )) : (
                    <div className="local-entry-empty"><FolderOpen size={19} /> This folder is empty.</div>
                  )}
                </div>
              </div>

              <div className="local-editor-panel">
                <div className="local-editor-header">
                  <div>
                    <span className="tools-eyebrow">Editor</span>
                    <strong>{editorPath ?? "Select a text or code file"}</strong>
                  </div>
                  <button type="button" disabled={!editorDirty || busy !== null} onClick={() => void saveEditor()}>
                    {busy === "save-file" ? <LoaderCircle className="spin" size={14} /> : editorDirty ? <Save size={14} /> : <CheckCircle2 size={14} />}
                    {editorDirty ? "Save" : "Saved"}
                  </button>
                </div>
                {editorPath ? (
                  <textarea
                    className="local-code-editor"
                    value={editorContent}
                    spellCheck={false}
                    onChange={(event) => setEditorContent(event.target.value)}
                  />
                ) : (
                  <div className="local-editor-empty"><FileCode2 size={25} /><span>Open a UTF-8 text file to edit it directly on your PC.</span></div>
                )}
              </div>
            </div>
          ) : null}

          <div className={status.fullPcAccess ? "local-terminal unlocked" : "local-terminal locked"}>
            <div className="local-terminal-heading">
              <div>
                <TerminalSquare size={17} />
                <span><strong>Project Terminal</strong><small>{status.fullPcAccess ? terminalCwd || activeRoot?.path : "Full PC access is disabled"}</small></span>
              </div>
              {status.fullPcAccess ? <span className="terminal-access-pill"><ShieldCheck size={12} /> OS permissions</span> : <span className="terminal-access-pill locked"><LockKeyhole size={12} /> Locked</span>}
            </div>

            {status.fullPcAccess ? (
              <>
                <div className="local-terminal-output" aria-live="polite">
                  {terminalHistory.length ? terminalHistory.map((item, index) => (
                    <div className="terminal-result" key={`${item.command}-${index}`}>
                      <div className="terminal-command-line"><span>$</span> {item.command}</div>
                      {item.stdout ? <pre>{item.stdout}</pre> : null}
                      {item.stderr ? <pre className="terminal-stderr">{item.stderr}</pre> : null}
                      <small>exit {item.exitCode} · {item.durationMs} ms{item.truncated ? " · output truncated" : ""}</small>
                    </div>
                  )) : <div className="terminal-empty">Terminal commands run with the current OS user permissions.</div>}
                </div>
                <div className="local-terminal-input">
                  <span>&gt;</span>
                  <input
                    value={terminalCommand}
                    onChange={(event) => setTerminalCommand(event.target.value)}
                    onKeyDown={(event) => {
                      if (event.key === "Enter" && !event.shiftKey) void runTerminal();
                    }}
                    placeholder="Enter PowerShell / shell command"
                    aria-label="Project terminal command"
                  />
                  <button type="button" disabled={!terminalCommand.trim() || busy !== null} onClick={() => void runTerminal()}>
                    {busy === "terminal" ? <LoaderCircle className="spin" size={14} /> : <Play size={14} />} Run
                  </button>
                </div>
              </>
            ) : (
              <div className="local-terminal-locked-copy">
                <LockKeyhole size={21} />
                <div><strong>Terminal is intentionally locked by default.</strong><span>Enable Full PC + Terminal access to run arbitrary local commands with your user account permissions.</span></div>
              </div>
            )}
          </div>
        </>
      ) : (
        <div className="local-workspace-empty">
          <div className="local-workspace-empty-icon"><FolderPlus size={26} /></div>
          <div>
            <strong>Attach a real project folder</strong>
            <span>Files stay in their original location. OpenMindAI gets direct read/write access to the folder you select.</span>
          </div>
          <button type="button" className="primary-button" onClick={() => void attachFolder()}><FolderPlus size={15} /> Choose folder</button>
        </div>
      )}

      {renameTarget ? (
        <div className="local-modal-backdrop" role="presentation" onMouseDown={() => setRenameTarget(null)}>
          <div className="local-mini-modal" role="dialog" aria-modal="true" aria-label="Rename local item" onMouseDown={(event) => event.stopPropagation()}>
            <h4>Rename {renameTarget.kind}</h4>
            <input autoFocus value={renameValue} onChange={(event) => setRenameValue(event.target.value)} onKeyDown={(event) => { if (event.key === "Enter") void renameItem(); }} />
            <div><button type="button" onClick={() => setRenameTarget(null)}>Cancel</button><button type="button" className="primary-button" disabled={!renameValue.trim() || busy !== null} onClick={() => void renameItem()}>Rename</button></div>
          </div>
        </div>
      ) : null}

      <ConfirmDialog
        open={confirmFullAccess}
        title="Enable Full PC + Terminal access?"
        description="This removes the attached-folder boundary for local path APIs and unlocks arbitrary PowerShell/shell commands. Commands run with the same operating-system permissions as OpenMindAI. Only enable this for a project you trust."
        confirmLabel="Enable full access"
        danger
        onConfirm={() => void enableFullAccess()}
        onCancel={() => setConfirmFullAccess(false)}
      />
      <ConfirmDialog
        open={detachTarget !== null}
        title={`Detach ${detachTarget?.label ?? "folder"}?`}
        description="OpenMindAI will stop using this folder. No files on your PC will be deleted."
        confirmLabel="Detach folder"
        onConfirm={() => void detachFolder()}
        onCancel={() => setDetachTarget(null)}
      />
      <ConfirmDialog
        open={deleteTarget !== null}
        title={`Delete ${deleteTarget?.name ?? "item"} from your PC?`}
        description={deleteTarget?.kind === "directory" ? "This permanently removes the folder and everything inside it from your local disk." : "This permanently removes the file from your local disk."}
        confirmLabel="Delete from PC"
        danger
        onConfirm={() => void deleteItem()}
        onCancel={() => setDeleteTarget(null)}
      />
    </section>
  );
}

function joinWorkspacePath(base: string, name: string) {
  if (!base) return name;
  const separator = base.includes("\\") && !base.includes("/") ? "\\" : "/";
  return `${base.replace(/[\\/]$/, "")}${separator}${name}`;
}

function parentWorkspacePath(path: string) {
  if (!path) return "";
  const normalized = path.replace(/[\\/]$/, "");
  if (/^[A-Za-z]:$/.test(normalized)) return `${normalized}\\`;
  const lastSlash = Math.max(normalized.lastIndexOf("/"), normalized.lastIndexOf("\\"));
  if (lastSlash < 0) return "";
  if (lastSlash === 0) return normalized.slice(0, 1);
  const parent = normalized.slice(0, lastSlash);
  return /^[A-Za-z]:$/.test(parent) ? `${parent}\\` : parent;
}

function isAbsoluteLike(path: string) {
  return path.startsWith("/") || path.startsWith("\\\\") || /^[A-Za-z]:[\\/]/.test(path);
}

function sameDisplayPath(left: string, right: string) {
  const normalize = (value: string) => value.replace(/[\\/]+$/, "").toLowerCase();
  return normalize(left) === normalize(right);
}
