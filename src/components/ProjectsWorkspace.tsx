import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Bot,
  BriefcaseBusiness,
  CheckCircle2,
  Clock3,
  FilePlus2,
  Files,
  FolderKanban,
  Link2,
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
import { ProjectLocalWorkspace } from "./ProjectLocalWorkspace";
import { OpenFolderProjectButton } from "./OpenFolderProjectButton";

const MAX_PROJECT_INSTRUCTIONS_CHARS = 20_000;

export function ProjectsWorkspace(props: {
  conversations: Conversation[];
  onOpenConversation: (id: string, draft?: string) => void;
  onCreateProjectChat: (project: Project) => Promise<Conversation>;
}) {
  const [projects, setProjects] = useState<Project[]>([]);
  const [activeId, setActiveId] = useState<string | null>(null);
  const [activePane, setActivePane] = useState<"overview" | "work">("overview");
  const [workDraft, setWorkDraft] = useState("");
  const [draftName, setDraftName] = useState("");
  const [linkSearch, setLinkSearch] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [busyAction, setBusyAction] = useState<string | null>(null);
  const [addingFiles, setAddingFiles] = useState(false);
  const [draggingFiles, setDraggingFiles] = useState(false);
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
    return props.conversations.filter((conversation) => allowed.has(conversation.id));
  }, [activeProject, props.conversations]);

  const conversationOwner = useMemo(() => {
    const owners = new Map<string, string>();
    for (const project of projects) {
      for (const conversationId of project.conversationIds) {
        if (!owners.has(conversationId)) owners.set(conversationId, project.name);
      }
    }
    return owners;
  }, [projects]);

  const linkableConversations = useMemo(() => {
    if (!activeProject) return [];
    const linked = new Set(activeProject.conversationIds);
    const query = linkSearch.trim().toLocaleLowerCase();
    return props.conversations
      .filter((conversation) => !linked.has(conversation.id))
      .filter((conversation) => !query || conversation.title.toLocaleLowerCase().includes(query))
      .slice(0, 50);
  }, [activeProject, linkSearch, props.conversations]);

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
    setSavedAt(null);
    setDraftName("");
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

  const createChat = async (draft?: string) => {
    if (!activeProject) return;
    const conversation = await runAction("create-chat", async () => {
      const created = await props.onCreateProjectChat(activeProject);
      await api.linkProjectConversation(activeProject.id, created.id);
      await refreshProjects(activeProject.id);
      return created;
    });
    if (conversation) props.onOpenConversation(conversation.id, draft);
    return conversation;
  };

  const openAgentChat = async (draft?: string) => {
    const existing = projectConversations[0];
    if (existing) {
      props.onOpenConversation(existing.id, draft);
      return;
    }
    await createChat(draft);
  };

  const linkConversation = async (conversationId: string) => {
    if (!activeProject) return;
    const linked = await runAction(`link-${conversationId}`, async () => {
      await api.linkProjectConversation(activeProject.id, conversationId);
      await refreshProjects(activeProject.id);
      return true;
    });
    if (linked) setLinkSearch("");
  };

  const unlinkConversation = async (conversationId: string) => {
    if (!activeProject) return;
    await runAction(`unlink-${conversationId}`, async () => {
      await api.unlinkProjectConversation(activeProject.id, conversationId);
      await refreshProjects(activeProject.id);
    });
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
      setDraggingFiles(false);
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
  const contextReady = Boolean(activeProject?.instructions.trim()) || readyFiles > 0;
  const busy = busyAction !== null;

  return (
    <div className="projects-workspace projects-workspace-v2">
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

      <aside className="projects-list-panel projects-sidebar-v2" aria-label="Projects">
        <div className="projects-sidebar-heading">
          <div>
            <span className="projects-kicker">Workspace</span>
            <h2>Projects</h2>
          </div>
          <span className="projects-count-badge">{projects.length}</span>
        </div>
        <p className="projects-sidebar-copy">
          Keep chats, working files, and instructions together in one focused space.
        </p>

        <div className="projects-create-row projects-create-row-v2">
          <input
            value={draftName}
            maxLength={120}
            disabled={busy}
            onChange={(event) => setDraftName(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter" && draftName.trim()) void createProject();
            }}
            placeholder="New project name"
            aria-label="New project name"
          />
          <button
            type="button"
            title="Create project"
            aria-label="Create project"
            disabled={busy || !draftName.trim()}
            onClick={() => void createProject()}
          >
            {busyAction === "create-project" ? (
              <LoaderCircle className="spin" size={16} />
            ) : (
              <Plus size={16} />
            )}
          </button>
        </div>

        <OpenFolderProjectButton
          disabled={busy}
          onCreateProjectChat={props.onCreateProjectChat}
          onCreated={async (project) => {
            await refreshProjects(project.id);
            setActivePane("work");
          }}
          onError={setError}
        />

        <div className="projects-list">
          {loading ? (
            <div className="projects-loading" aria-live="polite">
              <LoaderCircle className="spin" size={18} /> Loading projects…
            </div>
          ) : projects.length === 0 ? (
            <div className="projects-sidebar-empty">
              <FolderKanban size={20} />
              <span>Your projects will appear here.</span>
            </div>
          ) : (
            projects.map((project) => (
              <button
                type="button"
                key={project.id}
                className={
                  project.id === activeProject?.id
                    ? "project-list-item project-list-item-v2 active"
                    : "project-list-item project-list-item-v2"
                }
                onClick={() => {
                  setActiveId(project.id);
                  setLinkSearch("");
                  setSavedAt(null);
                  setActivePane("overview");
                  setWorkDraft("");
                  setError(null);
                }}
              >
                <span className="project-list-icon">
                  <FolderKanban size={16} />
                </span>
                <span className="project-list-copy">
                  <strong>{project.name}</strong>
                  <small>
                    {project.conversationIds.length} chats · {project.files.length} files
                  </small>
                </span>
                <small className="project-list-time">{formatTime(project.updatedAt)}</small>
              </button>
            ))
          )}
        </div>
      </aside>

      <section className="project-detail project-detail-v2">
        {error ? (
          <button className="error-banner project-error-banner" onClick={() => setError(null)}>
            <span>{error}</span>
            <X size={14} />
          </button>
        ) : null}

        {loading ? (
          <div className="project-empty project-loading-detail">
            <LoaderCircle className="spin" size={30} />
            <h2>Loading your workspace</h2>
            <p className="muted">Opening local project data…</p>
          </div>
        ) : activeProject ? (
          <>
            <header className="project-hero">
              <div className="project-hero-main">
                <div className="project-hero-icon">
                  <FolderKanban size={23} />
                </div>
                <div className="project-hero-copy">
                  <span className="tools-eyebrow">Project workspace</span>
                  <input
                    className="project-title-input project-title-input-v2"
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
                  <div className="project-meta-row">
                    <span>
                      <MessagesSquare size={13} /> {projectConversations.length} chats
                    </span>
                    <span>
                      <Files size={13} /> {activeProject.files.length} files
                    </span>
                    <span className={contextReady ? "project-meta-ready" : ""}>
                      {contextReady ? <CheckCircle2 size={13} /> : <Sparkles size={13} />}
                      {contextReady ? "Context ready" : "Add context"}
                    </span>
                    <span>
                      <Clock3 size={13} /> {formatTime(activeProject.updatedAt)}
                    </span>
                  </div>
                </div>
              </div>

              <div className="project-hero-actions">
                <button
                  type="button"
                  className="primary-button project-primary-action"
                  disabled={busy}
                  onClick={() => void createChat()}
                >
                  {busyAction === "create-chat" ? (
                    <LoaderCircle className="spin" size={16} />
                  ) : (
                    <MessageSquarePlus size={16} />
                  )}
                  New chat
                </button>
                <button
                  type="button"
                  className="ghost-button project-secondary-action"
                  disabled={addingFiles || busy}
                  onClick={() => fileInputRef.current?.click()}
                >
                  {addingFiles ? (
                    <LoaderCircle className="spin" size={16} />
                  ) : (
                    <FilePlus2 size={16} />
                  )}
                  {addingFiles ? "Adding…" : "Add files"}
                </button>
                <button
                  type="button"
                  className="project-icon-danger"
                  aria-label={`Delete ${activeProject.name}`}
                  title="Delete project"
                  disabled={busy}
                  onClick={() => setDeleteTarget(activeProject)}
                >
                  <Trash2 size={16} />
                </button>
              </div>
            </header>

            <div className="project-content-tabs" role="tablist" aria-label="Project content">
              <button
                type="button"
                role="tab"
                aria-selected={activePane === "overview"}
                className={activePane === "overview" ? "active" : ""}
                onClick={() => setActivePane("overview")}
              >
                <FolderKanban size={15} /> Overview
              </button>
              <button
                type="button"
                role="tab"
                aria-selected={activePane === "work"}
                className={activePane === "work" ? "active" : ""}
                onClick={() => setActivePane("work")}
              >
                <BriefcaseBusiness size={15} /> Work
              </button>
            </div>

            {activePane === "overview" ? (
              <>
                <section className="project-card project-instructions-card">
                  <div className="project-card-heading">
                    <div>
                      <h3>
                        <Pencil size={15} /> Project instructions
                      </h3>
                      <p>
                        Applied privately to every chat in this project together with ready text
                        files.
                      </p>
                    </div>
                    <span className="project-save-state">
                      {busyAction === "save-project" ? (
                        <>
                          <LoaderCircle className="spin" size={13} /> Saving…
                        </>
                      ) : savedAt ? (
                        <>
                          <CheckCircle2 size={13} /> Saved
                        </>
                      ) : (
                        "Auto-save on blur"
                      )}
                    </span>
                  </div>
                  <textarea
                    value={activeProject.instructions}
                    maxLength={MAX_PROJECT_INSTRUCTIONS_CHARS}
                    disabled={busyAction === "save-project"}
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
                    placeholder="Example: Keep answers concise, use this project's terminology, and treat attached specs as the source of truth."
                  />
                  <div className="project-instructions-footer">
                    <span>
                      {activeProject.instructions.length.toLocaleString()} /{" "}
                      {MAX_PROJECT_INSTRUCTIONS_CHARS.toLocaleString()}
                    </span>
                    <span>
                      {readyFiles
                        ? `${readyFiles} ready file${readyFiles === 1 ? "" : "s"} also included`
                        : "No file context yet"}
                    </span>
                  </div>
                </section>

                <div className="project-columns project-columns-v2">
                  <section className="project-card project-resource-card">
                    <div className="project-card-heading project-resource-heading">
                      <div>
                        <h3>
                          <MessagesSquare size={15} /> Chats{" "}
                          <span className="project-section-count">
                            {projectConversations.length}
                          </span>
                        </h3>
                        <p>Conversations here use this project's context automatically.</p>
                      </div>
                    </div>

                    <div className="project-resource-list">
                      {projectConversations.length ? (
                        projectConversations.map((conversation) => (
                          <div
                            className="project-chat-row project-chat-row-v2"
                            key={conversation.id}
                          >
                            <button
                              type="button"
                              onClick={() => props.onOpenConversation(conversation.id)}
                            >
                              <span>{conversation.title}</span>
                              <small>{formatTime(conversation.updatedAt)}</small>
                            </button>
                            <button
                              type="button"
                              title="Remove from project"
                              aria-label={`Remove ${conversation.title} from project`}
                              disabled={busy}
                              onClick={() => void unlinkConversation(conversation.id)}
                            >
                              {busyAction === `unlink-${conversation.id}` ? (
                                <LoaderCircle className="spin" size={14} />
                              ) : (
                                <X size={14} />
                              )}
                            </button>
                          </div>
                        ))
                      ) : (
                        <div className="project-inline-empty">
                          <MessageSquarePlus size={20} />
                          <div>
                            <strong>No project chats yet</strong>
                            <span>Start a new chat or move an existing one here.</span>
                          </div>
                        </div>
                      )}
                    </div>

                    {props.conversations.length > projectConversations.length ? (
                      <details className="project-link-menu project-link-menu-v2">
                        <summary>
                          <Link2 size={14} /> Add or move existing chat
                        </summary>
                        <div className="project-link-popover">
                          <label className="project-link-search">
                            <Search size={14} />
                            <input
                              value={linkSearch}
                              onChange={(event) => setLinkSearch(event.target.value)}
                              placeholder="Search chats"
                              aria-label="Search chats to add to project"
                            />
                          </label>
                          <div className="project-link-results">
                            {linkableConversations.length ? (
                              linkableConversations.map((conversation) => {
                                const owner = conversationOwner.get(conversation.id);
                                return (
                                  <button
                                    type="button"
                                    key={conversation.id}
                                    disabled={busy}
                                    onClick={() => void linkConversation(conversation.id)}
                                  >
                                    <span>
                                      <strong>{conversation.title}</strong>
                                      <small>
                                        {owner
                                          ? `Move from ${owner}`
                                          : `Updated ${formatTime(conversation.updatedAt)}`}
                                      </small>
                                    </span>
                                    {busyAction === `link-${conversation.id}` ? (
                                      <LoaderCircle className="spin" size={14} />
                                    ) : (
                                      <Plus size={14} />
                                    )}
                                  </button>
                                );
                              })
                            ) : (
                              <p className="muted project-link-empty">No matching chats.</p>
                            )}
                          </div>
                        </div>
                      </details>
                    ) : null}
                  </section>

                  <section className="project-card project-resource-card">
                    <div className="project-card-heading project-resource-heading">
                      <div>
                        <h3>
                          <Files size={15} /> Files{" "}
                          <span className="project-section-count">
                            {activeProject.files.length}
                          </span>
                        </h3>
                        <p>
                          Text and code files become bounded local context. Other files stay tracked
                          locally.
                        </p>
                      </div>
                    </div>

                    <button
                      type="button"
                      className={draggingFiles ? "project-drop-zone dragging" : "project-drop-zone"}
                      disabled={addingFiles || busy}
                      onClick={() => fileInputRef.current?.click()}
                      onDragEnter={(event) => {
                        event.preventDefault();
                        setDraggingFiles(true);
                      }}
                      onDragOver={(event) => {
                        event.preventDefault();
                        setDraggingFiles(true);
                      }}
                      onDragLeave={(event) => {
                        event.preventDefault();
                        if (event.currentTarget === event.target) setDraggingFiles(false);
                      }}
                      onDrop={(event) => {
                        event.preventDefault();
                        setDraggingFiles(false);
                        void addFiles(event.dataTransfer.files);
                      }}
                    >
                      {addingFiles ? (
                        <LoaderCircle className="spin" size={20} />
                      ) : (
                        <UploadCloud size={20} />
                      )}
                      <span>
                        <strong>{addingFiles ? "Adding files…" : "Drop files here"}</strong>
                        <small>or choose files from your device</small>
                      </span>
                    </button>

                    <div className="project-resource-list project-file-list-v2">
                      {activeProject.files.length ? (
                        activeProject.files.map((file) => (
                          <div className="project-file-row project-file-row-v2" key={file.id}>
                            <div>
                              <span>{file.name}</span>
                              <small>
                                {formatBytes(file.sizeBytes)}
                                {file.mimeType ? ` · ${file.mimeType}` : ""}
                              </small>
                              {file.error ? (
                                <small className="project-file-error">{file.error}</small>
                              ) : null}
                            </div>
                            <span
                              className={`project-file-status project-file-status-${file.status}`}
                            >
                              {formatFileStatus(file.status)}
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
                        ))
                      ) : (
                        <div className="project-inline-empty project-files-empty">
                          <Files size={20} />
                          <div>
                            <strong>No files yet</strong>
                            <span>Add notes, specs, code, or reference material.</span>
                          </div>
                        </div>
                      )}
                    </div>
                  </section>
                </div>
              </>
            ) : (
              <div className="project-work-pane project-work-chatgpt">
                <section className="project-work-start">
                  <div className="project-work-start-copy">
                    <span className="project-work-orb">
                      <Bot size={22} />
                    </span>
                    <span className="tools-eyebrow">Project Work</span>
                    <h2>What should I work on?</h2>
                    <p>
                      Describe the outcome. OpenMindAI can inspect the attached project, edit files,
                      run permitted commands, recover from errors, and validate its changes.
                    </p>
                  </div>

                  <form
                    className="project-work-composer"
                    onSubmit={(event) => {
                      event.preventDefault();
                      const draft = workDraft.trim();
                      if (!draft || busy) return;
                      setWorkDraft("");
                      void openAgentChat(draft);
                    }}
                  >
                    <textarea
                      value={workDraft}
                      rows={3}
                      placeholder="Ask OpenMindAI to build, fix, audit, test, or review this project…"
                      onChange={(event) => setWorkDraft(event.target.value)}
                      onKeyDown={(event) => {
                        if (
                          (event.ctrlKey || event.metaKey) &&
                          event.key === "Enter" &&
                          workDraft.trim()
                        ) {
                          event.preventDefault();
                          const draft = workDraft.trim();
                          setWorkDraft("");
                          void openAgentChat(draft);
                        }
                      }}
                    />
                    <div className="project-work-composer-footer">
                      <div
                        className="project-work-suggestions"
                        aria-label="Suggested project tasks"
                      >
                        {[
                          "Audit this project",
                          "Fix failing tests",
                          "Review recent changes",
                          "Find and fix bugs",
                        ].map((suggestion) => (
                          <button
                            type="button"
                            key={suggestion}
                            onClick={() => setWorkDraft(suggestion)}
                          >
                            {suggestion}
                          </button>
                        ))}
                      </div>
                      <button
                        type="submit"
                        className="primary-button project-work-submit"
                        disabled={busy || !workDraft.trim()}
                      >
                        <Sparkles size={15} /> Continue in chat
                      </button>
                    </div>
                  </form>

                  <div className="project-work-trust-row">
                    <span>
                      <CheckCircle2 size={13} /> Uses this project's files and instructions
                    </span>
                    <span>
                      <CheckCircle2 size={13} /> Validates workspace changes before completion
                    </span>
                    <span>
                      <CheckCircle2 size={13} /> Connected apps stay internal
                    </span>
                  </div>
                </section>

                {projectConversations.length ? (
                  <section className="project-card project-work-recent">
                    <div className="project-card-heading">
                      <div>
                        <h3>
                          <MessagesSquare size={15} /> Recent work
                        </h3>
                        <p>Continue a project conversation without leaving the project context.</p>
                      </div>
                    </div>
                    <div className="project-work-chat-list">
                      {projectConversations.slice(0, 5).map((conversation) => (
                        <button
                          type="button"
                          key={conversation.id}
                          onClick={() => props.onOpenConversation(conversation.id)}
                        >
                          <span>
                            <strong>{conversation.title}</strong>
                            <small>{formatTime(conversation.updatedAt)}</small>
                          </span>
                          <MessageSquarePlus size={15} />
                        </button>
                      ))}
                    </div>
                  </section>
                ) : null}

                <details className="project-card project-work-access">
                  <summary>
                    <span>
                      <FolderKanban size={15} />
                      <strong>Workspace access</strong>
                    </span>
                    <small>Folders, file access, and optional terminal permissions</small>
                  </summary>
                  <div className="project-work-access-body">
                    <ProjectLocalWorkspace
                      projectId={activeProject.id}
                      projectName={activeProject.name}
                    />
                  </div>
                </details>
              </div>
            )}
          </>
        ) : (
          <div className="project-empty project-empty-v2">
            <div className="project-empty-icon">
              <FolderKanban size={30} />
            </div>
            <span className="tools-eyebrow">Focused workspaces</span>
            <h2>Create your first project</h2>
            <p className="muted">
              Group chats, local files, and private instructions so OpenMindAI keeps the right
              context together.
            </p>
            <div className="project-empty-features">
              <span>
                <MessagesSquare size={15} /> Related chats
              </span>
              <span>
                <Files size={15} /> Local context files
              </span>
              <span>
                <Sparkles size={15} /> Project instructions
              </span>
            </div>
          </div>
        )}
      </section>

      <ConfirmDialog
        open={deleteTarget !== null}
        title={`Delete ${deleteTarget?.name ?? "project"}?`}
        description="This removes the project and its file links. Conversations are kept and return to your regular chat history."
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
    const trimmed = characters.slice(0, MAX_PROJECT_FILE_TEXT_CHARS).join("");
    return {
      contentText: trimmed,
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
