import { lazy, Suspense, useCallback, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { open as openFilePicker } from "@tauri-apps/plugin-dialog";
import * as THREE from "three";
import { OBJLoader } from "three/examples/jsm/loaders/OBJLoader.js";
import { STLLoader } from "three/examples/jsm/loaders/STLLoader.js";
import { GLTFLoader } from "three/examples/jsm/loaders/GLTFLoader.js";
import { GLTFExporter } from "three/examples/jsm/exporters/GLTFExporter.js";
import {
  Cpu,
  Database,
  Download,
  ImageUp,
  Loader2,
  Pencil,
  Play,
  RefreshCw,
  RotateCcw,
  Server,
  Sparkles,
  Square,
  Trash2,
  X,
} from "lucide-react";
import { cloud, type CloudConnection, type ExecutionMode, type Studio3DDependencyPlan, type StudioJob } from "../cloudApi";
import { selectExecutionRoute } from "../lib/executionRouting";
import { studio3dProgress } from "../lib/studio3dProgress";
import "../studio3d.css";

type Props = {
  mode: ExecutionMode;
  internetAvailable: boolean;
  connection: CloudConnection | null;
  onConnectCloud: (reason: string) => void;
};

type SceneMode = "object" | "architecture";
type UiMode = "auto" | SceneMode;
type HistoryEntry = {
  id: string;
  target?: "local" | "cloud";
  assetId?: number;
  localAssetId?: string;
  title: string;
  mode: SceneMode;
  prompt: string;
  createdAt: string;
};
type GpuStatus = { gpu_active: boolean; worker_ready?: boolean; cpu_ready?: boolean; geometry_backend?: string; gpu?: string; engine?: string; diagnostics?: { python?: string; python_version?: string; python_bits?: number; numpy?: string | null; pyopencl?: string | null; opencl_platforms?: string[]; opencl_devices?: string[]; error?: string | null } };
type NativeStudioProject = { id: string; name: string; sceneCount: number };
type NativeStudioScene = { id: string; projectId: string; title: string; prompt: string; mode: SceneMode | "imported"; engine: string; revision: number; contentSha256: string };
type LoadedStudioScene = NativeStudioScene & { sceneJson: Record<string, unknown>; glbBase64: string };
type NativeStudioAsset = { id: string; projectId: string; kind: "image" | "mesh"; originalName: string; mediaType: string; sizeBytes: number; contentSha256: string; width: number | null; height: number | null; previewBase64?: string | null };
type LoadedStudioAsset = NativeStudioAsset & { dataBase64: string; previewBase64?: string | null };
type NativeStudioJob = { id: string; projectId: string; generationMode: string; inferenceRoute: string; modelId: string; engineId: string; inputAssetIds: string[]; currentStage: string; status: string; inputSha256: string; outputAssetId: string | null; outputSha256: string; outputObjectCount: number; error: string; recovery: Record<string, unknown>; createdAt: string; updatedAt: string; startedAt: string | null; completedAt: string | null };
type ReferenceImage = { name: string; type: string; data: string; preview: string; assetId?: string };

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
    hint: "Buildings, systems, services and data flows",
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

const reasoningStageLabels: Record<string, string> = {
  "understanding-prompt": "Understanding your request",
  "planning-scene": "Planning your 3D scene",
  "reviewing-prompt-fidelity": "Checking design against your prompt",
  "refining-scene": "Refining missing details",
  "geometry-generation": "Building editable geometry",
};

function readSceneIntent(recovery: Record<string, unknown> | undefined): { summary: string; requirements: string[]; assumptions: string[] } | null {
  const value = recovery?.intent;
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  const intent = value as Record<string, unknown>;
  if (typeof intent.summary !== "string") return null;
  const strings = (items: unknown) => Array.isArray(items) ? items.filter((item): item is string => typeof item === "string") : [];
  return { summary: intent.summary, requirements: strings(intent.requirements), assumptions: strings(intent.assumptions) };
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

async function makeImagePreview(data: string, type: string): Promise<string> {
  const bytes = Uint8Array.from(window.atob(data), (character) => character.charCodeAt(0));
  const bitmap = await window.createImageBitmap(new Blob([bytes], { type }));
  const canvas = document.createElement("canvas");
  const ratio = Math.min(1, 480 / Math.max(bitmap.width, bitmap.height));
  canvas.width = Math.max(1, Math.floor(bitmap.width * ratio));
  canvas.height = Math.max(1, Math.floor(bitmap.height * ratio));
  const context = canvas.getContext("2d");
  if (!context) { bitmap.close(); throw new Error("Image preview could not be generated."); }
  context.drawImage(bitmap, 0, 0, canvas.width, canvas.height);
  bitmap.close();
  const preview = canvas.toDataURL("image/jpeg", 0.62);
  if (preview.length > 175_000) throw new Error("Image preview exceeds the Cloud limit.");
  return preview;
}

async function normalizeMeshAsset(data: string, mediaType: string): Promise<string> {
  if (mediaType === "model/gltf-binary") return data;
  const binary = window.atob(data);
  const bytes = Uint8Array.from(binary, (character) => character.charCodeAt(0));
  let animations: THREE.AnimationClip[] = [];
  let source: THREE.Object3D;
  if (mediaType === "model/gltf+json") {
    const gltf = await new GLTFLoader().parseAsync(new globalThis.TextDecoder("utf-8", { fatal: true }).decode(bytes), "");
    source = gltf.scene;
    animations = gltf.animations;
  } else source = mediaType === "model/obj"
    ? new OBJLoader().parse(new globalThis.TextDecoder("utf-8", { fatal: true }).decode(bytes))
    : new THREE.Mesh(new STLLoader().parse(bytes.buffer), new THREE.MeshStandardMaterial({ color: 0xb8c2cc, metalness: 0.05, roughness: 0.72 }));
  let meshCount = 0;
  source.traverse((object) => {
    if (!(object instanceof THREE.Mesh)) return;
    meshCount++;
    const positions = object.geometry.getAttribute("position");
    if (!positions || positions.count < 3 || positions.count > 6_000_000) throw new Error("Mesh geometry is empty or exceeds the vertex limit.");
    if (!object.geometry.getAttribute("normal")) object.geometry.computeVertexNormals();
    if (!object.geometry.boundingBox) object.geometry.computeBoundingBox();
    const bounds = object.geometry.boundingBox;
    if (!bounds || [bounds.min.x, bounds.min.y, bounds.min.z, bounds.max.x, bounds.max.y, bounds.max.z].some((value) => !Number.isFinite(value)))
      throw new Error("Mesh contains invalid geometry bounds.");
  });
  if (meshCount === 0) throw new Error("Mesh file contains no usable geometry.");
  const result = await new GLTFExporter().parseAsync(source, { binary: true, animations });
  if (!(result instanceof ArrayBuffer)) throw new Error("Mesh conversion did not produce a binary glTF asset.");
  let encoded = "";
  const converted = new Uint8Array(result);
  for (let offset = 0; offset < converted.length; offset += 8192) encoded += String.fromCharCode(...converted.subarray(offset, offset + 8192));
  return window.btoa(encoded);
}

async function encodeReference(file: File): Promise<ReferenceImage> {
  if (!/^(image\/(png|jpeg|webp|bmp))$/.test(file.type) || file.size > 2_950_000 || file.size === 0)
    throw new Error("Choose a PNG, JPEG, WebP or BMP reference image up to 2.95 MB.");
  const bytes = new Uint8Array(await file.arrayBuffer());
  let binary = "";
  for (let index = 0; index < bytes.length; index += 8192)
    binary += String.fromCharCode(...bytes.subarray(index, index + 8192));
  const data = window.btoa(binary);
  const preview = await makeImagePreview(data, file.type);
  if (file.type === "image/bmp") {
    const jpeg = preview.slice("data:image/jpeg;base64,".length);
    return { name: file.name.replace(/\.bmp$/i, ".jpg"), type: "image/jpeg", data: jpeg, preview };
  }
  return { name: file.name, type: file.type, data, preview };
}

const TERMINAL = ["completed", "complete", "done", "success", "failed", "cancelled", "canceled", "error"];
const delay = (ms: number) => new Promise<void>((resolve) => window.setTimeout(resolve, ms));

// Lazy-load the three.js viewport so the heavy renderer is only parsed when a
// scene is actually shown.
const Studio3DViewport = lazy(() =>
  import("./Studio3DViewport").then((module) => ({ default: module.Studio3DViewport })),
);

export function Studio3D({ mode, internetAvailable, connection, onConnectCloud }: Props) {
  const [uiMode, setUiMode] = useState<UiMode>("auto");
  const [furnishingMode, setFurnishingMode] = useState<"prompt" | "unfurnished" | "partially furnished" | "fully furnished">("prompt");
  const [prompt, setPrompt] = useState("");
  const [submittedPrompt, setSubmittedPrompt] = useState<string | null>(null);
  const [referenceImage, setReferenceImage] = useState<ReferenceImage | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [gpu, setGpu] = useState<GpuStatus | null>(null);
  const [dependencyPlan, setDependencyPlan] = useState<Studio3DDependencyPlan | null>(null);
  const [dependencyBusy, setDependencyBusy] = useState(false);
  const [dependencyMessage, setDependencyMessage] = useState<string | null>(null);
  const [neuralCapabilityReport, setNeuralCapabilityReport] = useState<Record<string, unknown> | null>(null);
  const [neuralCapabilityError, setNeuralCapabilityError] = useState<string | null>(null);
  const [autoInstallApproved, setAutoInstallApproved] = useState<string | null>(() => {
    try { return window.localStorage.getItem("openmindai.3d.autoInstallApprovedDependencies"); }
    catch { return null; }
  });
  const [job, setJob] = useState<StudioJob | null>(null);
  const [nativeJob, setNativeJob] = useState<NativeStudioJob | null>(null);
  const [jobTarget, setJobTarget] = useState<"local" | "cloud">("local");
  const [mesh, setMesh] = useState<{ filename: string; data_base64: string } | null>(null);
  const [history, setHistory] = useState<HistoryEntry[]>(loadHistory);
  const [activeId, setActiveId] = useState<string | null>(null);
  const [sceneTitle, setSceneTitle] = useState("");
  const [studioProjects, setStudioProjects] = useState<NativeStudioProject[]>([]);
  const [studioProjectId, setStudioProjectId] = useState<string | null>(null);
  const [savedScenes, setSavedScenes] = useState<NativeStudioScene[]>([]);
  const [projectAssets, setProjectAssets] = useState<NativeStudioAsset[]>([]);
  const [importProgress, setImportProgress] = useState<string | null>(null);
  const [assetPreviews, setAssetPreviews] = useState<Record<string, string>>({});
  const [persistentSceneId, setPersistentSceneId] = useState<string | null>(null);
  const [persistentSceneJson, setPersistentSceneJson] = useState<Record<string, unknown> | undefined>();
  const [persistentProjectId, setPersistentProjectId] = useState<string | null>(null);
  const promptRef = useRef<HTMLTextAreaElement>(null);
  const pending = useRef(false);
  const cancelRef = useRef(false);
  const nativeJobId = useRef<string | null>(null);
  const cloudConnected = Boolean(connection?.connected);

  useEffect(() => {
    void invoke<NativeStudioProject[]>("list_studio3d_projects")
      .then((projects) => {
        setStudioProjects(projects);
        if (projects.length) setStudioProjectId((current) => current ?? projects[0].id);
      })
      .catch((caught: unknown) => setError(explain(caught)));
  }, []);

  useEffect(() => {
    if (!studioProjectId) return;
    let live = true;
    void invoke<NativeStudioScene[]>("list_studio3d_scenes", { projectId: studioProjectId })
      .then((scenes) => { if (live) setSavedScenes(scenes); })
      .catch((caught: unknown) => { if (live) setError(explain(caught)); });
    void invoke<NativeStudioAsset[]>("list_studio3d_assets", { projectId: studioProjectId })
      .then((assets) => {
        if (!live) return;
        setProjectAssets(assets);
        setAssetPreviews(Object.fromEntries(assets.flatMap((asset) =>
          asset.previewBase64 ? [[asset.id, `data:image/png;base64,${asset.previewBase64}`]] : [],
        )));
      })
      .catch((caught: unknown) => { if (live) setError(explain(caught)); });
    void invoke<NativeStudioJob[]>("list_studio3d_jobs", { projectId: studioProjectId })
      .then((jobs) => { if (live && jobs.length) setNativeJob(jobs[0]); })
      .catch((caught: unknown) => { if (live) setError(explain(caught)); });
    let unlisten: (() => void) | undefined;
    void listen<NativeStudioJob>("studio3d://job-progress", ({ payload }) => {
      if (payload.projectId === studioProjectId) setNativeJob(payload);
    }).then((dispose) => { if (live) unlisten = dispose; else dispose(); });
    return () => { live = false; unlisten?.(); };
  }, [studioProjectId]);

  const createStudioProject = useCallback(async () => {
    const name = window.prompt("Name this 3D project", "Untitled 3D project")?.trim();
    if (!name) return;
    try {
      const project = await invoke<NativeStudioProject>("create_studio3d_project", { name });
      setStudioProjects((items) => [project, ...items]);
      setStudioProjectId(project.id);
      setNotice(`Created project: ${project.name}`);
    } catch (caught) {
      setError(explain(caught));
    }
  }, []);

  const importProjectFiles = useCallback(async (droppedPaths?: string[]) => {
    if (!studioProjectId) { setError("Create or select a 3D project before importing assets."); return; }
    const selected = droppedPaths ?? await openFilePicker({
      multiple: true,
      filters: [{ name: "3D Studio assets", extensions: ["jpg", "jpeg", "png", "webp", "bmp", "tif", "tiff", "glb", "gltf", "obj", "stl"] }],
    });
    if (!selected) return;
    const paths = Array.isArray(selected) ? selected : [selected];
    setError(null);
    const hasGltf = paths.some((path) => path.toLowerCase().endsWith(".gltf"));
    const approvedDirectory = hasGltf
      ? await openFilePicker({ title: "Choose approved glTF resource directory", directory: true, recursive: true })
      : null;
    if (hasGltf && typeof approvedDirectory !== "string") {
      setError("glTF import cancelled because no resource directory was approved.");
      return;
    }
    setImportProgress(`Importing 0 of ${paths.length}`);
    let openedMesh = false;
    const failures: string[] = [];
    for (const [index, path] of paths.entries()) {
      setImportProgress(`Importing ${index + 1} of ${paths.length}`);
      const pathParts = path.split(/[\\/]/);
      const fileName = pathParts[pathParts.length - 1] ?? "";
      try {
        const fileParts = fileName.split(".");
        const extension = (fileParts[fileParts.length - 1] ?? "").toLowerCase();
        if (["glb", "gltf", "obj", "stl"].includes(extension)) {
          const asset = extension === "glb"
            ? await invoke<NativeStudioAsset>("import_studio3d_glb", { projectId: studioProjectId, path })
            : extension === "gltf"
              ? await invoke<NativeStudioAsset>("import_studio3d_gltf", { projectId: studioProjectId, path, approvedDirectory })
              : await invoke<NativeStudioAsset>("import_studio3d_mesh", { projectId: studioProjectId, path });
          const loaded = await invoke<LoadedStudioAsset>("load_studio3d_asset", { assetId: asset.id });
          const editableGlb = await normalizeMeshAsset(loaded.dataBase64, asset.mediaType);
          setMesh({ filename: asset.originalName, data_base64: editableGlb });
          setActiveId(asset.id);
          setPersistentSceneId(null);
          setPersistentSceneJson(undefined);
          setPersistentProjectId(null);
          setSceneTitle(asset.originalName);
          setSubmittedPrompt(`Imported mesh: ${asset.originalName}`);
          setPrompt("");
          setNativeJob(null);
          setUiMode("object");
          setJobTarget("local");
          openedMesh = true;
        } else {
          await invoke<NativeStudioAsset>("import_studio3d_image", { projectId: studioProjectId, path });
        }
      } catch (caught) {
        failures.push(`${fileName || "Asset"}: ${explain(caught)}`);
      }
    }
    const assets = await invoke<NativeStudioAsset[]>("list_studio3d_assets", { projectId: studioProjectId });
    setProjectAssets(assets);
    setAssetPreviews(Object.fromEntries(assets.flatMap((asset) =>
      asset.previewBase64 ? [[asset.id, `data:image/png;base64,${asset.previewBase64}`]] : [],
    )));
    if (openedMesh) setNotice("Imported mesh opened in the editor. The original source is stored in this project; save a scene revision to retain edits.");
    setImportProgress(null);
    if (failures.length) setError(failures.join("\n"));
  }, [studioProjectId]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    void getCurrentWindow().onDragDropEvent(({ payload }) => {
      if (payload.type === "drop") void importProjectFiles(payload.paths);
    }).then((dispose) => { unlisten = dispose; });
    return () => unlisten?.();
  }, [importProjectFiles]);

  const handleUseAssetAsReference = useCallback(async (asset: NativeStudioAsset) => {
    try {
      const loaded = await invoke<LoadedStudioAsset>("load_studio3d_asset", { assetId: asset.id });
      if (!loaded.previewBase64) throw new Error("This asset has no image preview.");
      const preview = `data:image/png;base64,${loaded.previewBase64}`;
      const cloudName = loaded.originalName.replace(/\.(bmp|tiff?)$/i, ".png");
      setReferenceImage({ name: cloudName, type: "image/png", data: loaded.previewBase64, preview, assetId: asset.id });
      setAssetPreviews((items) => ({ ...items, [asset.id]: preview }));
      setNotice(`${loaded.originalName} selected as an image reference. It stays local until Cloud generation is started.`);
    } catch (caught) {
      setError(explain(caught));
    }
  }, []);

  const loadTextureAsset = useCallback(async (assetId: string) => {
    const loaded = await invoke<LoadedStudioAsset>("load_studio3d_asset", { assetId });
    if (loaded.kind !== "image") throw new Error("Only project images can be assigned as textures.");
    return { mediaType: loaded.mediaType, dataBase64: loaded.dataBase64 };
  }, []);

  const openSavedScene = useCallback(async (summary: NativeStudioScene) => {
    setBusy(true);
    try {
      const saved = await invoke<LoadedStudioScene>("load_studio3d_scene", { sceneId: summary.id });
      setMesh({ filename: `${saved.title}.glb`, data_base64: saved.glbBase64 });
      setActiveId(saved.id);
      setSceneTitle(saved.title);
      setPersistentSceneId(saved.id);
      setPersistentSceneJson(saved.sceneJson);
      setPersistentProjectId(saved.projectId);
      setSubmittedPrompt(saved.prompt);
      setPrompt("");
      setNativeJob(null);
      setUiMode(saved.mode === "architecture" ? "architecture" : "object");
      setJobTarget(saved.engine.includes("cloud") ? "cloud" : "local");
      setJob(null);
      setNotice(`Opened ${saved.title}, revision ${saved.revision}.`);
    } catch (caught) {
      setError(explain(caught));
    } finally {
      setBusy(false);
    }
  }, []);

  const saveProjectRevision = useCallback(async (glbBase64: string, sceneJson: Record<string, unknown>) => {
    const destinationProjectId = persistentSceneId ? persistentProjectId : studioProjectId;
    if (!destinationProjectId) throw new Error("Create or select a 3D project before saving a revision.");
    const entry = history.find((item) => item.id === activeId);
    const isNewScene = !persistentSceneId;
    const saved = await invoke<NativeStudioScene>("save_studio3d_scene", {
      projectId: destinationProjectId,
      sceneId: persistentSceneId,
      title: entry?.title ?? (sceneTitle || titleFrom(submittedPrompt ?? prompt)),
      prompt: submittedPrompt ?? prompt,
      mode: entry?.mode ?? (uiMode === "architecture" ? "architecture" : "object"),
      engine: jobTarget === "cloud" ? "openmindai-cloud" : gpu?.engine ?? "local-procedural",
      sceneJson,
      glbBase64,
    });
    setPersistentSceneId(saved.id);
    setPersistentSceneJson(sceneJson);
    setPersistentProjectId(destinationProjectId);
    setActiveId(saved.id);
    setSceneTitle(saved.title);
    if (destinationProjectId === studioProjectId) setSavedScenes((items) => [saved, ...items.filter((item) => item.id !== saved.id)]);
    if (isNewScene) setStudioProjects((items) => items.map((item) => item.id === destinationProjectId ? { ...item, sceneCount: item.sceneCount + 1 } : item));
    setNotice(`Saved ${saved.title} as revision ${saved.revision}.`);
  }, [studioProjectId, history, activeId, persistentSceneId, persistentProjectId, prompt, submittedPrompt, uiMode, jobTarget, gpu?.engine, sceneTitle]);
  const visibleSavedScenes = studioProjectId ? savedScenes.filter((scene) => scene.projectId === studioProjectId) : [];
  // The scenes list lives in the app's main left sidebar (not a second inner
  // column). Render it there through a portal so this view fills the container.
  const [sidebarSlot, setSidebarSlot] = useState<HTMLElement | null>(null);
  useEffect(() => {
    // Wait until the sidebar portal target exists after React commits the view.
    const frame = window.requestAnimationFrame(() => {
      setSidebarSlot(document.getElementById("studio-sidebar-slot"));
    });
    return () => window.cancelAnimationFrame(frame);
  }, []);

  // Resize the native textarea after prompt changes without synchronous state updates.
  useEffect(() => {
    const element = promptRef.current;
    if (!element) return;
    element.style.height = "auto";
    element.style.height = `${Math.min(element.scrollHeight, 200)}px`;
  }, [prompt]);

  useEffect(() => {
    if (!notice) return;
    const timeout = window.setTimeout(() => setNotice(null), 3500);
    return () => window.clearTimeout(timeout);
  }, [notice]);

  // Keep the local worker warm so the first generation is not a cold start.
  useEffect(() => {
    const routeToCloud = mode === "cloud" ||
      (mode === "auto" && internetAvailable && Boolean(connection?.connected));
    if (routeToCloud) return;
    cloud
      .startGpu()
      .then(setGpu)
      .catch(() => setGpu({ gpu_active: false }));
  }, [mode, internetAvailable, connection?.connected]);

  const rememberScene = useCallback((entry: HistoryEntry) => {
    setHistory((old) => {
      const next = [entry, ...old.filter((item) => item.id !== entry.id)].slice(0, 40);
      persistHistory(next);
      return next;
    });
  }, []);

  const updateNativeJob = useCallback(async (update: { status: string; currentStage: string; outputAssetId?: string | null; outputSha256?: string; error?: string }) => {
    if (!nativeJobId.current) return;
    const updated = await invoke<NativeStudioJob>("transition_studio3d_job", {
      jobId: nativeJobId.current,
      status: update.status,
      currentStage: update.currentStage,
      outputAssetId: update.outputAssetId ?? null,
      outputSha256: update.outputSha256 ?? "",
      error: update.error ?? "",
      recovery: { providerJobId: job?.id ?? null, lastProviderStatus: job?.status ?? null },
    });
    setNativeJob(updated);
  }, [job?.id, job?.status]);

  const poll = useCallback(async (jobId: string, target: "local" | "cloud"): Promise<StudioJob> => {
    for (let attempt = 0; attempt < 90; attempt++) {
      const updated = target === "cloud" ? await cloud.studioJob(jobId) : await cloud.gpuJob(jobId);
      setJob(updated);
      const providerStatus = updated.status.toLowerCase();
      if (["failed", "error"].includes(providerStatus))
        await updateNativeJob({ status: "failed", currentStage: "provider-failed", error: updated.error || "Provider job failed." });
      else if (["cancelled", "canceled"].includes(providerStatus))
        await updateNativeJob({ status: "cancelled", currentStage: "provider-cancelled" });
      else if (nativeJobId.current)
        await updateNativeJob({ status: "running", currentStage: `provider-${providerStatus}`.slice(0, 80) });
      if (cancelRef.current || TERMINAL.includes(updated.status.toLowerCase())) return updated;
      await delay(2000);
    }
    throw new Error("The local geometry worker did not finish in time.");
  }, [updateNativeJob]);

  const generate = useCallback(async (overridePrompt?: string) => {
    const text = (overridePrompt ?? prompt).trim();
    if (pending.current || text.length < 5) return;
    pending.current = true;
    cancelRef.current = false;
    nativeJobId.current = null;
    setSubmittedPrompt(text);
    setPrompt("");
    setNativeJob(null);
    setActiveId(null);
    setSceneTitle(titleFrom(text));
    setBusy(true);
    setError(null);
    setNotice(null);
    setMesh(null);
    setPersistentSceneId(null);
    setPersistentSceneJson(undefined);
    setPersistentProjectId(null);
    setJob(null);
    try {
      const sceneMode = resolveMode(uiMode, text);
      const requestPrompt = sceneMode === "architecture" && furnishingMode !== "prompt" ? `${text}\nFurnishing preference (physical buildings only): ${furnishingMode}.` : text;
      const requestKey = crypto.randomUUID();
      const route = selectExecutionRoute({
        preference: mode,
        task: referenceImage ? "image-to-3d" : "scene3d",
        online: internetAvailable,
        cloudConnected,
        localAvailable: true,
      });
      if (route.target === "deferred") {
        if (referenceImage && !cloudConnected) {
          const reason = "Image-based 3D creation needs OpenMindAI Cloud. Connect with an API key from api.openmindai.org, then choose Online mode and try again. Your image has not left this device.";
          onConnectCloud(reason);
          throw new Error("Connect OpenMindAI Cloud to create a 3D model from this image. Your image has not left this device.");
        }
        throw new Error(route.reason);
      }
      const target = route.target;
      if (!studioProjectId) throw new Error("Create or select a local 3D project before generating so the job and its output can be saved durably.");
      if (referenceImage && target === "local") throw new Error("Local neural image-to-3D is unavailable because no compatible reconstruction model is installed. Your reference image remains on this device.");
      setJobTarget(target);
      let finished: StudioJob | null = null;
      let cloudAssetId: number | undefined;
      if (target === "cloud" && referenceImage) {
        const catalog = await cloud.studioEngines();
        const engines = Array.isArray(catalog.data)
          ? catalog.data as Array<{ kind?: string; input?: string; available?: boolean }>
          : [];
        if (!engines.some((engine) => engine.kind === "model" && engine.input === "image" && engine.available))
          throw new Error("Cloud image-to-3D is not available right now. Your image was not uploaded; try again when a hosted image model is ready.");
        if (referenceImage.data.length > 3_950_000)
          throw new Error("This image is stored locally but exceeds the Cloud reference-upload limit. Choose a smaller image or use another available engine.");
        if (!window.confirm(`Send ${referenceImage.name} to OpenMindAI Cloud for image-conditioned generation? The image will leave this device.`))
          throw new Error("Cloud image upload cancelled. The image remains stored locally.");
      } else if (target === "cloud") {
        let catalog: Record<string, unknown>;
        try {
          catalog = await cloud.studioEngines();
        } catch {
          throw new Error("Cloud 3D generation is unavailable: the authorized 3D engine catalog could not be read. Use Local generation or choose Cloud reasoning for a scoped scene edit.");
        }
        const engines = Array.isArray(catalog.data) ? catalog.data as Array<{ kind?: string; input?: string; available?: boolean }> : [];
        if (!engines.some((engine) => engine.kind === "model" && engine.input === "text" && engine.available === true))
          throw new Error("Cloud 3D generation is disabled because no available text-to-3D engine is advertised. The separate text reasoning gateway remains available for scoped scene edits.");
      }
      if (cancelRef.current) { setNotice("Generation cancelled."); return; }
      const native = await invoke<NativeStudioJob>("create_studio3d_job", {
        projectId: studioProjectId,
        generationMode: referenceImage ? "image-to-3d" : sceneMode === "architecture" ? "architecture" : "text-to-3d",
        inferenceRoute: target,
        modelId: "",
        engineId: target === "local" ? "native-rust-scheduler/opencl-procedural-adapter" : "openmindai-cloud-studio",
        inputAssetIds: referenceImage?.assetId ? [referenceImage.assetId] : [],
        input: requestPrompt,
      });
      nativeJobId.current = native.id;
      setNativeJob(native);
      if (cancelRef.current) {
        await updateNativeJob({ status: "cancelled", currentStage: "cancelled" });
        setNotice("Generation cancelled.");
        return;
      }
      if (target === "cloud") await updateNativeJob({ status: "running", currentStage: "provider-submission" });
      let nativeOutputAsset: NativeStudioAsset | null = null;
      if (target === "local") {
        nativeOutputAsset = await invoke<NativeStudioAsset>("execute_studio3d_local_job", { jobId: native.id });
      } else if (referenceImage) {
        const upload = await cloud.uploadStudioImage(referenceImage.name, referenceImage.data, referenceImage.preview);
        const created = await cloud.imageToMesh(upload.id, text, requestKey);
        if ("asset" in created) {
          cloudAssetId = created.asset.id;
        } else {
          setJob(created);
          finished = await poll(created.id, "cloud");
        }
      } else {
        const started = target === "cloud"
          ? await cloud.studioScene(requestPrompt, sceneMode, requestKey)
          : await cloud.gpuScene(requestPrompt, sceneMode, requestKey);
        setJob(started);
        finished = await poll(started.id, target);
      }
      if (cancelRef.current) {
        if (target === "local") { setNotice("Stopped waiting for the local scene."); return; }
        const providerCancelled = finished && ["cancelled", "canceled"].includes(finished.status.toLowerCase());
        const nativeStatus = target === "cloud" || !providerCancelled ? "interrupted" : "cancelled";
        await updateNativeJob({
          status: nativeStatus,
          currentStage: nativeStatus === "cancelled" ? "cancelled" : "stopped-waiting",
          error: nativeStatus === "interrupted" ? "The provider did not confirm cancellation. The request will not be replayed automatically." : "",
        });
        setNotice(nativeStatus === "interrupted" ? "Stopped waiting. Provider cancellation was not confirmed; this job will not be replayed automatically." : "Generation cancelled.");
        return;
      }
      if (finished && !["completed", "complete", "done", "success"].includes(finished.status.toLowerCase()))
        throw new Error(finished.error || `The ${target === "cloud" ? "Cloud" : "local geometry"} worker could not build this scene.`);
      let file: { filename: string; data_base64: string };
      if (target === "local") {
        if (!nativeOutputAsset) throw new Error("Native job execution returned no published output asset.");
        const loaded = await invoke<LoadedStudioAsset>("load_studio3d_asset", { assetId: nativeOutputAsset.id });
        file = { filename: loaded.originalName, data_base64: loaded.dataBase64 };
      } else {
        if (!cloudAssetId && finished) {
          const result = (finished.result ?? finished.output ?? {}) as Record<string, unknown>;
          const nestedAssetId = result.asset && typeof result.asset === "object"
            ? (result.asset as { id?: unknown }).id
            : undefined;
          cloudAssetId = Number(finished.asset_id ?? result.asset_id ?? result.assetId ?? nestedAssetId);
        }
        if (!cloudAssetId || !Number.isSafeInteger(cloudAssetId) || cloudAssetId <= 0)
          throw new Error("Cloud finished the scene but did not return a downloadable mesh asset.");
        file = await cloud.downloadMesh(cloudAssetId);
      }
      const outputAsset = nativeOutputAsset ?? await invoke<NativeStudioAsset>("store_studio3d_generated_glb", {
        projectId: studioProjectId,
        filename: file.filename || `${titleFrom(text)}.glb`,
        dataBase64: file.data_base64,
      });
      if (target === "cloud") await updateNativeJob({ status: "completed", currentStage: "published", outputAssetId: outputAsset.id, outputSha256: outputAsset.contentSha256 });
      setProjectAssets((assets) => [outputAsset, ...assets.filter((asset) => asset.id !== outputAsset.id)]);
      setMesh(file);
      const sceneId = finished?.id ?? nativeOutputAsset?.id ?? `asset-${cloudAssetId}`;
      setActiveId(sceneId);
      setSceneTitle(titleFrom(text));
      setPersistentSceneId(null);
      setPersistentSceneJson(undefined);
      setPersistentProjectId(null);
      rememberScene({
        id: sceneId,
        target,
        ...(cloudAssetId ? { assetId: cloudAssetId } : {}),
        ...(target === "local" ? { localAssetId: outputAsset.id } : {}),
        title: titleFrom(text),
        mode: sceneMode,
        prompt: text,
        createdAt: new Date().toISOString(),
      });
      setNotice(`Scene built on ${target === "cloud" ? "OpenMindAI Cloud" : "your computer"} (${sceneMode}).`);
    } catch (caught) {
      if (nativeJobId.current && !cancelRef.current) {
        const message = explain(caught);
        await updateNativeJob({ status: "failed", currentStage: "failed", error: message }).catch(() => undefined);
      }
      if (cancelRef.current) setNotice("Generation stopped.");
      else setError(explain(caught));
    } finally {
      pending.current = false;
      setBusy(false);
    }
  }, [prompt, uiMode, furnishingMode, poll, rememberScene, mode, internetAvailable, cloudConnected, referenceImage, studioProjectId, updateNativeJob, onConnectCloud]);

  const cancelGeneration = useCallback(async () => {
    cancelRef.current = true;
    setNotice("Stopping the current generation…");
    if (nativeJobId.current && jobTarget === "local") {
      await invoke("cancel_studio3d_job", { jobId: nativeJobId.current }).catch(() => undefined);
    }
  }, [jobTarget]);

  const openScene = useCallback(async (entry: HistoryEntry) => {
    setBusy(true);
    setError(null);
    setNotice(null);
    setMesh(null);
    setActiveId(entry.id);
    setSceneTitle(entry.title);
    setPersistentSceneId(null);
    setPersistentSceneJson(undefined);
    setPersistentProjectId(null);
    setSubmittedPrompt(entry.prompt);
    setPrompt("");
    setNativeJob(null);
    setUiMode(entry.mode);
    setJobTarget(entry.target ?? "local");
    try {
      let file: { filename: string; data_base64: string };
      if (entry.target === "cloud") {
        if (!entry.assetId) {
          throw new Error("This Cloud scene no longer has a downloadable mesh reference.");
        }
        file = await cloud.downloadMesh(entry.assetId);
      } else {
        try {
          const loaded = await invoke<LoadedStudioAsset>("load_studio3d_asset", { assetId: entry.localAssetId ?? entry.id });
          file = { filename: loaded.originalName, data_base64: loaded.dataBase64 };
        } catch {
          // Older history entries may still refer to the worker cache.
          if (entry.localAssetId) throw new Error("The saved project asset is unavailable.");
          file = await cloud.gpuMesh(entry.id);
        }
      }
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
        setSceneTitle("");
        setSubmittedPrompt(null);
        setNativeJob(null);
        setMesh(null);
      }
    },
    [activeId],
  );

  const newScene = useCallback(() => {
    setSubmittedPrompt(null);
    setNativeJob(null);
    nativeJobId.current = null;
    setActiveId(null);
    setSceneTitle("");
    setPersistentSceneId(null);
    setPersistentSceneJson(undefined);
    setPersistentProjectId(null);
    setMesh(null);
    setJob(null);
    setPrompt("");
    setReferenceImage(null);
    setUiMode("auto");
    setFurnishingMode("prompt");
    setError(null);
    setNotice(null);
    promptRef.current?.focus();
  }, []);

  const showingResult = Boolean(mesh);
  const inWorkspace = submittedPrompt !== null || showingResult;
  const activeGenerationJob = nativeJob && nativeJob.id === nativeJobId.current ? nativeJob : null;
  const generationProgress = studio3dProgress(activeGenerationJob?.inferenceRoute ?? jobTarget, activeGenerationJob?.currentStage ?? "preparing", showingResult);
  const activeEntry = history.find((item) => item.id === activeId) ?? null;
  const statusLabel = (() => {
    const status = job?.status?.toLowerCase();
    const processor = jobTarget === "cloud" ? "OpenMindAI Cloud" : "your computer";
    if (status === "queued") return `Queued on ${processor}…`;
    if (status === "running") return `Building your scene on ${processor}…`;
    if (status === "cancel_requested") return "Stopping…";
    return jobTarget === "cloud" ? "Preparing the Cloud scene engine…" : "Preparing local geometry…";
  })();
  const gpuLabel = gpu?.worker_ready
    ? gpu.gpu_active
      ? `${gpu.gpu ?? "GPU"} · OpenCL geometry`
      : "CPU geometry ready"
    : "Local geometry worker not detected (127.0.0.1:8765)";
  const reviewDependencyPlan = async () => {
    setDependencyMessage(null);
    setDependencyBusy(true);
    try {
      const plan = await cloud.studio3dDependencyPlan();
      setDependencyPlan(plan);
      if (autoInstallApproved === plan.manifestVersion && plan.supported && plan.state !== "ready")
        await installOptionalDependencies(true, plan);
    } catch (caught) {
      setDependencyMessage(explain(caught));
    } finally {
      setDependencyBusy(false);
    }
  };

  const inspectNeuralCapabilities = async () => {
    setNeuralCapabilityError(null);
    try {
      setNeuralCapabilityReport(await cloud.studio3dNeuralCapabilities());
    } catch (caught) {
      setNeuralCapabilityError(explain(caught));
    }
  };

  const installOptionalDependencies = async (useSavedApproval = false, planOverride?: Studio3DDependencyPlan) => {
    const plan = planOverride ?? dependencyPlan;
    if (!plan?.supported || plan.state === "ready") return;
    const packages = plan.packages.map((item) => `${item.name} ${item.version} (${item.license})`).join("\n");
    const sizeMb = (plan.downloadBytes / (1024 * 1024)).toFixed(1);
    if (!useSavedApproval) {
      const approved = window.confirm(
        `Install optional 3D acceleration packages into an isolated OpenMindAI environment?\n\nPython ${plan.target?.python} · ${plan.target?.architecture}\nDownload: ${sizeMb} MB from files.pythonhosted.org\nPackages:\n${packages}\n\nThis does not change system Python, PATH, or GPU drivers. CPU geometry remains available.`,
      );
      if (!approved) return;
    }
    setDependencyBusy(true);
    setDependencyMessage("Downloading and verifying pinned packages… CPU geometry remains available.");
    try {
      const result = await cloud.installStudio3dDependencies();
      setDependencyMessage(`Verified packages installed. CPU worker smoke test: ${String(result.cpuWorkerSmoke ?? "not reported")}. Checking OpenCL hardware…`);
      const status = await cloud.startGpu();
      setGpu(status as GpuStatus);
      const refreshed = await cloud.studio3dDependencyPlan();
      setDependencyPlan(refreshed);
      setDependencyMessage(status.gpu_active
        ? `OpenCL worker ready: ${status.gpu ?? "compatible device detected"}.`
        : "Packages are installed, but no usable OpenCL device is ready. CPU geometry remains available; GPU drivers were not changed.");
    } catch (caught) {
      setDependencyMessage(explain(caught));
    } finally {
      setDependencyBusy(false);
    }
  };

  const setSavedDependencyApproval = (enabled: boolean) => {
    if (enabled) {
      if (!dependencyPlan?.supported) return;
      const sizeMb = (dependencyPlan.downloadBytes / (1024 * 1024)).toFixed(1);
      const approved = window.confirm(
        `Save permission to automatically install only this reviewed, hash-pinned package set in OpenMindAI's isolated environment when you open the dependency manager?\n\nPython ${dependencyPlan.target?.python} · ${sizeMb} MB maximum from files.pythonhosted.org. No system Python or GPU drivers will be changed.`,
      );
      if (!approved) return;
      window.localStorage.setItem("openmindai.3d.autoInstallApprovedDependencies", dependencyPlan.manifestVersion);
      setAutoInstallApproved(dependencyPlan.manifestVersion);
      if (dependencyPlan.state !== "ready") void installOptionalDependencies(true, dependencyPlan);
    } else {
      window.localStorage.removeItem("openmindai.3d.autoInstallApprovedDependencies");
      setAutoInstallApproved(null);
    }
  };

  const routeToCloud = mode === "cloud" ||
    (mode === "auto" && internetAvailable && Boolean(connection?.connected));

  return (
    <div className="studio3d-root">
      {sidebarSlot &&
        createPortal(
          <div className="studio-side">
            <button className="studio3d-new" type="button" disabled={busy} onClick={newScene}>
              <Pencil size={17} /> New scene
            </button>
            <div className="studio3d-project-picker">
              <label htmlFor="studio3d-project">Project</label>
              <select id="studio3d-project" disabled={busy} value={studioProjectId ?? ""} onChange={(event) => setStudioProjectId(event.target.value || null)}>
                <option value="">Select a project</option>
                {studioProjects.map((project) => <option key={project.id} value={project.id}>{project.name}</option>)}
              </select>
              <button type="button" disabled={busy} onClick={() => void createStudioProject()}>New project</button>
              <button type="button" disabled={!studioProjectId || busy} onClick={() => void importProjectFiles()}>
                <ImageUp size={14} /> Import files
              </button>
              {importProgress ? <span role="status" className="studio3d-import-progress">{importProgress}</span> : null}
            </div>
            <span className="studio3d-section-label">Project assets</span>
            <div className="studio3d-assets">
              {projectAssets.length ? projectAssets.map((asset) => <article className="studio3d-asset" key={asset.id}>
                {asset.kind === "image" && assetPreviews[asset.id] ? <img src={assetPreviews[asset.id]} alt={asset.originalName} /> : null}
                <span title={asset.originalName}>{asset.originalName}</span>
                <small>{asset.kind === "image" ? `${asset.width} × ${asset.height}` : asset.mediaType === "model/obj" ? "OBJ mesh" : asset.mediaType === "model/stl" ? "STL mesh" : asset.mediaType === "model/gltf+json" ? "glTF scene" : "GLB mesh"} · {(asset.sizeBytes / (1024 * 1024)).toFixed(1)} MB</small>
                {asset.kind === "image" ? <button type="button" disabled={busy} onClick={() => void handleUseAssetAsReference(asset)}>Use as reference</button> : <button type="button" disabled={busy} onClick={() => {
                  void invoke<LoadedStudioAsset>("load_studio3d_asset", { assetId: asset.id }).then((loaded) => {
                    void normalizeMeshAsset(loaded.dataBase64, asset.mediaType).then((editableGlb) => setMesh({ filename: asset.originalName, data_base64: editableGlb }));
                    setActiveId(asset.id); setPersistentSceneId(null); setPersistentSceneJson(undefined); setPersistentProjectId(null);
                    setSceneTitle(asset.originalName); setSubmittedPrompt(`Imported mesh: ${asset.originalName}`); setPrompt(""); setNativeJob(null); setUiMode("object");
                  }).catch((caught: unknown) => setError(explain(caught)));
                }}>Open mesh</button>}
              </article>) : <p className="studio3d-empty">{studioProjectId ? "Imported images and GLB models will appear here." : "Select a project to import files."}</p>}
            </div>
            <span className="studio3d-section-label">Saved project scenes</span>
            <div className="studio3d-history">
              {visibleSavedScenes.length ? visibleSavedScenes.map((scene) => <div key={scene.id} className={`studio3d-history-item ${activeId === scene.id ? "active" : ""}`}>
                <button className="studio3d-history-title" title={`${scene.title} · revision ${scene.revision}`} disabled={busy} onClick={() => void openSavedScene(scene)}>{scene.title}<small>Rev {scene.revision}</small></button>
                <button className="studio3d-history-delete" aria-label={`Delete saved scene ${scene.title}`} disabled={busy} onClick={() => {
                  if (!studioProjectId || !window.confirm(`Delete ${scene.title} and all its saved revisions?`)) return;
                  void invoke("delete_studio3d_scene", { projectId: studioProjectId, sceneId: scene.id }).then(() => {
                    setSavedScenes((items) => items.filter((item) => item.id !== scene.id));
                    if (persistentSceneId === scene.id) { setPersistentSceneId(null); setPersistentSceneJson(undefined); setPersistentProjectId(null); setActiveId(null); setSceneTitle(""); setSubmittedPrompt(null); setNativeJob(null); setMesh(null); }
                    setStudioProjects((items) => items.map((item) => item.id === studioProjectId ? { ...item, sceneCount: Math.max(0, item.sceneCount - 1) } : item));
                  }).catch((caught: unknown) => setError(explain(caught)));
                }}><Trash2 size={14} /></button>
              </div>) : <p className="studio3d-empty">{studioProjectId ? "No saved scenes in this project." : "Create a project to save scenes."}</p>}
            </div>
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
          </div>,
          sidebarSlot,
        )}

      <div className={`studio-shell ${showingResult ? "has-result" : inWorkspace ? "has-conversation" : "is-empty"}`}>
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
                {sceneTitle || "3D scene"}
                {activeEntry ? (
                  <span className="studio3d-badge">
                    {activeEntry.mode === "architecture" ? "Architecture" : "3D model"}
                  </span>
                ) : null}
              </h1>
              {submittedPrompt ? <div className="studio3d-result-prompt"><span>You</span><p>{submittedPrompt}</p></div> : null}
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
              <Studio3DViewport
                key={activeId ?? mesh.filename}
                dataBase64={mesh.data_base64}
                onSaveRevision={studioProjectId || persistentProjectId ? saveProjectRevision : undefined}
                sceneJson={persistentSceneJson}
                imageAssets={projectAssets.filter((asset) => asset.kind === "image").map((asset) => ({ id: asset.id, name: asset.originalName }))}
                loadTextureAsset={loadTextureAsset}
              />
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
                disabled={busy || (submittedPrompt ?? prompt).trim().length < 5}
                onClick={() => void generate(submittedPrompt ?? prompt)}
              >
                <RotateCcw size={15} /> Regenerate
              </button>
            </div>
          </div>
        ) : inWorkspace ? (
          <section className="studio3d-conversation" aria-label="3D generation conversation">
            <div className="studio3d-user-turn"><span>You</span><p>{submittedPrompt}</p></div>
            <div className="studio3d-assistant-turn">
              <div className="studio3d-assistant-label"><Sparkles size={16} /> OpenMindAI</div>
              {busy ? (
                <div className="studio3d-generation-card" role="status" aria-live="polite">
                  <div className="studio3d-generation-art" aria-hidden="true">
                    <div className="studio3d-generation-grid" />
                    <div className="studio3d-generation-orb">
                      <svg viewBox="0 0 120 120" fill="none">
                        <path d="M60 12 103 36 103 84 60 108 17 84 17 36Z M60 12V60 M17 36 60 60 103 36 M60 60V108 M17 84 60 60 103 84" stroke="currentColor" strokeWidth="1.5" strokeLinejoin="round"/>
                        <path d="M38 24 82 48 82 96 M82 24 38 48 38 96" stroke="currentColor" strokeWidth="1" opacity=".45"/>
                      </svg>
                    </div>
                    <div className="studio3d-generation-scan" />
                  </div>
                  <div className="studio3d-generation-info">
                    <div className="studio3d-generation-heading"><strong>Generating 3D model</strong><strong className="studio3d-generation-percent">{generationProgress.percent}%</strong></div>
                    <div className="studio3d-generation-track" role="progressbar" aria-label="Completed generation stages" aria-valuemin={0} aria-valuemax={100} aria-valuenow={generationProgress.percent} aria-valuetext={generationProgress.label}>
                      <div style={{ width: `${generationProgress.percent}%` }} />
                    </div>
                    <div className="studio3d-generation-stage"><span>{generationProgress.label}</span><small>Stage progress</small></div>
                    <button type="button" className="studio3d-cancel" onClick={() => void cancelGeneration()}><Square size={13} /> Stop generating</button>
                  </div>
                </div>
              ) : (
                <div className="studio3d-generation-ended">
                  <p>{error ? "The scene could not be generated." : "No new model is displayed."}</p>
                  <button type="button" className="studio3d-cancel" disabled={!submittedPrompt} onClick={() => void generate(submittedPrompt ?? "")}><RotateCcw size={14} /> Try again</button>
                </div>
              )}
            </div>
          </section>
        ) : (
          <div className="studio3d-welcome">
            <h1>What would you like to create?</h1>
            <p className="studio-muted">
              Describe a 3D object or an architecture. Choose Offline or Online in the app header;
              both processors can produce a downloadable GLB mesh.
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
                inWorkspace ? "Describe your next 3D scene…" : uiMode === "object"
                  ? "Describe an object, its materials and proportions"
                  : uiMode === "architecture"
                    ? "Describe buildings, systems, layers or connections"
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
                {resolveMode(uiMode, prompt) === "architecture" ? <label className="studio3d-furnishing">Furnishing
                  <select aria-label="Furnishing mode" value={furnishingMode} disabled={busy} onChange={(event) => setFurnishingMode(event.target.value as typeof furnishingMode)}>
                    <option value="prompt">Follow prompt</option>
                    <option value="unfurnished">Unfurnished</option>
                    <option value="partially furnished">Partially furnished</option>
                    <option value="fully furnished">Fully furnished</option>
                  </select>
                </label> : null}
                <label className="studio3d-attach" title="Reference image is uploaded only when an available Cloud image model is used">
                  <ImageUp size={15} /> Reference image
                  <input
                    type="file"
                    accept="image/png,image/jpeg,image/webp,image/bmp"
                    aria-label="Upload a reference image"
                    disabled={busy}
                    onChange={(event) => {
                      const file = event.currentTarget.files?.[0];
                      if (file) void encodeReference(file).then(setReferenceImage).catch((caught: unknown) => setError(explain(caught)));
                      event.currentTarget.value = "";
                    }}
                  />
                </label>
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

          {referenceImage ? (
            <div className="studio3d-reference" role="status">
              <img src={referenceImage.preview} alt="" /><span><ImageUp size={14} /> {referenceImage.name}</span>
              <button type="button" aria-label="Remove reference image" disabled={busy} onClick={() => setReferenceImage(null)}><X size={14} /></button>
            </div>
          ) : null}
          {referenceImage && mode === "local" ? (
            <p className="studio3d-note">Image-conditioned 3D needs Online mode and an available Cloud image model. Text-only scenes use local CPU geometry with optional OpenCL acceleration.</p>
          ) : null}

          {!inWorkspace && (
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
            <Cpu size={13} /> {routeToCloud ? "OpenMindAI Cloud" : gpuLabel}
            {!routeToCloud && !gpu?.worker_ready ? (
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
          <div className="studio3d-note">
            {routeToCloud
              ? "Online scenes use OpenMindAI Cloud. Reference images need an available Cloud image model."
              : "Offline scenes use local procedural triangle meshes. OpenCL accelerates transforms when its Python package and a compatible driver are ready; CPU geometry remains available."}
          </div>
          {!inWorkspace && !routeToCloud && gpu?.diagnostics ? <details className="studio3d-dependencies">
            <summary>Dependency Manager · Verify Runtime</summary>
            <dl>
              <dt>Python</dt><dd>{gpu.diagnostics.python_version ?? "Unknown"} · {gpu.diagnostics.python_bits ?? "?"}-bit · {gpu.diagnostics.python ?? "path unavailable"}</dd>
              <dt>NumPy</dt><dd>{gpu.diagnostics.numpy ?? "Missing (CPU geometry available)"}</dd>
              <dt>PyOpenCL</dt><dd>{gpu.diagnostics.pyopencl ?? "Missing (optional)"}</dd>
              <dt>OpenCL platforms</dt><dd>{gpu.diagnostics.opencl_platforms?.join(", ") || "None detected"}</dd>
              <dt>OpenCL devices</dt><dd>{gpu.diagnostics.opencl_devices?.join(", ") || "None detected"}</dd>
              <dt>Geometry backend</dt><dd>{gpu.geometry_backend ?? "Unavailable"}</dd>
            </dl>
            {gpu.diagnostics.error ? <p role="status">Optional acceleration unavailable: {gpu.diagnostics.error}. CPU geometry remains ready.</p> : null}
            <p>Optional acceleration packages install only into an isolated, app-managed Python environment after review and approval. GPU drivers are never installed; CPU geometry remains available.</p>
            {!dependencyPlan ? <button type="button" className="studio3d-link" disabled={dependencyBusy} onClick={() => void reviewDependencyPlan()}>
              {dependencyBusy ? "Checking approved package set…" : "Review optional acceleration setup"}
            </button> : null}
            {dependencyPlan && !dependencyPlan.supported ? <p role="status">No reviewed package set supports the selected Python ({dependencyPlan.basePython}) on this system yet. CPU geometry stays available.</p> : null}
            {dependencyPlan?.supported ? <>
              <p>Reviewed set {dependencyPlan.manifestVersion} · Python {dependencyPlan.target?.python} / {dependencyPlan.target?.architecture} · {(dependencyPlan.downloadBytes / (1024 * 1024)).toFixed(1)} MB download · {(dependencyPlan.requiredFreeBytes / (1024 * 1024)).toFixed(0)} MB required · {(dependencyPlan.availableBytes / (1024 * 1024)).toFixed(0)} MB free · files.pythonhosted.org · {dependencyPlan.packages.length} pinned packages.</p>
              <p>Install location: {dependencyPlan.managedEnvironmentPath}</p>
              {dependencyPlan.packages.map((item) => <p key={item.name}>{item.name} {item.version} · {item.license} · {item.reason}</p>)}
              <label className="studio3d-note">
                <input type="checkbox" checked={autoInstallApproved === dependencyPlan.manifestVersion} disabled={dependencyBusy} onChange={(event) => setSavedDependencyApproval(event.target.checked)} />
                Remember approval for this exact package manifest and auto-install only when I open this dependency manager
              </label>
              {dependencyPlan.state === "ready" ? <p role="status">Managed Python environment is installed. OpenCL availability is checked separately from package installation.</p> : <button type="button" className="studio3d-link" disabled={dependencyBusy} onClick={() => void installOptionalDependencies()}>
                {dependencyBusy ? "Installing and verifying…" : dependencyPlan.state === "repair_required" ? "Repair managed environment" : "Install in isolated environment"}
              </button>}
            </> : null}
            {dependencyBusy && dependencyPlan?.state === "not_installed" ? <button type="button" className="studio3d-link" onClick={() => void cloud.cancelStudio3dDependencyInstall().then(() => setDependencyMessage("Cancellation requested. The previous environment will be preserved."))}>
              Cancel installation
            </button> : null}
            {dependencyMessage ? <p role="status">{dependencyMessage}</p> : null}
            <hr />
            <strong>Neural image-to-mesh capability</strong>
            <p>Scene planning uses a local language model. Neural image-to-mesh requires a dedicated engine.</p>
            <button type="button" className="studio3d-link" onClick={() => void inspectNeuralCapabilities()}>
              {neuralCapabilityReport ? "Refresh model and engine status" : "Check model and engine status"}
            </button>
            {neuralCapabilityError ? <p role="status">{neuralCapabilityError}</p> : null}
            {neuralCapabilityReport ? <>
              <p role="status">Local scene planner: {(() => {
                const planner = neuralCapabilityReport.localSceneReasoning as { model?: string; detail?: string } | undefined;
                return [planner?.model, planner?.detail].filter(Boolean).join(" ? ");
              })()}</p>
              <p>Detected device: {String((neuralCapabilityReport.hardware as { operatingSystem?: string } | undefined)?.operatingSystem ?? "unknown OS")} · GPU: {(() => {
                const hardware = neuralCapabilityReport.hardware as { gpus?: Array<{ name?: string; dedicatedVramBytes?: number }> } | undefined;
                return hardware?.gpus?.map((gpu) => `${gpu.name ?? "GPU"}${gpu.dedicatedVramBytes ? ` (${(gpu.dedicatedVramBytes / 1024 ** 3).toFixed(1)} GB dedicated)` : ""}`).join(", ") || "no compatible accelerator detected";
              })()}</p>
              {(Array.isArray(neuralCapabilityReport.engines) ? neuralCapabilityReport.engines : []).map((entry) => {
                const engine = entry as { id: string; name: string; status: string; reason: string; requirements: string; license: string; weightStorage: string; source: string };
                return <div key={engine.id} className="studio3d-neural-engine">
                  <strong>{engine.name} · {engine.status}</strong>
                  <p>{engine.reason}</p>
                  <p>Requirements: {engine.requirements}</p>
                  <p>License: {engine.license} · Weights: {engine.weightStorage}</p>
                  <a href={engine.source} target="_blank" rel="noreferrer">Official source and setup details</a>
                </div>;
              })}
              <p>Recommended next step: use the official TripoSR repository for evaluation; OpenMindAI has not installed or integrated it. Its upstream README states about 6 GB VRAM for the default image path and notes a CPU fallback. This Desktop cannot claim model output until an adapter, reviewed runtime, and real image-to-mesh acceptance are completed.</p>
            </> : null}
          </details> : null}
        </div>

        {busy && !mesh && !inWorkspace ? (
          <div className="studio3d-status" role="status">
            <Loader2 size={15} className="studio3d-spin" />
            <span>{nativeJob?.status === "running" ? reasoningStageLabels[nativeJob.currentStage] ?? `Native job · ${nativeJob.currentStage}` : statusLabel}</span>
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
        {inWorkspace && nativeJob?.inferenceRoute === "local" && readSceneIntent(nativeJob.recovery) ? (
          <details className="studio3d-note">
            <summary>Understood request · {readSceneIntent(nativeJob.recovery)?.summary}</summary>
            {readSceneIntent(nativeJob.recovery)?.requirements.map((requirement, index) => <p key={index}>{requirement}</p>)}
            {readSceneIntent(nativeJob.recovery)?.assumptions.length ? <p>Assumptions: {readSceneIntent(nativeJob.recovery)?.assumptions.join("; ")}</p> : null}
            {nativeJob.recovery.reviewAccepted === true ? <p>Local AI prompt review passed. Inspect the scene for final visual accuracy.</p> : null}
          </details>
        ) : null}
        {inWorkspace && !busy && nativeJob ? (
          <p className="studio3d-note" role="status">
            Saved job: {nativeJob.status} · {nativeJob.currentStage}{nativeJob.status === "interrupted" ? " · restart recovery is manual" : ""}
          </p>
        ) : null}
      </div>
    </div>
  );
}
