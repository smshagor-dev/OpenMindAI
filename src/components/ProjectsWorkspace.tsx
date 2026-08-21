import { useEffect, useMemo, useRef, useState } from "react";
import { FilePlus2, FolderKanban, Link2, MessageSquarePlus, Pencil, Plus, Trash2, X } from "lucide-react";
import { api } from "../api";
import type { Conversation, Project, ProjectFile } from "../types";
import { formatBytes, formatError, formatTime } from "../lib/format";
import { ConfirmDialog } from "./ConfirmDialog";

export function ProjectsWorkspace(props: {
  conversations: Conversation[];
  onOpenConversation: (id: string) => void;
  onCreateProjectChat: (project: Project) => Promise<Conversation>;
}) {
  const [projects, setProjects] = useState<Project[]>([]);
  const [activeId, setActiveId] = useState<string | null>(null);
  const [draftName, setDraftName] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [addingFiles, setAddingFiles] = useState(false);
  const [deleteTarget, setDeleteTarget] = useState<Project | null>(null);
  const fileInputRef = useRef<HTMLInputElement | null>(null);

  const refreshProjects = async () => {
    const next = await api.projects();
    setProjects(next);
    setActiveId((current) => current ?? next[0]?.id ?? null);
  };

  useEffect(() => {
    void refreshProjects().catch((caught) => setError(formatError(caught)));
  }, []);

  const activeProject = projects.find((project) => project.id === activeId) ?? projects[0] ?? null;
  const projectConversations = useMemo(() => {
    if (!activeProject) return [];
    const allowed = new Set(activeProject.conversationIds);
    return props.conversations.filter((conversation) => allowed.has(conversation.id));
  }, [activeProject, props.conversations]);
  const unlinkedConversations = useMemo(() => {
    if (!activeProject) return [];
    const linked = new Set(activeProject.conversationIds);
    return props.conversations.filter((conversation) => !linked.has(conversation.id));
  }, [activeProject, props.conversations]);

  const createProject = async () => {
    try {
      const project = await api.createProject(draftName.trim() || "New project");
      await refreshProjects();
      setActiveId(project.id);
      setDraftName("");
    } catch (caught) {
      setError(formatError(caught));
    }
  };

  const updateProject = async (project: Project, patch: Partial<Pick<Project, "name" | "instructions">>) => {
    const next = await api.updateProject(
      project.id,
      patch.name ?? project.name,
      patch.instructions ?? project.instructions,
    );
    setProjects((items) => items.map((item) => (item.id === next.id ? next : item)));
  };

  const createChat = async () => {
    if (!activeProject) return;
    try {
      const conversation = await props.onCreateProjectChat(activeProject);
      await api.linkProjectConversation(activeProject.id, conversation.id);
      await refreshProjects();
    } catch (caught) {
      setError(formatError(caught));
    }
  };

  const linkConversation = async (conversationId: string) => {
    if (!activeProject) return;
    await api.linkProjectConversation(activeProject.id, conversationId);
    await refreshProjects();
  };

  const unlinkConversation = async (conversationId: string) => {
    if (!activeProject) return;
    await api.unlinkProjectConversation(activeProject.id, conversationId);
    await refreshProjects();
  };

  const addFiles = async (files: FileList | null) => {
    if (!files || !activeProject) return;
    setAddingFiles(true);
    try {
      for (const file of Array.from(files)) {
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
      }
      await refreshProjects();
    } finally {
      setAddingFiles(false);
    }
  };

  const deleteFile = async (fileId: string) => {
    if (!activeProject) return;
    await api.deleteProjectFile(activeProject.id, fileId);
    await refreshProjects();
  };

  const deleteProject = async () => {
    if (!deleteTarget) return;
    await api.deleteProject(deleteTarget.id);
    setDeleteTarget(null);
    await refreshProjects();
  };

  return (
    <div className="projects-workspace">
      <input
        ref={fileInputRef}
        type="file"
        multiple
        className="hidden-file-input"
        onChange={(event) => {
          void addFiles(event.currentTarget.files).catch((caught) => setError(formatError(caught)));
          event.currentTarget.value = "";
        }}
      />
      <aside className="projects-list-panel">
        <div className="projects-create-row">
          <input
            value={draftName}
            onChange={(event) => setDraftName(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter") void createProject();
            }}
            placeholder="Project name"
          />
          <button type="button" title="Create project" onClick={() => void createProject()}>
            <Plus size={16} />
          </button>
        </div>
        <div className="projects-list">
          {projects.length === 0 ? (
            <p className="muted">Create a project to group chats, files, and custom instructions.</p>
          ) : (
            projects.map((project) => (
              <button
                type="button"
                key={project.id}
                className={project.id === activeProject?.id ? "project-list-item active" : "project-list-item"}
                onClick={() => setActiveId(project.id)}
              >
                <FolderKanban size={16} />
                <span>{project.name}</span>
              </button>
            ))
          )}
        </div>
      </aside>

      <section className="project-detail">
        {error ? <button className="error-banner" onClick={() => setError(null)}>{error}</button> : null}
        {activeProject ? (
          <>
            <div className="project-header">
              <div>
                <span className="tools-eyebrow">Project</span>
                <input
                  className="project-title-input"
                  value={activeProject.name}
                  onChange={(event) => {
                    setProjects((items) =>
                      items.map((item) => item.id === activeProject.id ? { ...item, name: event.target.value } : item),
                    );
                  }}
                  onBlur={(event) => void updateProject(activeProject, { name: event.target.value })}
                />
              </div>
              <div className="button-row">
                <button type="button" className="ghost-button" onClick={() => void createChat()}>
                  <MessageSquarePlus size={16} /> New chat
                </button>
                <button
                  type="button"
                  className="ghost-button"
                  disabled={addingFiles}
                  onClick={() => fileInputRef.current?.click()}
                >
                  <FilePlus2 size={16} /> {addingFiles ? "Adding..." : "Add files"}
                </button>
                <button type="button" className="danger-button" onClick={() => setDeleteTarget(activeProject)}>
                  <Trash2 size={16} />
                </button>
              </div>
            </div>

            <div className="project-section">
              <h3><Pencil size={15} /> Instructions</h3>
              <textarea
                value={activeProject.instructions}
                onChange={(event) => {
                  setProjects((items) =>
                    items.map((item) => item.id === activeProject.id ? { ...item, instructions: event.target.value } : item),
                  );
                }}
                onBlur={(event) => void updateProject(activeProject, { instructions: event.target.value })}
                placeholder="Hidden instructions OpenMindAI should use for chats inside this project."
              />
            </div>

            <div className="project-columns">
              <div className="project-section">
                <h3>Chats</h3>
                {projectConversations.length ? (
                  projectConversations.map((conversation) => (
                    <div className="project-chat-row" key={conversation.id}>
                      <button type="button" onClick={() => props.onOpenConversation(conversation.id)}>
                        <span>{conversation.title}</span>
                        <small>{formatTime(conversation.updatedAt)}</small>
                      </button>
                      <button type="button" title="Remove from project" onClick={() => void unlinkConversation(conversation.id)}>
                        <X size={14} />
                      </button>
                    </div>
                  ))
                ) : (
                  <p className="muted">No chats yet. Start one from this project.</p>
                )}
                {unlinkedConversations.length ? (
                  <details className="project-link-menu">
                    <summary><Link2 size={14} /> Add existing chat</summary>
                    {unlinkedConversations.slice(0, 12).map((conversation) => (
                      <button type="button" key={conversation.id} onClick={() => void linkConversation(conversation.id)}>
                        {conversation.title}
                      </button>
                    ))}
                  </details>
                ) : null}
              </div>

              <div className="project-section">
                <h3>Files</h3>
                {activeProject.files.length ? (
                  activeProject.files.map((file) => (
                    <div className="project-file-row" key={file.id}>
                      <div>
                        <span>{file.name}</span>
                        <small>
                          {formatBytes(file.sizeBytes)} {file.mimeType ? `- ${file.mimeType}` : ""}
                        </small>
                        {file.error ? <small className="project-file-error">{file.error}</small> : null}
                      </div>
                      <span className={`project-file-status project-file-status-${file.status}`}>
                        {formatFileStatus(file.status)}
                      </span>
                      <button type="button" title="Remove file" onClick={() => void deleteFile(file.id)}>
                        <X size={14} />
                      </button>
                    </div>
                  ))
                ) : (
                  <p className="muted">Text and code files become hidden project context. Other files are tracked locally.</p>
                )}
              </div>
            </div>
          </>
        ) : (
          <div className="project-empty">
            <FolderKanban size={28} />
            <h2>Create your first project</h2>
            <p className="muted">Projects group chats, local files, and hidden instructions for a focused workspace.</p>
          </div>
        )}
      </section>
      <ConfirmDialog
        open={deleteTarget !== null}
        title={`Delete ${deleteTarget?.name ?? "project"}?`}
        description="This removes the project and its file links. Conversations are kept."
        confirmLabel="Delete project"
        danger
        onConfirm={() => void deleteProject()}
        onCancel={() => setDeleteTarget(null)}
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
    const trimmed = text.slice(0, MAX_PROJECT_FILE_TEXT_CHARS);
    return {
      contentText: trimmed,
      status: "ready",
      error: text.length > trimmed.length ? "Ready for context. Stored preview was truncated." : null,
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
