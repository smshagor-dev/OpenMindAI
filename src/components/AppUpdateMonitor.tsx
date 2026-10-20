import { useEffect } from "react";
import { api } from "../api";
import { appUpdateCoordinator } from "../lib/appUpdate";
import { notifyUser } from "../lib/notify";

const UPDATE_CHECK_INTERVAL_MS = 15 * 60 * 1000;

function notificationKey(version: string) {
  return `openmindai:update-notified:${version}`;
}

function readyKey(version: string) {
  return `openmindai:update-ready:${version}`;
}

/** All automatic update triggers share the same coordinator as Settings. */
async function checkForPublishedUpdate() {
  const preferences = await api.preferences();
  if (!preferences.autoCheckAppUpdates) return;

  const result = await appUpdateCoordinator.check();
  if (!result.version || result.state === "failed") return;
  const version = result.version;

  const notifiedKey = notificationKey(version);
  if (window.sessionStorage.getItem(notifiedKey) !== "1") {
    // Claim the notification before awaiting OS delivery: React StrictMode
    // and overlapping focus/interval requests must not duplicate it.
    window.sessionStorage.setItem(notifiedKey, "1");
    await notifyUser("OpenMindAI update available", `Version ${version} is ready to download.`);
  }
  if (!preferences.autoDownloadAppUpdates) return;

  const installed = await appUpdateCoordinator.downloadAndInstall();
  if (installed.state !== "ready") return;
  const doneKey = readyKey(version);
  if (window.sessionStorage.getItem(doneKey) !== "1") {
    window.sessionStorage.setItem(doneKey, "1");
    await notifyUser(
      "OpenMindAI is ready to update",
      "Restart OpenMindAI to finish installing the update.",
    );
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

    // One orchestrator owns startup, periodic and focus-triggered checks.
    // Overlapping triggers and React remounts converge on the same flight.
    runCheck();
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
