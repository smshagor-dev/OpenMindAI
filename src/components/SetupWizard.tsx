import { useCallback, useEffect, useRef, useState } from "react";
import { open as openFolderPicker } from "@tauri-apps/plugin-dialog";
import { relaunch } from "@tauri-apps/plugin-process";
import { api } from "../api";
import type {
  HardwareProfile,
  ModelRecord,
  PendingSetup,
  PortableRootInfo,
  RuntimeInstallStatus,
  RuntimeInventory,
  StorageLocationCheck,
} from "../types";
import { formatBytes, formatError } from "../lib/format";
import { notifyUser } from "../lib/notify";
import { ModelsManager } from "./ModelsManager";

const isTauri = "__TAURI_INTERNALS__" in window;
const DEFAULT_STORAGE_PATH = "C:\\OpenMindAI";

function previewSanitizedName(input: string): string {
  const cleaned = input
    .trim()
    .toLowerCase()
    .split("")
    .filter((char) => /[a-z0-9-_]/.test(char))
    .join("");
  return cleaned || "openmindai";
}

export function SetupWizard(props: {
  mode: "full" | "preparing";
  hardware?: HardwareProfile | null;
  models?: ModelRecord[];
  runtime?: RuntimeInventory | null;
  root?: PortableRootInfo | null;
  pending?: PendingSetup | null;
  refresh?: () => Promise<void>;
  onDismiss?: () => void;
}) {
  return (
    <div className="setup-wizard">
      <div className={`setup-card ${props.mode === "preparing" ? "setup-card-wide" : ""}`}>
        {props.mode === "full" ? (
          <FullSetupFlow pending={props.pending ?? null} />
        ) : (
          <PreparingFlow
            hardware={props.hardware ?? null}
            models={props.models ?? []}
            runtime={props.runtime ?? null}
            root={props.root ?? null}
            refresh={props.refresh}
            onDismiss={props.onDismiss}
          />
        )}
      </div>
    </div>
  );
}

type FullStep = "welcome" | "storage" | "profile" | "restarting";

function FullSetupFlow(props: { pending: PendingSetup | null }) {
  // A pending choice means the user already got past "welcome" and picked a
  // storage folder in an earlier, interrupted session — resume right after
  // that instead of restarting the wizard from scratch.
  const [step, setStep] = useState<FullStep>(props.pending ? "profile" : "welcome");
  const [path, setPath] = useState(props.pending?.root ?? DEFAULT_STORAGE_PATH);
  const [check, setCheck] = useState<StorageLocationCheck | null>(null);
  const [profileName, setProfileName] = useState(props.pending?.profileName ?? "openmindai");
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  useEffect(() => {
    if (step !== "storage") return;
    let cancelled = false;
    const timer = window.setTimeout(() => {
      void api.checkStorageLocation(path).then((result) => {
        if (!cancelled) setCheck(result);
      });
    }, 350);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [path, step]);

  // Persists the in-progress choice so closing the app mid-wizard resumes
  // here next launch instead of restarting from "welcome" with defaults.
  // Best-effort only — must never block or surface an error in the UI.
  useEffect(() => {
    if (step !== "storage" && step !== "profile") return;
    if (!path.trim()) return;
    const timer = window.setTimeout(() => {
      void api
        .saveSetupProgress(path, profileName, step === "profile" ? "profileChosen" : "storageChosen")
        .catch(() => undefined);
    }, 500);
    return () => window.clearTimeout(timer);
  }, [step, path, profileName]);

  const browse = useCallback(async () => {
    if (!isTauri) return;
    try {
      const selected = await openFolderPicker({ multiple: false, directory: true });
      if (typeof selected === "string") setPath(selected);
    } catch (caught) {
      setError(formatError(caught));
    }
  }, []);

  const finishSetup = useCallback(async () => {
    setSubmitting(true);
    setError(null);
    try {
      await api.completeSetup(path, profileName);
      setStep("restarting");
      if (isTauri) {
        window.setTimeout(() => void relaunch(), 900);
      }
    } catch (caught) {
      setError(formatError(caught));
      setSubmitting(false);
    }
  }, [path, profileName]);

  if (step === "welcome") {
    return (
      <div className="setup-step">
        <h1>Welcome to OpenMindAI</h1>
        <p className="setup-lede">A private AI assistant that runs locally on your computer.</p>
        <ul className="setup-bullets">
          <li>No cloud AI required</li>
          <li>No account required</li>
          <li>Your conversations stay local</li>
          <li>Works offline after setup</li>
        </ul>
        <div className="setup-actions">
          <button type="button" className="primary-button" onClick={() => setStep("storage")}>
            Get Started
          </button>
        </div>
      </div>
    );
  }

  if (step === "storage") {
    const availableLabel = check?.availableBytes != null ? formatBytes(check.availableBytes) : null;
    const recommendedLabel = check ? formatBytes(check.recommendedBytes) : null;
    const lowSpace = check?.availableBytes != null && check.availableBytes < check.recommendedBytes;
    return (
      <div className="setup-step">
        <h1>Choose AI Storage</h1>
        <p className="setup-lede">
          OpenMindAI stores AI models, conversations, and generated files here.
        </p>
        <div className="setup-path-row">
          <input
            type="text"
            value={path}
            onChange={(event) => setPath(event.target.value)}
            spellCheck={false}
          />
          {isTauri ? (
            <button type="button" className="ghost-button" onClick={() => void browse()}>
              Browse…
            </button>
          ) : null}
        </div>
        {check ? (
          <p className={lowSpace ? "setup-warning" : "muted"}>
            {check.writable ? "Location looks writable." : "This location may not be writable."}
            {availableLabel ? ` Available: ${availableLabel}` : ""}
            {recommendedLabel ? ` · Recommended: ${recommendedLabel}` : ""}
          </p>
        ) : null}
        {error ? <p className="setup-warning">{error}</p> : null}
        <div className="setup-actions">
          <button type="button" className="ghost-button" onClick={() => setStep("welcome")}>
            Back
          </button>
          <button
            type="button"
            className="primary-button"
            disabled={!path.trim()}
            onClick={() => {
              setError(null);
              setStep("profile");
            }}
          >
            Continue
          </button>
        </div>
      </div>
    );
  }

  if (step === "profile") {
    return (
      <div className="setup-step">
        <h1>Create Your Local AI Profile</h1>
        <p className="setup-lede">
          No account is required. Your conversations are stored locally.
        </p>
        <label className="setup-field-label" htmlFor="setup-profile-name">
          Database / Profile Name
        </label>
        <input
          id="setup-profile-name"
          type="text"
          value={profileName}
          onChange={(event) => setProfileName(event.target.value)}
          spellCheck={false}
        />
        <p className="muted">Will be stored as {previewSanitizedName(profileName)}.db</p>
        {error ? <p className="setup-warning">{error}</p> : null}
        <div className="setup-actions">
          <button type="button" className="ghost-button" onClick={() => setStep("storage")} disabled={submitting}>
            Back
          </button>
          <button type="button" className="primary-button" disabled={submitting} onClick={() => void finishSetup()}>
            {submitting ? "Setting up…" : "Set Up OpenMindAI"}
          </button>
        </div>
      </div>
    );
  }

  return (
    <div className="setup-step">
      <h1>Restarting OpenMindAI…</h1>
      <p className="setup-lede">
        {isTauri
          ? "Applying your storage location. OpenMindAI will reopen in a moment."
          : "Storage configured. Restart the desktop app to continue."}
      </p>
    </div>
  );
}

function PreparingFlow(props: {
  hardware: HardwareProfile | null;
  models: ModelRecord[];
  runtime: RuntimeInventory | null;
  root: PortableRootInfo | null;
  refresh?: () => Promise<void>;
  onDismiss?: () => void;
}) {
  const modelReady = props.models.some(
    (model) =>
      (model.state === "ready" || model.state === "loaded") &&
      model.enabled &&
      model.capabilities.toLowerCase().includes("chat"),
  );
  const runtimeReady = props.runtime?.selected != null;

  const notifiedRef = useRef(false);
  const modelReadyPersistedRef = useRef(false);

  const [runtimeInstallStatus, setRuntimeInstallStatus] = useState<RuntimeInstallStatus | null>(null);
  const [runtimeInstallError, setRuntimeInstallError] = useState<string | null>(null);
  const runtimeInstallStartedRef = useRef(false);
  const runtimeReadyPersistedRef = useRef(false);

  useEffect(() => {
    if (runtimeReady || runtimeInstallStartedRef.current) return;
    runtimeInstallStartedRef.current = true;
    api
      .installRecommendedRuntime()
      .then((status) => {
        setRuntimeInstallStatus(status);
        return props.refresh?.();
      })
      .catch((caught) => setRuntimeInstallError(formatError(caught)));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [runtimeReady]);

  useEffect(() => {
    if (runtimeReady) return;
    const interval = window.setInterval(() => {
      void api.runtimeInstallStatus().then(setRuntimeInstallStatus);
    }, 1000);
    return () => window.clearInterval(interval);
  }, [runtimeReady]);

  const retryRuntimeInstall = useCallback(() => {
    runtimeInstallStartedRef.current = true;
    setRuntimeInstallError(null);
    api
      .installRecommendedRuntime()
      .then((status) => {
        setRuntimeInstallStatus(status);
        return props.refresh?.();
      })
      .catch((caught) => setRuntimeInstallError(formatError(caught)));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const cancelRuntimeInstall = useCallback(() => {
    void api
      .cancelRuntimeInstall()
      .then(setRuntimeInstallStatus)
      .catch((caught) => setRuntimeInstallError(formatError(caught)));
  }, []);

  // Runtime installation and model downloads already trigger one explicit
  // refresh when they complete. Avoid refreshing the entire application every
  // three seconds while setup is running: that refresh recursively scans model
  // and storage directories and creates unnecessary disk I/O on Windows.

  // Persists the setup-state bookkeeping in install.json once each stage's
  // own live readiness check (above) actually confirms it — best-effort,
  // fires once per stage, never blocks or surfaces an error in the UI.
  useEffect(() => {
    if (!runtimeReady || runtimeReadyPersistedRef.current) return;
    runtimeReadyPersistedRef.current = true;
    void api.markRuntimeReady().catch(() => undefined);
  }, [runtimeReady]);

  useEffect(() => {
    if (!modelReady || modelReadyPersistedRef.current) return;
    modelReadyPersistedRef.current = true;
    void api.markModelReady().catch(() => undefined);
  }, [modelReady]);

  useEffect(() => {
    if (!modelReady || !runtimeReady || notifiedRef.current) return;
    notifiedRef.current = true;
    void notifyUser(
      "OpenMindAI is ready",
      "Your local AI has been successfully installed and is ready for offline use.",
    );
  }, [modelReady, runtimeReady]);

  if (modelReady && runtimeReady) {
    return (
      <div className="setup-step">
        <h1>OpenMindAI is Ready 🎉</h1>
        <p className="setup-lede">Your private AI is installed and ready.</p>
        <ul className="setup-checklist">
          <li>✓ Local AI</li>
          <li>✓ Chat history</li>
          <li>✓ GPU acceleration (when supported)</li>
          <li>✓ Offline mode</li>
        </ul>
        <div className="setup-actions">
          <button type="button" className="primary-button" onClick={() => props.onDismiss?.()}>
            Launch OpenMindAI
          </button>
        </div>
      </div>
    );
  }

  return (
    <div className="setup-step">
      <h1>Preparing OpenMindAI</h1>
      <ul className="setup-checklist">
        <li>✓ Storage initialized</li>
        <li>✓ Database configured</li>
        <li>✓ Hardware detected{props.hardware ? ` (${props.hardware.cpu.name})` : ""}</li>
        <li className={runtimeReady ? "" : "setup-checklist-pending"}>
          {runtimeReady ? "✓ AI engine ready" : "○ Installing AI engine"}
        </li>
        <li className={modelReady ? "" : "setup-checklist-pending"}>
          {modelReady ? "✓ Chat model installed" : "○ Choose and download a chat model"}
        </li>
      </ul>

      {!runtimeReady ? (
        <div className="setup-progress">
          <div className="setup-progress-bar">
            <div
              className="setup-progress-fill"
              style={{ width: `${Math.min(100, Math.max(0, runtimeInstallStatus?.percentage ?? 0))}%` }}
            />
          </div>
          <p className="muted">
            {runtimeInstallStatus?.backend ? `${runtimeInstallStatus.backend} · ` : ""}
            {runtimeInstallStatus?.state ?? "resolving"}
          </p>
        </div>
      ) : null}

      {!modelReady ? (
        <div className="setup-model-catalog">
          <ModelsManager
            hardware={props.hardware}
            models={props.models}
            runtime={props.runtime}
            root={props.root}
            refresh={props.refresh ?? (() => undefined)}
            variant="setup"
            showRootInfo={false}
            showRuntimeInfo={false}
            allowDelete={false}
          />
        </div>
      ) : null}

      {runtimeInstallError || runtimeInstallStatus?.state === "failed" ? (
        <p className="setup-warning">
          Automatic AI engine install didn't complete
          {runtimeInstallStatus?.error ? `: ${runtimeInstallStatus.error}` : runtimeInstallError ? `: ${runtimeInstallError}` : ""}.
          You can also run <code>scripts\install-llama-runtime.ps1</code> from the OpenMindAI
          installation folder, then retry.
        </p>
      ) : null}

      <div className="setup-actions">
        {!runtimeReady && runtimeInstallStatus?.state !== "cancelled" ? (
          <button type="button" className="ghost-button" onClick={cancelRuntimeInstall}>
            Cancel engine install
          </button>
        ) : null}
        <button
          type="button"
          className="ghost-button"
          onClick={() => {
            void props.refresh?.();
            if (!runtimeReady) retryRuntimeInstall();
          }}
        >
          Retry
        </button>
      </div>
    </div>
  );
}
