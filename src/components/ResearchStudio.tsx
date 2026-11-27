import { useCallback, useEffect, useState } from "react";
import { createPortal } from "react-dom";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open as openFilePicker } from "@tauri-apps/plugin-dialog";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import {
  CheckCircle2,
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
import { cloud, type ExecutionMode, type ResearchDetail, type ResearchEvent, type ResearchProject } from "../cloudApi";
import type { ModelRecord } from "../types";
import { ResearchManuscript } from "./ResearchManuscript";
import "../research-studio.css";

// Research projects and orchestration stay in the desktop database. Cloud is
// available only as an inference route selected for a native run.

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

const PAGE_PRESETS = [6, 8, 10, 12, 15, 20, 24, 25] as const;
const DEFAULT_CUSTOM_PAGE_TARGET = 30;

function planManuscriptPages(pageTarget: number, wordTarget: number) {
  const sections = [
    { name: "Abstract", weight: 5 },
    { name: "Introduction", weight: 15 },
    { name: "Methods", weight: 20 },
    { name: "Results", weight: 25 },
    { name: "Discussion", weight: 25 },
    { name: "Conclusion and limitations", weight: 10 },
  ];
  const minimumPages = pageTarget >= sections.length ? 1 : 0;
  const distributablePages = pageTarget - minimumPages * sections.length;
  const allocations = sections.map((section) => ({
    ...section,
    rawPages: distributablePages * section.weight / 100,
    words: Math.round(wordTarget * section.weight / 100),
  }));
  const floors = allocations.map((section) => Math.floor(section.rawPages));
  let remaining = distributablePages - floors.reduce((sum, pages) => sum + pages, 0);
  const order = allocations
    .map((section, index) => ({ index, fraction: section.rawPages - floors[index] }))
    .sort((a, b) => b.fraction - a.fraction || a.index - b.index);
  for (let index = 0; index < remaining; index += 1) floors[order[index].index] += 1;
  return allocations.map((section, index) => ({ name: section.name, pages: floors[index] + minimumPages, words: section.words }));
}

const CREATE_DRAFT_KEY = "openmindai.researchStudio.createDraft.v1";
type ResearchCreateDraft = {
  title: string;
  prompt: string;
  outputLanguage: string;
  experimentRequested: boolean;
  manuscriptTargetWords: number;
  manuscriptPageTarget: number;
  customPageTarget: number;
  selectedModelId: string;
  selectedResearchRoute: "local" | "cloud" | "auto";
};
const DEFAULT_CREATE_DRAFT: ResearchCreateDraft = {
  title: "",
  prompt: "",
  outputLanguage: "English",
  experimentRequested: false,
  manuscriptTargetWords: 3500,
  manuscriptPageTarget: 24,
  customPageTarget: DEFAULT_CUSTOM_PAGE_TARGET,
  selectedModelId: "",
  selectedResearchRoute: "auto",
};

function readResearchCreateDraft(): ResearchCreateDraft {
  try {
    const raw = window.localStorage.getItem(CREATE_DRAFT_KEY);
    if (!raw) return DEFAULT_CREATE_DRAFT;
    const saved = JSON.parse(raw) as Partial<ResearchCreateDraft>;
    const pageTarget = saved.manuscriptPageTarget;
    const customPageTarget = saved.customPageTarget;
    return {
      title: typeof saved.title === "string" ? saved.title.slice(0, 200) : DEFAULT_CREATE_DRAFT.title,
      prompt: typeof saved.prompt === "string" ? saved.prompt.slice(0, 16000) : DEFAULT_CREATE_DRAFT.prompt,
      outputLanguage: typeof saved.outputLanguage === "string" && LANGUAGES.includes(saved.outputLanguage) ? saved.outputLanguage : DEFAULT_CREATE_DRAFT.outputLanguage,
      experimentRequested: saved.experimentRequested === true,
      manuscriptTargetWords: saved.manuscriptTargetWords === 3000 || saved.manuscriptTargetWords === 4000 ? saved.manuscriptTargetWords : 3500,
      manuscriptPageTarget: Number.isInteger(pageTarget) && Number(pageTarget) >= 1 && Number(pageTarget) <= 100 ? Number(pageTarget) : 24,
      customPageTarget: Number.isInteger(customPageTarget) && Number(customPageTarget) >= 1 && Number(customPageTarget) <= 100 ? Number(customPageTarget) : 30,
      selectedModelId: typeof saved.selectedModelId === "string" ? saved.selectedModelId : "",
      selectedResearchRoute: saved.selectedResearchRoute === "local" || saved.selectedResearchRoute === "cloud" ? saved.selectedResearchRoute : "auto",
    };
  } catch {
    return DEFAULT_CREATE_DRAFT;
  }
}

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

function researchDocumentPreview(metadataJson: string): string | null {
  try {
    const metadata = JSON.parse(metadataJson) as { preview?: unknown };
    return typeof metadata.preview === "string" ? metadata.preview : null;
  } catch {
    return null;
  }
}

function researchDocumentSourceLabels(metadataJson: string): string[] {
  try {
    const metadata = JSON.parse(metadataJson) as { sourceLabels?: unknown };
    return Array.isArray(metadata.sourceLabels)
      ? metadata.sourceLabels.filter((label): label is string => typeof label === "string" && Boolean(label.trim()))
      : [];
  } catch {
    return [];
  }
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
type LocalResearchProject = {
  id: string;
  title: string;
  prompt: string;
  output_language: string;
  status: string;
  created_at: string;
  updated_at: string;
  experiment_requested: boolean;
  manuscript_target_words: number;
  manuscript_page_target: number;
  revision: number;
};
type LocalResearchTitleJob = {
  id: string;
  projectId: string;
  modelId: string;
  status: string;
  proposedTitle: string | null;
  acceptedTitle: string | null;
  error: string | null;
  startedAt: string;
  completedAt: string | null;
};
type LocalResearchDocument = {
  id: string;
  name: string;
  mediaType: string;
  sizeBytes: number;
  contentSha256: string;
  chunkCount: number;
  importedAt: string;
  extractionMetadataJson: string;
};
type LocalResearchFigureAsset = {
  id: string;
  projectId: string;
  name: string;
  mediaType: string;
  sizeBytes: number;
  contentSha256: string;
  widthPx: number;
  heightPx: number;
  caption: string;
  altText: string;
  importedAt: string;
};
type LocalResearchRun = {
  id: string;
  projectId: string;
  conversationId: string;
  modelId: string;
  status: string;
  error: string | null;
  startedAt: string;
  updatedAt: string;
  completedAt: string | null;
};
type LocalResearchTask = {
  id: string;
  runId: string;
  agent: string;
  title: string;
  status: string;
  attempt: number;
  result: string | null;
  error: string | null;
  cloudProvider?: string | null;
  cloudRequestState?: string | null;
  cloudResponseError?: string | null;
  cloudClientRequestId?: string | null;
  cloudUpstreamRequestId?: string | null;
};
type LocalResearchEvent = {
  eventId: number;
  projectId: string;
  runId: string;
  taskId: string | null;
  agent: string | null;
  type: string;
  status: string | null;
  timestamp: string;
  payload: Record<string, unknown>;
};
type LocalManuscriptRevision = { id: string; revision: number; content: string; contentHash: string; status: string; createdAt: string };
type LocalResearchRunDetails = {
  run: LocalResearchRun;
  tasks: LocalResearchTask[];
  dependencies: Array<[string, string]>;
  events: LocalResearchEvent[];
  sources: Array<{ id: string; url: string; title: string; metadata: Record<string, unknown>; discoveredAt: string }>;
  evidence: Array<{ id: string; sourceId: string | null; claim: string; quotation: string | null; verificationStatus: string; provenance: Record<string, unknown>; createdAt: string }>;
  manuscript: LocalManuscriptRevision | null;
  manuscriptRevisions: LocalManuscriptRevision[];
};
const LOCAL_AGENT_GRAPH: Record<string, { x: number; y: number; label: string }> = {
  planning: { x: 30, y: 55, label: "Planning" },
  discovery: { x: 220, y: 55, label: "Discovery" },
  literature: { x: 420, y: 12, label: "Literature" },
  verification: { x: 420, y: 98, label: "Verification" },
  investigation: { x: 625, y: 12, label: "Investigation" },
  gap_analysis: { x: 830, y: 12, label: "Gap analysis" },
  citation_manager: { x: 625, y: 98, label: "Citations" },
  experiment_planning: { x: 830, y: 98, label: "Experiment plan" },
  manuscript_section_abstract: { x: 1035, y: 0, label: "Abstract" },
  manuscript_section_introduction: { x: 1035, y: 48, label: "Introduction" },
  manuscript_section_related_work: { x: 1035, y: 96, label: "Related work" },
  manuscript_section_methods: { x: 1035, y: 144, label: "Methods" },
  manuscript_section_findings: { x: 1035, y: 192, label: "Findings" },
  manuscript_section_discussion: { x: 1035, y: 240, label: "Discussion" },
  manuscript_section_conclusion: { x: 1035, y: 288, label: "Conclusion" },
  manuscript_assembly: { x: 1240, y: 144, label: "Assemble" },
  quality: { x: 1450, y: 144, label: "Quality" },
};
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
  onRunStarted,
  onOpenModelManager,
  initialRunId,
  onNewResearch,
}: {
  connected: boolean;
  mode: ExecutionMode;
  onConnect: () => void;
  onRunStarted: (runId: string) => void;
  onOpenModelManager: () => void;
  initialRunId: string | null;
  onNewResearch: () => void;
}) {
  const [projects, setProjects] = useState<ResearchProject[]>([]);
  const [localProjects, setLocalProjects] = useState<LocalResearchProject[]>([]);
  const [pendingDocumentPaths, setPendingDocumentPaths] = useState<string[]>([]);
  const [documentImportProgress, setDocumentImportProgress] = useState<string | null>(null);
  const [documentDropActive, setDocumentDropActive] = useState(false);
  const [selectedLocalProjectId, setSelectedLocalProjectId] = useState<string | null>(null);
  const [researchProjectConflict, setResearchProjectConflict] = useState(false);
  const [localDocumentsState, setLocalDocumentsState] = useState<{ projectId: string; documents: LocalResearchDocument[] } | null>(null);
  const localDocuments = localDocumentsState?.projectId === selectedLocalProjectId ? localDocumentsState.documents : [];
  const [localFigureAssetsState, setLocalFigureAssetsState] = useState<{ projectId: string; assets: LocalResearchFigureAsset[] } | null>(null);
  const localFigureAssets = localFigureAssetsState?.projectId === selectedLocalProjectId ? localFigureAssetsState.assets : [];
  const [localRunsState, setLocalRunsState] = useState<{ projectId: string; runs: LocalResearchRun[] } | null>(null);
  const localRuns = localRunsState?.projectId === selectedLocalProjectId ? localRunsState.runs : [];
  const setLocalRuns = useCallback((change: LocalResearchRun[] | ((current: LocalResearchRun[]) => LocalResearchRun[])) => {
    if (!selectedLocalProjectId) return;
    setLocalRunsState((current) => {
      const previous = current?.projectId === selectedLocalProjectId ? current.runs : [];
      const runs = typeof change === "function" ? change(previous) : change;
      return { projectId: selectedLocalProjectId, runs };
    });
  }, [selectedLocalProjectId]);
  const [selectedLocalRunId, setSelectedLocalRunId] = useState<string | null>(null);
  const [runWorkspace, setRunWorkspace] = useState(Boolean(initialRunId));
  const [changeRequest, setChangeRequest] = useState("");
  const [changeBusy, setChangeBusy] = useState(false);
  const [startingResearch, setStartingResearch] = useState(false);
  const [localRunDetailsState, setLocalRunDetailsState] = useState<LocalResearchRunDetails | null>(null);
  const localRunDetails = localRunDetailsState?.run.id === selectedLocalRunId && localRunDetailsState.run.projectId === selectedLocalProjectId
    ? localRunDetailsState
    : null;
  const setLocalRunDetails = useCallback((detail: LocalResearchRunDetails) => setLocalRunDetailsState(detail), []);
  const [localModels, setLocalModels] = useState<ModelRecord[]>([]);
  const [titleJob, setTitleJob] = useState<LocalResearchTitleJob | null>(null);
  const [titleProposalDraft, setTitleProposalDraft] = useState("");
  const [titleProposalExpectedTitle, setTitleProposalExpectedTitle] = useState("");
  const [titleInferenceActive, setTitleInferenceActive] = useState(false);
  const [titleInferenceProjectId, setTitleInferenceProjectId] = useState<string | null>(null);
  const [initialCreateDraft] = useState(readResearchCreateDraft);
  const [selectedModelId, setSelectedModelId] = useState(initialCreateDraft.selectedModelId);
  const [selectedResearchRoute, setSelectedResearchRoute] = useState<"local" | "cloud" | "auto">(initialCreateDraft.selectedResearchRoute);
  const [cloudResearchModel, setCloudResearchModel] = useState<string | null>(null);
  const manuscriptKey = localRunDetails?.manuscript
    ? `${localRunDetails.run.id}:${localRunDetails.manuscript.id}:${localRunDetails.manuscript.contentHash}`
    : "";
  const [manuscriptDraftState, setManuscriptDraftState] = useState<{ key: string; content: string } | null>(null);
  const manuscriptDraft = manuscriptDraftState?.key === manuscriptKey
    ? manuscriptDraftState.content
    : localRunDetails?.manuscript?.content ?? "";
  const setManuscriptDraft = (content: string) => setManuscriptDraftState({ key: manuscriptKey, content });
  const [editingManuscriptKey, setEditingManuscriptKey] = useState<string | null>(null);
  const editingManuscript = Boolean(manuscriptKey && editingManuscriptKey === manuscriptKey);
  const setEditingManuscript = (editing: boolean) => setEditingManuscriptKey(editing ? manuscriptKey : null);
  const [selectedTaskId, setSelectedTaskId] = useState<string | null>(null);
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
  const [title, setTitle] = useState(initialCreateDraft.title);
  const [prompt, setPrompt] = useState(initialCreateDraft.prompt);
  const [outputLanguage, setOutputLanguage] = useState(initialCreateDraft.outputLanguage);
  const [experimentRequested, setExperimentRequested] = useState(initialCreateDraft.experimentRequested);
  const [manuscriptTargetWords, setManuscriptTargetWords] = useState<number>(initialCreateDraft.manuscriptTargetWords);
  const [manuscriptPageTarget, setManuscriptPageTarget] = useState(initialCreateDraft.manuscriptPageTarget);
  const [customPageTarget, setCustomPageTarget] = useState(initialCreateDraft.customPageTarget);
  const [customPageTargetInput, setCustomPageTargetInput] = useState(String(initialCreateDraft.customPageTarget));
  const [pageTargetError, setPageTargetError] = useState<string | null>(null);

  useEffect(() => {
    try {
      const draft: ResearchCreateDraft = {
        title,
        prompt,
        outputLanguage,
        experimentRequested,
        manuscriptTargetWords,
        manuscriptPageTarget,
        customPageTarget,
        selectedModelId,
        selectedResearchRoute,
      };
      window.localStorage.setItem(CREATE_DRAFT_KEY, JSON.stringify(draft));
    } catch {
      // Keep editing available when the WebView profile is read-only or full.
    }
  }, [title, prompt, outputLanguage, experimentRequested, manuscriptTargetWords, manuscriptPageTarget, customPageTarget, selectedModelId, selectedResearchRoute]);

  useEffect(() => {
    if (!initialRunId) return;
    setRunWorkspace(true);
    let alive = true;
    void invoke<LocalResearchRunDetails>("local_research_run_details", { runId: initialRunId }).then((detail) => {
      if (!alive) return;
      setLocalRunDetails(detail);
      setSelectedLocalRunId(detail.run.id);
      setSelectedLocalProjectId(detail.run.projectId);
      setLocalRunsState((runs) => ({
        projectId: detail.run.projectId,
        runs: [detail.run, ...(runs?.projectId === detail.run.projectId ? runs.runs : []).filter((run) => run.id !== detail.run.id)],
      }));
      void invoke<LocalResearchProject[]>("list_local_research_projects").then((items) => {
        if (!alive) return;
        setLocalProjects(items);
        const project = items.find((item) => item.id === detail.run.projectId);
        if (project) {
          setExperimentRequested(project.experiment_requested);
          setManuscriptTargetWords(project.manuscript_target_words ?? 3500);
          setManuscriptPageTarget(project.manuscript_page_target ?? 24);
          if (project.manuscript_page_target && !PAGE_PRESETS.includes(project.manuscript_page_target as typeof PAGE_PRESETS[number])) {
            setCustomPageTarget(project.manuscript_page_target);
            setCustomPageTargetInput(String(project.manuscript_page_target));
          }
        }
      });
    }).catch((caught: unknown) => alive && setError(explain(caught)));
    return () => { alive = false; };
  }, [initialRunId, setLocalRunDetails]);

  // The projects list lives in the app's main left sidebar (portal target).
  const [sidebarSlot] = useState(() => document.getElementById("studio-sidebar-slot"));

  const active = projects.find((project) => project.id === activeId) ?? null;
  // Never send project, run, task, or manuscript operations to the hosted
  // Research Studio API. The selected route below controls inference only.
  const useCloud = false;

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
    let alive = true;
    void invoke<LocalResearchProject[]>("list_local_research_projects")
      .then((items) => {
        if (!alive) return;
        setLocalProjects(items);
        setSelectedLocalProjectId((selected) => selected ?? items[0]?.id ?? null);
        const initialProject = items[0];
        if (initialProject) {
          setExperimentRequested(initialProject.experiment_requested);
          setManuscriptTargetWords(initialProject.manuscript_target_words ?? 3500);
          setManuscriptPageTarget(initialProject.manuscript_page_target ?? 24);
          if (initialProject.manuscript_page_target && !PAGE_PRESETS.includes(initialProject.manuscript_page_target as typeof PAGE_PRESETS[number])) {
            setCustomPageTarget(initialProject.manuscript_page_target);
            setCustomPageTargetInput(String(initialProject.manuscript_page_target));
          }
        }
      })
      .catch((caught: unknown) => { if (alive) setError(explain(caught)); });
    void invoke<ModelRecord[]>("discover_models")
      .then((models) => {
        if (!alive) return;
        // Keep the full registry result visible so stale, invalid, and
        // unvalidated entries are not silently presented as ready.
        setLocalModels(models);
        const ready = models.filter((item) => item.enabled && ["installed", "ready", "loaded"].includes(item.state));
        setSelectedModelId((selected) => ready.some((item) => item.id === selected) ? selected : ready[0]?.id ?? "");
      })
      .catch((caught: unknown) => { if (alive) setError(explain(caught)); });
    return () => { alive = false; };
  }, []);

  useEffect(() => {
    if (!selectedLocalProjectId) return;
    let alive = true;
    void invoke<LocalResearchTitleJob | null>("get_local_research_title_job", { projectId: selectedLocalProjectId })
      .then((job) => {
        if (!alive) return;
        setTitleJob(job);
        setTitleProposalDraft(job?.proposedTitle ?? job?.acceptedTitle ?? "");
        setTitleProposalExpectedTitle(localProjects.find((project) => project.id === selectedLocalProjectId)?.title ?? "");
      })
      .catch((caught: unknown) => alive && setError(explain(caught)));
    return () => { alive = false; };
  }, [selectedLocalProjectId, localProjects]);

  useEffect(() => {
    if (!selectedLocalProjectId) return;
    let alive = true;
    void invoke<LocalResearchDocument[]>("list_local_research_documents", { projectId: selectedLocalProjectId })
      .then((documents) => alive && setLocalDocumentsState({ projectId: selectedLocalProjectId, documents }))
      .catch((caught: unknown) => alive && setError(explain(caught)));
    return () => { alive = false; };
  }, [selectedLocalProjectId]);

  useEffect(() => {
    if (!selectedLocalProjectId) return;
    let alive = true;
    void invoke<LocalResearchFigureAsset[]>("list_local_research_figure_assets", { projectId: selectedLocalProjectId })
      .then((assets) => alive && setLocalFigureAssetsState({ projectId: selectedLocalProjectId, assets }))
      .catch((caught: unknown) => alive && setError(explain(caught)));
    return () => { alive = false; };
  }, [selectedLocalProjectId]);

  useEffect(() => {
    if (useCloud || !selectedLocalProjectId) return;
    let alive = true;
    void invoke<LocalResearchRun[]>("list_local_research_runs", { projectId: selectedLocalProjectId })
      .then((runs) => {
        if (!alive) return;
        setLocalRuns(runs);
        setSelectedLocalRunId((selected) => runs.some((run) => run.id === selected) ? selected : runs[0]?.id ?? null);
      })
      .catch((caught: unknown) => { if (alive) setError(explain(caught)); });
    return () => { alive = false; };
  }, [useCloud, selectedLocalProjectId, setLocalRuns]);

  useEffect(() => {
    if (useCloud || !selectedLocalRunId) return;
    let alive = true;
    let timer: number | null = null;
    let unlisten: (() => void) | null = null;
    const refresh = async () => {
      try {
        const detail = await invoke<LocalResearchRunDetails>("local_research_run_details", { runId: selectedLocalRunId });
        if (!alive) return;
        setLocalRunDetails(detail);
        setLocalRuns((runs) => [detail.run, ...runs.filter((run) => run.id !== detail.run.id)]);
        if (LIVE.has(detail.run.status)) timer = window.setTimeout(() => void refresh(), 1000);
      } catch (caught) {
        if (alive) setError(explain(caught));
      }
    };
    void refresh();
    void listen<LocalResearchEvent>("research:event", (event) => {
      if (event.payload.runId === selectedLocalRunId) void refresh();
    }).then((stop) => { if (alive) unlisten = stop; else stop(); });
    return () => {
      alive = false;
      if (timer != null) window.clearTimeout(timer);
      unlisten?.();
    };
  }, [useCloud, selectedLocalRunId, setLocalRunDetails, setLocalRuns]);

  const refreshCurrentRun = useCallback(() => {
    if (!selectedLocalRunId) return;
    void invoke<LocalResearchRunDetails>("local_research_run_details", { runId: selectedLocalRunId })
      .then((detail) => {
        setLocalRunDetails(detail);
        setLocalRuns((runs) => [detail.run, ...runs.filter((run) => run.id !== detail.run.id)]);
      })
      .catch((caught: unknown) => setError(explain(caught)));
  }, [selectedLocalRunId, setLocalRunDetails, setLocalRuns]);

  // Load and poll the active project's detail + events while the run is live.
  useEffect(() => {
    if (!useCloud || activeId == null) return;
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
  }, [useCloud, activeId, pollTick]);

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

  const persistLocalResearchDraft = useCallback(async () => {
    const question = prompt.trim();
    if (question.length < 5) throw new Error("Describe your research question (at least a few words).");
    if (pageTargetError) throw new Error(pageTargetError);
    const existing = localProjects.find((item) => item.id === selectedLocalProjectId)
      ?? localProjects.find((item) => item.prompt === question);
    const projectTitle = title.trim() || existing?.title || "Untitled research";
    const args = {
      title: projectTitle,
      prompt: question,
      outputLanguage,
      experimentRequested,
      manuscriptTargetWords,
      manuscriptPageTarget,
    };
    let project: LocalResearchProject;
    try {
      project = existing
        ? await invoke<LocalResearchProject>("update_local_research_project", { projectId: existing.id, expectedRevision: existing.revision, ...args })
        : await invoke<LocalResearchProject>("create_local_research_project", args);
    } catch (caught) {
      const message = explain(caught);
      if (/changed in another session|valid project revision|project was not found/i.test(message)) setResearchProjectConflict(true);
      throw caught;
    }
    setLocalProjects((items) => [project, ...items.filter((item) => item.id !== project.id)]);
    setSelectedLocalProjectId(project.id);
    setResearchProjectConflict(false);
    return project;
  }, [prompt, title, pageTargetError, localProjects, selectedLocalProjectId, outputLanguage, experimentRequested, manuscriptTargetWords, manuscriptPageTarget]);

  const reloadConflictedResearchProject = useCallback(async () => {
    if (!selectedLocalProjectId) return;
    setBusy(true);
    try {
      const items = await invoke<LocalResearchProject[]>("list_local_research_projects");
      setLocalProjects(items);
      const saved = items.find((item) => item.id === selectedLocalProjectId);
      if (!saved) throw new Error("This project no longer exists. The project list has been refreshed.");
      setTitle(saved.title);
      setPrompt(saved.prompt);
      setOutputLanguage(saved.output_language || "English");
      setExperimentRequested(saved.experiment_requested);
      setManuscriptTargetWords(saved.manuscript_target_words ?? 3500);
      const pages = saved.manuscript_page_target ?? 24;
      setManuscriptPageTarget(pages);
      if (!PAGE_PRESETS.includes(pages as typeof PAGE_PRESETS[number])) {
        setCustomPageTarget(pages);
        setCustomPageTargetInput(String(pages));
      }
      setPageTargetError(null);
      setResearchProjectConflict(false);
      setError(null);
      setNotice(`Reloaded the saved project at revision ${saved.revision}.`);
    } catch (caught) {
      setError(explain(caught));
    } finally {
      setBusy(false);
    }
  }, [selectedLocalProjectId]);

  const saveLocalResearchDraft = useCallback(() => {
    setBusy(true);
    setError(null);
    void persistLocalResearchDraft()
      .then(() => setNotice("Research draft and target page count saved on this device."))
      .catch((caught: unknown) => setError(explain(caught)))
      .finally(() => setBusy(false));
  }, [persistLocalResearchDraft]);

  const generateLocalTitleProposal = useCallback(async () => {
    setBusy(true);
    setTitleInferenceActive(true);
    setError(null);
    try {
      const project = await persistLocalResearchDraft();
      setTitleInferenceProjectId(project.id);
      const job = await invoke<LocalResearchTitleJob>("suggest_local_research_title", {
        projectId: project.id,
        modelId: selectedModelId || null,
      });
      setTitleJob(job);
      setTitleProposalDraft(job.proposedTitle ?? "");
      setTitleProposalExpectedTitle(project.title);
      setNotice("Local model title proposal is ready for review. It has not changed the saved project title.");
    } catch (caught) {
      setError(explain(caught));
    } finally {
      setTitleInferenceActive(false);
      setTitleInferenceProjectId(null);
      setBusy(false);
    }
  }, [persistLocalResearchDraft, selectedModelId]);

  const cancelLocalTitleProposal = useCallback(async () => {
    if (!titleInferenceProjectId) return;
    try {
      await invoke("cancel_local_research_title_job", { projectId: titleInferenceProjectId });
      const job = await invoke<LocalResearchTitleJob | null>("get_local_research_title_job", { projectId: titleInferenceProjectId });
      setTitleJob(job);
      setNotice("Local title inference was cancelled. Partial output was not saved as a proposal.");
    } catch (caught) {
      setError(explain(caught));
    }
  }, [titleInferenceProjectId]);

  const acceptLocalTitleProposal = useCallback(async () => {
    if (!titleJob || titleJob.status !== "proposal") return;
    setBusy(true);
    setError(null);
    try {
      const project = await invoke<LocalResearchProject>("accept_local_research_title", {
        jobId: titleJob.id,
        acceptedTitle: titleProposalDraft,
        expectedCurrentTitle: titleProposalExpectedTitle,
      });
      setTitle(project.title);
      setLocalProjects((items) => [project, ...items.filter((item) => item.id !== project.id)]);
      setTitleJob({ ...titleJob, status: "accepted", acceptedTitle: project.title });
      setNotice("The reviewed title was saved with this project.");
    } catch (caught) {
      setError(explain(caught));
    } finally {
      setBusy(false);
    }
  }, [titleJob, titleProposalDraft, titleProposalExpectedTitle]);

  const startLocalResearch = useCallback(() => {
    const question = prompt.trim();
    if (question.length < 5) {
      setError("Describe your research question (at least a few words).");
      return;
    }
    setBusy(true);
    setError(null);
    setStartingResearch(true);
    setRunWorkspace(true);
    void (async () => {
      try {
        const project = await persistLocalResearchDraft();
        for (let index = 0; index < pendingDocumentPaths.length; index += 1) {
          setDocumentImportProgress(`Validating and indexing ${index + 1} of ${pendingDocumentPaths.length}…`);
          await invoke<LocalResearchDocument>("import_local_research_document", { projectId: project.id, path: pendingDocumentPaths[index] });
        }
        if (pendingDocumentPaths.length) {
          const documents = await invoke<LocalResearchDocument[]>("list_local_research_documents", { projectId: project.id });
          setLocalDocumentsState({ projectId: project.id, documents });
          setPendingDocumentPaths([]);
        }
        setDocumentImportProgress(null);
        setSelectedLocalProjectId(project.id);
        const run = await invoke<LocalResearchRun>("start_local_research_run", {
          projectId: project.id,
          modelId: selectedModelId || null,
          executionMode: selectedResearchRoute,
          manuscriptTargetWords,
        });
        setLocalRunsState((current) => ({
          projectId: project!.id,
          runs: [run, ...(current?.projectId === project!.id ? current.runs : []).filter((item) => item.id !== run.id)],
        }));
        setSelectedLocalRunId(run.id);
        onRunStarted(run.id);
        setNotice(`${selectedResearchRoute === "auto" ? "Auto" : selectedResearchRoute === "cloud" ? "Cloud" : "Local"} research run started. Each task's provider and results are persisted in the native run history.`);
      } catch (caught) {
        setDocumentImportProgress(null);
        setError(explain(caught));
        setRunWorkspace(false);
      } finally {
        setBusy(false);
        setStartingResearch(false);
      }
    })();
  }, [prompt, persistLocalResearchDraft, pendingDocumentPaths, selectedModelId, selectedResearchRoute, manuscriptTargetWords, onRunStarted]);

  const chooseResearchDocuments = useCallback(async () => {
    try {
      const selection = await openFilePicker({
        multiple: true,
        filters: [{ name: "Supported local research documents", extensions: ["pdf", "docx", "md", "markdown", "csv", "json", "xlsx"] }],
      });
      if (!selection) return;
      const paths = Array.isArray(selection) ? selection : [selection];
      const selectedProject = localProjects.find((item) => item.id === selectedLocalProjectId);
      if (selectedProject && selectedProject.prompt === prompt.trim()) {
        for (let index = 0; index < paths.length; index += 1) {
          setDocumentImportProgress(`Validating and indexing ${index + 1} of ${paths.length}…`);
          const path = paths[index];
          await invoke<LocalResearchDocument>("import_local_research_document", { projectId: selectedProject.id, path });
        }
        const documents = await invoke<LocalResearchDocument[]>("list_local_research_documents", { projectId: selectedProject.id });
        setLocalDocumentsState({ projectId: selectedProject.id, documents });
        setDocumentImportProgress(null);
        setNotice(`${paths.length} document(s) extracted, indexed, and saved locally.`);
      } else {
        setPendingDocumentPaths((current) => [...new Set([...current, ...paths])]);
        setNotice("Files are staged on this screen and will be validated, extracted, and saved locally when generation starts.");
      }
    } catch (caught) {
      setDocumentImportProgress(null);
      setError(explain(caught));
    }
  }, [localProjects, prompt, selectedLocalProjectId]);

  const chooseResearchFigure = useCallback(async () => {
    if (!selectedLocalProjectId) return;
    try {
      const selection = await openFilePicker({
        multiple: false,
        filters: [{ name: "Scientific figures", extensions: ["svg", "png", "jpg", "jpeg"] }],
      });
      if (!selection || Array.isArray(selection)) return;
      const filename = selection.split(/[\\/]/).pop() ?? "figure";
      const caption = window.prompt("Figure caption", filename.replace(/\.[^.]+$/, ""));
      if (caption == null) return;
      const altText = window.prompt("Accessible figure description", caption);
      if (altText == null) return;
      await invoke<LocalResearchFigureAsset>("register_local_research_figure", {
        projectId: selectedLocalProjectId,
        sourceRunId: selectedLocalRunId,
        path: selection,
        caption,
        altText,
      });
      const assets = await invoke<LocalResearchFigureAsset[]>("list_local_research_figure_assets", { projectId: selectedLocalProjectId });
      setLocalFigureAssetsState({ projectId: selectedLocalProjectId, assets });
      setNotice("Figure source registered to this Research Studio project with its source hash and provenance run.");
    } catch (caught) {
      setError(explain(caught));
    }
  }, [selectedLocalProjectId, selectedLocalRunId]);

  const stageDroppedDocuments = useCallback((files: FileList | File[] | string[]) => {
    const dropped: Array<File | string> = typeof FileList !== "undefined" && files instanceof FileList
      ? Array.from(files)
      : files as Array<File | string>;
    const paths = dropped.length > 0 && typeof dropped[0] === "string"
      ? dropped as string[]
      : (dropped as File[]).map((file) => (file as File & { path?: string }).path).filter((path): path is string => Boolean(path));
    if (paths.length !== dropped.length) {
      setError("OpenMindAI could not read the dropped file path. Use Choose files to import local research documents.");
      return;
    }
    const allowed = paths.filter((path) => /\.(pdf|docx|md|markdown|csv|json|xlsx)$/i.test(path));
    if (allowed.length !== paths.length) {
      setError("This native import accepts PDF, DOCX, Markdown, CSV, JSON, and XLSX files.");
    }
    if (allowed.length) {
      setPendingDocumentPaths((current) => [...new Set([...current, ...allowed])]);
      setNotice(`${allowed.length} file(s) staged for local validation and extraction.`);
    }
  }, []);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    void getCurrentWebview().onDragDropEvent((event) => {
      const uploader = document.querySelector<HTMLElement>(".rs-evidence-drop");
      if (!uploader) return;
      const { payload } = event;
      if (payload.type === "leave") {
        setDocumentDropActive(false);
        return;
      }
      const scale = window.devicePixelRatio || 1;
      const bounds = uploader.getBoundingClientRect();
      const x = payload.position.x / scale;
      const y = payload.position.y / scale;
      const overUploader = x >= bounds.left && x <= bounds.right && y >= bounds.top && y <= bounds.bottom;
      setDocumentDropActive(overUploader && payload.type !== "drop");
      if (payload.type === "drop" && overUploader) stageDroppedDocuments(payload.paths);
    }).then((dispose) => { unlisten = dispose; });
    return () => unlisten?.();
  }, [stageDroppedDocuments]);

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

  const submitResearchChange = async () => {
    const source = localRunDetails?.run;
    const request = changeRequest.trim();
    if (!source || changeBusy || busy || request.length < 5 || !["completed", "cancelled", "failed", "interrupted"].includes(source.status)) return;
    if (editingManuscript) { setError("Save or cancel the open manuscript edit before requesting a research revision."); return; }
    setChangeBusy(true);
    setError(null);
    setNotice(null);
    try {
      const project = localProjects.find((item) => item.id === source.projectId);
      const run = await invoke<LocalResearchRun>("start_local_research_run", {
        projectId: source.projectId, modelId: source.modelId, executionMode: selectedResearchRoute,
        manuscriptTargetWords: project?.manuscript_target_words ?? manuscriptTargetWords,
        sourceRunId: source.id, changeRequest: request,
      });
      setLocalRuns((items) => [run, ...items.filter((item) => item.id !== run.id)]);
      setSelectedLocalRunId(run.id);
      setSelectedTaskId(null);
      setChangeRequest("");
      setRunWorkspace(true);
      onRunStarted(run.id);
      setNotice("Change request started a linked research run. The previous run and manuscript remain saved.");
    } catch (caught) { setError(explain(caught)); }
    finally { setChangeBusy(false); }
  };
  const briefEvent = localRunDetails?.events.find((event) => event.type === "research.run.queued");
  const currentChange = typeof briefEvent?.payload.changeRequest === "string" ? briefEvent.payload.changeRequest : null;
  const sourceRunId = typeof briefEvent?.payload.sourceRunId === "string" ? briefEvent.payload.sourceRunId : null;
  const projectPrompt = localProjects.find((project) => project.id === selectedLocalProjectId)?.prompt ?? prompt;
  const currentBrief = typeof briefEvent?.payload.researchBrief === "string" ? briefEvent.payload.researchBrief : projectPrompt;
  const canRequestChange = Boolean(localRunDetails && ["completed", "cancelled", "failed", "interrupted"].includes(localRunDetails.run.status));

  // Every mode starts a local Rust research run. Cloud access is available
  // only through its bounded inference command.
  if (!useCloud) {
    return (
      <div className="research-root">
        {sidebarSlot &&
          createPortal(
            <div className="studio-side rs-side">
              <button className="rs-new-btn" type="button" disabled={startingResearch || changeBusy} onClick={() => { setRunWorkspace(false); onNewResearch(); setChangeRequest(""); setPrompt(""); setTitle(""); setError(null); setNotice(null); setExperimentRequested(false); setManuscriptTargetWords(3500); setManuscriptPageTarget(24); setCustomPageTarget(30); setCustomPageTargetInput("30"); setPageTargetError(null); setSelectedModelId(""); setSelectedResearchRoute("auto"); setOutputLanguage("English"); setSelectedLocalProjectId(null); setSelectedLocalRunId(null); setPendingDocumentPaths([]); }}><Pencil size={16} /> New local research</button>
              <h2>Local projects</h2>
              {localProjects.length ? <div className="rs-projects">{localProjects.map((project) => <button key={project.id} className={`rs-project-title ${selectedLocalProjectId === project.id ? "active" : ""}`} title={project.prompt} disabled={startingResearch || changeBusy} onClick={() => { setRunWorkspace(false); onNewResearch(); setChangeRequest(""); setSelectedLocalProjectId(project.id); setSelectedLocalRunId(null); setPendingDocumentPaths([]); setTitle(project.title); setPrompt(project.prompt); setOutputLanguage(project.output_language || "English"); setExperimentRequested(project.experiment_requested); setManuscriptTargetWords(project.manuscript_target_words ?? 3500); const pages = project.manuscript_page_target ?? 24; setManuscriptPageTarget(pages); if (!PAGE_PRESETS.includes(pages as typeof PAGE_PRESETS[number])) { setCustomPageTarget(pages); setCustomPageTargetInput(String(pages)); } setPageTargetError(null); setResearchProjectConflict(false); setNotice(`Loaded local project: ${project.title} (revision ${project.revision})`); }}>{project.title}</button>)}</div> : <p className="rs-sidebar-empty">Local research projects will appear here.</p>}
              {localRuns.length > 0 ? <><h2>Runs</h2><div className="rs-projects">{localRuns.map((run) => <button key={run.id} className={`rs-project-title ${selectedLocalRunId === run.id ? "active" : ""}`} disabled={startingResearch || changeBusy} onClick={() => { setRunWorkspace(true); setChangeRequest(""); setSelectedTaskId(null); setSelectedLocalRunId(run.id); onRunStarted(run.id); }}>{new Date(run.startedAt).toLocaleString()} · {run.status}</button>)}</div></> : null}
            </div>,
            sidebarSlot,
          )}
        <main className={`rs rs-main${runWorkspace ? " rs-chat-workspace" : ""}`}>
          <div className={runWorkspace ? "rs-chat-scroll" : "rs-create-content"}>
          {runWorkspace ? <section className="rs-research-thread">
            <header><span className="rs-eyebrow">Research conversation</span><h1>{localProjects.find((project) => project.id === selectedLocalProjectId)?.title || title || "Research Studio"}</h1></header>
            <div className="rs-chat-user"><small>You</small><p dir="auto">{currentChange ?? currentBrief}</p></div>
            {sourceRunId ? <p className="rs-linked-run">Based on an earlier run · <button type="button" disabled={changeBusy} onClick={() => { setSelectedLocalRunId(sourceRunId); setChangeRequest(""); onRunStarted(sourceRunId); }}>Open previous version</button></p> : null}
            {currentChange ? <details><summary>Accumulated research instructions</summary><pre className="rs-local-result">{currentBrief}</pre></details> : null}
            <div className="rs-chat-assistant"><Sparkles size={17} /> OpenMindAI Research</div>
            {!localRunDetails ? <div className="rs-chat-generating" role="status"><Loader2 className="rs-spin" size={23} /><strong>{startingResearch ? "Preparing research…" : "Loading research run…"}</strong><span>{documentImportProgress ?? "0% · Waiting for persisted task progress"}</span></div> : null}
            {error ? <p className="rs-error" role="alert">{error}</p> : null}
            {notice ? <p className="rs-notice" role="status">{notice}</p> : null}
            {localRunDetails ? <div className="rs-actions">
              {["queued", "running"].includes(localRunDetails.run.status) ? <button type="button" disabled={changeBusy} onClick={() => void invoke("pause_local_research_run", { runId: localRunDetails.run.id }).then(refreshCurrentRun).catch((caught: unknown) => setError(explain(caught)))}><PauseCircle size={15} /> Pause run</button> : null}
              {LIVE.has(localRunDetails.run.status) ? <button type="button" disabled={changeBusy} onClick={() => void invoke("cancel_local_research_run", { runId: localRunDetails.run.id }).then(refreshCurrentRun).catch((caught: unknown) => setError(explain(caught)))}><Square size={15} /> Stop run</button> : null}
              {["interrupted", "failed", "paused"].includes(localRunDetails.run.status) ? <button type="button" disabled={changeBusy} onClick={() => void invoke<LocalResearchRun>("resume_local_research_run", { runId: localRunDetails.run.id }).then(refreshCurrentRun).catch((caught: unknown) => setError(explain(caught)))}><Play size={15} /> Resume run</button> : null}
            </div> : null}
          </section> : <section className="rs-card rs-new">
            <span className="rs-eyebrow">Local Research</span>
            <h1>Research with your chosen model route.</h1>
            <p>Projects, runs, task results, and activity are stored on this device. The native orchestrator schedules every research task locally; models provide inference through an installed local runtime or the configured secure Cloud gateway.</p>
            <label>
              Research Title (Optional)
              <input aria-label="Research Title (Optional)" value={title} maxLength={200} onChange={(event) => setTitle(event.target.value)} placeholder="Optional; saved as Untitled research until you add one" />
            </label>
            <div className="rs-title-proposal">
              <button type="button" disabled={busy || !selectedModelId || prompt.trim().length < 5} onClick={() => void generateLocalTitleProposal()}>Generate local title proposal</button>
              {titleInferenceActive ? <button type="button" onClick={() => void cancelLocalTitleProposal()}>Cancel title inference</button> : null}
              {!selectedModelId ? <small role="status">BLOCKED — no registered, file-valid local chat model is available. Model runtime load has not been proven in this session.</small> : null}
              {titleJob?.status === "running" ? <small role="status">Local title inference was interrupted or is still running. Regenerate only after reviewing job history.</small> : null}
              {titleJob?.status === "failed" || titleJob?.status === "interrupted" ? <small role="status">{titleJob.status === "failed" ? "Title inference failed" : "Title inference was interrupted"}: {titleJob.error ?? "No further details"}</small> : null}
              {titleJob?.proposedTitle ? <div><label>Review or edit title proposal<input aria-label="Review or edit title proposal" value={titleProposalDraft} maxLength={200} onChange={(event) => setTitleProposalDraft(event.target.value)} /></label>{titleJob.status === "accepted" ? <small role="status">Saved title: {titleJob.acceptedTitle}</small> : <button type="button" disabled={busy || titleJob.status !== "proposal" || titleProposalDraft.trim().length < 8} onClick={() => void acceptLocalTitleProposal()}>Accept reviewed title</button>}<small>Model {titleJob.modelId} · {titleJob.status} · proposal is not a research conclusion.</small></div> : null}
            </div>
            <label>
              Research Prompt (Required)
              <textarea dir="auto" rows={8} required aria-label="Research Prompt (Required)" value={prompt} maxLength={16000} onChange={(event) => setPrompt(event.target.value)} placeholder="Describe your topic, goals, and evidence requirements…" />
            </label>
            <label>
              Target Pages
              <select value={PAGE_PRESETS.includes(manuscriptPageTarget as typeof PAGE_PRESETS[number]) ? manuscriptPageTarget : "custom"} onChange={(event) => {
                setPageTargetError(null);
                if (event.target.value === "custom") {
                  const nextCustom = PAGE_PRESETS.includes(customPageTarget as typeof PAGE_PRESETS[number]) ? DEFAULT_CUSTOM_PAGE_TARGET : customPageTarget;
                  setManuscriptPageTarget(nextCustom);
                  setCustomPageTarget(nextCustom);
                  setCustomPageTargetInput(String(nextCustom));
                }
                else setManuscriptPageTarget(Number(event.target.value));
              }}>
                {PAGE_PRESETS.map((pages) => <option key={pages} value={pages}>{pages} pages</option>)}
                <option value="custom">Custom</option>
              </select>
            </label>
            {!PAGE_PRESETS.includes(manuscriptPageTarget as typeof PAGE_PRESETS[number]) ? <label>
              Custom page target (1–100)
              <input type="number" min={1} max={100} value={customPageTargetInput} aria-invalid={Boolean(pageTargetError)} aria-describedby={pageTargetError ? "rs-page-target-error" : undefined} onChange={(event) => {
                const raw = event.target.value;
                setCustomPageTargetInput(raw);
                const parsed = Number(raw);
                if (!raw || !Number.isInteger(parsed) || parsed < 1 || parsed > 100) {
                  setPageTargetError("Enter a whole number from 1 to 100 pages.");
                  return;
                }
                setManuscriptPageTarget(parsed);
                setCustomPageTarget(parsed);
                setPageTargetError(null);
              }} />
            </label> : null}
            {pageTargetError ? <small id="rs-page-target-error" className="rs-error" role="alert">{pageTargetError}</small> : null}
            <small className="rs-page-target-note">Saved with this project when you save the draft or generate. Actual rendered pages are checked after export; this target does not add unsupported content.</small>
            <details className="rs-page-plan">
              <summary>Planning allocation (not generated pages)</summary>
              <p>Selected target: {manuscriptPageTarget} pages. Planned allocation: {manuscriptPageTarget} pages across manuscript sections. Word budget: {manuscriptTargetWords.toLocaleString()} words. Rendered PDF pages: not yet measured.</p>
              <ul>{planManuscriptPages(manuscriptPageTarget, manuscriptTargetWords).map((section) => <li key={section.name}>{section.name}: {section.pages} planned pages, about {section.words.toLocaleString()} words</li>)}</ul>
              <small>This is a planning estimate only; it does not add content or guarantee the exported PDF page count.</small>
            </details>
            {!title.trim() ? <small className="rs-title-suggestion-blocked" role="status">The title is optional. Save Draft keeps the project available as “Untitled research”; use the explicit local proposal action when a compatible model is installed.</small> : null}
            <section className={`rs-evidence-drop${documentDropActive ? " active" : ""}`} onDragOver={(event) => { event.preventDefault(); setDocumentDropActive(true); }} onDragLeave={(event) => { if (!event.currentTarget.contains(event.relatedTarget as Node | null)) setDocumentDropActive(false); }} onDrop={(event) => { event.preventDefault(); setDocumentDropActive(false); stageDroppedDocuments(event.dataTransfer.files); }}>
              <strong>Multiple Evidence Upload</strong>
              <span>Choose or drop PDF, DOCX, Markdown, CSV, JSON, or XLSX files. Sources stay on this device.</span>
              <button type="button" disabled={busy} onClick={() => void chooseResearchDocuments()}>Choose evidence files</button>
              {pendingDocumentPaths.map((path) => <div className="rs-evidence-pending" key={path}><span>{path.split(/[\\/]/).pop()}</span><button type="button" aria-label={`Remove ${path.split(/[\\/]/).pop()}`} disabled={busy} onClick={() => setPendingDocumentPaths((current) => current.filter((item) => item !== path))}>Remove</button></div>)}
              {documentImportProgress ? <small role="status">{documentImportProgress}</small> : null}
              {localDocuments.length ? <ul className="rs-local-sources">{localDocuments.map((document) => <li key={document.id}><strong>{document.name}</strong><small>{document.chunkCount} indexed chunks · {Math.round(document.sizeBytes / 1024)} KB · SHA-256 {document.contentSha256.slice(0, 16)}…</small>{researchDocumentSourceLabels(document.extractionMetadataJson).map((label) => <small key={label}>Source origin: {label}</small>)}{researchDocumentPreview(document.extractionMetadataJson) ? <small className="rs-source-preview">Extraction preview: {researchDocumentPreview(document.extractionMetadataJson)}</small> : null}</li>)}</ul> : null}
            </section>
            <section className="rs-evidence-drop" aria-label="Project figure assets">
              <strong>Project Figure Assets</strong>
              <span>Register a validated SVG, PNG, or JPEG source with project and run provenance. Unresolved manuscript figure references stop export instead of disappearing.</span>
              <button type="button" disabled={busy || !selectedLocalProjectId} onClick={() => void chooseResearchFigure()}>Register scientific figure</button>
              {localFigureAssets.length ? <ul className="rs-local-sources">{localFigureAssets.map((asset) => <li key={asset.id}><strong>{asset.caption}</strong><small>{asset.name} · {asset.widthPx} × {asset.heightPx} · {asset.mediaType} · SHA-256 {asset.contentSha256.slice(0, 16)}…</small><small>Asset ID: {asset.id} · Alt: {asset.altText}</small>{localRunDetails?.manuscript && localRunDetails.manuscript.status !== "accepted" ? <button type="button" disabled={busy || localRunDetails.run.projectId !== asset.projectId} onClick={() => { const alt = asset.altText.split("[").join("").split("]").join(""); const caption = asset.caption.split("\\").join("").split('"').join("'"); const reference = `![${alt}](research-figure:${asset.id} "${caption}")`; const base = editingManuscript ? manuscriptDraft : localRunDetails.manuscript!.content; setManuscriptDraft(`${base.trimEnd()}\n\n${reference}\n`); setEditingManuscript(true); setNotice(`Linked ${asset.name} into the manuscript draft using research-figure:${asset.id}. Save as a new revision before export.`); }}>Insert into manuscript draft</button> : null}</li>)}</ul> : <small>No figure assets registered to this project.</small>}
            </section>
            {selectedLocalProjectId ? <ResearchManuscript key={selectedLocalProjectId} projectId={selectedLocalProjectId} models={localModels} selectedModelId={selectedModelId} onOpenModelManager={onOpenModelManager} /> : null}
            <div className="rs-readiness" role="status">
              <p><strong>Model readiness:</strong> {selectedModelId ? `${localModels.find((model) => model.id === selectedModelId)?.name ?? "Selected model"} — registry state ${localModels.find((model) => model.id === selectedModelId)?.state}; file/package validation is not proof of a successful runtime load or generated token.` : "BLOCKED — no compatible installed local chat model is available for Research inference."}</p>
              {localModels.filter((model) => !model.enabled || !["installed", "ready", "loaded"].includes(model.state)).map((model) => <small key={model.id}>{model.name}: {model.state === "notInstalled" ? "not installed" : model.state === "failed" ? "failed validation or unavailable" : `installed / not ready (${model.state})`}</small>)}
              <button type="button" onClick={onOpenModelManager}>Open Model Manager to install or select a model</button>
              <p><strong>Evidence readiness:</strong> {pendingDocumentPaths.length ? `${pendingDocumentPaths.length} file(s) staged for validation and extraction` : localDocuments.length ? `${localDocuments.length} project file(s) indexed; review extraction previews` : "No evidence uploaded; evidence-dependent claims cannot be supported."}</p>
            </div>
            <details className="rs-advanced-settings">
              <summary>Advanced settings</summary>
            <label>
              Manuscript language
              <select value={outputLanguage} onChange={(event) => setOutputLanguage(event.target.value)}>
                {LANGUAGES.map((language) => <option key={language}>{language}</option>)}
              </select>
            </label>
            <label>
              Manuscript word target
              <select value={manuscriptTargetWords} onChange={(event) => setManuscriptTargetWords(Number(event.target.value))}>
                <option value={3000}>3,000 words</option>
                <option value={3500}>3,500 words (default)</option>
                <option value={4000}>4,000 words</option>
              </select>
            </label>
            <label className="rs-checkbox-label"><input type="checkbox" checked={experimentRequested} onChange={(event) => setExperimentRequested(event.target.checked)} /> Include an experiment plan proposal (no code execution)</label>
            <label>
              Research model
              <select value={selectedModelId} onChange={(event) => setSelectedModelId(event.target.value)} disabled={!localModels.length}>
                {localModels.filter((model) => model.enabled && ["installed", "ready", "loaded"].includes(model.state)).length ? localModels.filter((model) => model.enabled && ["installed", "ready", "loaded"].includes(model.state)).map((model) => <option key={model.id} value={model.id}>{model.name} · {model.state}</option>) : <option value="">No compatible installed model</option>}
              </select>
            </label>
            <label>
              Research model routing
              <select value={selectedResearchRoute} onChange={(event) => setSelectedResearchRoute(event.target.value as "local" | "cloud" | "auto")}>
                <option value="auto">Auto · choose a compatible configured model</option>
                <option value="local">Local · installed llama.cpp model</option>
                <option value="cloud">Cloud · inference through the secure gateway</option>
              </select>
            </label>
            <div className="rs-inline-status">
              <button type="button" disabled={busy} onClick={() => void invoke<string>("local_research_cloud_model").then((model) => { setCloudResearchModel(model); setNotice(`Cloud inference preflight passed: ${model}`); }).catch((caught: unknown) => { setCloudResearchModel(null); setError(explain(caught)); })}>Check Cloud inference availability</button>
              {cloudResearchModel ? <small>Connected inference catalog selected: {cloudResearchModel}. {cloudResearchModel.toLowerCase().includes("nemotron") ? "The advertised model identifier is Nemotron." : "This identifier is not Nemotron; no accessible chat model with a Nemotron identifier was selected by the current catalog preflight."}</small> : <small>No Cloud inference model preflight recorded in this session.</small>}
            </div>
            </details>
            {error ? <p className="rs-error" role="alert">{error}</p> : null}
            {researchProjectConflict ? <div role="alert" className="rs-error"><p>Your edits were not saved because this project changed elsewhere. Review or copy your current form values, then reload the saved project before continuing.</p><button type="button" disabled={busy} onClick={() => void reloadConflictedResearchProject()}>Reload saved project</button></div> : null}
            {notice ? <p className="rs-notice" role="status">{notice}</p> : null}
            <div className="rs-actions">
              <button type="button" disabled={busy || prompt.trim().length < 5 || Boolean(pageTargetError)} onClick={saveLocalResearchDraft}>Save Draft</button>
              <button className="rs-primary" type="button" disabled={busy || !selectedModelId || prompt.trim().length < 5 || Boolean(pageTargetError)} onClick={startLocalResearch}>{busy ? <Loader2 size={16} className="rs-spin" /> : <Sparkles size={16} />} Generate Research</button>
              {localRunDetails && ["queued", "running"].includes(localRunDetails.run.status) ? <button type="button" disabled={busy} onClick={() => void invoke("pause_local_research_run", { runId: localRunDetails.run.id }).then(refreshCurrentRun).catch((caught: unknown) => setError(explain(caught)))}><PauseCircle size={15} /> Pause run</button> : null}
              {localRunDetails && LIVE.has(localRunDetails.run.status) ? <button type="button" disabled={busy} onClick={() => void invoke("cancel_local_research_run", { runId: localRunDetails.run.id }).then(() => refreshCurrentRun()).catch((caught: unknown) => setError(explain(caught)))}><Square size={15} /> Stop run</button> : null}
              {localRunDetails && ["interrupted", "failed", "paused"].includes(localRunDetails.run.status) ? <button type="button" disabled={busy} onClick={() => void invoke<LocalResearchRun>("resume_local_research_run", { runId: localRunDetails.run.id }).then((run) => { setLocalRuns((items) => [run, ...items.filter((item) => item.id !== run.id)]); setNotice("Run resumed from its last completed task."); }).catch((caught: unknown) => setError(explain(caught)))}><Play size={15} /> Resume run</button> : null}
            </div>
          </section>}
          {runWorkspace && localRunDetails ? <section className="rs-card">
            <span className="rs-eyebrow">Native run · {localRunDetails.run.status}</span>
            <h2>Research task graph</h2>
            {(() => { const total = localRunDetails.tasks.length; const completed = localRunDetails.tasks.filter((task) => ["completed", "complete", "done"].includes(task.status)).length; const percent = total ? Math.round(completed * 100 / total) : 0; return <><div className="rs-chat-progress-heading">{LIVE.has(localRunDetails.run.status) ? <Loader2 className="rs-spin" size={20} /> : null}<strong>{LIVE.has(localRunDetails.run.status) ? "Researching…" : localRunDetails.run.status === "completed" ? "Research run complete" : localRunDetails.run.status}</strong><strong>{percent}%</strong></div><p>{completed} of {total} tasks completed</p><div className="rs-progress" role="progressbar" aria-label="Research task completion" aria-valuemin={0} aria-valuemax={100} aria-valuenow={percent}><div className="rs-progress-bar" style={{ width: `${percent}%` }} /></div></>; })()}
            <div className="rs-local-graph-scroll"><svg className="rs-local-graph" viewBox="0 0 1640 350" role="group" aria-label="Research task dependency graph">
              {localRunDetails.dependencies.map(([taskId, parentId]) => {
                const task = localRunDetails.tasks.find((item) => item.id === taskId);
                const parent = localRunDetails.tasks.find((item) => item.id === parentId);
                const target = task && LOCAL_AGENT_GRAPH[task.agent];
                const source = parent && LOCAL_AGENT_GRAPH[parent.agent];
                if (!target || !source) return null;
                const activeEdge = task.status === "running" && ["completed", "running"].includes(parent.status);
                return <path key={`${taskId}-${parentId}`} className={`rs-graph-edge${activeEdge ? " is-active" : ""}${task.status === "completed" ? " is-complete" : ""}`} d={`M ${source.x + 156} ${source.y + 22} C ${source.x + 185} ${source.y + 22}, ${target.x - 28} ${target.y + 22}, ${target.x} ${target.y + 22}`} />;
              })}
              {localRunDetails.tasks.map((task) => {
                const node = LOCAL_AGENT_GRAPH[task.agent];
                if (!node) return null;
                return <g key={task.id} className={`rs-graph-node status-${task.status}${selectedTaskId === task.id ? " selected" : ""}`} role="button" tabIndex={0} aria-label={`${task.title}: ${task.status}`} onClick={() => setSelectedTaskId(task.id)} onKeyDown={(event) => { if (event.key === "Enter" || event.key === " ") setSelectedTaskId(task.id); }}>
                  <rect x={node.x} y={node.y} width="156" height="44" rx="10" />
                  <text x={node.x + 12} y={node.y + 18} className="rs-graph-label">{node.label}</text>
                  <text x={node.x + 12} y={node.y + 34} className="rs-graph-status">{task.status} · { ["completed", "complete", "done"].includes(task.status) ? "100%" : LIVE.has(task.status) ? "In progress" : "0%"}</text>
                </g>;
              })}
            </svg></div>
            <div className="rs-tasks">{localRunDetails.tasks.map((task) => <details key={task.id} className="rs-task">
              <summary><TaskIcon status={task.status} /><strong>{task.title}</strong><span>{task.agent}</span><small>{task.status}{task.attempt > 0 ? ` · attempt ${task.attempt}` : ""}</small></summary>
              {task.error ? <p className="rs-error">{task.error}</p> : null}
              {task.result ? <pre className="rs-local-result">{task.result}</pre> : null}
              {localRunDetails.run.status !== "running" && ["failed", "interrupted"].includes(task.status) && task.cloudProvider?.startsWith("openmindai-") && ["submitted", "prepared"].includes(task.cloudRequestState ?? "") ? <div><p role="status">Cloud request outcome is unknown. {task.cloudResponseError ?? "No finalized response was saved."}</p><small>Client request ID: {task.cloudClientRequestId ?? "not recorded"}{task.cloudUpstreamRequestId ? ` · Provider response ID: ${task.cloudUpstreamRequestId}` : " · Provider supplied no response ID; no status endpoint is configured."}</small><button type="button" onClick={() => { if (!window.confirm("The previous Cloud request outcome is unknown. A new attempt may create duplicate billable usage. Authorize a new request?")) return; void invoke("authorize_local_research_cloud_retry", { runId: localRunDetails.run.id, taskId: task.id }).then(() => invoke<LocalResearchRun>("resume_local_research_run", { runId: localRunDetails.run.id })).then((run) => { setLocalRuns((items) => [run, ...items.filter((item) => item.id !== run.id)]); setNotice("A new Cloud request was explicitly authorized. Its correlation ID and provider response will be recorded."); }).catch((caught: unknown) => setError(explain(caught))); }}>Authorize a new Cloud attempt (may incur duplicate usage)</button></div> : null}
            </details>)}</div>
            <h3>Activity</h3>
            <ol className="rs-local-events">{[...localRunDetails.events].reverse().map((event) => <li key={event.eventId}><time>{new Date(event.timestamp).toLocaleTimeString()}</time><span>{event.type.replace(/\./g, " · ")}</span>{event.agent ? <small>{event.agent}</small> : null}</li>)}</ol>
            <h3>Retrieved sources · {localRunDetails.sources.length}</h3>
            <ul className="rs-local-sources">{localRunDetails.sources.map((source) => <li key={source.id}>{source.metadata.localDocumentId ? <strong>{source.title}</strong> : <a href={source.url} target="_blank" rel="noreferrer">{source.title || source.url}</a>}<small>{source.metadata.localDocumentId ? `User-imported local document · SHA-256 ${String(source.metadata.sha256 ?? "").slice(0, 16)}…` : source.metadata.isRead === true ? "Page text retrieved" : source.metadata.provider ? `Registry metadata · ${String(source.metadata.provider)}` : "Search result only"}{source.metadata.isPdf === true ? " · PDF" : ""}{source.metadata.verificationStatus ? ` · DOI ${String(source.metadata.verificationStatus)} (metadata only)` : ""}</small></li>)}</ul>
            {localRunDetails.evidence.length ? <><h3>Extracted evidence · human claim review required</h3><ul className="rs-local-evidence">{localRunDetails.evidence.map((item) => <li key={item.id}><strong>{item.claim}</strong>{item.quotation ? <blockquote>{item.quotation}</blockquote> : null}<small>{item.verificationStatus} · source {item.sourceId ? localRunDetails.sources.findIndex((source) => source.id === item.sourceId) + 1 : "not linked"} · review the saved passage against the manuscript claim</small>{item.verificationStatus !== "verified" && item.quotation && item.sourceId ? <div className="rs-actions"><button type="button" onClick={() => { const claim = window.prompt("Enter the specific manuscript claim this exact passage supports:"); if (!claim?.trim()) return; if (!window.confirm("Confirm that you reviewed the source passage and it supports this claim?")) return; void invoke("review_local_research_evidence", { runId: localRunDetails.run.id, evidenceId: item.id, claim, supported: true }).then(refreshCurrentRun).catch((caught: unknown) => setError(explain(caught))); }}>Mark claim supported</button><button type="button" onClick={() => { if (!window.confirm("Mark this passage as reviewed but unsupported? It will block final manuscript approval.")) return; void invoke("review_local_research_evidence", { runId: localRunDetails.run.id, evidenceId: item.id, claim: item.claim, supported: false }).then(refreshCurrentRun).catch((caught: unknown) => setError(explain(caught))); }}>Mark unsupported</button></div> : null}</li>)}</ul></> : null}
            {localRunDetails.manuscript ? <section className="rs-manuscript-review">
              <h3>Manuscript proposal · revision {localRunDetails.manuscript.revision}</h3>
              {editingManuscript ? <textarea aria-label="Edit manuscript revision" rows={18} maxLength={100000} value={manuscriptDraft} onChange={(event) => setManuscriptDraft(event.target.value)} /> : <pre className="rs-local-result">{localRunDetails.manuscript.content}</pre>}
              <p>{localRunDetails.manuscript.content.trim().split(/\s+/).filter(Boolean).length.toLocaleString()} words · target {localProjects.find((project) => project.id === localRunDetails.run.projectId)?.manuscript_target_words?.toLocaleString() ?? "3,500"} · SHA-256 {localRunDetails.manuscript.contentHash} · {localRunDetails.manuscript.status}</p>
              {localRunDetails.manuscriptRevisions.length > 1 ? <details className="rs-manuscript-history"><summary>Revision history · {localRunDetails.manuscriptRevisions.length} saved versions</summary>{localRunDetails.manuscriptRevisions.slice(1).map((revision) => <details key={revision.id}><summary>Revision {revision.revision} · {new Date(revision.createdAt).toLocaleString()} · {revision.status}</summary><p>SHA-256 {revision.contentHash}</p><pre className="rs-local-result">{revision.content}</pre></details>)}</details> : null}
              {localRunDetails.run.status === "completed" && localRunDetails.manuscript.status !== "accepted" ? (editingManuscript ? <><button type="button" onClick={() => { setManuscriptDraft(localRunDetails.manuscript!.content); setEditingManuscript(false); }}>Cancel editing</button><button type="button" disabled={!manuscriptDraft.trim()} onClick={() => void invoke("save_local_research_manuscript_revision", { runId: localRunDetails.run.id, expectedHash: localRunDetails.manuscript!.contentHash, content: manuscriptDraft }).then(() => { setEditingManuscript(false); setNotice("A new manuscript revision was saved to research history."); return refreshCurrentRun(); }).catch((caught: unknown) => setError(explain(caught)))}>Save as new revision</button></> : <button type="button" onClick={() => setEditingManuscript(true)}>Edit manuscript</button>) : null}
              {localRunDetails.manuscript.status !== "accepted" ? <button type="button" disabled={localRunDetails.run.status !== "completed"} onClick={() => void invoke("approve_local_research_manuscript", { runId: localRunDetails.run.id, expectedHash: localRunDetails.manuscript!.contentHash }).then(() => refreshCurrentRun()).catch((caught: unknown) => setError(explain(caught)))}><CheckCircle2 size={15} /> Accept this manuscript revision</button> : <p role="status">This exact revision was explicitly accepted.</p>}
              <button type="button" onClick={() => void invoke<{ id: string; name: string }>("export_local_research_markdown", { runId: localRunDetails.run.id, allowDraft: true }).then((artifact) => setNotice(`Draft Markdown export created: ${artifact.name}`)).catch((caught: unknown) => setError(explain(caught)))}>Export marked draft</button>
              <button type="button" onClick={() => void invoke<{ id: string; name: string }>("export_local_research_document", { runId: localRunDetails.run.id, format: "pdf", allowDraft: true }).then((artifact) => setNotice(`Validated PDF export created: ${artifact.name}`)).catch((caught: unknown) => setError(explain(caught)))}>Export draft PDF</button>
              <button type="button" onClick={() => void invoke<{ id: string; name: string }>("export_local_research_document", { runId: localRunDetails.run.id, format: "docx", allowDraft: true }).then((artifact) => setNotice(`Validated DOCX export created: ${artifact.name}`)).catch((caught: unknown) => setError(explain(caught)))}>Export draft DOCX</button>
              <button type="button" onClick={() => void invoke<{ id: string; name: string }>("export_local_research_academic_text", { runId: localRunDetails.run.id, format: "latex", allowDraft: true }).then((artifact) => setNotice(`LaTeX draft export created: ${artifact.name}`)).catch((caught: unknown) => setError(explain(caught)))}>Export draft LaTeX</button>
              <button type="button" onClick={() => void invoke<{ id: string; name: string }>("export_local_research_academic_text", { runId: localRunDetails.run.id, format: "bibtex", allowDraft: true }).then((artifact) => setNotice(`BibTeX draft export created: ${artifact.name}`)).catch((caught: unknown) => setError(explain(caught)))}>Export draft BibTeX</button>
              {localRunDetails.manuscript.status === "accepted" ? <button type="button" onClick={() => void invoke<{ id: string; name: string }>("export_local_research_markdown", { runId: localRunDetails.run.id, allowDraft: false }).then((artifact) => setNotice(`Accepted Markdown export created: ${artifact.name}`)).catch((caught: unknown) => setError(explain(caught)))}>Export accepted Markdown</button> : null}
              {localRunDetails.manuscript.status === "accepted" ? <><button type="button" onClick={() => void invoke<{ id: string; name: string }>("export_local_research_document", { runId: localRunDetails.run.id, format: "pdf", allowDraft: false }).then((artifact) => setNotice(`Accepted PDF export created: ${artifact.name}`)).catch((caught: unknown) => setError(explain(caught)))}>Export accepted PDF</button><button type="button" onClick={() => void invoke<{ id: string; name: string }>("export_local_research_document", { runId: localRunDetails.run.id, format: "docx", allowDraft: false }).then((artifact) => setNotice(`Accepted DOCX export created: ${artifact.name}`)).catch((caught: unknown) => setError(explain(caught)))}>Export accepted DOCX</button></> : null}
            </section> : null}
            {localRunDetails.run.error ? <p className="rs-error" role="alert">{localRunDetails.run.error}</p> : null}
          </section> : !runWorkspace && selectedLocalProjectId ? <section className="rs-card"><h2>Research history</h2><p>Select a prior run or start the first native run for this project.</p></section> : null}
          </div>
          {runWorkspace ? <form className="rs-change-composer" onSubmit={(event) => { event.preventDefault(); void submitResearchChange(); }}>
            <div className="rs-change-input"><textarea aria-label="Research change request" rows={2} maxLength={800} dir="auto" value={changeRequest} onChange={(event) => setChangeRequest(event.target.value)} placeholder="Ask for changes to this research…" onKeyDown={(event) => { if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) { event.preventDefault(); void submitResearchChange(); } }} />
              <button className="rs-primary rs-send" type="submit" aria-label="Send research change request" disabled={!canRequestChange || changeBusy || busy || changeRequest.trim().length < 5 || editingManuscript}>{changeBusy ? <Loader2 size={18} className="rs-spin" /> : <Sparkles size={18} />}</button></div>
            <small>{canRequestChange ? "Starts a linked research revision with fresh evidence checks. Previous versions are retained." : "Draft your changes now; send after the active run finishes or is stopped."}</small>
          </form> : null}
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
                placeholder="Describe your research topic, goals, and constraints in English (or another language)…"
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
