import { check } from "@tauri-apps/plugin-updater";
import { api } from "../api";
import { createUpdateCoordinator } from "./updateCoordinator";

/** Host the updater manifest and installers are served from (see tauri.conf.json). */
const UPDATE_HOST_URL = "https://github.com/";

/**
 * The one owner of Tauri updater handles for this renderer. Automatic
 * startup/focus/interval checks and manual Settings actions share one check,
 * download, progress stream, installed-version state, and resource lifetime.
 */
export const appUpdateCoordinator = createUpdateCoordinator(async () => {
  const proxy = await api.networkProxyForUrl(UPDATE_HOST_URL).catch(() => null);
  const update = await check(proxy ? { proxy } : undefined);
  if (!update) return null;

  return {
    version: update.version,
    downloadAndInstall: async (onProgress: (event: import("./updateCoordinator").UpdateProgress) => void) => {
      await update.downloadAndInstall((event) => {
        if (event.event === "Started") {
          onProgress({ event: "Started", contentLength: event.data.contentLength ?? null });
        } else if (event.event === "Progress") {
          onProgress({ event: "Progress", chunkLength: event.data.chunkLength });
        } else if (event.event === "Finished") {
          onProgress({ event: "Finished" });
        }
      });
    },
    close: async () => { await update.close(); },
  };
});
