import { invoke } from "@tauri-apps/api/core";

const isTauri = "__TAURI_INTERNALS__" in window;

export interface PlatformCapabilities {
  target: "android" | "ios" | "windows" | "macos" | "linux" | "unknown";
  mobile: boolean;
  desktop: boolean;
  localWorkspace: boolean;
  fullPcTerminal: boolean;
  managedDesktopRuntime: boolean;
  mobileModelRuntimeReady: boolean;
}

export function isLikelyNativeMobile() {
  const userAgent = navigator.userAgent;
  const explicitMobile = /Android|iPhone|iPad|iPod/i.test(userAgent);
  const iPadDesktopMode = navigator.platform === "MacIntel" && navigator.maxTouchPoints > 1;
  return explicitMobile || iPadDesktopMode;
}

function browserFallback(): PlatformCapabilities {
  const mobile = isLikelyNativeMobile();
  return {
    target: /Android/i.test(navigator.userAgent) ? "android" : mobile ? "ios" : "unknown",
    mobile,
    desktop: !mobile,
    localWorkspace: !mobile,
    fullPcTerminal: !mobile,
    managedDesktopRuntime: !mobile,
    mobileModelRuntimeReady: false,
  };
}

let cachedCapabilities: PlatformCapabilities | null = null;

export async function getPlatformCapabilities(): Promise<PlatformCapabilities> {
  if (cachedCapabilities) return cachedCapabilities;
  if (!isTauri) {
    cachedCapabilities = browserFallback();
    return cachedCapabilities;
  }

  try {
    cachedCapabilities = await invoke<PlatformCapabilities>("platform_capabilities");
  } catch {
    cachedCapabilities = browserFallback();
  }
  return cachedCapabilities;
}

export function bootstrapPlatformDataset() {
  const fallback = browserFallback();
  applyDataset(fallback);
  void getPlatformCapabilities().then(applyDataset);
}

function applyDataset(capabilities: PlatformCapabilities) {
  document.documentElement.dataset.openmindPlatform = capabilities.mobile ? "mobile" : "desktop";
  document.documentElement.dataset.openmindTarget = capabilities.target;
  document.documentElement.dataset.fullPcTerminal = String(capabilities.fullPcTerminal);
  document.documentElement.dataset.localWorkspace = String(capabilities.localWorkspace);
}
