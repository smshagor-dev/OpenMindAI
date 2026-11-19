import { lazy, Suspense, useCallback, useEffect, useRef, useState } from "react";
import {
  Cpu,
  Database,
  Download,
  Loader2,
  Pencil,
  Play,
  RefreshCw,
  RotateCcw,
  Server,
  Sparkles,
  Square,
  Trash2,
} from "lucide-react";
import { cloud, type CloudConnection, type ExecutionMode, type StudioJob } from "../cloudApi";
import "../studio3d.css";

type Props = {
  mode: ExecutionMode;
  internetAvailable: boolean;
  connection: CloudConnection | null;
};

type SceneMode = "object" | "architecture";
type UiMode = "auto" | SceneMode;
type HistoryEntry = {
  id: string;
  title: string;
  mode: SceneMode;
  prompt: string;
  createdAt: string;
};
type GpuStatus = { gpu_active: boolean; gpu?: string; engine?: string };

const HISTORY_KEY = "openmindai.studio3d.history";
const ARCH_HINTS = [
  "architecture",
  "service",
  "services",
  "layer",
  "pipeline",
  "gateway",
  "database",
  "api",
  "microservice",
  "queue",
  "broker",
  "data flow",
  "dataflow",
  "system",
  "→",
  "->",
  "connects",
];

const modeOptions: { id: UiMode; label: string; hint: string; icon: typeof Database }[] = [
  {
    id: "auto",
    label: "Auto detect",
    hint: "Pick model or architecture from your prompt",
    icon: Sparkles,
  },
  { id: "object", label: "3D model", hint: "Objects, products and furniture", icon: Database },
  {
    id: "architecture",
    label: "3D architecture",
    hint: "Systems, services and data flows",
    icon: Server,
  },
];

const suggestions: { title: string; mode: SceneMode; icon: typeof Database; text: string }[] = [
  {
    title: "Delivery robot",
    mode: "object",
    icon: Database,
    text: "A four-wheel delivery robot with a blue cargo compartment, front cameras and a green status light.",
  },
  {
    title: "Reading chair",
    mode: "object",
    icon: Database,
    text: "A modern reading chair with a curved back, wooden legs and a matching side table.",
  },
  {
    title: "Web app architecture",
    mode: "architecture",
    icon: Server,
    text: "Users to frontend to API gateway. Gateway connects to auth, backend and AI service. Backend connects to database and cache. A queue connects the backend to workers.",
  },
  {
    title: "IoT pipeline",
    mode: "architecture",
    icon: Cpu,
    text: "Temperature sensors to IoT gateway to message broker to stream processor to time-series database to monitoring dashboard.",
  },
];

function resolveMode(mode: UiMode, prompt: string): SceneMode {
  if (mode !== "auto") return mode;
  const lower = prompt.toLowerCase();
  return ARCH_HINTS.some((hint) => lower.includes(hint)) ? "architecture" : "object";
}

function explain(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  if (error && typeof error === "object" && "message" in error) return String(error.message);
  return "3D generation failed. Please try again.";
}

function loadHistory(): HistoryEntry[] {
  try {
    const raw = window.localStorage.getItem(HISTORY_KEY);
    const parsed = raw ? (JSON.parse(raw) as HistoryEntry[]) : [];
    return Array.isArray(parsed) ? parsed.slice(0, 40) : [];
  } catch {
    return [];
  }
}

function persistHistory(entries: HistoryEntry[]) {
  try {
    window.localStorage.setItem(HISTORY_KEY, JSON.stringify(entries.slice(0, 40)));
  } catch {
    /* Private mode or disabled storage: history is a convenience only. */
  }
}

function titleFrom(prompt: string): string {
  const clean = prompt.trim().replace(/\s+/g, " ");
  return clean.length > 60 ? clean.slice(0, 57) + "…" : clean || "Untitled scene";
}

function saveMesh(data: { filename: string; data_base64: string }) {
  const bytes = Uint8Array.from(window.atob(data.data_base64), (character) =>
    character.charCodeAt(0),
  );
  if (bytes.byteLength < 12 || String.fromCharCode(...bytes.subarray(0, 4)) !== "glTF")
    throw new Error("The returned asset is not a valid GLB mesh.");
  const blob = new Blob([bytes], { type: "model/gltf-binary" });
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download =
    data.filename.replace(/[^a-z0-9._-]/gi, "_").slice(0, 120) || "openmindai-3d.glb";
  document.body.appendChild(anchor);
  anchor.click();
  anchor.remove();
  window.setTimeout(() => URL.revokeObjectURL(url), 3000);
}

const TERMINAL = ["completed", "failed", "cancelled", "error"];
const delay = (ms: number) => new Promise<void>((resolve) => window.setTimeout(resolve, ms));

// Lazy-load the three.js viewport so the heavy renderer is only parsed when a
// scene is actually shown.
const Studio3DViewport = lazy(() =>
  import("./Studio3DViewport").then((module) => ({ default: module.Studio3DViewport })),
);

export function Studio3D({ mode }: Props) {
  const [uiMode, setUiMode] = useState<UiMode>("auto");
  const [prompt, setPrompt] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [gpu, setGpu] = useState<GpuStatus | null>(null);
  const [job, setJob] = useState<StudioJob | null>(null);
  const [mesh, setMesh] = useState<{ filename: string; data_base64: string } | null>(null);
  const [history, setHistory] = useState<HistoryEntry[]>(loadHistory);
  const [activeId, setActiveId] = useState<string | null>(null);
  const promptRef = useRef<HTMLTextAreaElement>(null);
  const pending = useRef(false);
  const cancelRef = useRef(false);

  // Synchronize the native textarea directly; no React state update is needed.
  useEffect(() => {
    const element = promptRef.current;
    if (!element) return;
    element.style.height = "auto";
    element.style.height = `${Math.min(element.scrollHeight, 200)}px`;
  }, [prompt]);

  // Keep the local worker warm so the first generation is not a cold start.
  useEffect(() => {
    if (mode === "cloud") return;
    cloud
      .startGpu()
      .then(setGpu)
      .catch(() => setGpu({ gpu_active: false }));
  }, [mode]);

  const rememberScene = useCallback((entry: HistoryEntry) => {
    setHistory((old) => {
      const next = [entry, ...old.filter((item) => item.id !== entry.id)].slice(0, 40);
      persistHistory(next);
      return next;
    });
  }, []);

  const poll = useCallback(async (jobId: string): Promise<StudioJob> => {
    for (let attempt = 0; attempt < 90; attempt++) {
      const updated = await cloud.gpuJob(jobId);
      setJob(updated);
      if (cancelRef.current || TERMINAL.includes(updated.status.toLowerCase())) return updated;
      await delay(2000);
    }
    throw new Error("The local GPU worker did not finish in time.");
  }, []);

  const generate = useCallback(async () => {
    const text = prompt.trim();
    if (pending.current || text.length < 5) return;
    pending.current = true;
    cancelRef.current = false;
    setBusy(true);
    setError(null);
    setNotice(null);
    setMesh(null);
    setJob(null);
    try {
      const status = await cloud.startGpu();
      setGpu(status);
      if (!status.gpu_active)
        throw new Error(
          "Local GPU worker is not ready. Install OpenCL drivers and the Python worker dependencies.",
        );
      const sceneMode = resolveMode(uiMode, text);
      const requestKey = crypto.randomUUID();
      const started = await cloud.gpuScene(text, sceneMode, requestKey);
      setJob(started);
      const finished = await poll(started.id);
      if (cancelRef.current) {
        setNotice("Generation cancelled.");
        return;
      }
      if (finished.status.toLowerCase() !== "completed")
        throw new Error(finished.error || "The local GPU worker could not build this scene.");
      const file = await cloud.gpuMesh(finished.id);
      setMesh(file);
      setActiveId(finished.id);
      rememberScene({
        id: finished.id,
        title: titleFrom(text),
        mode: sceneMode,
        prompt: text,
        createdAt: new Date().toISOString(),
      });
      setNotice(`Scene built on the local GPU (${sceneMode}).`);
    } catch (caught) {
      setError(explain(caught));
    } finally {
      pending.current = false;
      setBusy(false);
    }
  }, [prompt, uiMode, poll, rememberScene]);

  const cancelGeneration = useCallback(async () => {
    cancelRef.current = true;
    setNotice("Stopping the current generation…");
    const id = job?.id;
    if (id) {
      // Best-effort: ask the worker to drop the job it is building.
      await cloud.gpu("jobs/" + id + "/cancel", "POST", {}).catch(() => undefined);
    }
  }, [job?.id]);

  const openScene = useCallback(async (entry: HistoryEntry) => {
    setBusy(true);
    setError(null);
    setNotice(null);
    setMesh(null);
    setActiveId(entry.id);
    setPrompt(entry.prompt);
    setUiMode(entry.mode);
    try {
      const file = await cloud.gpuMesh(entry.id);
      setMesh(file);
    } catch {
      setNotice(
        "This scene's mesh is no longer cached by the worker. Run the prompt again to rebuild it.",
      );
    } finally {
      setBusy(false);
    }
  }, []);

  const removeScene = useCallback(
    (entry: HistoryEntry) => {
      setHistory((old) => {
        const next = old.filter((item) => item.id !== entry.id);
        persistHistory(next);
        return next;
      });
      if (activeId === entry.id) {
        setActiveId(null);
        setMesh(null);
      }
    },
    [activeId],
  );

  const newScene = useCallback(() => {
    setActiveId(null);
    setMesh(null);
    setJob(null);
    setPrompt("");
    setUiMode("auto");
    setError(null);
    setNotice(null);
    promptRef.current?.focus();
  }, []);

  const showingResult = Boolean(mesh);
  const activeEntry = history.find((item) => item.id === activeId) ?? null;
  const statusLabel = (() => {
    const status = job?.status?.toLowerCase();
    if (status === "queued") return "Queued on the local GPU…";
    if (status === "running") return "Building your scene on the local GPU…";
    if (status === "cancel_requested") return "Stopping…";
    return "Preparing the local GPU…";
  })();
  const gpuLabel = gpu?.gpu_active
    ? `${gpu.gpu ?? "GPU"} · ${gpu.engine ?? "OpenCL procedural mesh"}`
    : "Local GPU worker not detected (127.0.0.1:8765)";

  return (
    <div className="studio3d-root">
      <aside className="studio3d-sidebar">
        <button className="studio3d-new" type="button" disabled={busy} onClick={newScene}>
          <Pencil size={17} /> New scene
        </button>
        <span className="studio3d-section-label">Your scenes</span>
        <div className="studio3d-history">
          {history.length === 0 ? (
            <p className="studio3d-empty">Generated scenes will appear here.</p>
          ) : (
            history.map((item) => (
              <div
                key={item.id}
                className={`studio3d-history-item ${activeId === item.id ? "active" : ""}`}
              >
                <button
                  className="studio3d-history-title"
                  title={item.title}
                  disabled={busy}
                  onClick={() => void openScene(item)}
                >
                  {item.title}
                </button>
                <button
                  className="studio3d-history-delete"
                  aria-label={`Delete ${item.title}`}
                  disabled={busy}
                  onClick={() => removeScene(item)}
                >
                  <Trash2 size={14} />
                </button>
              </div>
            ))
          )}
        </div>
      </aside>

      <div className={`studio-shell ${showingResult ? "" : "is-empty"}`}>
        {error && (
          <div className="studio-error" role="alert">
            <span>{error}</span>
            <button
              className="icon-button"
              aria-label="Dismiss error"
              onClick={() => setError(null)}
            >
              ×
            </button>
          </div>
        )}

        {showingResult && mesh ? (
          <div className="studio-result">
            <header className="studio-page-header">
              <h1>
                {history.find((item) => item.id === activeId)?.title ?? "3D scene"}
                {activeEntry ? (
                  <span className="studio3d-badge">
                    {activeEntry.mode === "architecture" ? "Architecture" : "3D model"}
                  </span>
                ) : null}
              </h1>
              {job?.output && typeof job.output === "object" ? (
                <p className="studio-meta">
                  {(job.output as { vertices?: number }).vertices ?? 0} vertices ·{" "}
                  {(job.output as { triangles?: number }).triangles ?? 0} triangles
                </p>
              ) : null}
            </header>
            <Suspense
              fallback={
                <p className="studio-muted" role="status">
                  Loading 3D renderer…
                </p>
              }
            >
              <Studio3DViewport dataBase64={mesh.data_base64} />
            </Suspense>
            <div className="studio-tool-row">
              <button
                type="button"
                onClick={() => {
                  try {
                    saveMesh(mesh);
                  } catch (caught) {
                    setError(explain(caught));
                  }
                }}
              >
                <Download size={15} /> Download 3D mesh (.glb)
              </button>
              <button
                type="button"
                className="studio3d-secondary"
                disabled={busy || prompt.trim().length < 5}
                onClick={() => void generate()}
              >
                <RotateCcw size={15} /> Regenerate
              </button>
            </div>
          </div>
        ) : (
          <div className="studio3d-welcome">
            <h1>What would you like to create?</h1>
            <p className="studio-muted">
              Describe a 3D object or an architecture. Generation runs on this computer's GPU and
              produces a real GLB mesh.
            </p>
          </div>
        )}

        <div className="studio-composer-area">
          <form
            className="studio3d-composer"
            onSubmit={(event) => {
              event.preventDefault();
              void generate();
            }}
          >
            <textarea
              ref={promptRef}
              aria-label="Describe your scene"
              value={prompt}
              onChange={(event) => setPrompt(event.target.value)}
              minLength={5}
              maxLength={8000}
              rows={2}
              disabled={busy}
              placeholder={
                uiMode === "object"
                  ? "Describe an object, its materials and proportions"
                  : uiMode === "architecture"
                    ? "Describe services, layers and connections"
                    : "Describe a 3D model or architecture"
              }
              onKeyDown={(event) => {
                if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
                  event.preventDefault();
                  void generate();
                }
              }}
            />
            <div className="studio3d-composer-toolbar">
              <div className="studio3d-mode-choice" role="group" aria-label="Scene type">
                {modeOptions.map((option) => (
                  <button
                    type="button"
                    key={option.id}
                    className={uiMode === option.id ? "active" : ""}
                    aria-pressed={uiMode === option.id}
                    title={option.hint}
                    disabled={busy}
                    onClick={() => {
                      setUiMode(option.id);
                      promptRef.current?.focus();
                    }}
                  >
                    <option.icon size={15} /> {option.label}
                  </button>
                ))}
              </div>
              <button
                type="submit"
                className="studio3d-send"
                aria-label="Generate scene"
                disabled={busy || prompt.trim().length < 5}
              >
                {busy ? <Loader2 size={18} className="studio3d-spin" /> : <Play size={18} />}
              </button>
            </div>
          </form>

          {!showingResult && (
            <div className="studio3d-suggestions" aria-label="Try a prompt">
              {suggestions.map((item) => (
                <button
                  key={item.title}
                  type="button"
                  disabled={busy}
                  onClick={() => {
                    setUiMode(item.mode);
                    setPrompt(item.text);
                    promptRef.current?.focus();
                  }}
                >
                  <item.icon size={17} /> <span>{item.title}</span>
                </button>
              ))}
            </div>
          )}

          <p className="studio3d-processor">
            <Cpu size={13} /> {gpuLabel}
            {!gpu?.gpu_active ? (
              <button
                type="button"
                className="studio3d-link"
                disabled={busy}
                onClick={() =>
                  void cloud
                    .startGpu()
                    .then(setGpu)
                    .catch(() => setGpu({ gpu_active: false }))
                }
              >
                <RefreshCw size={12} /> Retry
              </button>
            ) : null}
          </p>
          <p className="studio3d-note">
            Each generation is saved to Your scenes on this computer. Models use basic shapes;
            dimensions are illustrative.
          </p>
        </div>

        {busy && !mesh ? (
          <div className="studio3d-status" role="status">
            <Loader2 size={15} className="studio3d-spin" />
            <span>{statusLabel}</span>
            <button
              type="button"
              className="studio3d-cancel"
              onClick={() => void cancelGeneration()}
            >
              <Square size={13} /> Stop
            </button>
          </div>
        ) : null}
        {notice ? (
          <div className="studio-banner" role="status">
            {notice}
          </div>
        ) : null}
      </div>
    </div>
  );
}
