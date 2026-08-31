import { invoke } from "@tauri-apps/api/core";
import { getPlatformCapabilities } from "./platform";

type MobileModelReleaseResult = {
  released: boolean;
  busy: boolean;
};

const BACKGROUND_RELEASE_DELAY_MS = 3_000;
const BUSY_RETRY_DELAY_MS = 5_000;

let bootstrapped = false;
let releaseTimer: number | null = null;

function clearReleaseTimer() {
  if (releaseTimer == null) return;
  window.clearTimeout(releaseTimer);
  releaseTimer = null;
}

async function releaseCachedModel(force = false) {
  if (!force && document.visibilityState !== "hidden") return;

  const capabilities = await getPlatformCapabilities();
  if (capabilities.target !== "android" && capabilities.target !== "ios") return;
  if (!force && document.visibilityState !== "hidden") return;

  try {
    const result = await invoke<MobileModelReleaseResult>("mobile_release_inference_model");
    if (result.busy && document.visibilityState === "hidden") {
      clearReleaseTimer();
      releaseTimer = window.setTimeout(() => {
        releaseTimer = null;
        void releaseCachedModel();
      }, BUSY_RETRY_DELAY_MS);
    }
  } catch {
    // Lifecycle cleanup is best-effort. A later background transition can retry.
  }
}

function scheduleBackgroundRelease() {
  clearReleaseTimer();
  releaseTimer = window.setTimeout(() => {
    releaseTimer = null;
    void releaseCachedModel();
  }, BACKGROUND_RELEASE_DELAY_MS);
}

export function bootstrapMobileLifecyclePolicy() {
  if (bootstrapped) return;
  bootstrapped = true;

  document.addEventListener("visibilitychange", () => {
    clearReleaseTimer();
    if (document.visibilityState === "hidden") {
      scheduleBackgroundRelease();
    }
  });

  window.addEventListener("pagehide", () => {
    clearReleaseTimer();
    void releaseCachedModel(true);
  });
}
