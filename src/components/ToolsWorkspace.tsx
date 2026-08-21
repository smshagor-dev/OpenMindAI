import { useRef, useState } from "react";
import type { ReactNode } from "react";
import {
  Activity,
  Bot,
  FileSearch,
  FileText,
  FileType,
  FolderOpen,
  Image,
  LibraryBig,
  Mic,
  Server,
  Sparkles,
  Video,
  Wrench,
} from "lucide-react";
import { api } from "../api";
import type { DiagnosticReport, ModelRecord, RuntimeInventory, StorageSummary } from "../types";
import { formatBytes, formatError } from "../lib/format";

export function ToolsWorkspace(props: {
  models: ModelRecord[];
  runtime: RuntimeInventory | null;
  storage: StorageSummary | null;
  onStartPrompt: (prompt: string) => void;
  onAttachFiles: (files: FileList | null) => void;
  onOpenLibrary: () => void;
  onOpenModels: () => void;
  onOpenSettings: (section?: string) => void;
}) {
  const fileInputRef = useRef<HTMLInputElement | null>(null);
  const [diagnostics, setDiagnostics] = useState<DiagnosticReport | null>(null);
  const [diagnosticsRunning, setDiagnosticsRunning] = useState(false);
  const [diagnosticsError, setDiagnosticsError] = useState<string | null>(null);
  const chatModelReady = props.models.some((model) => model.enabled && (model.state === "ready" || model.state === "loaded"));
  const runtimeReady = props.runtime?.selected != null;

  const runDiagnostics = async () => {
    setDiagnosticsRunning(true);
    setDiagnosticsError(null);
    try {
      setDiagnostics(await api.runDiagnostics());
    } catch (caught) {
      setDiagnosticsError(formatError(caught));
    } finally {
      setDiagnosticsRunning(false);
    }
  };

  return (
    <div className="tools-workspace">
      <input
        ref={fileInputRef}
        type="file"
        multiple
        className="hidden-file-input"
        onChange={(event) => {
          props.onAttachFiles(event.currentTarget.files);
          event.currentTarget.value = "";
        }}
      />
      <section className="tools-header">
        <div>
          <span className="tools-eyebrow">OpenMindAI tools</span>
          <h2>What do you want to do?</h2>
        </div>
        <div className="tools-status">
          <StatusChip label="Model" ready={chatModelReady} />
          <StatusChip label="Runtime" ready={runtimeReady} />
          <span>{formatBytes(props.storage?.availableBytes) ?? "Storage unknown"} free</span>
        </div>
      </section>

      <section className="tools-grid">
        <ToolCard icon={<Sparkles size={18} />} title="Write or debug code" onClick={() => props.onStartPrompt("Write or debug code: ")} />
        <ToolCard icon={<Image size={18} />} title="Generate an image" onClick={() => props.onStartPrompt("Generate an image of: ")} />
        <ToolCard icon={<Video size={18} />} title="Generate a video" onClick={() => props.onStartPrompt("Generate a video of: ")} />
        <ToolCard icon={<Mic size={18} />} title="Generate voice" onClick={() => props.onStartPrompt("Generate a voice narration for: ")} />
        <ToolCard icon={<FileText size={18} />} title="Create a document" onClick={() => props.onStartPrompt("Create a Word document about: ")} />
        <ToolCard icon={<FileType size={18} />} title="Create a PDF" onClick={() => props.onStartPrompt("Create a PDF document about: ")} />
        <ToolCard icon={<FileSearch size={18} />} title="Analyze a local file" onClick={() => fileInputRef.current?.click()} />
        <ToolCard icon={<LibraryBig size={18} />} title="Open generated files" onClick={props.onOpenLibrary} />
        <ToolCard icon={<Bot size={18} />} title="Manage models" onClick={props.onOpenModels} />
        <ToolCard icon={<Server size={18} />} title="AI runtime" onClick={() => props.onOpenSettings("runtime")} />
        <ToolCard icon={<FolderOpen size={18} />} title="Storage" onClick={() => props.onOpenSettings("storage")} />
        <ToolCard icon={<Wrench size={18} />} title="Maintenance" onClick={() => props.onOpenSettings("maintenance")} />
      </section>

      <section className="tools-panel">
        <div>
          <h3>System check</h3>
          <p className="muted">Run diagnostics before heavy local generation or model changes.</p>
        </div>
        <button type="button" className="ghost-button" onClick={() => void runDiagnostics()} disabled={diagnosticsRunning}>
          <Activity size={16} />
          {diagnosticsRunning ? "Running..." : "Run diagnostics"}
        </button>
        {diagnosticsError ? <p className="setup-warning">{diagnosticsError}</p> : null}
        {diagnostics ? (
          <ul className="tools-diagnostics">
            {diagnostics.checks.map((check) => (
              <li key={check.id} className={`tools-diagnostic tools-diagnostic-${check.status}`}>
                <span>{check.label}</span>
                <small>{check.detail ?? (check.status === "ok" ? "OK" : check.status)}</small>
              </li>
            ))}
          </ul>
        ) : null}
      </section>
    </div>
  );
}

function ToolCard(props: { icon: ReactNode; title: string; onClick: () => void }) {
  return (
    <button type="button" className="tool-card" onClick={props.onClick}>
      <span>{props.icon}</span>
      <strong>{props.title}</strong>
    </button>
  );
}

function StatusChip(props: { label: string; ready: boolean }) {
  return (
    <span className={props.ready ? "tools-status-ready" : ""}>
      {props.label}: {props.ready ? "Ready" : "Needed"}
    </span>
  );
}
