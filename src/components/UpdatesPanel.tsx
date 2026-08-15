import { useEffect, useState } from "react";
import { check as checkForAppUpdate, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { api } from "../api";
import type { AppPreferences, ModelCatalogReport } from "../types";
import { SaveBar, SelectRow, ToggleRow } from "./SettingsDialog";
import { formatBytes, formatError } from "../lib/format";
import { notifyUser } from "../lib/notify";

type DownloadState = "idle" | "downloading" | "ready" | "failed";

export function UpdatesPanel(props: {
  preferenceDraft: AppPreferences;
  setPreference: <K extends keyof AppPreferences>(key: K, value: AppPreferences[K]) => void;
  saveState: "idle" | "saving" | "saved";
  onSave: () => void;
}) {
  const [appUpdateResult, setAppUpdateResult] = useState<string | null>(null);
  const [appUpdateChecking, setAppUpdateChecking] = useState(false);
  const [pendingUpdate, setPendingUpdate] = useState<Update | null>(null);
  const [downloadState, setDownloadState] = useState<DownloadState>("idle");
  const [downloadedBytes, setDownloadedBytes] = useState(0);
  const [totalBytes, setTotalBytes] = useState<number | null>(null);
  const [downloadError, setDownloadError] = useState<string | null>(null);
  const [modelReport, setModelReport] = useState<ModelCatalogReport | null>(null);
  const [modelChecking, setModelChecking] = useState(false);

  // The `Update` object holds a backend resource handle -- release it
  // whenever it's replaced by a fresh check or the panel unmounts.
  useEffect(() => {
    return () => void pendingUpdate?.close();
  }, [pendingUpdate]);

  const checkAppUpdate = async () => {
    setAppUpdateChecking(true);
    setAppUpdateResult(null);
    try {
      const update = await checkForAppUpdate();
      setPendingUpdate((current) => {
        void current?.close();
        return update;
      });
      setDownloadState("idle");
      setDownloadError(null);
      setAppUpdateResult(update ? `OpenMindAI v${update.version} is available.` : "You're up to date.");
    } catch {
      // Update checks must never surface a scary raw error — a stale
      // "checked, nothing found" style state is the honest quiet fallback.
      setAppUpdateResult("Couldn't check for updates right now.");
    } finally {
      setAppUpdateChecking(false);
    }
  };

  const downloadAndInstall = async () => {
    if (!pendingUpdate) return;
    setDownloadState("downloading");
    setDownloadError(null);
    setDownloadedBytes(0);
    setTotalBytes(null);
    try {
      await pendingUpdate.downloadAndInstall((event) => {
        if (event.event === "Started") {
          setTotalBytes(event.data.contentLength ?? null);
        } else if (event.event === "Progress") {
          setDownloadedBytes((current) => current + event.data.chunkLength);
        }
      });
      setDownloadState("ready");
      void notifyUser("OpenMindAI is ready to update", "Restart OpenMindAI to finish installing the update.");
    } catch (caught) {
      setDownloadState("failed");
      setDownloadError(formatError(caught));
    }
  };

  const checkModelUpdate = async () => {
    setModelChecking(true);
    try {
      setModelReport(await api.checkModelUpdates());
    } catch {
      setModelReport(null);
    } finally {
      setModelChecking(false);
    }
  };

  return (
    <>
      <ToggleRow
        label="Automatically check for updates"
        checked={props.preferenceDraft.autoCheckAppUpdates}
        onChange={(value) => props.setPreference("autoCheckAppUpdates", value)}
      />
      <ToggleRow
        label="Automatically download application updates"
        checked={props.preferenceDraft.autoDownloadAppUpdates}
        onChange={(value) => props.setPreference("autoDownloadAppUpdates", value)}
      />
      <ToggleRow
        label="Notify me when recommended model updates are available"
        checked={props.preferenceDraft.notifyModelUpdates}
        onChange={(value) => props.setPreference("notifyModelUpdates", value)}
      />
      <ToggleRow
        label="Automatically download large AI model updates"
        checked={props.preferenceDraft.autoDownloadModelUpdates}
        onChange={(value) => props.setPreference("autoDownloadModelUpdates", value)}
      />
      <SelectRow
        label="Update channel"
        value={props.preferenceDraft.updateChannel}
        options={["Stable"]}
        onChange={(value) => props.setPreference("updateChannel", value)}
      />
      <SaveBar state={props.saveState} onSave={props.onSave} />

      <div className="maintenance-block">
        <h3>Application Updates</h3>
        <div className="button-row">
          <button
            type="button"
            className="ghost-button"
            onClick={() => void checkAppUpdate()}
            disabled={appUpdateChecking}
          >
            {appUpdateChecking ? "Checking…" : "Check for Updates Now"}
          </button>
        </div>
        {appUpdateResult ? <p className="muted">{appUpdateResult}</p> : null}

        {pendingUpdate && downloadState === "idle" ? (
          <div className="button-row">
            <button type="button" className="primary-button" onClick={() => void downloadAndInstall()}>
              Download &amp; Install Update
            </button>
          </div>
        ) : null}

        {downloadState === "downloading" ? (
          <div className="setup-progress">
            <div className="setup-progress-bar">
              <div
                className="setup-progress-fill"
                style={{
                  width: `${totalBytes ? Math.min(100, Math.max(0, (downloadedBytes / totalBytes) * 100)) : 0}%`,
                }}
              />
            </div>
            <p className="muted">
              {formatBytes(downloadedBytes)}
              {totalBytes ? ` / ${formatBytes(totalBytes)}` : ""}
            </p>
          </div>
        ) : null}

        {downloadState === "ready" ? (
          <div className="button-row">
            <p className="muted">Update downloaded. Restart OpenMindAI to finish installing it.</p>
            <button type="button" className="primary-button" onClick={() => void relaunch()}>
              Restart Now
            </button>
          </div>
        ) : null}

        {downloadError ? <p className="setup-warning">{downloadError}</p> : null}
      </div>

      <div className="maintenance-block">
        <h3>Model Updates</h3>
        <div className="button-row">
          <button
            type="button"
            className="ghost-button"
            onClick={() => void checkModelUpdate()}
            disabled={modelChecking}
          >
            {modelChecking ? "Checking…" : "Check for Model Updates"}
          </button>
        </div>
        {modelReport ? (
          <ul className="maintenance-check-list">
            {modelReport.entries.map((status) => (
              <li key={status.entry.id}>
                <span>{status.entry.name}</span>
                <span className="muted">
                  {status.installed ? "Installed" : "Not installed"}
                  {status.updateAvailable ? " · Update available" : " · Up to date"}
                  {!status.compatible ? " · Hardware may not support this model" : ""}
                </span>
              </li>
            ))}
          </ul>
        ) : null}
      </div>
    </>
  );
}
