import { useEffect, useState } from "react";
import { api } from "../api";
import type {
  BackupInfo,
  DiagnosticReport,
  HardwareProfile,
  LlamaRuntimeStatus,
  ModelRecord,
  PortableRootInfo,
  RepairSummary,
  StorageSummary,
} from "../types";
import { formatBytes, formatError, formatTime } from "../lib/format";

export function MaintenanceCenter(props: {
  root: PortableRootInfo | null;
  hardware: HardwareProfile | null;
  storage: StorageSummary | null;
  models: ModelRecord[];
  runtimeStatus: LlamaRuntimeStatus | null;
  refresh: () => void | Promise<void>;
}) {
  const [diagnostics, setDiagnostics] = useState<DiagnosticReport | null>(null);
  const [diagnosticsRunning, setDiagnosticsRunning] = useState(false);
  const [repairSummary, setRepairSummary] = useState<RepairSummary | null>(null);
  const [repairRunning, setRepairRunning] = useState(false);
  const [repairError, setRepairError] = useState<string | null>(null);
  const [backups, setBackups] = useState<BackupInfo[]>([]);
  const [backupCreating, setBackupCreating] = useState(false);
  const [backupError, setBackupError] = useState<string | null>(null);
  const [recentLogs, setRecentLogs] = useState<string | null>(null);
  const [logsLoading, setLogsLoading] = useState(false);

  useEffect(() => {
    void api.listBackups().then(setBackups);
  }, []);

  const modelReady = props.models.some((model) => model.state === "ready" || model.state === "loaded");

  const runDiagnostics = async () => {
    setDiagnosticsRunning(true);
    try {
      setDiagnostics(await api.runDiagnostics());
    } finally {
      setDiagnosticsRunning(false);
    }
  };

  const repair = async () => {
    setRepairRunning(true);
    setRepairError(null);
    try {
      setRepairSummary(await api.repairInstallation());
      await props.refresh();
      await runDiagnostics();
    } catch (caught) {
      setRepairError(formatError(caught));
    } finally {
      setRepairRunning(false);
    }
  };

  const loadRecentLogs = async () => {
    setLogsLoading(true);
    try {
      setRecentLogs(await api.readRecentLogs());
    } finally {
      setLogsLoading(false);
    }
  };

  const createBackup = async () => {
    setBackupCreating(true);
    setBackupError(null);
    try {
      await api.backupDatabase();
      setBackups(await api.listBackups());
    } catch (caught) {
      setBackupError(formatError(caught));
    } finally {
      setBackupCreating(false);
    }
  };

  return (
    <>
      <div className="maintenance-block">
        <h3>System Health</h3>
        <Info label="Storage" value={props.root?.writable ? "Healthy" : "Not writable"} />
        <Info label="AI Runtime" value={props.runtimeStatus?.available ? "Ready" : "Not installed"} />
        <Info label="AI Model" value={modelReady ? "Verified" : "Not installed"} />
        <Info label="Available space" value={formatBytes(props.storage?.availableBytes) ?? "Unknown"} />
      </div>

      <div className="maintenance-block">
        <h3>Diagnostics</h3>
        <div className="button-row">
          <button type="button" className="ghost-button" onClick={() => void runDiagnostics()} disabled={diagnosticsRunning}>
            {diagnosticsRunning ? "Running…" : "Run Diagnostics"}
          </button>
        </div>
        {diagnostics ? (
          <ul className="maintenance-check-list">
            {diagnostics.checks.map((check) => (
              <li key={check.id} className={`maintenance-check maintenance-check-${check.status}`}>
                <span>{check.label}</span>
                <span className="muted">{check.detail ?? (check.status === "ok" ? "OK" : check.status)}</span>
              </li>
            ))}
          </ul>
        ) : null}
      </div>

      <div className="maintenance-block">
        <h3>Repair</h3>
        <p className="muted">
          Re-verifies required folders and re-installs the AI engine or AI model only if they're missing.
          Your conversations and settings are never touched.
        </p>
        <div className="button-row">
          <button type="button" className="ghost-button" onClick={() => void repair()} disabled={repairRunning}>
            {repairRunning ? "Repairing…" : "Repair OpenMindAI"}
          </button>
        </div>
        {repairError ? <p className="setup-warning">{repairError}</p> : null}
        {repairSummary ? (
          <ul className="maintenance-check-list">
            {repairSummary.actions.map((action, index) => (
              <li key={index}>{action}</li>
            ))}
          </ul>
        ) : null}
      </div>

      <div className="maintenance-block">
        <h3>Backups</h3>
        <div className="button-row">
          <button type="button" className="ghost-button" onClick={() => void createBackup()} disabled={backupCreating}>
            {backupCreating ? "Creating…" : "Create Backup Now"}
          </button>
          <button type="button" className="ghost-button" onClick={() => void api.openMaintenanceFolder("backups")}>
            Open Backups Folder
          </button>
        </div>
        {backupError ? <p className="setup-warning">{backupError}</p> : null}
        {backups.length > 0 ? (
          <ul className="maintenance-check-list">
            {backups.map((backup) => (
              <li key={backup.name}>
                <span>{backup.name}</span>
                <span className="muted">
                  {formatBytes(backup.sizeBytes)} · {formatTime(backup.createdAt)}
                </span>
              </li>
            ))}
          </ul>
        ) : (
          <p className="muted">No backups yet.</p>
        )}
      </div>

      <div className="maintenance-block">
        <h3>Logs</h3>
        <div className="button-row">
          <button type="button" className="ghost-button" onClick={() => void loadRecentLogs()} disabled={logsLoading}>
            {logsLoading ? "Loading…" : "View Recent Activity"}
          </button>
          <button type="button" className="ghost-button" onClick={() => void api.openMaintenanceFolder("logs")}>
            Open Logs Folder
          </button>
        </div>
        {recentLogs !== null ? (
          recentLogs.trim() ? (
            <pre className="log-viewer">{recentLogs}</pre>
          ) : (
            <p className="muted">No recent activity recorded yet.</p>
          )
        ) : null}
      </div>
    </>
  );
}

function Info(props: { label: string; value?: string | null }) {
  return (
    <div className="info-row">
      <span>{props.label}</span>
      <strong>{props.value ?? "Unavailable"}</strong>
    </div>
  );
}
