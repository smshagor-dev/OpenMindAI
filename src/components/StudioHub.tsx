import { useCallback, useEffect, useState } from "react";
import { Database, Cloud, Cpu, Globe, Loader2, Play, RefreshCw, Search, ShieldCheck, Square, TableProperties, X } from "lucide-react";
import { cloud, submitQueuedScene, type CloudConnection, type ExecutionMode, type QueuedScene, type ResearchProject, type StudioJob, type StudioSection } from "../cloudApi";
import { isRecoverableCloudFailure, offlineSourceNotice, requestsLocalOnly, selectExecutionRoute } from "../lib/executionRouting";
import "../cloud-studio.css";

type Props = {
  section: StudioSection;
  mode: ExecutionMode;
  internetAvailable: boolean;
  connection: CloudConnection | null;
  onConnected: (connection: CloudConnection) => void;
  onDisconnect: () => void;
  onLocalPrompt: (prompt: string) => void;
};

function explain(error: unknown) {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  if (error && typeof error === "object" && "message" in error) return String(error.message);
  return "Request failed. Please check the service and try again.";
}

function safeWebLink(value: string | undefined): string | null {
  if (!value) return null;
  try {
    const url = new URL(value);
    if ((url.protocol === "https:" || url.protocol === "http:") && !url.username && !url.password)
      return url.href;
  } catch { /* Invalid remote URL. */ }
  return null;
}

async function encodeReference(file: File): Promise<{ data: string; preview: string }> {
  if (!["image/png", "image/jpeg", "image/webp"].includes(file.type) || file.size > 2_950_000 || file.size === 0)
    throw new Error("Choose a PNG, JPEG or WebP reference up to 2.95 MB.");
  const bytes = new Uint8Array(await file.arrayBuffer());
  let binary = "";
  for (let i = 0; i < bytes.length; i += 8192)
    binary += String.fromCharCode(...bytes.subarray(i, i + 8192));
  const bitmap = await window.createImageBitmap(file);
  const canvas = document.createElement("canvas");
  const ratio = Math.min(1, 480 / Math.max(bitmap.width, bitmap.height));
  canvas.width = Math.max(1, Math.floor(bitmap.width * ratio));
  canvas.height = Math.max(1, Math.floor(bitmap.height * ratio));
  const context = canvas.getContext("2d");
  if (!context) { bitmap.close(); throw new Error("Image preview could not be generated."); }
  let preview = "";
  try {
    for (let attempt = 0; attempt < 4; attempt++) {
      context.drawImage(bitmap, 0, 0, canvas.width, canvas.height);
      preview = canvas.toDataURL("image/jpeg", attempt === 0 ? 0.65 : 0.52);
      if (preview.length <= 175_000) break;
      canvas.width = Math.max(1, Math.floor(canvas.width * 0.65));
      canvas.height = Math.max(1, Math.floor(canvas.height * 0.65));
    }
  } finally { bitmap.close(); }
  if (preview.length > 175_000) throw new Error("Image preview exceeds Cloud's 175 KB limit.");
  return { data: window.btoa(binary), preview };
}

function saveMesh(data: { filename: string; data_base64: string }) {
  const bytes = Uint8Array.from(window.atob(data.data_base64), (character) => character.charCodeAt(0));
  if (bytes.byteLength < 12 || String.fromCharCode(...bytes.subarray(0, 4)) !== "glTF")
    throw new Error("The returned asset is not a valid GLB mesh.");
  const blob = new Blob([Uint8Array.from(bytes).buffer], { type: "model/gltf-binary" });
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = data.filename.replace(/[^a-z0-9._-]/gi, "_").slice(0, 120) || "openmindai-3d.glb";
  document.body.appendChild(anchor);
  anchor.click();
  anchor.remove();
  window.setTimeout(() => URL.revokeObjectURL(url), 3000);
}

const sectionLabels: Record<StudioSection, string> = {
  "research-studio": "Research Studio",
  "3d-studio": "3D Studio",
  "web-search": "Web Search",
};

export function StudioHub({ section, mode, internetAvailable, connection, onConnected, onDisconnect, onLocalPrompt }: Props) {
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [twoFactor, setTwoFactor] = useState("");
  const [token, setToken] = useState("");
  const [advanced, setAdvanced] = useState(false);
  const [showCloudLogin, setShowCloudLogin] = useState(false);
  const [input, setInput] = useState("");
  const [title, setTitle] = useState("");
  const [shapeMode, setShapeMode] = useState<"object" | "architecture">("object");
  const [referenceImage, setReferenceImage] = useState<File | null>(null);
  const [directAsset, setDirectAsset] = useState<number | null>(null);
  const [projects, setProjects] = useState<ResearchProject[]>([]);
  const [selectedProject, setSelectedProject] = useState<number | null>(null);
  const [researchInfo, setResearchInfo] = useState<Record<string, unknown> | null>(null);
  const [job, setJob] = useState<StudioJob | null>(null);
  const [jobTarget, setJobTarget] = useState<"local" | "cloud">("local");
  const [queuedScenes, setQueuedScenes] = useState<QueuedScene[]>([]);
  const [engines, setEngines] = useState<Record<string, unknown> | null>(null);
  const [gpu, setGpu] = useState<{ gpu_active: boolean; gpu?: string; engine?: string } | null>(null);
  const [results, setResults] = useState<Array<{ title?: string; url?: string; snippet?: string; description?: string }>>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const decision = selectExecutionRoute({
    preference: mode, task: section === "3d-studio" ? "scene3d" : section === "research-studio" ? "research" : "search",
    online: internetAvailable, cloudConnected: Boolean(connection?.connected), cloudModelReady: true,
    forceLocal: requestsLocalOnly(input),
  });
  const online = decision.target === "cloud";
  const requireCloudLogin = internetAvailable && !connection?.connected && showCloudLogin;

  const action = useCallback(async (run: () => Promise<void>) => {
    setBusy(true); setError(null); setNotice(null);
    try { await run(); } catch (caught) { setError(explain(caught)); }
    finally { setBusy(false); }
  }, []);

  const refresh = useCallback(async () => {
    if (section === "3d-studio") setQueuedScenes(await cloud.outboxList());
    if (section === "research-studio" && online && connection?.connected) {
      const data = await cloud.researchProjects();
      setProjects(data);
    } else if (section === "3d-studio") {
      if (online && connection?.connected) setEngines(await cloud.studioEngines());
      if (!online) {
        const status = await cloud.startGpu();
        setGpu(status);
        if (!status.gpu_active) setError("OpenCL GPU not ready. Install local worker dependencies and GPU drivers.");
      }
    }
  }, [section, online, connection?.connected]);

  useEffect(() => {
    setError(null);
    setNotice(null);
    setResults([]);
  }, [section]);
  useEffect(() => {
    if ((online && connection?.connected) || (!online && section === "3d-studio")) {
      void refresh().catch((caught: unknown) => setError(explain(caught)));
    }
  }, [refresh, online, section, connection?.connected]);

  // Poll only an actual saved job. Stop when the server reports a terminal status.
  useEffect(() => {
    if (section !== "3d-studio" || !job?.id ||
        !["queued", "running", "pending", "processing", "submitting"].includes(job.status.toLowerCase())) return;
    let live = true;
    const timer = window.setInterval(() => {
      const request = jobTarget === "cloud" ? cloud.studioJob(job.id) : cloud.gpuJob(job.id);
      void request.then((updated) => { if (live) setJob(updated); })
        .catch((caught: unknown) => { if (live) setError(explain(caught)); });
    }, 4000);
    return () => { live = false; window.clearInterval(timer); };
  }, [section, jobTarget, job?.id, job?.status]);

  const signIn = () => action(async () => {
    const connected = advanced ? await cloud.connect(token) : await cloud.login(email, password, twoFactor);
    onConnected(connected);
    setPassword(""); setToken(""); setTwoFactor("");
    setShowCloudLogin(false);
    setNotice("Cloud is connected. Requests can now switch between Cloud and Local.");
  });

  const submit = () => action(async () => {
    const prompt = input.trim();
    if (!prompt) throw new Error("Enter a prompt or search query.");
    if (section === "web-search") {
      if (!online) {
        onLocalPrompt(internetAvailable && !requestsLocalOnly(prompt) ? "[Mode: Web Search]\n" + prompt : offlineSourceNotice(prompt));
        setNotice(internetAvailable ? "Opened local web research." : "Opened local offline analysis. Live sources are unavailable.");
        return;
      }
      try {
        const response = await cloud.search(prompt);
        setResults(response.results ?? []);
        setNotice((response.results ?? []).length ? null : "The search returned no results.");
      } catch (caught) {
        if (!isRecoverableCloudFailure(caught)) throw caught;
        onLocalPrompt(internetAvailable && !requestsLocalOnly(prompt) ? "[Mode: Web Search]\n" + prompt : offlineSourceNotice(prompt));
        setNotice("Cloud search is unavailable. Switched to the local search workflow.");
      }
    } else if (section === "research-studio") {
      if (!online) {
        onLocalPrompt(internetAvailable && !requestsLocalOnly(prompt) ? "[Mode: Deep Research]\n" + prompt : offlineSourceNotice(prompt));
        setNotice(internetAvailable ? "Opened local research workflow." : "Opened local offline research, without live source verification.");
        return;
      }
      const project = await cloud.createResearch(title.trim() || prompt.slice(0, 100), prompt);
      setSelectedProject(project.id);
      await cloud.startResearch(project.id);
      setResearchInfo(await cloud.researchDetail(project.id));
      setProjects(await cloud.researchProjects());
      setNotice("Research run submitted. The Cloud queue controls its execution.");
    } else {
      if (online && referenceImage) {
        const catalog = await cloud.studioEngines();
        const engines = Array.isArray(catalog.data) ? catalog.data as Array<{kind?: string; input?: string; available?: boolean}> : [];
        if (!engines.some((engine) => engine.kind === "model" && engine.input === "image" && engine.available)) {
          setNotice("Cloud image-to-mesh is not available. Your reference image is still selected; retry when a hosted mesh engine becomes ready.");
          return;
        }
        const encoded = await encodeReference(referenceImage);
        const upload = await cloud.uploadStudioImage(referenceImage.name, encoded.data, encoded.preview);
        const created = await cloud.imageToMesh(upload.id);
        if ("asset" in created) {
          setDirectAsset(created.asset.id);
          setJobTarget("cloud");
          setJob(null);
          setNotice("Cloud created a 3D mesh asset. Download the GLB below.");
        } else {
          setJob(created);
          setJobTarget("cloud");
          setDirectAsset(null);
          setNotice("Image-to-mesh job submitted. Refresh or follow the live job status.");
        }
      } else if (referenceImage && !online) {
        setNotice("Reference image preserved. Image-conditioned 3D needs a hosted model; reconnect to Cloud to complete this request. No fake 2.5D fallback was used.");
        return;
      } else {
        // Stage first, so a lost network or missing GPU never discards the scene request.
        const key = crypto.randomUUID();
        const pending = await cloud.queueScene(prompt, shapeMode, key);
        setQueuedScenes(pending);
        const item = pending.find((entry) => entry.requestKey === key);
        if (!item) throw new Error("The 3D request could not be saved to the local outbox.");
        let target: "local" | "cloud" = online ? "cloud" : "local";
        if (target === "cloud") {
          // Read-only preflight: avoid sending a costly Cloud job if connectivity is down.
          try {
            const catalog = await cloud.studioEngines();
            const engines = Array.isArray(catalog.data) ? catalog.data as Array<{id?: string; available?: boolean}> : [];
            const needed = shapeMode === "architecture" ? "architecture" : "procedural";
            if (!engines.some((entry) => entry.id === needed && entry.available === true)) target = "local";
          } catch (caught) {
            if (!isRecoverableCloudFailure(caught)) throw caught;
            target = "local";
          }
        }
        if (target === "local") {
          try {
            const status = await cloud.startGpu();
            setGpu(status);
            if (!status.gpu_active) {
              setNotice("3D request saved locally. No active GPU; resume it after installing the GPU worker or reconnecting to Cloud.");
              return;
            }
          } catch {
            setNotice("3D request saved locally. Neither Cloud nor the local GPU worker is ready; resume it after either engine becomes available.");
            return;
          }
        }
        try {
          const created = await submitQueuedScene(item, target);
          if (created) {
            setJob(created);
            setJobTarget(target);
            setDirectAsset(null);
            setNotice("3D scene accepted by " + (target === "cloud" ? "Cloud" : "the local GPU") + ".");
          }
        } catch {
          setNotice("The 3D submission outcome is uncertain. The same idempotency key was saved; check the pending jobs before retrying.");
        } finally {
          setQueuedScenes(await cloud.outboxList());
        }
      }
    }
  });

  const titleText = sectionLabels[section];
  const Icon = section === "3d-studio" ? Database : section === "research-studio" ? TableProperties : Search;
  return (
    <div className="studio-hub">
      <header className="studio-hero">
        <div className="studio-eyebrow"><Icon size={16} /> OpenMindAI Workspace <span> / </span>{mode === "auto" ? "Auto · " : ""}{online ? "Cloud" : "Local"}</div>
        <h2>{titleText}</h2>
        <p>{section === "research-studio"
          ? "Evidence-led investigations, saved projects and resumable research runs."
          : section === "3d-studio"
            ? "Actual 3D jobs, engine capability checks and progress from the selected processor."
            : "Search the public web with visible source links and no fabricated results."}</p>
        <div className="studio-tags">
          <span><span className={online && connection?.connected ? "studio-dot good" : "studio-dot"} />{online ? connection?.connected ? "Cloud connected" : "Cloud disconnected" : "Local processing"}</span>
          <span>{online ? <Cloud size={13} /> : <Cpu size={13} />}{online ? "Remote compute" : "This computer"}</span>
          <span><ShieldCheck size={13} /> {mode === "auto" ? "Automatic routing" : "Safe fallback available"}</span>
        </div>
      </header>

      {requireCloudLogin ? (
        <section className="studio-panel studio-auth">
          <div className="studio-section-title"><Cloud size={18} /><h3>Connect your OpenMindAI Cloud account</h3></div>
          <p>Use your existing Cloud account. Access is scoped to your own projects and jobs.</p>
          <form onSubmit={(event) => { event.preventDefault(); void signIn(); }}>
            {advanced ? (
              <label>User-session token<input autoComplete="off" type="password" value={token} onChange={(event) => setToken(event.target.value)} placeholder="Paste a user-session token" required /></label>
            ) : (
              <>
                <label>Email<input type="email" autoComplete="username" value={email} onChange={(event) => setEmail(event.target.value)} required /></label>
                <label>Password<input type="password" autoComplete="current-password" value={password} onChange={(event) => setPassword(event.target.value)} required /></label>
                <label>2FA code (if enabled)<input value={twoFactor} onChange={(event) => setTwoFactor(event.target.value)} placeholder="Authenticator or recovery code" /></label>
              </>
            )}
            <div className="studio-actions">
              <button className="studio-primary" type="submit" disabled={busy}>{busy ? <Loader2 size={16} className="studio-spin" /> : <Cloud size={16} />} Connect Cloud</button>
              <button type="button" className="studio-secondary" onClick={() => setAdvanced(!advanced)}>{advanced ? "Use email and password" : "Use existing session token"}</button>
            </div>
          </form>
          <small>Developer API keys cannot access these user-owned Studio endpoints. Credentials never go into localStorage.</small>
        </section>
      ) : (
        <>
          {!connection?.connected && internetAvailable ? (
            <button className="studio-secondary studio-cloud-connect" type="button" onClick={() => setShowCloudLogin(true)}>
              <Cloud size={14} /> Connect Cloud to enable automatic routing
            </button>
          ) : null}
          <div className="studio-toolhead">
            <h3>{section === "web-search" ? "New search" : section === "3d-studio" ? "Create a 3D scene" : "Start an investigation"}</h3>
            <div className="studio-actions">
              <button className="studio-secondary" onClick={() => void action(refresh)} disabled={busy}><RefreshCw size={15} /> Refresh</button>
              {online ? <button className="studio-secondary" onClick={() => void action(async () => { await cloud.disconnect(); onDisconnect(); })}><X size={14} /> Disconnect</button> : null}
            </div>
          </div>
          <section className="studio-panel">
            {section === "research-studio" ? <label>Project title<input value={title} maxLength={200} onChange={(event) => setTitle(event.target.value)} placeholder="e.g. Robustness of UAV swarm coordination" /></label> : null}
            {section === "3d-studio" ? (
              <div className="studio-actions studio-mode-choice">
                <button type="button" className={shapeMode === "object" ? "studio-choice active" : "studio-choice"} onClick={() => setShapeMode("object")}>Object</button>
                <button type="button" className={shapeMode === "architecture" ? "studio-choice active" : "studio-choice"} onClick={() => setShapeMode("architecture")}>Architecture</button>
              </div>
            ) : null}
            {section === "3d-studio" ? (
              <label>Optional reference image for image-to-3D (Cloud model required)
                <input type="file" accept="image/png,image/jpeg,image/webp"
                  onChange={(event) => setReferenceImage(event.currentTarget.files?.[0] ?? null)} />
                <small>{referenceImage ? referenceImage.name : "No image selected. Text prompt creates a 3D scene on either processor."}</small>
              </label>
            ) : null}
            <label>{section === "web-search" ? "Search query" : "Instructions"}
              <textarea rows={4} value={input} onChange={(event) => setInput(event.target.value)}
                placeholder={section === "web-search" ? "What would you like to find?" : section === "3d-studio" ? "Describe a full 3D object or architectural scene…" : "Describe a research question, scope and evidence requirements…"}
                maxLength={section === "web-search" ? 500 : section === "3d-studio" ? 8000 : 16000} />
            </label>
            <div className="studio-actions">
              <button className="studio-primary" disabled={busy || !input.trim()} onClick={() => void submit()}>
                {busy ? <Loader2 size={16} className="studio-spin" /> : section === "web-search" ? <Globe size={16} /> : <Play size={16} />}
                {section === "web-search" ? "Search" : online ? "Run on Cloud" : section === "3d-studio" ? "Run on local GPU" : "Open local Research"}
              </button>
              {!online && section === "3d-studio" ? <span className="studio-muted">{gpu?.gpu_active ? "GPU: " + (gpu.gpu ?? "Available") : "Requires an active GPU worker at 127.0.0.1:8765"}</span> : null}
            </div>
          </section>

          {section === "research-studio" && online ? (
            <section className="studio-panel">
              <div className="studio-section-title"><TableProperties size={17} /><h3>Cloud research projects</h3></div>
              {projects.length === 0 ? <p className="studio-muted">No projects returned for this account.</p> : (
                <div className="studio-list">{projects.map((project) => (
                  <div className="studio-list-item" key={project.id}>
                    <div><strong>{project.title}</strong><small>Project #{project.id} {project.status || ""}</small></div>
                    <button className="studio-secondary" disabled={busy} onClick={() => void action(async () => { setSelectedProject(project.id); setResearchInfo(await cloud.researchDetail(project.id)); })}>Open</button>
                  </div>
                ))}</div>
              )}
              {selectedProject ? (
                <div className="studio-project-controls">
                  <div className="studio-actions">
                    <button className="studio-secondary" disabled={busy} onClick={() => void action(async () => { setResearchInfo(await cloud.researchDetail(selectedProject)); })}><RefreshCw size={14} /> Refresh run</button>
                    {(["pause", "resume", "cancel"] as const).map((command) => (
                      <button key={command} className="studio-secondary" disabled={busy} onClick={() => void action(async () => {
                        await cloud.researchControl(selectedProject, command);
                        setResearchInfo(await cloud.researchDetail(selectedProject));
                      })}>{command === "cancel" ? <Square size={13} /> : null}{command}</button>
                    ))}
                  </div>
                  {researchInfo ? (
                    <div className="studio-run">
                      <strong>Run status: {String((researchInfo.run as { status?: string } | null)?.status ?? "Not started")}</strong>
                      <p>Tasks: {Array.isArray(researchInfo.tasks) ? researchInfo.tasks.length : 0} · Evidence and sources are retrieved from the live project.</p>
                      <details><summary>Project details</summary><pre>{JSON.stringify(researchInfo, null, 2).slice(0, 18000)}</pre></details>
                    </div>
                  ) : null}
                </div>
              ) : null}
            </section>
          ) : null}

          {section === "3d-studio" && queuedScenes.length > 0 ? (
            <section className="studio-panel">
              <div className="studio-section-title"><RefreshCw size={17} /><h3>Saved 3D requests</h3></div>
              <p>Offline requests remain on this computer. Uncertain remote submissions are never replayed automatically.</p>
              <div className="studio-list">
                {queuedScenes.map((item) => (
                  <div className="studio-list-item" key={item.id}>
                    <div>
                      <strong>{item.payload.prompt.slice(0, 100)}</strong>
                      <small>{item.status === "review" ? "Needs review: may already have been submitted" : item.status === "sending" ? "Submission pending confirmation" : "Saved locally, waiting for a processor"}</small>
                      {item.note ? <small>{item.note}</small> : null}
                    </div>
                    <button className="studio-secondary" type="button" disabled={busy || item.status === "sending"} onClick={() => void action(async () => {
                      if (item.status === "review") {
                        await cloud.outboxRetry(item.id);
                      }
                      const target: "cloud" | "local" = internetAvailable && connection?.connected && mode !== "local" && !requestsLocalOnly(item.payload.prompt) ? "cloud" : "local";
                      if (target === "local") {
                        const health = await cloud.startGpu();
                        if (!health.gpu_active) throw new Error("Local GPU unavailable. The request remains saved.");
                      } else {
                        await cloud.studioEngines();
                      }
                      const created = await submitQueuedScene(item, target);
                      if (created) { setJob(created); setJobTarget(target); setNotice("Saved 3D scene resumed on " + target + "."); }
                      setQueuedScenes(await cloud.outboxList());
                    })}>{item.status === "review" ? "Review & retry" : "Resume"}</button>
                  </div>
                ))}
              </div>
            </section>
          ) : null}
          {section === "3d-studio" ? (
            <section className="studio-panel">
              <div className="studio-section-title"><Database size={18} /><h3>3D processor</h3></div>
              {online ? <p>Cloud engine catalog: {engines ? "loaded" : "unavailable"}. Only configured, ready engines can complete jobs.</p>
                : <p>Local GPU: {gpu?.gpu_active ? (gpu.gpu ?? "active") + " · " + (gpu.engine ?? "3D worker") : "not detected"}. A real installed worker is required for GPU mesh generation.</p>}
              {!online ? <button type="button" className="studio-secondary" disabled={busy} onClick={() => void action(async () => {
                await cloud.stopGpu(); setGpu(null); setNotice("Owned local GPU worker stopped.");
              })}>Stop local worker</button> : null}
              {job ? (
                <div className="studio-run">
                  <div className="studio-list-item"><strong>Job {job.id}</strong><span>{job.status}</span></div>
                  <button className="studio-secondary" disabled={busy} onClick={() => void action(async () => setJob(jobTarget === "cloud" ? await cloud.studioJob(job.id) : await cloud.gpuJob(job.id)))}><RefreshCw size={14} /> Refresh job</button>
                  {job.error ? <p role="alert">{job.error}</p> : null}
                  <details><summary>Job result and events</summary><pre>{JSON.stringify(job, null, 2).slice(0, 18000)}</pre></details>
                </div>
              ) : null}
              {(directAsset ?? (job?.result && typeof job.result === "object" && "asset_id" in job.result
                ? Number((job.result as { asset_id: number }).asset_id) : null)) || (jobTarget === "local" && job?.status === "completed") ? (
                <button className="studio-primary studio-download" disabled={busy} onClick={() => void action(async () => {
                  const assetId = directAsset ?? (job?.result && typeof job.result === "object" && "asset_id" in job.result
                    ? Number((job.result as { asset_id: number }).asset_id) : null);
                  const file = jobTarget === "cloud"
                    ? await cloud.downloadMesh(assetId ?? 0)
                    : await cloud.gpuMesh(job?.id ?? "");
                  saveMesh(file);
                  setNotice("Full 3D GLB mesh downloaded.");
                })}>Download 3D mesh (.glb)</button>
              ) : null}
              <small>Job completion and downloadable mesh availability are reported by the backend; no 2.5D image is presented as a full mesh.</small>
            </section>
          ) : null}

          {section === "web-search" && online ? (
            <section className="studio-panel">
              <div className="studio-section-title"><Globe size={17} /><h3>Search results</h3></div>
              {results.map((result, index) => {
                const href = safeWebLink(result.url);
                return <article className="studio-search-result" key={String(index) + (result.url ?? "")}>
                  <strong>{href ? <a href={href} target="_blank" rel="noopener noreferrer">{result.title ?? href}</a> : result.title ?? "Source"}</strong>
                  {href ? <small>{href}</small> : null}
                  <p>{result.snippet ?? result.description}</p>
                </article>;
              })}
              {results.length === 0 ? <p className="studio-muted">Search results appear here after running a query.</p> : null}
            </section>
          ) : null}
        </>
      )}
      {error ? <div className="studio-banner error" role="alert">{error}</div> : null}
      {notice ? <div className="studio-banner" role="status">{notice}</div> : null}
    </div>
  );
}
