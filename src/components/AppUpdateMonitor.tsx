import { useEffect } from "react";
import { check as checkForAppUpdate } from "@tauri-apps/plugin-updater";
import { api } from "../api";
import { notifyUser } from "../lib/notify";

const UPDATE_CHECK_INTERVAL_MS = 15 * 60 * 1000;

function notificationKey(version: string) {
  return `openmindai:update-notified:${version}`;
}

function downloadKey(version: string) {
  return `openmindai:update-download:${version}`;
}

async function checkForPublishedUpdate() {
  const preferences = await api.preferences();
  if (!preferences.autoCheckAppUpdates) return;

  const update = await checkForAppUpdate();
  if (!update) return;

  try {
    const notifiedStorageKey = notificationKey(update.version);
    if (window.sessionStorage.getItem(notifiedStorageKey) !== "1") {
      await notifyUser(
        "OpenMindAI update available",
        `Version ${update.version} is ready to download.`,
      );
      window.sessionStorage.setItem(notifiedStorageKey, "1");
    }

    if (!preferences.autoDownloadAppUpdates) return;

    const downloadStorageKey = downloadKey(update.version);
    if (window.sessionStorage.getItem(downloadStorageKey)) return;

    window.sessionStorage.setItem(downloadStorageKey, "downloading");
    try {
      await update.downloadAndInstall();
      window.sessionStorage.setItem(downloadStorageKey, "ready");
      await notifyUser(
        "OpenMindAI is ready to update",
        "Restart OpenMindAI to finish installing the update.",
      );
    } catch (error) {
      window.sessionStorage.removeItem(downloadStorageKey);
      throw error;
    }
  } finally {
    await update.close();
  }
}

export function AppUpdateMonitor() {
  useEffect(() => {
    let disposed = false;
    let inFlight = false;

    const runCheck = () => {
      if (disposed || inFlight) return;
      inFlight = true;
      void checkForPublishedUpdate()
        .catch((error) => console.warn("[updater] automatic update check failed", error))
        .finally(() => {
          inFlight = false;
        });
    };

    const handleVisibility = () => {
      if (!document.hidden) runCheck();
    };

    const interval = window.setInterval(runCheck, UPDATE_CHECK_INTERVAL_MS);
    window.addEventListener("focus", runCheck);
    document.addEventListener("visibilitychange", handleVisibility);

    return () => {
      disposed = true;
      window.clearInterval(interval);
      window.removeEventListener("focus", runCheck);
      document.removeEventListener("visibilitychange", handleVisibility);
    };
  }, []);

  return null;
}
