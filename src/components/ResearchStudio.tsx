import { useCallback, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import {
  CheckCircle2,
  Cloud,
  Download,
  FileText,
  Loader2,
  Minus,
  PauseCircle,
  Pencil,
  Play,
  RefreshCw,
  Sparkles,
  Square,
  XCircle,
} from "lucide-react";
import { cloud, type ResearchDetail, type ResearchEvent, type ResearchProject } from "../cloudApi";
import "../research-studio.css";

// Desktop Research Studio runs entirely on the OpenMindAI Cloud research
// pipeline (projects, runs, events and manuscripts live on the Cloud backend).
// There is no local research backend: when Cloud is not connected the whole
// section is disabled. Payload shapes from the hosted pipeline vary by run
// stage, so detail/events are parsed defensively — never fabricated.

const TABS = ["Progress", "Findings", "Manuscript", "Sources", "Activity", "Settings"] as const;
type Tab = (typeof TABS)[number];

const LANGUAGES = [
  "auto",
  "English",
  "বাংলা",
  "Русский",
  "العربية",
  "हिन्दी",
  "中文",
  "Español",
  "Français",
  "Deutsch",
];

const LIVE = new Set([
  "queued",
  "pending",
  "planning",
  "running",
  "in_progress",
  "processing",
  "analyzing",
  "writing",
  "reviewing",
  "searching",
  "extracting",
]);
const TERMINAL = new Set([
  "completed",
  "complete",
  "done",
  "failed",
  "error",
  "cancelled",
  "canceled",
]);
const PAUSED = new Set(["paused", "suspended"]);
const NOT_STARTED = new Set(["", "draft", "created", "idle", "new", "ready", "pending_start"]);

function explain(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  if (error && typeof error === "object" && "message" in error) return String(error.message);
  return "Something went wrong talking to OpenMindAI Cloud. Please try again.";
}

function asString(value: unknown): string | null {
  return typeof value === "string" && value.trim() ? value : null;
}
function pickString(obj: unknown, keys: string[]): string | null {
  if (!obj || typeof obj !== "object") return null;
  const rec = obj as Record<string, unknown>;
  for (const key of keys) {
    const found = asString(rec[key]);
    if (found) return found;
  }
  return null;
}
function pickArray(obj: unknown, keys: string[]): unknown[] {
  if (!obj || typeof obj !== "object") return [];
  const rec = obj as Record<string, unknown>;
  for (const key of keys) {
    const value = rec[key];
    if (Array.isArray(value)) return value;
  }
  return [];
}
function pickNumber(obj: unknown, keys: string[]): number | null {
  if (!obj || typeof obj !== "object") return null;
  const rec = obj as Record<string, unknown>;
  for (const key of keys) {
    const value = rec[key];
    if (typeof value === "number" && Number.isFinite(value)) return value;
  }
  return null;
}
function statusOf(value: unknown): string {
  return (pickString(value, ["status", "state"]) ?? "").toLowerCase();
}
function normProgress(detail: unknown): number | null {
  const raw = pickNumber(detail, ["progress", "progress_percent", "percent", "completion"]);
  if (raw == null) return null;
  const pct = raw <= 1 ? raw * 100 : raw;
  return Math.max(0, Math.min(100, Math.round(pct)));
}

type TaskRow = { id: string; title: string; status: string; agent: string | null };
function parseTasks(detail: unknown): TaskRow[] {
  const plan =
    detail && typeof detail === "object" ? (detail as Record<string, unknown>).plan : null;
  const raw = [
    ...pickArray(detail, ["tasks", "agents", "steps"]),
    ...pickArray(plan, ["tasks", "steps"]),
  ];
  const seen = new Set<string>();
  const rows: TaskRow[] = [];
  raw.forEach((item, index) => {
    if (!item || typeof item !== "object") return;
    const id = asString((item as Record<string, unknown>).id) ?? `task-${index}`;
    if (seen.has(id)) return;
    seen.add(id);
    rows.push({
      id,
      title:
        pickString(item, ["title", "name", "label", "description", "role"]) ?? `Task ${index + 1}`,
      status: (pickString(item, ["status", "state"]) ?? "pending").toLowerCase(),
      agent: pickString(item, ["agent", "role", "assignee"]),
    });
  });
  return rows;
}

type SourceRow = { id: string; title: string; url: string | null; date: string | null };
function parseSources(detail: unknown): SourceRow[] {
  const raw = pickArray(detail, ["sources", "references", "citations", "evidence"]);
  return raw
    .filter((item): item is Record<string, unknown> => !!item && typeof item === "object")
    .map((item, index) => ({
      id: asString(item.id) ?? `source-${index}`,
      title:
        pickString(item, ["title", "name", "headline"]) ??
        pickString(item, ["url", "link"]) ??
        `Source ${index + 1}`,
      url: pickString(item, ["url", "link", "href"]),
      date: pickString(item, ["published_at", "date", "year", "published"]),
    }));
}

function manuscriptOf(detail: unknown): string | null {
  return pickString(detail, [
    "manuscript",
    "document",
    "draft",
    "report",
    "manuscript_markdown",
    "content",
    "body",
    "markdown",
  ]);
}
function findingsOf(detail: unknown): string | null {
  return pickString(detail, ["findings", "summary", "analysis", "abstract", "result", "results"]);
}

function downloadText(name: string, text: string) {
  const blob = new Blob([text], { type: "text/markdown;charset=utf-8" });
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = name.replace(/[^a-z0-9._-]/gi, "_").slice(0, 120) || "research.md";
  document.body.appendChild(anchor);
  anchor.click();
  anchor.remove();
  window.setTimeout(() => URL.revokeObjectURL(url), 3000);
}

function TaskIcon({ status }: { status: string }) {
  if (status === "completed" || status === "complete" || status === "done")
    return <CheckCircle2 size={15} className="rs-task-ok" />;
  if (LIVE.has(status)) return <Loader2 size={15} className="rs-spin rs-task-live" />;
  if (status === "failed" || status === "error")
    return <XCircle size={15} className="rs-task-bad" />;
  if (PAUSED.has(status) || status === "cancelled" || status === "canceled")
    return <PauseCircle size={15} className="rs-task-paused" />;
  return <Minus size={15} className="rs-task-idle" />;
}

export function ResearchStudio({
  connected,
  onConnect,
}: {
  connected: boolean;
  onConnect: () => void;
}) {
  const [projects, setProjects] = useState<ResearchProject[]>([]);
  const [activeId, setActiveId] = useState<number | null>(null);
  const [creating, setCreating] = useState(false);
  const [tab, setTab] = useState<Tab>("Progress");
  const [detail, setDetail] = useState<ResearchDetail | null>(null);
  const [events, setEvents] = useState<ResearchEvent[]>([]);
  const [loadingProjects, setLoadingProjects] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [pollTick, setPollTick] = useState(0);

  // New-project form
  const [title, setTitle] = useState("");
  const [prompt, setPrompt] = useState("");
  const [outputLanguage, setOutputLanguage] = useState("auto");

  // The projects list lives in the app's main left sidebar (portal target).
  const [sidebarSlot, setSidebarSlot] = useState<HTMLElement | null>(null);
  useEffect(() => {
    setSidebarSlot(document.getElementById("studio-sidebar-slot"));
  }, []);

  const active = projects.find((project) => project.id === activeId) ?? null;

  const loadProjects = useCallback(async () => {
    setLoadingProjects(true);
    try {
      const list = await cloud.researchProjects();
      setProjects(list);
    } catch (caught) {
      setError(explain(caught));
    } finally {
      setLoadingProjects(false);
    }
  }, []);

  useEffect(() => {
    if (!connected) {
      setProjects([]);
      setActiveId(null);
      setDetail(null);
      setEvents([]);
      return;
    }
    void loadProjects();
  }, [connected, loadProjects]);

  // Load and poll the active project's detail + events while the run is live.
  useEffect(() => {
    if (!connected || activeId == null) {
      setDetail(null);
      setEvents([]);
      return;
    }
    let alive = true;
    let timer: number | null = null;
    const tick = async () => {
      try {
        const [nextDetail, nextEvents] = await Promise.all([
          cloud.researchDetail(activeId),
          cloud.researchEvents(activeId).catch(() => [] as ResearchEvent[]),
        ]);
        if (!alive) return;
        setDetail(nextDetail);
        setEvents(nextEvents);
        const status = statusOf(nextDetail);
        if (status)
          setProjects((prev) =>
            prev.map((project) => (project.id === activeId ? { ...project, status } : project)),
          );
        if (alive && LIVE.has(status)) timer = window.setTimeout(() => void tick(), 2500);
      } catch (caught) {
        if (alive) setError(explain(caught));
      }
    };
    void tick();
    return () => {
      alive = false;
      if (timer != null) window.clearTimeout(timer);
    };
  }, [connected, activeId, pollTick]);

  const choose = useCallback((id: number | null) => {
    setActiveId(id);
    setCreating(false);
    setTab("Progress");
    setError(null);
    setNotice(null);
    setDetail(null);
    setEvents([]);
  }, []);

  const createProject = useCallback(async () => {
    const question = prompt.trim();
    if (question.length < 5) {
      setError("Describe your research question (at least a few words).");
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const projectTitle = title.trim() || question.slice(0, 80);
      const project = await cloud.createResearch(projectTitle, question, outputLanguage);
      setProjects((prev) => [project, ...prev.filter((item) => item.id !== project.id)]);
      setActiveId(project.id);
      setCreating(false);
      setTitle("");
      setPrompt("");
      setTab("Progress");
      setNotice("Project created on OpenMindAI Cloud. Start the research run to begin.");
    } catch (caught) {
      setError(explain(caught));
    } finally {
      setBusy(false);
    }
  }, [title, prompt, outputLanguage]);

  const startRun = useCallback(async () => {
    if (activeId == null) return;
    setBusy(true);
    setError(null);
    try {
      await cloud.startResearch(activeId);
      setNotice("Research run started.");
      setPollTick((value) => value + 1);
    } catch (caught) {
      setError(explain(caught));
    } finally {
      setBusy(false);
    }
  }, [activeId]);

  const control = useCallback(
    async (action: "pause" | "resume" | "cancel") => {
      if (activeId == null) return;
      setBusy(true);
      setError(null);
      try {
        await cloud.researchControl(activeId, action);
        setNotice(
          action === "pause"
            ? "Research paused."
            : action === "resume"
              ? "Research resumed."
              : "Research cancelled.",
        );
        setPollTick((value) => value + 1);
      } catch (caught) {
        setError(explain(caught));
      } finally {
        setBusy(false);
      }
    },
    [activeId],
  );

  // ---- Not connected: the whole section is disabled. --------------------
  if (!connected) {
    return (
      <div className="research-root">
        {sidebarSlot &&
          createPortal(
            <div className="studio-side rs-side">
              <p className="rs-sidebar-empty">
                Connect to OpenMindAI Cloud to use Research Studio.
              </p>
            </div>,
            sidebarSlot,
          )}
        <main className="rs rs-main">
          <section className="rs-locked" role="status">
            <Cloud size={40} />
            <h1>Research Studio needs OpenMindAI Cloud</h1>
            <p>
              Research runs on the OpenMindAI Cloud research pipeline. Connect your account to
              create projects, run research and generate manuscripts.
            </p>
            <button className="rs-primary" type="button" onClick={onConnect}>
              Connect OpenMindAI Cloud
            </button>
          </section>
        </main>
      </div>
    );
  }

  const dashboard = !active && !creating;

  const status = statusOf(detail) || (active?.status ?? "").toLowerCase();
  const progress = normProgress(detail);
  const tasks = parseTasks(detail);
  const sources = parseSources(detail);
  const manuscript = manuscriptOf(detail);
  const findings = findingsOf(detail);
  const highlights = pickArray(detail, ["key_findings", "highlights"]).filter(
    (item): item is string => typeof item === "string",
  );
  const isLive = LIVE.has(status);
  const isPaused = PAUSED.has(status);
  const isTerminal = TERMINAL.has(status);
  const notStarted = NOT_STARTED.has(status);
  const detailLoading = active != null && detail == null && !error;

  return (
    <div className="research-root">
      {sidebarSlot &&
        createPortal(
          <div className="studio-side rs-side">
            <button
              className="rs-new-btn"
              type="button"
              disabled={busy}
              onClick={() => {
                setCreating(true);
                setActiveId(null);
                setError(null);
                setNotice(null);
              }}
            >
              <Pencil size={16} /> New research
            </button>
            <button
              className={`rs-nav-item ${dashboard ? "selected" : ""}`}
              type="button"
              onClick={() => choose(null)}
            >
              <Sparkles size={17} /> Research dashboard
            </button>

            <h2>
              Projects
              <button
                className="rs-refresh"
                type="button"
                title="Refresh projects"
                disabled={loadingProjects}
                onClick={() => void loadProjects()}
              >
                {loadingProjects ? (
                  <Loader2 size={13} className="rs-spin" />
                ) : (
                  <RefreshCw size={13} />
                )}
              </button>
            </h2>
            {projects.length === 0 ? (
              <p className="rs-sidebar-empty">
                {loadingProjects
                  ? "Loading your Cloud projects…"
                  : "Your research projects will appear here."}
              </p>
            ) : (
              <div className="rs-projects">
                {projects.map((project) => (
                  <button
                    key={project.id}
                    className={`rs-project-title ${activeId === project.id ? "active" : ""}`}
                    title={project.title}
                    onClick={() => choose(project.id)}
                  >
                    {project.title}
                  </button>
                ))}
              </div>
            )}
          </div>,
          sidebarSlot,
        )}

      <main className="rs rs-main">
        {error && (
          <div className="rs-error" role="alert">
            {error}
            <button aria-label="Dismiss error" onClick={() => setError(null)}>
              ×
            </button>
          </div>
        )}
        {notice && (
          <div className="rs-notice" role="status">
            {notice}
          </div>
        )}

        {creating ? (
          <section className="rs-card rs-new">
            <span className="rs-eyebrow">New research project</span>
            <h1>Start with a question.</h1>
            <p>
              Your research question and language are sent to the OpenMindAI Cloud research
              pipeline, which plans and runs the investigation.
            </p>
            <label>
              Project title
              <input
                value={title}
                maxLength={200}
                onChange={(event) => setTitle(event.target.value)}
                placeholder="e.g. Robustness of UAV swarm coordination"
              />
            </label>
            <label>
              Research prompt
              <textarea
                dir="auto"
                rows={8}
                value={prompt}
                maxLength={16000}
                onChange={(event) => setPrompt(event.target.value)}
                placeholder="বাংলায়, English বা অন্য ভাষায় আপনার গবেষণার বিষয়, লক্ষ্য এবং সীমাবদ্ধতা লিখুন…"
              />
            </label>
            <label>
              Manuscript language
              <select
                value={outputLanguage}
                onChange={(event) => setOutputLanguage(event.target.value)}
              >
                {LANGUAGES.map((language) => (
                  <option key={language}>{language}</option>
                ))}
              </select>
            </label>
            <div className="rs-actions">
              <button
                className="rs-primary"
                type="button"
                disabled={busy}
                onClick={() => void createProject()}
              >
                {busy ? <Loader2 size={16} className="rs-spin" /> : <Sparkles size={16} />} Create
                project
              </button>
              <button type="button" disabled={busy} onClick={() => setCreating(false)}>
                Cancel
              </button>
            </div>
          </section>
        ) : dashboard ? (
          <section>
            <span className="rs-eyebrow">Research Studio</span>
            <h1>Cloud research on this computer.</h1>
            <p>
              Investigate a question with the OpenMindAI Cloud research pipeline. Projects, runs and
              manuscripts are stored on your Cloud account.
            </p>
            <div className="rs-dashboard">
              <button
                className="rs-card rs-dash-cta"
                type="button"
                onClick={() => setCreating(true)}
              >
                <Pencil size={20} />
                <strong>Start new research</strong>
                <span>Define a question, language and scope.</span>
              </button>
              <div className="rs-card rs-stats">
                <span>Projects on your Cloud account</span>
                <strong>{projects.length}</strong>
                <span>Synced from OpenMindAI Cloud</span>
              </div>
            </div>
            {projects.length > 0 && (
              <div className="rs-card">
                <h3>Recent projects</h3>
                {projects.slice(0, 8).map((project) => (
                  <button
                    key={project.id}
                    className="rs-recent"
                    type="button"
                    onClick={() => choose(project.id)}
                  >
                    <FileText size={15} /> <span>{project.title}</span>
                    {project.status ? <em className="rs-recent-status">{project.status}</em> : null}
                  </button>
                ))}
              </div>
            )}
          </section>
        ) : active ? (
          <>
            <nav className="rs-topbar" aria-label="Research tools">
              {TABS.map((item) => (
                <button
                  key={item}
                  className={`rs-tool ${tab === item ? "active" : ""}`}
                  aria-current={tab === item ? "page" : undefined}
                  onClick={() => setTab(item)}
                >
                  {item}
                </button>
              ))}
            </nav>

            <header className="rs-project-header">
              <span className="rs-eyebrow">{tab}</span>
              <h1>{active.title}</h1>
              <p>
                {(active.prompt ?? pickString(detail, ["prompt"]) ?? "").slice(0, 220)}
                {(active.prompt ?? "").length > 220 ? "…" : ""}
              </p>
              <span
                className={`rs-badge rs-badge-${isTerminal ? "done" : isLive ? "live" : isPaused ? "paused" : "idle"}`}
              >
                {status || "not started"}
              </span>
            </header>

            {tab === "Progress" && (
              <section className="rs-card">
                <div className="rs-run-controls">
                  {notStarted ? (
                    <button
                      className="rs-primary"
                      type="button"
                      disabled={busy}
                      onClick={() => void startRun()}
                    >
                      {busy ? <Loader2 size={16} className="rs-spin" /> : <Play size={16} />} Start
                      research
                    </button>
                  ) : null}
                  {isLive ? (
                    <button type="button" disabled={busy} onClick={() => void control("pause")}>
                      <PauseCircle size={15} /> Pause
                    </button>
                  ) : null}
                  {isPaused ? (
                    <button type="button" disabled={busy} onClick={() => void control("resume")}>
                      <Play size={15} /> Resume
                    </button>
                  ) : null}
                  {isLive || isPaused ? (
                    <button type="button" disabled={busy} onClick={() => void control("cancel")}>
                      <Square size={15} /> Cancel
                    </button>
                  ) : null}
                  <button
                    type="button"
                    disabled={busy}
                    onClick={() => setPollTick((value) => value + 1)}
                    title="Refresh status"
                  >
                    <RefreshCw size={15} /> Refresh
                  </button>
                </div>

                {progress != null ? (
                  <div className="rs-progress" aria-label={`Progress ${progress}%`}>
                    <div className="rs-progress-bar" style={{ width: `${progress}%` }} />
                    <span className="rs-progress-label">{progress}%</span>
                  </div>
                ) : null}

                {detailLoading ? (
                  <p className="rs-chat-empty">
                    <Loader2 size={14} className="rs-spin" /> Loading run status from Cloud…
                  </p>
                ) : tasks.length > 0 ? (
                  <ul className="rs-tasks">
                    {tasks.map((task) => (
                      <li key={task.id} className="rs-task">
                        <TaskIcon status={task.status} />
                        <span className="rs-task-title">{task.title}</span>
                        {task.agent ? <em className="rs-task-agent">{task.agent}</em> : null}
                        <span className="rs-task-status">{task.status}</span>
                      </li>
                    ))}
                  </ul>
                ) : isLive ? (
                  <p className="rs-chat-empty">
                    <Loader2 size={14} className="rs-spin" /> Planning research tasks…
                  </p>
                ) : notStarted ? (
                  <p className="rs-chat-empty">
                    Start the research run to generate a plan and tasks.
                  </p>
                ) : (
                  <p className="rs-chat-empty">No task breakdown was returned for this run.</p>
                )}
              </section>
            )}

            {tab === "Findings" && (
              <section className="rs-card">
                <h3>Research findings</h3>
                {findings || highlights.length > 0 ? (
                  <>
                    {findings ? <div className="rs-prose">{findings}</div> : null}
                    {highlights.length > 0 ? (
                      <ul className="rs-highlights">
                        {highlights.map((item, index) => (
                          <li key={index}>{item}</li>
                        ))}
                      </ul>
                    ) : null}
                  </>
                ) : (
                  <p className="rs-chat-empty">
                    {isTerminal
                      ? "No findings were returned for this run."
                      : "Findings appear here once the Cloud run has analyzed evidence."}
                  </p>
                )}
              </section>
            )}

            {tab === "Manuscript" && (
              <section className="rs-card">
                <div className="rs-source-heading">
                  <h3>
                    <FileText size={16} /> Manuscript
                  </h3>
                  {manuscript ? (
                    <button
                      type="button"
                      onClick={() => downloadText(`${active.title}.md`, manuscript)}
                    >
                      <Download size={15} /> Download .md
                    </button>
                  ) : null}
                </div>
                {manuscript ? (
                  <div className="rs-prose rs-manuscript">{manuscript}</div>
                ) : (
                  <p className="rs-chat-empty">
                    {isTerminal
                      ? "This run did not produce a manuscript."
                      : "The manuscript is generated by the Cloud pipeline as the run completes."}
                  </p>
                )}
              </section>
            )}

            {tab === "Sources" && (
              <section className="rs-card">
                <h3>Sources &amp; evidence</h3>
                {sources.length > 0 ? (
                  <ul className="rs-sources">
                    {sources.map((source) => (
                      <li key={source.id} className="rs-source-row">
                        {source.url ? (
                          <a href={source.url} target="_blank" rel="noreferrer noopener">
                            {source.title}
                          </a>
                        ) : (
                          <span>{source.title}</span>
                        )}
                        {source.date ? <em className="rs-source-date">{source.date}</em> : null}
                      </li>
                    ))}
                  </ul>
                ) : (
                  <p className="rs-chat-empty">
                    Sources collected by the Cloud run will be listed here with links and dates.
                  </p>
                )}
              </section>
            )}

            {tab === "Activity" && (
              <section className="rs-card">
                <h3>Live activity</h3>
                {events.length > 0 ? (
                  <ul className="rs-events">
                    {[...events]
                      .slice()
                      .reverse()
                      .map((event, index) => (
                        <li key={event.id ?? event.sequence ?? index} className="rs-event">
                          <span
                            className={`rs-event-dot rs-event-${(event.status ?? "").toLowerCase() || "info"}`}
                          />
                          <div className="rs-event-body">
                            <span className="rs-event-type">
                              {event.type ?? event.agent ?? event.task ?? "event"}
                            </span>
                            {event.message ? (
                              <span className="rs-event-msg">{event.message}</span>
                            ) : null}
                          </div>
                          {event.created_at ? (
                            <time className="rs-event-time">
                              {new Date(event.created_at).toLocaleTimeString()}
                            </time>
                          ) : null}
                        </li>
                      ))}
                  </ul>
                ) : (
                  <p className="rs-chat-empty">
                    {isLive
                      ? "Waiting for live events from the Cloud run…"
                      : "No activity events yet."}
                  </p>
                )}
              </section>
            )}

            {tab === "Settings" && (
              <section className="rs-card">
                <h3>Project</h3>
                <label>
                  Research prompt
                  <textarea
                    readOnly
                    rows={8}
                    value={active.prompt ?? pickString(detail, ["prompt"]) ?? ""}
                  />
                </label>
                <div className="rs-grid">
                  <div>
                    <span className="rs-eyebrow">Status</span>
                    <p>{status || "not started"}</p>
                  </div>
                  <div>
                    <span className="rs-eyebrow">Language</span>
                    <p>
                      {active.output_language ?? pickString(detail, ["output_language"]) ?? "auto"}
                    </p>
                  </div>
                  <div>
                    <span className="rs-eyebrow">Created</span>
                    <p>{active.created_at ? new Date(active.created_at).toLocaleString() : "—"}</p>
                  </div>
                </div>
                <div className="rs-actions">
                  {isLive ? (
                    <button type="button" disabled={busy} onClick={() => void control("pause")}>
                      <PauseCircle size={15} /> Pause run
                    </button>
                  ) : null}
                  {isPaused ? (
                    <button type="button" disabled={busy} onClick={() => void control("resume")}>
                      <Play size={15} /> Resume run
                    </button>
                  ) : null}
                  {isLive || isPaused ? (
                    <button type="button" disabled={busy} onClick={() => void control("cancel")}>
                      <Square size={15} /> Cancel run
                    </button>
                  ) : null}
                </div>
              </section>
            )}
          </>
        ) : null}
      </main>
    </div>
  );
}
