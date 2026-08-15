import { isPermissionGranted, requestPermission, sendNotification } from "@tauri-apps/plugin-notification";

const isTauri = "__TAURI_INTERNALS__" in window;

/**
 * Best-effort OS notification. Silently no-ops if permission is denied,
 * unavailable, or running outside Tauri -- notifications are a nice-to-have
 * and must never surface a failure or block whatever triggered them.
 */
export async function notifyUser(title: string, body: string): Promise<void> {
  if (!isTauri) return;
  try {
    let granted = await isPermissionGranted();
    if (!granted) granted = (await requestPermission()) === "granted";
    if (granted) sendNotification({ title, body });
  } catch {
    // Best-effort only.
  }
}
