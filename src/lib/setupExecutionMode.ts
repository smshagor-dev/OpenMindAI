import type { ExecutionMode } from "./executionRouting";

// Kept across the mandatory native-app relaunch after choosing the database root.
// The choice is subsequently saved to native SQLite preferences; storage only
// carries the in-progress first-run wizard, not authentication credentials.
const MODE_KEY = "openmindai.setup.executionMode.v1";
const PREPARING_KEY = "openmindai.setup.preparing.v1";

export function isExecutionMode(value: string | null): value is ExecutionMode {
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
  window.localStorage.setItem(MODE_KEY, mode);
  window.localStorage.setItem(PREPARING_KEY, "pending");
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
