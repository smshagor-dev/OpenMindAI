import { useCallback, useEffect, useRef, useState } from "react";
import {
  FileText,
  Globe,
  Loader2,
  MessageSquare,
  Pencil,
  Search,
  Sparkles,
  Trash2,
} from "lucide-react";
import { api as local } from "../api_legacy";
import type { Message } from "../types";
import "../research-studio.css";

// Research Studio (PC) — mirrors the cloud research-studio design (left tool
// nav, projects list, new-project card, view switcher) but runs on the local
// model and the app's own web research instead of the hosted Laravel pipeline.
// Phase 1: shell + Chat (local model) + Overview + Web activity + Settings;
// the remaining tool views are presented with the same panel design and marked
// as being brought to the desktop build.

const VIEWS = [
  "Chat",
  "Overview",
  "Live agents",
  "Web activity",
  "Literature",
  "PDF reader",
  "Research gaps",
  "Experiments",
  "Manuscript",
  "Citations",
  "Quality review",
  "Collaboration",
  "Export",
  "History",
  "Settings",
] as const;
type View = (typeof VIEWS)[number];

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

const DESKTOP_PHASE: Partial<Record<View, string>> = {
  "Live agents":
    "Live agent activity will stream here as the desktop research loop runs on your local model.",
  Literature:
    "Literature search and screening is being brought to the desktop build on local web research.",
  "PDF reader": "Attach and read source PDFs here; desktop PDF grounding is on the roadmap.",
  "Research gaps": "Gap analysis across your evidence will appear here.",
  Experiments: "Design and track experiments against your research questions here.",
  Manuscript: "The manuscript canvas (academic editor) is a later desktop phase.",
  Citations: "Citations and bibliography (APA/BibTeX) will be generated here.",
  "Quality review": "Automated quality and consistency review will run here.",
  Collaboration: "Collaboration and sharing is a later desktop phase.",
  Export: "Export to Word, PDF and Markdown will be available here.",
  History: "A timeline of this project's research activity will appear here.",
};

type Project = {
  id: string;
  title: string;
  prompt: string;
  progressLanguage: string;
  outputLanguage: string;
  conversationId: string;
  createdAt: string;
};

const PROJECTS_KEY = "openmindai.research.projects";

function loadProjects(): Project[] {
  try {
    const raw = window.localStorage.getItem(PROJECTS_KEY);
    const parsed = raw ? (JSON.parse(raw) as Project[]) : [];
    return Array.isArray(parsed) ? parsed : [];
  } catch {
    return [];
  }
}
function persistProjects(projects: Project[]) {
  try {
    window.localStorage.setItem(PROJECTS_KEY, JSON.stringify(projects.slice(0, 60)));
  } catch {
    /* storage is a convenience only */
  }
}
function explain(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  if (error && typeof error === "object" && "message" in error) return String(error.message);
  return "Something went wrong. Please try again.";
}
const delay = (ms: number) => new Promise<void>((resolve) => window.setTimeout(resolve, ms));

export function ResearchStudio() {
  const [projects, setProjects] = useState<Project[]>(loadProjects);
  const [activeId, setActiveId] = useState<string | null>(null);
  const [view, setView] = useState<View>("Chat");
  const [creating, setCreating] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  // New-project form
  const [title, setTitle] = useState("");
  const [prompt, setPrompt] = useState("");
  const [progressLanguage, setProgressLanguage] = useState("auto");
  const [outputLanguage, setOutputLanguage] = useState("auto");

  // Chat
  const [messages, setMessages] = useState<Message[]>([]);
  const [input, setInput] = useState("");
  const [sending, setSending] = useState(false);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const scrollRef = useRef<HTMLDivElement>(null);

  const active = projects.find((project) => project.id === activeId) ?? null;

  const save = useCallback((next: Project[]) => {
    setProjects(next);
    persistProjects(next);
  }, []);

  const loadMessages = useCallback(async (conversationId: string) => {
    try {
      setMessages(await local.messages(conversationId));
    } catch (caught) {
      setError(explain(caught));
    }
  }, []);

  useEffect(() => {
    if (active && view === "Chat") void loadMessages(active.conversationId);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeId, view]);

  useEffect(() => {
    scrollRef.current?.scrollTo({ top: scrollRef.current.scrollHeight });
  }, [messages, sending]);

  const choose = useCallback((id: string | null) => {
    setActiveId(id);
    setView("Chat");
    setCreating(false);
    setError(null);
    setNotice(null);
    setMessages([]);
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
      const conversation = await local.createConversation(`Research: ${projectTitle}`);
      const project: Project = {
        id: crypto.randomUUID(),
        title: projectTitle,
        prompt: question,
        progressLanguage,
        outputLanguage,
        conversationId: conversation.id,
        createdAt: new Date().toISOString(),
      };
      save([project, ...projects]);
      setActiveId(project.id);
      setView("Chat");
      setCreating(false);
      setTitle("");
      setPrompt("");
      setMessages([]);
      setNotice("Research project created. Start the investigation in Chat.");
    } catch (caught) {
      setError(explain(caught));
    } finally {
      setBusy(false);
    }
  }, [title, prompt, progressLanguage, outputLanguage, projects, save]);

  const removeProject = useCallback(
    (project: Project) => {
      const next = projects.filter((item) => item.id !== project.id);
      save(next);
      if (activeId === project.id) {
        setActiveId(null);
        setMessages([]);
      }
      void local.deleteConversation(project.conversationId).catch(() => undefined);
    },
    [projects, activeId, save],
  );

  const send = useCallback(async () => {
    const text = input.trim();
    if (!active || !text || sending) return;
    setSending(true);
    setError(null);
    setInput("");
    try {
      await local.sendChatMessage(active.conversationId, text, "chat");
      // send_chat_message may resolve while the reply is still streaming; reload
      // and briefly poll until the last assistant turn is no longer in progress.
      for (let attempt = 0; attempt < 40; attempt++) {
        const list = await local.messages(active.conversationId);
        setMessages(list);
        const last = list[list.length - 1];
        if (
          last &&
          last.role === "assistant" &&
          ["completed", "cancelled", "failed"].includes(last.status)
        )
          break;
        await delay(700);
      }
    } catch (caught) {
      setError(explain(caught));
    } finally {
      setSending(false);
      inputRef.current?.focus();
    }
  }, [active, input, sending]);

  const dashboard = !active && !creating;

  return (
    <div className="research-root">
      <aside className="rs-nav">
        <button
          className="rs-primary rs-new-btn"
          type="button"
          disabled={busy}
          onClick={() => {
            setCreating(true);
            setActiveId(null);
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

        {active && (
          <>
            <h2>Research tools</h2>
            <div className="rs-tools">
              {VIEWS.map((item) => (
                <button
                  key={item}
                  className={`rs-tool ${view === item ? "active" : ""}`}
                  aria-current={view === item ? "page" : undefined}
                  onClick={() => setView(item)}
                >
                  {item}
                </button>
              ))}
            </div>
          </>
        )}

        <h2>Projects</h2>
        {projects.length === 0 ? (
          <p className="rs-sidebar-empty">Your research projects will appear here.</p>
        ) : (
          <div className="rs-projects">
            {projects.map((project) => (
              <div
                key={project.id}
                className={`rs-project-row ${activeId === project.id ? "active" : ""}`}
              >
                <button
                  className="rs-project-title"
                  title={project.title}
                  onClick={() => choose(project.id)}
                >
                  {project.title}
                </button>
                <button
                  className="rs-project-delete"
                  aria-label={`Delete ${project.title}`}
                  onClick={() => removeProject(project)}
                >
                  <Trash2 size={13} />
                </button>
              </div>
            ))}
          </div>
        )}
      </aside>

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
              Your original language, research constraints and source evidence stay with the project
              on this computer.
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
            <div className="rs-grid">
              <label>
                Progress language
                <select
                  value={progressLanguage}
                  onChange={(event) => setProgressLanguage(event.target.value)}
                >
                  {LANGUAGES.map((language) => (
                    <option key={language}>{language}</option>
                  ))}
                </select>
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
            </div>
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
            <h1>Evidence-led research on this computer.</h1>
            <p>
              Investigate a question with your local model and live web research. Projects, prompts
              and evidence stay on this device.
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
                <span>Projects on this device</span>
                <strong>{projects.length}</strong>
                <span>Stored locally in this build</span>
              </div>
            </div>
            {projects.length > 0 && (
              <div className="rs-card">
                <h3>Recent projects</h3>
                {projects.slice(0, 6).map((project) => (
                  <button
                    key={project.id}
                    className="rs-recent"
                    type="button"
                    onClick={() => choose(project.id)}
                  >
                    <FileText size={15} /> <span>{project.title}</span>
                  </button>
                ))}
              </div>
            )}
          </section>
        ) : active ? (
          <>
            <header className="rs-project-header">
              <span className="rs-eyebrow">{view}</span>
              <h1>{active.title}</h1>
              <p>
                {active.prompt.slice(0, 220)}
                {active.prompt.length > 220 ? "…" : ""}
              </p>
              <span className="rs-status">
                Local model ·{" "}
                {active.outputLanguage === "auto" ? "Auto language" : active.outputLanguage}
              </span>
            </header>

            {view === "Chat" && (
              <section className="rs-chat-view">
                <div className="rs-messages" ref={scrollRef}>
                  {messages.length === 0 && !sending ? (
                    <p className="rs-chat-empty">
                      Ask a question about “{active.title}”. Answers come from your local model;
                      switch to Web activity for live sources.
                    </p>
                  ) : (
                    messages.map((message) => (
                      <div key={message.id} className={`rs-msg rs-msg-${message.role}`}>
                        <div className="rs-msg-role">
                          {message.role === "user" ? "You" : "Assistant"}
                        </div>
                        <div className="rs-msg-body">
                          {message.content || (message.status === "streaming" ? "…" : "")}
                        </div>
                      </div>
                    ))
                  )}
                  {sending && (
                    <div className="rs-msg rs-msg-assistant">
                      <div className="rs-msg-role">Assistant</div>
                      <div className="rs-msg-body rs-typing">
                        <Loader2 size={14} className="rs-spin" /> Thinking…
                      </div>
                    </div>
                  )}
                </div>
                <form
                  className="rs-composer"
                  onSubmit={(event) => {
                    event.preventDefault();
                    void send();
                  }}
                >
                  <textarea
                    ref={inputRef}
                    rows={2}
                    dir="auto"
                    value={input}
                    disabled={sending}
                    placeholder="Ask about your research question…"
                    onChange={(event) => setInput(event.target.value)}
                    onKeyDown={(event) => {
                      if (
                        event.key === "Enter" &&
                        !event.shiftKey &&
                        !event.nativeEvent.isComposing
                      ) {
                        event.preventDefault();
                        void send();
                      }
                    }}
                  />
                  <button
                    className="rs-primary rs-send"
                    type="submit"
                    disabled={sending || !input.trim()}
                    aria-label="Send"
                  >
                    {sending ? (
                      <Loader2 size={16} className="rs-spin" />
                    ) : (
                      <MessageSquare size={16} />
                    )}
                  </button>
                </form>
              </section>
            )}

            {view === "Overview" && (
              <section className="rs-card">
                <h3>Project overview</h3>
                <label>
                  Research prompt
                  <textarea readOnly rows={8} value={active.prompt} />
                </label>
                <div className="rs-grid">
                  <div>
                    <span className="rs-eyebrow">Progress language</span>
                    <p>{active.progressLanguage}</p>
                  </div>
                  <div>
                    <span className="rs-eyebrow">Manuscript language</span>
                    <p>{active.outputLanguage}</p>
                  </div>
                  <div>
                    <span className="rs-eyebrow">Created</span>
                    <p>{new Date(active.createdAt).toLocaleString()}</p>
                  </div>
                </div>
              </section>
            )}

            {view === "Web activity" && <WebActivity prompt={active.prompt} />}

            {view === "Settings" && (
              <section className="rs-card">
                <h3>Project settings</h3>
                <div className="rs-grid">
                  <label>
                    Progress language
                    <select
                      value={active.progressLanguage}
                      onChange={(event) =>
                        save(
                          projects.map((item) =>
                            item.id === active.id
                              ? { ...item, progressLanguage: event.target.value }
                              : item,
                          ),
                        )
                      }
                    >
                      {LANGUAGES.map((language) => (
                        <option key={language}>{language}</option>
                      ))}
                    </select>
                  </label>
                  <label>
                    Manuscript language
                    <select
                      value={active.outputLanguage}
                      onChange={(event) =>
                        save(
                          projects.map((item) =>
                            item.id === active.id
                              ? { ...item, outputLanguage: event.target.value }
                              : item,
                          ),
                        )
                      }
                    >
                      {LANGUAGES.map((language) => (
                        <option key={language}>{language}</option>
                      ))}
                    </select>
                  </label>
                </div>
                <div className="rs-actions">
                  <button type="button" onClick={() => removeProject(active)}>
                    <Trash2 size={15} /> Delete project
                  </button>
                </div>
              </section>
            )}

            {DESKTOP_PHASE[view] && (
              <section className="rs-card rs-phase">
                <h3>{view}</h3>
                <p>{DESKTOP_PHASE[view]}</p>
                <span className="rs-status">Coming to the desktop build</span>
              </section>
            )}
          </>
        ) : null}
      </main>
    </div>
  );
}

// Live web activity uses the app's own keyless web research (the same engine the
// chat Web Search mode uses), so the Research Studio shows real sources offline
// of the hosted pipeline.
function WebActivity({ prompt }: { prompt: string }) {
  const [query, setQuery] = useState(prompt.slice(0, 200));
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const run = useCallback(async () => {
    const text = query.trim();
    if (!text || busy) return;
    setBusy(true);
    setError(null);
    setResult(null);
    try {
      const context = await local.webSearchContext(text, "search");
      setResult(context || "No usable live sources were returned.");
    } catch (caught) {
      setError(explain(caught));
    } finally {
      setBusy(false);
    }
  }, [query, busy]);

  return (
    <section className="rs-card">
      <div className="rs-source-heading">
        <h3>
          <Globe size={16} /> Live web activity
        </h3>
      </div>
      <label>
        Search query
        <textarea
          rows={2}
          dir="auto"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
        />
      </label>
      <div className="rs-actions">
        <button className="rs-primary" type="button" disabled={busy} onClick={() => void run()}>
          {busy ? <Loader2 size={16} className="rs-spin" /> : <Search size={16} />} Search the web
        </button>
      </div>
      {error && <p className="rs-error-text">{error}</p>}
      {result && <div className="rs-source-text">{result}</div>}
    </section>
  );
}
