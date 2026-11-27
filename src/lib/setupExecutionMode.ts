import type { ExecutionMode } from "./executionRouting";

// Kept across the mandatory native-app relaunch after choosing the database root.
// The choice is subsequently saved to native SQLite preferences; storage only
// carries the in-progress first-run wizard, not authentication credentials.
const MODE_KEY = "openmindai.setup.executionMode.v1";
const PREPARING_KEY = "openmindai.setup.preparing.v1";

export function isExecutionMode(value: unknown): value is ExecutionMode {
  return value === "auto" || value === "local" || value === "cloud";
}

export function setupExecutionMode(): ExecutionMode | null {
  try {
    const value = window.localStorage.getItem(MODE_KEY);
    return isExecutionMode(value) ? value : null;
  } catch {
    return null;
  }
}

export function beginSetupPreparation(mode: ExecutionMode): void {
  // Runtime callers may bypass TypeScript. Never persist an invalid preference.
  if (!isExecutionMode(mode)) {
    throw new TypeError("Invalid setup execution mode");
  }

  // A previous attempt may have left a pending marker. Invalidate it before
  // writing a new mode, and publish pending only after that write succeeds.
  window.localStorage.removeItem(PREPARING_KEY);
  try {
    window.localStorage.setItem(MODE_KEY, mode);
    window.localStorage.setItem(PREPARING_KEY, "pending");
  } catch (error) {
    // Storage may reject a write after persisting the mode. Never resume with
    // a stale or partially written pending marker.
    try {
      window.localStorage.removeItem(PREPARING_KEY);
    } catch {
      // Best effort: the original storage failure remains authoritative.
    }
    throw error;
  }
}

export function setupPreparationPending(): boolean {
  try {
    return window.localStorage.getItem(PREPARING_KEY) === "pending" &&
      setupExecutionMode() !== null;
  } catch {
    return false;
  }
}

export function completeSetupPreparation(): void {
  window.localStorage.removeItem(PREPARING_KEY);
}
