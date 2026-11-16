/**
 * Owns update handles for the entire desktop window. React mounts, focus
 * events, timed checks and manual Settings actions share this coordinator.
 * Only this module's caller owns or closes the updater plugin handle.
 */
export interface UpdateHandle {
  version: string;
  downloadAndInstall: (onProgress: (event: UpdateProgress) => void) => Promise<void>;
  close: () => Promise<void>;
}

export type UpdateProgress =
  | { event: "Started"; contentLength: number | null }
  | { event: "Progress"; chunkLength: number }
  | { event: "Finished" };

export type UpdateState = "idle" | "checking" | "available" | "downloading" | "ready" | "failed" | "up-to-date";

export interface UpdateSnapshot {
  state: UpdateState;
  version: string | null;
  downloadedBytes: number;
  totalBytes: number | null;
  error: string | null;
}

export function createUpdateCoordinator(checkForUpdate: () => Promise<UpdateHandle | null>) {
  let snapshot: UpdateSnapshot = {
    state: "idle", version: null, downloadedBytes: 0, totalBytes: null, error: null,
  };
  let pending: UpdateHandle | null = null;
  let checkPromise: Promise<UpdateSnapshot> | null = null;
  let installPromise: Promise<UpdateSnapshot> | null = null;
  const listeners = new Set<(state: UpdateSnapshot) => void>();

  function publish(changes: Partial<UpdateSnapshot>): UpdateSnapshot {
    snapshot = { ...snapshot, ...changes };
    for (const listener of listeners) listener(snapshot);
    return snapshot;
  }

  function getSnapshot(): UpdateSnapshot {
    return snapshot;
  }

  function subscribe(listener: (state: UpdateSnapshot) => void): () => void {
    listeners.add(listener);
    return () => { listeners.delete(listener); };
  }

  function check(): Promise<UpdateSnapshot> {
    // Never replace/close a resource while it is downloading or installed.
    if (installPromise || snapshot.state === "ready") return Promise.resolve(snapshot);
    if (checkPromise) return checkPromise;

    checkPromise = (async () => {
      publish({ state: "checking", error: null });
      try {
        const update = await checkForUpdate();
        if (snapshot.state === "downloading" || snapshot.state === "ready") {
          await update?.close();
          return snapshot;
        }
        if (!update) {
          const previous = pending;
          pending = null;
          await previous?.close();
          return publish({
            state: "up-to-date", version: null, downloadedBytes: 0, totalBytes: null, error: null,
          });
        }
        if (pending?.version === update.version) {
          await update.close();
        } else {
          const previous = pending;
          pending = update;
          await previous?.close();
        }
        return publish({
          state: "available", version: pending?.version ?? null,
          downloadedBytes: 0, totalBytes: null, error: null,
        });
      } catch (error) {
        return publish({ state: "failed", error: String(error) });
      }
    })().finally(() => { checkPromise = null; });
    return checkPromise;
  }

  function downloadAndInstall(): Promise<UpdateSnapshot> {
    if (installPromise) return installPromise;
    if (snapshot.state === "ready") return Promise.resolve(snapshot);

    // Start (or reuse) a check before reserving the install flight, so
    // an absent cached handle can be fetched without racing this guard.
    const readiness = checkPromise ?? (pending ? null : check());
    installPromise = (async () => {
      if (readiness) await readiness;
      const update = pending;
      if (!update) {
        const message = "No application update is available to install.";
        publish({ state: "failed", error: message });
        throw new Error(message);
      }
      const version = update.version;
      publish({
        state: "downloading", version, downloadedBytes: 0, totalBytes: null, error: null,
      });
      try {
        await update.downloadAndInstall((event) => {
          if (event.event === "Started") {
            publish({ totalBytes: event.contentLength });
          } else if (event.event === "Progress") {
            publish({ downloadedBytes: snapshot.downloadedBytes + event.chunkLength });
          }
        });
        pending = null;
        await update.close().catch(() => undefined);
        return publish({ state: "ready", version, error: null });
      } catch (error) {
        // Failed plugin handles are never reused: a fresh check is required
        // before retry, and the version remains visible with its error.
        pending = null;
        await update.close().catch(() => undefined);
        publish({ state: "failed", version, error: String(error) });
        throw error;
      }
    })().finally(() => { installPromise = null; });
    return installPromise;
  }

  return { check, downloadAndInstall, subscribe, getSnapshot };
}
