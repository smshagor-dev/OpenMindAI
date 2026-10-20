import { useEffect, useState } from "react";
import { relaunch } from "@tauri-apps/plugin-process";
import { api } from "../api";
import type { AppPreferences, ModelCatalogReport } from "../types";
import { SaveBar, SelectRow, ToggleRow } from "./SettingsDialog";
import { appUpdateCoordinator } from "../lib/appUpdate";
import { formatBytes } from "../lib/format";

export function UpdatesPanel(props: {
  preferenceDraft: AppPreferences;
  setPreference: <K extends keyof AppPreferences>(key: K, value: AppPreferences[K]) => void;
  saveState: "idle" | "saving" | "saved";
  onSave: () => void;
}) {
  const [appUpdateResult, setAppUpdateResult] = useState<string | null>(null);
  const [updateSnapshot, setUpdateSnapshot] = useState(appUpdateCoordinator.getSnapshot);
  const [modelReport, setModelReport] = useState<ModelCatalogReport | null>(null);
  const [modelChecking, setModelChecking] = useState(false);

  // Automatic and manual installs publish the same progress and ready state.
  useEffect(() => appUpdateCoordinator.subscribe(setUpdateSnapshot), []);

  const checkAppUpdate = async () => {
    setAppUpdateResult(null);
    const result = await appUpdateCoordinator.check();
    setAppUpdateResult(
      result.state === "failed"
        ? "Couldn't check for updates right now."
        : result.version
          ? `OpenMindAI v${result.version} is available.`
          : "You're up to date.",
    );
  };

  const downloadAndInstall = async () => {
    try {
      await appUpdateCoordinator.downloadAndInstall();
    } catch {
      // The shared coordinator publishes the failure and a fresh check
      // acquires a new installer resource before a version-specific retry.
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
            disabled={updateSnapshot.state === "checking" || updateSnapshot.state === "downloading"}
          >
            {updateSnapshot.state === "checking" ? "Checking…" : "Check for Updates Now"}
          </button>
        </div>
        {appUpdateResult ? <p className="muted">{appUpdateResult}</p> : null}

        {updateSnapshot.version && (updateSnapshot.state === "available" || updateSnapshot.state === "failed") ? (
          <div className="button-row">
            <button type="button" className="primary-button" onClick={() => void downloadAndInstall()}>
              {updateSnapshot.state === "failed" ? "Retry Download & Install" : "Download & Install Update"}
            </button>
          </div>
        ) : null}

        {updateSnapshot.state === "downloading" ? (
          <div className="setup-progress">
            <div className="setup-progress-bar">
              <div
                className="setup-progress-fill"
                style={{
                  width: `${updateSnapshot.totalBytes ? Math.min(100, Math.max(0, (updateSnapshot.downloadedBytes / updateSnapshot.totalBytes) * 100)) : 0}%`,
                }}
              />
            </div>
            <p className="muted">
              {formatBytes(updateSnapshot.downloadedBytes)}
              {updateSnapshot.totalBytes ? ` / ${formatBytes(updateSnapshot.totalBytes)}` : ""}
            </p>
          </div>
        ) : null}

        {updateSnapshot.state === "ready" ? (
          <div className="button-row">
            <p className="muted">Update downloaded. Restart OpenMindAI to finish installing it.</p>
            <button type="button" className="primary-button" onClick={() => void relaunch()}>
              Restart Now
            </button>
          </div>
        ) : null}

        {updateSnapshot.error ? <p className="setup-warning">{updateSnapshot.error}</p> : null}
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
