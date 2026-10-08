import { check, type Update } from "@tauri-apps/plugin-updater";
import { api } from "../api";

/** Host the updater manifest and installers are served from (see tauri.conf.json). */
const UPDATE_HOST_URL = "https://github.com/";

/**
 * Checks for an app update through the same proxy as every other OpenMindAI
 * download. The updater plugin runs its own HTTP client, so the proxy is passed
 * explicitly; it also applies to the update's downloadAndInstall().
 */
export async function checkForAppUpdate(): Promise<Update | null> {
  const proxy = await api.networkProxyForUrl(UPDATE_HOST_URL).catch(() => null);
  return check(proxy ? { proxy } : undefined);
}
