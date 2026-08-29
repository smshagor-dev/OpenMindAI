import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  ArrowUp,
  CheckCircle2,
  FilePlus2,
  FileText,
  Files,
  FolderKanban,
  LoaderCircle,
  MessageSquarePlus,
  MessagesSquare,
  Pencil,
  Plus,
  Search,
  Sparkles,
  Trash2,
  UploadCloud,
  X,
} from "lucide-react";
import { api } from "../api";
import type { Conversation, Project, ProjectFile } from "../types";
import { formatBytes, formatError, formatTime } from "../lib/format";
import { ConfirmDialog } from "./ConfirmDialog";
import { OpenFolderProjectButton } from "./OpenFolderProjectButton";
import { ProjectLocalWorkspace } from "./ProjectLocalWorkspace";

const MAX_PROJECT_INSTRUCTIONS_CHARS = 20_000;
const QUICK_TASKS = [
  {
    title: "Research and summarize",
    description: "Pull together the important details and give me a clear brief.",
    prompt: "Research this project and give me a concise brief with the key findings, risks, and next steps.",
    icon: Search,
  },
  {
    title: "Create a polished deliverable",
    description: "Turn the project context into something ready to use or share.",
    prompt: "Create a polished deliverable from this project context. Choose the most useful format and make it ready to use.",
    icon: FileText,
  },
  {
    title: "Analyze the project",
    description: "Review what is here, find gaps, and recommend what to do next.",
    prompt: "Analyze this project thoroughly, identify the most important gaps or issues, and recommend the next actions in priority order.",
    icon: Sparkles,
  },
  {
    title: "Fix or improve something",
    description: "Inspect the workspace, make the changes, and validate the result.",
    prompt: "Inspect this project, find the highest-impact issue or improvement, implement it, and validate the result.",
    icon: CheckCircle2,
  },
] as const;

export function ProjectsWorkspace(props: {
  conversations: Conversation[];
  onOpenConversation: (id: string, draft?: string) => void;
  onCreateProjectChat: (project: Project) => Promise<Conversation>;
}) {
  const [projects, setProjects] = useState<Project[]>([]);
  const [activeId, setActiveId] = useState<string | null>(null);
  const [workDraft, setWorkDraft] = useState("");
  const [draftName, setDraftName] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [busyAction, setBusyAction] = useState<string | null>(null);
  const [addingFiles, setAddingFiles] = useState(false);
  const [savedAt, setSavedAt] = useState<number | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<Project | null>(null);
  const [deleteFileTarget, setDeleteFileTarget] = useState<ProjectFile | null>(null);
  const fileInputRef = useRef<HTMLInputElement | null>(null);

  const refreshProjects = useCallback(async (preferredId?: string | null) => {
    const next = await api.projects();
    setProjects(next);
    setActiveId((current) => {
      const candidate = preferredId ?? current;
      if (candidate && next.some((project) => project.id === candidate)) return candidate;
      return next[0]?.id ?? null;
    });
    return next;
  }, []);

  useEffect(() => {
    let alive = true;
    void refreshProjects()
      .catch((caught) => {
        if (alive) setError(formatError(caught));
      })
      .finally(() => {
        if (alive) setLoading(false);
      });
    return () => {
      alive = false;
    };
  }, [refreshProjects]);

  const activeProject = useMemo(
    () => projects.find((project) => project.id === activeId) ?? projects[0] ?? null,
    [activeId, projects],
  );

  const projectConversations = useMemo(() => {
    if (!activeProject) return [];
    const allowed = new Set(activeProject.conversationIds);
    return props.conversations
      .filter((conversation) => allowed.has(conversation.id))
      .sort((a, b) => b.updatedAt.localeCompare(a.updatedAt));
  }, [activeProject, props.conversations]);

  const runAction = async <T,>(key: string, action: () => Promise<T>): Promise<T | undefined> => {
    if (busyAction) return undefined;
    setBusyAction(key);
    setError(null);
    try {
      return await action();
    } catch (caught) {
      setError(formatError(caught));
      return undefined;
    } finally {
      setBusyAction(null);
    }
  };

  const createProject = async () => {
    const name = draftName.trim();
    if (!name) return;
    const project = await runAction("create-project", () => api.createProject(name));
    if (!project) return;
    await refreshProjects(project.id).catch((caught) => setError(formatError(caught)));
    setDraftName("");
    setSavedAt(null);
  };

  const updateProject = async (
    projectId: string,
    patch: Partial<Pick<Project, "name" | "instructions">>,
  ) => {
    const current = projects.find((project) => project.id === projectId);
    if (!current) return;
    const nextName = patch.name ?? current.name;
    if (!nextName.trim()) {
      setError("Project name cannot be empty.");
      await refreshProjects(projectId).catch(() => undefined);
      return;
    }
    const updated = await runAction("save-project", () =>
      api.updateProject(projectId, nextName, patch.instructions ?? current.instructions),
    );
    if (!updated) {
      await refreshProjects(projectId).catch(() => undefined);
      return;
    }
    setProjects((items) => items.map((item) => (item.id === updated.id ? updated : item)));
    setSavedAt(Date.now());
  };

  const startWork = async (draft?: string) => {
    const task = draft?.trim() || workDraft.trim();
    if (!activeProject || !task || busyAction) return;
    const conversation = await runAction("start-work", async () => {
      const created = await props.onCreateProjectChat(activeProject);
      await api.linkProjectConversation(activeProject.id, created.id);
      await refreshProjects(activeProject.id);
      return created;
    });
    if (!conversation) return;
    setWorkDraft("");
    props.onOpenConversation(conversation.id, task);
  };

  const addFiles = async (files: FileList | null) => {
    if (!files || !activeProject || files.length === 0 || addingFiles) return;
    setAddingFiles(true);
    setError(null);
    const failures: string[] = [];
    try {
      for (const file of Array.from(files)) {
        try {
          const ingestion = await ingestProjectFile(file);
          await api.addProjectFile(
            activeProject.id,
            file.name,
            file.size,
            file.type || null,
            ingestion.contentText,
            ingestion.status,
            ingestion.error,
          );
        } catch (caught) {
          failures.push(`${file.name}: ${formatError(caught)}`);
        }
      }
      await refreshProjects(activeProject.id);
      if (failures.length) {
        setError(
          failures.length === 1
            ? failures[0]
            : `${failures.length} files could not be added. ${failures[0]}`,
        );
      }
    } catch (caught) {
      setError(formatError(caught));
    } finally {
      setAddingFiles(false);
    }
  };

  const deleteFile = async () => {
    if (!activeProject || !deleteFileTarget) return;
    const fileId = deleteFileTarget.id;
    const deleted = await runAction(`delete-file-${fileId}`, async () => {
      await api.deleteProjectFile(activeProject.id, fileId);
      await refreshProjects(activeProject.id);
      return true;
    });
    if (deleted) setDeleteFileTarget(null);
  };

  const deleteProject = async () => {
    if (!deleteTarget) return;
    const projectId = deleteTarget.id;
    const deleted = await runAction("delete-project", async () => {
      await api.deleteProject(projectId);
      await refreshProjects(null);
      return true;
    });
    if (deleted) {
      setSavedAt(null);
      setDeleteTarget(null);
    }
  };

  const readyFiles = activeProject?.files.filter((file) => file.status === "ready").length ?? 0;
  const busy = busyAction !== null;

  return (
    <div className="cg-work-page">
      <input
        ref={fileInputRef}
        type="file"
        multiple
        className="hidden-file-input"
        onChange={(event) => {
          void addFiles(event.currentTarget.files);
          event.currentTarget.value = "";
        }}
      />

      {error ? (
        <button className="error-banner cg-work-error" onClick={() => setError(null)}>
          <span>{error}</span>
          <X size={14} />
        </button>
      ) : null}

      <section className="cg-work-main" aria-label="Work">
        <div className="cg-work-context-bar">
          <span className="cg-work-mode-label">Work</span>
          {projects.length ? (
            <label className="cg-work-project-select">
              <FolderKanban size={14} />
              <select
                aria-label="Project context"
                value={activeProject?.id ?? ""}
                onChange={(event) => {
                  setActiveId(event.target.value || null);
                  setSavedAt(null);
                  setWorkDraft("");
                  setError(null);
                }}
              >
                {projects.map((project) => (
                  <option key={project.id} value={project.id}>
                    {project.name}
                  </option>
                ))}
              </select>
            </label>
          ) : null}
        </div>

        <div className="cg-work-hero">
          <h1>What can I help you get done?</h1>
        </div>

        {loading ? (
          <div className="cg-work-loading">
            <LoaderCircle className="spin" size={20} /> Loading your work context…
          </div>
        ) : activeProject ? (
          <>
            <form
              className="cg-work-composer"
              onSubmit={(event) => {
                event.preventDefault();
                void startWork();
              }}
            >
              <textarea
                autoFocus
                rows={3}
                value={workDraft}
                placeholder="Describe a task or deliverable"
                onChange={(event) => setWorkDraft(event.target.value)}
                onKeyDown={(event) => {
                  if ((event.ctrlKey || event.metaKey) && event.key === "Enter") {
                    event.preventDefault();
                    void startWork();
                  }
                }}
              />
              <div className="cg-work-composer-footer">
                <div className="cg-work-composer-tools">
                  <button
                    type="button"
                    className="cg-work-round-button"
                    title="Add project files"
                    aria-label="Add project files"
                    disabled={addingFiles || busy}
                    onClick={() => fileInputRef.current?.click()}
                  >
                    {addingFiles ? (
                      <LoaderCircle className="spin" size={18} />
                    ) : (
                      <Plus size={19} />
                    )}
                  </button>
                  <span className="cg-work-context-pill">
                    <FolderKanban size={14} />
                    <span>{activeProject.name}</span>
                  </span>
                  {readyFiles > 0 ? (
                    <span className="cg-work-context-pill cg-work-context-pill-muted">
                      <Files size={14} /> {readyFiles} file{readyFiles === 1 ? "" : "s"}
                    </span>
                  ) : null}
                </div>
                <button
                  type="submit"
                  className="cg-work-send"
                  aria-label="Start work"
                  title="Start work"
                  disabled={busy || !workDraft.trim()}
                >
                  {busyAction === "start-work" ? (
                    <LoaderCircle className="spin" size={18} />
                  ) : (
                    <ArrowUp size={19} />
                  )}
                </button>
              </div>
            </form>

            <div className="cg-work-shortcuts" aria-label="Suggested tasks">
              {QUICK_TASKS.map((task) => {
                const Icon = task.icon;
                return (
                  <button
                    key={task.title}
                    type="button"
                    onClick={() => setWorkDraft(task.prompt)}
                  >
                    <span className="cg-work-shortcut-icon">
                      <Icon size={17} />
                    </span>
                    <span>
                      <strong>{task.title}</strong>
                      <small>{task.description}</small>
                    </span>
                  </button>
                );
              })}
            </div>

            {projectConversations.length ? (
              <section className="cg-work-recents">
                <div className="cg-work-section-heading">
                  <h2>Recent work</h2>
                </div>
                <div className="cg-work-recent-list">
                  {projectConversations.slice(0, 6).map((conversation) => (
                    <button
                      type="button"
                      key={conversation.id}
                      onClick={() => props.onOpenConversation(conversation.id)}
                    >
                      <span className="cg-work-recent-icon">
                        <MessagesSquare size={16} />
                      </span>
                      <span className="cg-work-recent-copy">
                        <strong>{conversation.title}</strong>
                        <small>{formatTime(conversation.updatedAt)}</small>
                      </span>
                    </button>
                  ))}
                </div>
              </section>
            ) : null}
          </>
        ) : (
          <div className="cg-work-empty">
            <div className="cg-work-empty-icon">
              <FolderKanban size={24} />
            </div>
            <h2>Add a project to start working</h2>
            <p>Open a local folder or create a project, then describe the outcome you want.</p>
            <OpenFolderProjectButton
              disabled={busy}
              onCreateProjectChat={props.onCreateProjectChat}
              onCreated={async (project) => {
                await refreshProjects(project.id);
              }}
              onError={setError}
            />
          </div>
        )}
      </section>

      <details className="cg-work-manage">
        <summary>
          <span>
            <FolderKanban size={15} /> Projects and context
          </span>
          <small>Manage folders, files, instructions, and local access</small>
        </summary>
        <div className="cg-work-manage-body">
          <div className="cg-work-project-create">
            <div className="cg-work-project-create-row">
              <input
                value={draftName}
                maxLength={120}
                disabled={busy}
                placeholder="New project name"
                onChange={(event) => setDraftName(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === "Enter" && draftName.trim()) void createProject();
                }}
              />
              <button
                type="button"
                disabled={busy || !draftName.trim()}
                onClick={() => void createProject()}
              >
                {busyAction === "create-project" ? (
                  <LoaderCircle className="spin" size={15} />
                ) : (
                  <Plus size={15} />
                )}
                Create
              </button>
            </div>
            <OpenFolderProjectButton
              disabled={busy}
              onCreateProjectChat={props.onCreateProjectChat}
              onCreated={async (project) => {
                await refreshProjects(project.id);
              }}
              onError={setError}
            />
          </div>

          {activeProject ? (
            <div className="cg-work-project-settings">
              <div className="cg-work-project-settings-head">
                <div>
                  <span className="cg-work-eyebrow">Current project</span>
                  <input
                    className="cg-work-project-name"
                    value={activeProject.name}
                    maxLength={120}
                    aria-label="Project name"
                    onChange={(event) => {
                      const name = event.target.value;
                      setProjects((items) =>
                        items.map((item) =>
                          item.id === activeProject.id ? { ...item, name } : item,
                        ),
                      );
                    }}
                    onBlur={(event) =>
                      void updateProject(activeProject.id, { name: event.currentTarget.value })
                    }
                  />
                </div>
                <button
                  type="button"
                  className="cg-work-delete-project"
                  title="Delete project"
                  aria-label={`Delete ${activeProject.name}`}
                  disabled={busy}
                  onClick={() => setDeleteTarget(activeProject)}
                >
                  <Trash2 size={15} />
                </button>
              </div>

              <label className="cg-work-instructions">
                <span>
                  <Pencil size={14} /> Instructions
                  <small>
                    {busyAction === "save-project" ? "Saving…" : savedAt ? "Saved" : "Auto-save"}
                  </small>
                </span>
                <textarea
                  value={activeProject.instructions}
                  maxLength={MAX_PROJECT_INSTRUCTIONS_CHARS}
                  disabled={busyAction === "save-project"}
                  placeholder="Add private instructions for this project…"
                  onChange={(event) => {
                    const instructions = event.target.value;
                    setProjects((items) =>
                      items.map((item) =>
                        item.id === activeProject.id ? { ...item, instructions } : item,
                      ),
                    );
                  }}
                  onBlur={(event) =>
                    void updateProject(activeProject.id, {
                      instructions: event.currentTarget.value,
                    })
                  }
                />
              </label>

              <div className="cg-work-files-panel">
                <div className="cg-work-subheading">
                  <span>
                    <Files size={14} /> Files
                  </span>
                  <button
                    type="button"
                    disabled={addingFiles || busy}
                    onClick={() => fileInputRef.current?.click()}
                  >
                    {addingFiles ? (
                      <LoaderCircle className="spin" size={14} />
                    ) : (
                      <FilePlus2 size={14} />
                    )}
                    Add files
                  </button>
                </div>
                {activeProject.files.length ? (
                  <div className="cg-work-file-list">
                    {activeProject.files.map((file) => (
                      <div key={file.id} className="cg-work-file-row">
                        <span className="cg-work-file-icon">
                          <FileText size={15} />
                        </span>
                        <span className="cg-work-file-copy">
                          <strong>{file.name}</strong>
                          <small>
                            {formatBytes(file.sizeBytes)} · {formatFileStatus(file.status)}
                          </small>
                        </span>
                        <button
                          type="button"
                          title="Remove file"
                          aria-label={`Remove ${file.name}`}
                          disabled={busy}
                          onClick={() => setDeleteFileTarget(file)}
                        >
                          <X size={14} />
                        </button>
                      </div>
                    ))}
                  </div>
                ) : (
                  <button
                    type="button"
                    className="cg-work-file-empty"
                    disabled={addingFiles || busy}
                    onClick={() => fileInputRef.current?.click()}
                  >
                    <UploadCloud size={18} />
                    <span>
                      <strong>Add files</strong>
                      <small>Notes, specs, code, or other project context</small>
                    </span>
                  </button>
                )}
              </div>

              <div className="cg-work-project-chat-panel">
                <div className="cg-work-subheading">
                  <span>
                    <MessagesSquare size={14} /> Project chats
                  </span>
                  <button type="button" disabled={busy} onClick={() => void startWork("Start a new work thread for this project.")}>
                    <MessageSquarePlus size={14} /> New work
                  </button>
                </div>
                {projectConversations.length ? (
                  <div className="cg-work-project-chat-list">
                    {projectConversations.slice(0, 8).map((conversation) => (
                      <button
                        type="button"
                        key={conversation.id}
                        onClick={() => props.onOpenConversation(conversation.id)}
                      >
                        <span>{conversation.title}</span>
                        <small>{formatTime(conversation.updatedAt)}</small>
                      </button>
                    ))}
                  </div>
                ) : (
                  <p className="cg-work-muted">No work threads yet.</p>
                )}
              </div>

              <details className="cg-work-local-access">
                <summary>
                  <FolderKanban size={14} /> Local workspace access
                </summary>
                <ProjectLocalWorkspace projectId={activeProject.id} projectName={activeProject.name} />
              </details>
            </div>
          ) : null}

          {projects.length > 1 ? (
            <div className="cg-work-project-grid">
              {projects.map((project) => (
                <button
                  type="button"
                  key={project.id}
                  className={project.id === activeProject?.id ? "active" : ""}
                  onClick={() => {
                    setActiveId(project.id);
                    setSavedAt(null);
                    setWorkDraft("");
                    setError(null);
                  }}
                >
                  <FolderKanban size={16} />
                  <span>
                    <strong>{project.name}</strong>
                    <small>
                      {project.conversationIds.length} chats · {project.files.length} files
                    </small>
                  </span>
                </button>
              ))}
            </div>
          ) : null}
        </div>
      </details>

      <ConfirmDialog
        open={deleteTarget !== null}
        title={`Delete ${deleteTarget?.name ?? "project"}?`}
        description="This removes the project and its file links. Conversations are kept in chat history."
        confirmLabel="Delete project"
        danger
        onConfirm={() => void deleteProject()}
        onCancel={() => setDeleteTarget(null)}
      />
      <ConfirmDialog
        open={deleteFileTarget !== null}
        title={`Remove ${deleteFileTarget?.name ?? "file"}?`}
        description="This removes the file from project context. The original file on your device is not deleted."
        confirmLabel="Remove file"
        danger
        onConfirm={() => void deleteFile()}
        onCancel={() => setDeleteFileTarget(null)}
      />
    </div>
  );
}

type FileIngestion = {
  contentText: string | null;
  status: ProjectFile["status"];
  error: string | null;
};

const MAX_PROJECT_FILE_BYTES = 1_500_000;
const MAX_PROJECT_FILE_TEXT_CHARS = 120_000;
const TEXT_EXTENSIONS = new Set([
  "txt",
  "md",
  "markdown",
  "json",
  "jsonl",
  "csv",
  "ts",
  "tsx",
  "js",
  "jsx",
  "mjs",
  "cjs",
  "py",
  "rs",
  "go",
  "java",
  "kt",
  "swift",
  "c",
  "cpp",
  "h",
  "hpp",
  "cs",
  "php",
  "rb",
  "css",
  "scss",
  "html",
  "xml",
  "yaml",
  "yml",
  "toml",
  "sql",
  "sh",
  "bash",
  "ps1",
  "bat",
  "env",
  "log",
]);

async function ingestProjectFile(file: File): Promise<FileIngestion> {
  if (file.size > MAX_PROJECT_FILE_BYTES) {
    return {
      contentText: null,
      status: "skipped",
      error: "File is too large for chat context.",
    };
  }

  if (!isTextFile(file)) {
    return {
      contentText: null,
      status: "tracked",
      error: "Tracked locally. Text extraction is not available for this file type yet.",
    };
  }

  try {
    const text = await file.text();
    const characters = Array.from(text);
    const truncated = characters.length > MAX_PROJECT_FILE_TEXT_CHARS;
    return {
      contentText: characters.slice(0, MAX_PROJECT_FILE_TEXT_CHARS).join(""),
      status: "ready",
      error: truncated ? "Ready for context. Stored preview was truncated." : null,
    };
  } catch {
    return {
      contentText: null,
      status: "failed",
      error: "Could not read this file.",
    };
  }
}

function isTextFile(file: File) {
  if (file.type.startsWith("text/")) return true;
  const extension = file.name.split(".").pop()?.toLowerCase();
  return extension ? TEXT_EXTENSIONS.has(extension) : false;
}

function formatFileStatus(status: ProjectFile["status"]) {
  if (status === "ready") return "Ready";
  if (status === "skipped") return "Skipped";
  if (status === "failed") return "Failed";
  return "Tracked";
}
