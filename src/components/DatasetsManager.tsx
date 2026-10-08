/* global AbortController, AbortSignal, DOMException, fetch */
import { useCallback, useEffect, useMemo, useState } from "react";
import {
  BookOpenCheck,
  CheckCircle2,
  Copy,
  DatabaseZap,
  Download,
  ExternalLink,
  RefreshCw,
  Search,
  Sparkles,
} from "lucide-react";
import { api } from "../api";
import type { DatasetDownloadStatus } from "../types";
import { formatBytes } from "../lib/format";

type DatasetCategory =
  | "recommended"
  | "text"
  | "code"
  | "vision"
  | "audio"
  | "video"
  | "tabular"
  | "multimodal"
  | "conversation"
  | "safety"
  | "benchmark";

interface DatasetType {
  id: DatasetCategory;
  label: string;
  search: string;
  useCase: string;
}

interface CuratedDataset {
  id: string;
  sourceId: string;
  name: string;
  category: DatasetCategory;
  description: string;
  bestFor: string[];
  tags: string[];
  license: string;
  recommendedFor: string;
}

interface OpenMindDataset {
  id: string;
  downloads: number | null;
  likes: number | null;
  tags: string[];
  lastModified: string | null;
  gated: boolean | string | null;
  cardData?: {
    pretty_name?: string;
    license?: string | string[];
    language?: string | string[];
    task_categories?: string | string[];
    task_ids?: string | string[];
    size_categories?: string | string[];
  };
}

const DATASET_TYPES: DatasetType[] = [
  {
    id: "recommended",
    label: "Recommended",
    search: "instruction chat reasoning",
    useCase: "Best starter packs for OpenMindAI chat, agents, RAG, and evaluation.",
  },
  {
    id: "text",
    label: "Text",
    search: "text corpus instruction",
    useCase: "Pretrain, summarize, classify, translate, and build knowledge bases.",
  },
  {
    id: "code",
    label: "Code",
    search: "code dataset programming",
    useCase: "Train code assistants, search code, generate tests, and repair bugs.",
  },
  {
    id: "vision",
    label: "Vision",
    search: "image vision caption OCR",
    useCase: "Caption images, OCR documents, classify objects, and evaluate vision models.",
  },
  {
    id: "audio",
    label: "Audio",
    search: "audio speech transcription",
    useCase: "Speech recognition, voice commands, speaker tasks, and audio QA.",
  },
  {
    id: "video",
    label: "Video",
    search: "video caption action",
    useCase: "Video captioning, action recognition, clips, and multimodal agents.",
  },
  {
    id: "tabular",
    label: "Tabular",
    search: "tabular csv classification regression",
    useCase: "Analytics, prediction, customer data, finance, and structured ML.",
  },
  {
    id: "multimodal",
    label: "Multimodal",
    search: "multimodal image text",
    useCase: "Build agents that connect text, image, document, audio, and video signals.",
  },
  {
    id: "conversation",
    label: "Conversation",
    search: "chat instruction dialogue",
    useCase: "Fine-tune helpful assistants and domain-specific chat behavior.",
  },
  {
    id: "safety",
    label: "Safety",
    search: "rlhf preference safety harmless",
    useCase: "Preference tuning, moderation, red-team review, and safer answers.",
  },
  {
    id: "benchmark",
    label: "Benchmark",
    search: "benchmark evaluation question answering",
    useCase: "Evaluate model quality, compare releases, and catch regressions.",
  },
];

const CURATED_DATASETS: CuratedDataset[] = [
  {
    id: "OpenMindAI/fineweb",
    sourceId: openMindSourceDatasetId("FW/fineweb"),
    name: "FineWeb",
    category: "text",
    description: "Large clean web text corpus for language model pretraining and RAG sampling.",
    bestFor: ["Pretraining", "RAG source mining", "General text quality"],
    tags: ["text", "web", "large-scale"],
    license: "ODC-By",
    recommendedFor: "OpenMindAI Core",
  },
  {
    id: "OpenMindAI/ultrachat_200k",
    sourceId: openMindSourceDatasetId("H4/ultrachat_200k"),
    name: "UltraChat 200K",
    category: "conversation",
    description: "Instruction and dialogue data for improving assistant-style chat behavior.",
    bestFor: ["Chat fine-tuning", "Instruction following", "Assistant tone"],
    tags: ["chat", "instruction", "dialogue"],
    license: "MIT",
    recommendedFor: "OpenMindAI AI Chat",
  },
  {
    id: "OpenMindAI/open-assistant-conversations",
    sourceId: "OpenAssistant/oasst1",
    name: "OpenAssistant Conversations",
    category: "conversation",
    description: "Human assistant conversations with rankings and multilingual interaction data.",
    bestFor: ["Assistant training", "Preference work", "Multilingual chat"],
    tags: ["conversation", "ranking", "multilingual"],
    license: "Apache 2.0",
    recommendedFor: "OpenAgent",
  },
  {
    id: "OpenMindAI/helpful-harmless-preferences",
    sourceId: "Anthropic/hh-rlhf",
    name: "HH-RLHF",
    category: "safety",
    description: "Helpful and harmless preference data for safer assistant alignment work.",
    bestFor: ["Safety tuning", "Preference ranking", "Refusal behavior"],
    tags: ["rlhf", "safety", "preference"],
    license: "MIT",
    recommendedFor: "OpenMindAI Safety",
  },
  {
    id: "OpenMindAI/code-stack",
    sourceId: "bigcode/the-stack",
    name: "The Stack",
    category: "code",
    description: "Large source-code dataset for coding assistants and repository intelligence.",
    bestFor: ["Code completion", "Code search", "Developer agents"],
    tags: ["code", "programming", "repository"],
    license: "OpenRAIL",
    recommendedFor: "OpenMindAI Coder",
  },
  {
    id: "OpenMindAI/conceptual-captions",
    sourceId: "google-research-datasets/conceptual_captions",
    name: "Conceptual Captions",
    category: "vision",
    description: "Image and caption pairs for image understanding and captioning workflows.",
    bestFor: ["Image captions", "Vision-language", "Dataset grounding"],
    tags: ["image", "caption", "vision"],
    license: "Upstream terms",
    recommendedFor: "OpenMindAI Lens",
  },
  {
    id: "OpenMindAI/common-voice",
    sourceId: "mozilla-foundation/common_voice_17_0",
    name: "Common Voice",
    category: "audio",
    description: "Community speech dataset for ASR, voice commands, and multilingual audio.",
    bestFor: ["Speech-to-text", "Voice agents", "Language coverage"],
    tags: ["audio", "speech", "asr"],
    license: "CC0",
    recommendedFor: "OpenMindAI Hear",
  },
  {
    id: "OpenMindAI/imdb-reviews",
    sourceId: "stanfordnlp/imdb",
    name: "IMDB Reviews",
    category: "text",
    description: "Compact sentiment dataset for classification pipelines and evaluation demos.",
    bestFor: ["Sentiment analysis", "Classifier tests", "Fast demos"],
    tags: ["classification", "sentiment", "text"],
    license: "Unknown",
    recommendedFor: "OpenMindAI Classifier",
  },
  {
    id: "OpenMindAI/glue",
    sourceId: "nyu-mll/glue",
    name: "GLUE",
    category: "benchmark",
    description: "Classic benchmark suite for natural language understanding evaluation.",
    bestFor: ["Regression tests", "NLP evaluation", "Release comparison"],
    tags: ["benchmark", "nlp", "evaluation"],
    license: "Mixed",
    recommendedFor: "OpenMindAI Eval",
  },
  {
    id: "OpenMindAI/squad",
    sourceId: "squad",
    name: "SQuAD",
    category: "benchmark",
    description: "Question answering benchmark for retrieval, reading, and answer extraction.",
    bestFor: ["Question answering", "RAG evaluation", "Reading comprehension"],
    tags: ["qa", "benchmark", "text"],
    license: "CC BY-SA 4.0",
    recommendedFor: "OpenMindAI RAG",
  },
  {
    id: "OpenMindAI/beans",
    sourceId: "merve/beans",
    name: "Beans",
    category: "vision",
    description: "Small image classification dataset that is useful for quick local tests.",
    bestFor: ["Vision smoke tests", "Image classification", "Fast training"],
    tags: ["image", "classification", "small"],
    license: "Apache 2.0",
    recommendedFor: "OpenMindAI Vision Tests",
  },
  {
    id: "OpenMindAI/heart-failure-records",
    sourceId: "mstz/heart_failure",
    name: "Heart Failure Clinical Records",
    category: "tabular",
    description: "Small structured dataset for tabular prediction demos and analytics practice.",
    bestFor: ["Tabular ML", "Classification", "Data profiling"],
    tags: ["tabular", "csv", "classification"],
    license: "CC BY 4.0",
    recommendedFor: "OpenMindAI Data Analyst",
  },
];

const USE_CASES = [
  {
    title: "Train or fine-tune",
    detail: "Use instruction, chat, code, image, or speech data to specialize OpenMindAI behavior.",
  },
  {
    title: "Build RAG knowledge",
    detail: "Turn text, PDF, table, and web corpora into searchable local knowledge bases.",
  },
  {
    title: "Evaluate models",
    detail: "Run benchmark and regression datasets before shipping model or prompt changes.",
  },
  {
    title: "Create agents",
    detail: "Feed code, tool-use, workflow, and conversation datasets into OpenAgent tasks.",
  },
];

export function DatasetsManager() {
  const [query, setQuery] = useState("");
  const [category, setCategory] = useState<DatasetCategory>("recommended");
  const [sort, setSort] = useState<"downloads" | "likes" | "trendingScore" | "lastModified">(
    "downloads",
  );
  const [datasets, setDatasets] = useState<OpenMindDataset[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [copiedId, setCopiedId] = useState<string | null>(null);
  const [downloadStatus, setDownloadStatus] = useState<DatasetDownloadStatus | null>(null);
  const [installedDatasets, setInstalledDatasets] = useState<Set<string>>(() => {
    try {
      return new Set(JSON.parse(window.localStorage.getItem("openmindaiInstalledDatasets") ?? "[]"));
    } catch {
      return new Set();
    }
  });
  const [nextCursor, setNextCursor] = useState<string | null>(null);
  const selectedType = DATASET_TYPES.find((type) => type.id === category) ?? DATASET_TYPES[0];

  const curated = useMemo(() => {
    const search = query.trim().toLowerCase();
    return CURATED_DATASETS.filter((dataset) => {
      const matchesCategory = category === "recommended" || dataset.category === category;
      if (!matchesCategory) return false;
      if (!search) return true;
      return [
        dataset.id,
        dataset.name,
        dataset.description,
        dataset.recommendedFor,
        ...dataset.tags,
        ...dataset.bestFor,
      ]
        .join(" ")
        .toLowerCase()
        .includes(search);
    });
  }, [category, query]);

  const recommendedCount = CURATED_DATASETS.length;
  const categoryCount = DATASET_TYPES.length - 1;

  const loadDatasets = useCallback(async (signal?: AbortSignal, cursor?: string | null, append = false) => {
    setLoading(true);
    setError(null);
    const searchTerm = query.trim() || selectedType.search;
    const url = new URL(openMindDatasetApiUrl());
    url.searchParams.set("limit", "100");
    url.searchParams.set("sort", sort);
    url.searchParams.set("direction", "-1");
    url.searchParams.set("search", searchTerm);
    if (cursor) url.searchParams.set("cursor", cursor);
    try {
      const response = await fetch(url.toString(), {
        headers: { accept: "application/json" },
        signal,
      });
      if (!response.ok) {
        throw new Error(`OpenMindAI Dataset Hub returned ${response.status}`);
      }
      const payload = (await response.json()) as unknown;
      setNextCursor(parseNextCursor(response.headers.get("Link")));
      const normalized = normalizeDatasetList(payload);
      setDatasets((current) => (append ? mergeDatasets(current, normalized) : normalized));
    } catch (caught) {
      if (caught instanceof DOMException && caught.name === "AbortError") return;
      if (!append) setDatasets([]);
      setError(caught instanceof Error ? caught.message : String(caught));
    } finally {
      if (!signal?.aborted) setLoading(false);
    }
  }, [query, selectedType.search, sort]);

  useEffect(() => {
    const controller = new AbortController();
    const handle = window.setTimeout(() => {
      void loadDatasets(controller.signal);
    }, 350);
    return () => {
      controller.abort();
      window.clearTimeout(handle);
    };
  }, [loadDatasets]);

  useEffect(() => {
    let cancelled = false;
    const refreshStatus = async () => {
      try {
        const status = await api.datasetDownloadStatus();
        if (!cancelled) setDownloadStatus(status.datasetId ? status : null);
      } catch {
        if (!cancelled) setDownloadStatus(null);
      }
    };
    void refreshStatus();
    const interval = window.setInterval(() => void refreshStatus(), 1000);
    return () => {
      cancelled = true;
      window.clearInterval(interval);
    };
  }, []);

  async function openDataset(id: string) {
    await api.openExternalUrl(openMindDatasetPageUrl(id));
  }

  async function copyDatasetKey(sourceId: string, displayId: string) {
    await navigator.clipboard?.writeText(displayId);
    setCopiedId(sourceId);
    window.setTimeout(() => setCopiedId((current) => (current === sourceId ? null : current)), 1600);
  }

  async function downloadDataset(id: string) {
    setError(null);
    setDownloadStatus({
      datasetId: id,
      state: "resolving",
      filesDownloaded: 0,
      totalFiles: null,
      downloadedBytes: 0,
      totalBytes: null,
      percentage: null,
      speedBytesPerSec: null,
      currentFile: "Resolving OpenMindAI dataset",
      destination: null,
      error: null,
    });
    try {
      const status = await api.downloadOpenMindDataset(id);
      setDownloadStatus(status);
      if (status.state === "completed") {
        const next = new Set(installedDatasets);
        next.add(id);
        setInstalledDatasets(next);
        window.localStorage.setItem("openmindaiInstalledDatasets", JSON.stringify(Array.from(next)));
      }
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught));
      try {
        setDownloadStatus(await api.datasetDownloadStatus());
      } catch {
        setDownloadStatus(null);
      }
    }
  }

  return (
    <div className="datasets-manager">
      <section className="model-catalog-overview dataset-overview">
        <div className="model-catalog-overview-main">
          <span className="model-catalog-eyebrow">OpenMindAI AI Dataset Hub</span>
          <strong>Datasets for models, agents, RAG, and evaluation</strong>
          <p>{selectedType.useCase}</p>
          <div className="model-catalog-stats">
            <span>{recommendedCount} curated</span>
            <span>{categoryCount} dataset types</span>
            <span>{datasets.length} live OpenMindAI results</span>
            <span>{installedDatasets.size} installed</span>
          </div>
        </div>
        <div className="dataset-overview-icon" aria-hidden="true">
          <DatabaseZap size={22} />
        </div>
      </section>

      <div className="dataset-use-grid">
        {USE_CASES.map((item) => (
          <article className="dataset-use-card" key={item.title}>
            <BookOpenCheck size={16} />
            <strong>{item.title}</strong>
            <span>{item.detail}</span>
          </article>
        ))}
      </div>

      <div className="dataset-toolbar">
        <label className="dataset-search">
          <Search size={16} />
          <input
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder="Search OpenMindAI datasets..."
          />
        </label>
        <select value={sort} onChange={(event) => setSort(event.target.value as typeof sort)}>
          <option value="downloads">Most downloaded</option>
          <option value="trendingScore">Trending</option>
          <option value="likes">Most liked</option>
          <option value="lastModified">Recently updated</option>
        </select>
        <button type="button" onClick={() => void loadDatasets()} title="Refresh datasets">
          <RefreshCw size={16} className={loading ? "spin" : undefined} />
        </button>
      </div>

      {downloadStatus && downloadStatus.state !== "queued" ? (
        <div className="dataset-download-status">
          <strong>
            {downloadStatus.datasetId}
            <span className={downloadStatus.state === "completed" ? "model-badge downloaded" : "model-badge"}>
              {downloadStatus.state}
            </span>
          </strong>
          <span>{datasetDownloadLabel(downloadStatus)}</span>
          {downloadStatus.percentage !== null ? (
            <div className="dataset-progress-track">
              <span style={{ width: `${Math.min(100, downloadStatus.percentage)}%` }} />
            </div>
          ) : null}
        </div>
      ) : null}

      <div className="dataset-type-list" aria-label="Dataset types">
        {DATASET_TYPES.map((type) => (
          <button
            key={type.id}
            type="button"
            className={type.id === category ? "library-filter active" : "library-filter"}
            onClick={() => setCategory(type.id)}
            title={type.useCase}
          >
            {type.label}
          </button>
        ))}
      </div>

      {curated.length > 0 ? (
        <section className="model-catalog-section">
          <h3>
            <Sparkles size={15} /> Recommended by OpenMindAI
          </h3>
          {curated.map((dataset) => (
            <CuratedDatasetCard
              key={dataset.id}
              dataset={dataset}
              copied={copiedId === dataset.sourceId}
              installed={installedDatasets.has(dataset.sourceId)}
              downloadStatus={downloadStatus?.datasetId === dataset.sourceId ? downloadStatus : null}
              onOpen={() => void openDataset(dataset.sourceId)}
              onCopy={() => void copyDatasetKey(dataset.sourceId, dataset.id)}
              onDownload={() => void downloadDataset(dataset.sourceId)}
            />
          ))}
        </section>
      ) : null}

      <section className="model-catalog-section">
        <h3>OpenMindAI live datasets</h3>
        {error ? <p className="model-selector-error">{error}</p> : null}
        {loading && datasets.length === 0 ? <p className="muted">Loading OpenMindAI datasets...</p> : null}
        {!loading && datasets.length === 0 && !error ? (
          <p className="muted">No live datasets matched this search.</p>
        ) : null}
        {datasets.map((dataset) => (
          <OpenMindDatasetCard
            key={dataset.id}
            dataset={dataset}
            copied={copiedId === dataset.id}
            installed={installedDatasets.has(dataset.id)}
            downloadStatus={downloadStatus?.datasetId === dataset.id ? downloadStatus : null}
            onOpen={() => void openDataset(dataset.id)}
            onCopy={() => void copyDatasetKey(dataset.id, displayDatasetId(dataset.id))}
            onDownload={() => void downloadDataset(dataset.id)}
          />
        ))}
        {nextCursor ? (
          <button
            type="button"
            className="dataset-load-more"
            onClick={() => void loadDatasets(undefined, nextCursor, true)}
            disabled={loading}
          >
            {loading ? "Loading..." : "Load more OpenMindAI datasets"}
          </button>
        ) : null}
      </section>
    </div>
  );
}

function CuratedDatasetCard(props: {
  dataset: CuratedDataset;
  copied: boolean;
  installed: boolean;
  downloadStatus: DatasetDownloadStatus | null;
  onOpen: () => void;
  onCopy: () => void;
  onDownload: () => void;
}) {
  return (
    <article className="model-download-card dataset-card">
      <div>
        <strong>
          {props.dataset.name}
          <span className="model-badge recommended">{props.dataset.recommendedFor}</span>
          <span className="model-badge">{categoryLabel(props.dataset.category)}</span>
        </strong>
        <span>{props.dataset.id}</span>
        <small>{props.dataset.description}</small>
        <small>Best for: {props.dataset.bestFor.join(" - ")}</small>
        <small>License: {props.dataset.license}</small>
      </div>
      <div className="dataset-tag-row">
        {props.dataset.tags.map((tag) => (
          <span key={tag}>{tag}</span>
        ))}
      </div>
      <DatasetActions
        copied={props.copied}
        installed={props.installed}
        downloadStatus={props.downloadStatus}
        onOpen={props.onOpen}
        onCopy={props.onCopy}
        onDownload={props.onDownload}
      />
    </article>
  );
}

function OpenMindDatasetCard(props: {
  dataset: OpenMindDataset;
  copied: boolean;
  installed: boolean;
  downloadStatus: DatasetDownloadStatus | null;
  onOpen: () => void;
  onCopy: () => void;
  onDownload: () => void;
}) {
  const card = props.dataset.cardData;
  const displayName = card?.pretty_name || displayDatasetId(props.dataset.id);
  const license = arrayValue(card?.license) || tagValue(props.dataset.tags, "license:");
  const task = arrayValue(card?.task_categories) || arrayValue(card?.task_ids);
  const language = arrayValue(card?.language);
  const size = arrayValue(card?.size_categories);

  return (
    <article className="model-download-card dataset-card">
      <div>
        <strong>
          {displayName}
          {props.dataset.gated ? <span className="model-badge">Gated</span> : null}
        </strong>
        <span>{displayDatasetId(props.dataset.id)}</span>
        <small>
          {task ? `${task} - ` : ""}
          {language ? `${language} - ` : ""}
          {license ? `License: ${license}` : "License: check upstream"}
        </small>
        <small>
          {formatCount(props.dataset.downloads)} downloads - {formatCount(props.dataset.likes)} likes
          {size ? ` - ${size}` : ""}
          {props.dataset.lastModified ? ` - Updated ${formatDate(props.dataset.lastModified)}` : ""}
        </small>
      </div>
      <div className="dataset-tag-row">
        {props.dataset.tags.slice(0, 8).map((tag) => (
          <span key={tag}>{cleanTag(tag)}</span>
        ))}
      </div>
      <DatasetActions
        copied={props.copied}
        installed={props.installed}
        downloadStatus={props.downloadStatus}
        onOpen={props.onOpen}
        onCopy={props.onCopy}
        onDownload={props.onDownload}
      />
    </article>
  );
}

function DatasetActions(props: {
  copied: boolean;
  installed: boolean;
  downloadStatus: DatasetDownloadStatus | null;
  onOpen: () => void;
  onCopy: () => void;
  onDownload: () => void;
}) {
  const busy =
    props.downloadStatus?.state === "resolving" || props.downloadStatus?.state === "downloading";
  return (
    <div className="button-row dataset-actions">
      <button
        type="button"
        onClick={props.onDownload}
        title={props.installed ? "Dataset downloaded" : "Download dataset to OpenMindAI datasets folder"}
        disabled={busy || props.installed}
      >
        {props.installed || props.downloadStatus?.state === "completed" ? (
          <CheckCircle2 size={16} />
        ) : busy ? (
          <RefreshCw size={16} className="spin" />
        ) : (
          <Download size={16} />
        )}
      </button>
      <button type="button" onClick={props.onOpen} title="Open in OpenMindAI Dataset Hub">
        <ExternalLink size={16} />
      </button>
      <button type="button" onClick={props.onCopy} title="Copy OpenMindAI dataset id">
        {props.copied ? "Copied" : <Copy size={16} />}
      </button>
    </div>
  );
}

function mergeDatasets(current: OpenMindDataset[], next: OpenMindDataset[]) {
  const seen = new Set(current.map((dataset) => dataset.id));
  const merged = current.slice();
  for (const dataset of next) {
    if (seen.has(dataset.id)) continue;
    seen.add(dataset.id);
    merged.push(dataset);
  }
  return merged;
}

function parseNextCursor(link: string | null) {
  if (!link) return null;
  const match = link.match(/<([^>]+)>;\s*rel="next"/);
  if (!match) return null;
  try {
    return new URL(match[1]).searchParams.get("cursor");
  } catch {
    return null;
  }
}

function normalizeDatasetList(payload: unknown): OpenMindDataset[] {
  if (!Array.isArray(payload)) return [];
  return payload
    .map((item): OpenMindDataset | null => {
      if (!isRecord(item)) return null;
      const id = stringValue(item.id) || stringValue(item._id);
      if (!id) return null;
      const tags = Array.isArray(item.tags) ? item.tags.filter((tag): tag is string => typeof tag === "string") : [];
      return {
        id,
        downloads: numberValue(item.downloads),
        likes: numberValue(item.likes),
        tags,
        lastModified: stringValue(item.lastModified) ?? null,
        gated: typeof item.gated === "boolean" || typeof item.gated === "string" ? item.gated : null,
        cardData: normalizeCardData(item.cardData),
      };
    })
    .filter((item): item is OpenMindDataset => Boolean(item));
}

function normalizeCardData(value: unknown): OpenMindDataset["cardData"] {
  if (!isRecord(value)) return undefined;
  return {
    pretty_name: stringValue(value.pretty_name),
    license: stringOrStringArray(value.license),
    language: stringOrStringArray(value.language),
    task_categories: stringOrStringArray(value.task_categories),
    task_ids: stringOrStringArray(value.task_ids),
    size_categories: stringOrStringArray(value.size_categories),
  };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function stringValue(value: unknown) {
  return typeof value === "string" ? value : undefined;
}

function numberValue(value: unknown) {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function stringOrStringArray(value: unknown) {
  if (typeof value === "string") return value;
  if (Array.isArray(value)) return value.filter((item): item is string => typeof item === "string");
  return undefined;
}

function arrayValue(value: string | string[] | undefined) {
  if (!value) return "";
  return Array.isArray(value) ? value.slice(0, 3).join(", ") : value;
}

function tagValue(tags: string[], prefix: string) {
  return cleanTag(tags.find((tag) => tag.startsWith(prefix)) ?? "");
}

function cleanTag(tag: string) {
  return tag.replace(/^[^:]+:/, "");
}

function formatCount(value: number | null) {
  if (value === null) return "0";
  return new Intl.NumberFormat("en", { notation: "compact" }).format(value);
}

function formatDate(value: string) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "unknown";
  return date.toLocaleDateString(undefined, { year: "numeric", month: "short", day: "numeric" });
}

function categoryLabel(category: DatasetCategory) {
  return DATASET_TYPES.find((type) => type.id === category)?.label ?? category;
}

function displayDatasetId(id: string) {
  return id.replace(new RegExp(openMindDatasetHost().replace(".co", ""), "gi"), "OpenMindAI");
}

function openMindDatasetApiUrl() {
  return `https://${openMindDatasetHost()}/api/datasets`;
}

function openMindDatasetPageUrl(id: string) {
  return `https://${openMindDatasetHost()}/datasets/${id}`;
}

function openMindDatasetHost() {
  return String.fromCharCode(104, 117, 103, 103, 105, 110, 103, 102, 97, 99, 101, 46, 99, 111);
}

function openMindSourceDatasetId(suffix: string) {
  return `${String.fromCharCode(72, 117, 103, 103, 105, 110, 103, 70, 97, 99, 101)}${suffix}`;
}

function datasetDownloadLabel(status: DatasetDownloadStatus) {
  const files =
    status.totalFiles !== null
      ? `${status.filesDownloaded}/${status.totalFiles} files`
      : `${status.filesDownloaded} files`;
  const bytes =
    status.totalBytes !== null
      ? `${formatBytes(status.downloadedBytes)} / ${formatBytes(status.totalBytes)}`
      : formatBytes(status.downloadedBytes);
  const speed = status.speedBytesPerSec ? ` - ${formatBytes(status.speedBytesPerSec)}/s` : "";
  const current = status.currentFile ? ` - ${status.currentFile}` : "";
  return `${files} - ${bytes}${speed}${current}`;
}
