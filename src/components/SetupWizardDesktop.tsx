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
import { cloud } from "../cloudApi";
import { beginSetupPreparation, completeSetupPreparation, setupExecutionMode } from "../lib/setupExecutionMode";
import type { ExecutionMode } from "../lib/executionRouting";

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

type FullStep = "welcome" | "execution" | "storage" | "profile" | "restarting";

function FullSetupFlow(props: { pending: PendingSetup | null }) {
  // A pending choice means the user already got past "welcome" and picked a
  // storage folder in an earlier, interrupted session — resume right after
  // that instead of restarting the wizard from scratch.
  const [step, setStep] = useState<FullStep>(props.pending ? (setupExecutionMode() ? "profile" : "execution") : "welcome");
  const [selectedMode, setSelectedMode] = useState<ExecutionMode | null>(setupExecutionMode());
  const [path, setPath] = useState(props.pending?.root ?? DEFAULT_STORAGE_PATH);
  const [check, setCheck] = useState<StorageLocationCheck | null>(null);
  const [checkingStorage, setCheckingStorage] = useState(false);
  const [storageCheckError, setStorageCheckError] = useState<{ path: string; message: string } | null>(null);
  const storageCheckVersion = useRef(0);
  const [profileName, setProfileName] = useState(props.pending?.profileName ?? "openmindai");
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  // A stale check must never authorize a different path, even during the
  // debounce interval or after switching from A to B and back to A.
  const changeStoragePath = useCallback((next: string) => {
    storageCheckVersion.current += 1;
    setPath(next);
    setCheck(null);
    setCheckingStorage(false);
    setStorageCheckError(null);
  }, []);

  useEffect(() => {
    if (step !== "storage") return;
    let cancelled = false;
    const version = ++storageCheckVersion.current;
    setCheck(null);
    setStorageCheckError(null);
    if (!path.trim()) {
      setCheckingStorage(false);
      return () => {
        cancelled = true;
        storageCheckVersion.current += 1;
      };
    }
    setCheckingStorage(true);
    const timer = window.setTimeout(() => {
      void api.checkStorageLocation(path)
        .then((result) => {
          if (cancelled || storageCheckVersion.current !== version) return;
          if (result.path !== path) {
            setCheck(null);
            setStorageCheckError({ path, message: "Storage check returned a different path. Select this location again." });
            return;
          }
          setCheck(result);
        })
        .catch((caught: unknown) => {
          if (cancelled || storageCheckVersion.current !== version) return;
          setCheck(null);
          setStorageCheckError({ path, message: `Could not check storage: ${formatError(caught)}` });
        })
        .finally(() => {
          if (!cancelled && storageCheckVersion.current === version) setCheckingStorage(false);
        });
    }, 350);
    return () => {
      cancelled = true;
      storageCheckVersion.current += 1;
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
      if (typeof selected === "string") changeStoragePath(selected);
    } catch (caught) {
      setError(formatError(caught));
    }
  }, [changeStoragePath]);

  const finishSetup = useCallback(async () => {
    setSubmitting(true);
    setError(null);
    try {
      if (!selectedMode) throw new Error("Choose Cloud, Local or Both before continuing.");
      // The marker survives the native restart and makes the selected
      // preparation path appear before any runtime/model download begins.
      beginSetupPreparation(selectedMode);
      await api.completeSetup(path, profileName);
      setStep("restarting");
      if (isTauri) {
        window.setTimeout(() => void relaunch(), 900);
      }
    } catch (caught) {
      completeSetupPreparation();
      setError(formatError(caught));
      setSubmitting(false);
    }
  }, [path, profileName, selectedMode]);

  if (step === "welcome") {
    return (
      <div className="setup-step">
        <h1>Welcome to OpenMindAI</h1>
        <p className="setup-lede">Choose Cloud AI, an offline local model, or both during setup.</p>
        <ul className="setup-bullets">
          <li>Cloud AI works after you connect your account.</li>
          <li>Local AI requires a downloaded runtime and model.</li>
          <li>Both switches automatically when Cloud is unavailable.</li>
        </ul>
        <div className="setup-actions">
          <button type="button" className="primary-button" onClick={() => setStep("execution")}>
            Get Started
          </button>
        </div>
      </div>
    );
  }

  if (step === "execution") {
    return (
      <div className="setup-step">
        <h1>How do you want to use OpenMindAI?</h1>
        <p className="setup-lede">Choose before downloading any AI models. This choice also applies to the Coding Agent.</p>
        <div className="setup-execution-options" role="radiogroup" aria-label="AI execution setup">
          {([
            ["cloud", "Cloud", "Connect your OpenMindAI account. No local AI model download is required. Internet is needed."],
            ["local", "Local", "Download local models, keep inference on this computer, and work offline."],
            ["auto", "Both", "Use your Cloud account when available, with installed Local models as fallback."],
          ] as const).map(([value, label, description]) => (
            <label key={value} className={selectedMode === value ? "setup-execution-option selected" : "setup-execution-option"}>
              <input type="radio" name="setup-execution" checked={selectedMode === value}
                onChange={() => {
                  setSelectedMode(value);
                  window.localStorage.setItem("openmindai.setup.executionMode.v1", value);
                }}/>
              <span><strong>{label}</strong><small>{description}</small></span>
            </label>
          ))}
        </div>
        <p className="muted">The same initial mode is applied to Chat and Coding Agent. You can change them later in Settings.</p>
        <div className="setup-actions">
          <button type="button" className="ghost-button" onClick={() => setStep("welcome")}>Back</button>
          <button type="button" className="primary-button" disabled={!selectedMode} onClick={() => setStep("storage")}>Continue</button>
        </div>
      </div>
    );
  }

  if (step === "storage") {
    // Ignore any previously rendered result before React's effect cleanup
    // runs: a writable check is only valid for its exact selected path.
    const currentCheck = check?.path === path ? check : null;
    const currentError = storageCheckError?.path === path ? storageCheckError.message : null;
    const availableLabel = currentCheck?.availableBytes != null ? formatBytes(currentCheck.availableBytes) : null;
    const recommendedLabel = currentCheck ? formatBytes(currentCheck.recommendedBytes) : null;
    const lowSpace = currentCheck?.availableBytes != null && currentCheck.availableBytes < currentCheck.recommendedBytes;
    const checking = checkingStorage || (!!path.trim() && !currentCheck && !currentError);
    const enoughSpace = selectedMode === "cloud" || !lowSpace;
    const canContinue = !!selectedMode && !!path.trim() && !!currentCheck?.writable && enoughSpace && !checking;
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
            onChange={(event) => changeStoragePath(event.target.value)}
            spellCheck={false}
          />
          {isTauri ? (
            <button type="button" className="ghost-button" onClick={() => void browse()}>
              Browse…
            </button>
          ) : null}
        </div>
        {checking ? <p className="muted" role="status">Checking selected storage location…</p> : null}
        {currentCheck ? (
          <p className={lowSpace || !currentCheck.writable ? "setup-warning" : "muted"}>
            {currentCheck.writable ? "Location looks writable." : "This location is not writable. Choose another folder."}
            {availableLabel ? ` Available: ${availableLabel}` : " Free space could not be measured; downloads will verify available space again."}
            {recommendedLabel ? ` · Recommended for Local AI: ${recommendedLabel}` : ""}
            {lowSpace && selectedMode !== "cloud" ? " More free space is needed for Local AI setup." : ""}
          </p>
        ) : null}
        {currentError ? <p className="setup-warning" role="alert">{currentError}</p> : null}
        {error ? <p className="setup-warning" role="alert">{error}</p> : null}
        <div className="setup-actions">
          <button type="button" className="ghost-button" onClick={() => setStep("execution")}>
            Back
          </button>
          <button
            type="button"
            className="primary-button"
            disabled={!canContinue}
            onClick={() => {
              if (!canContinue) return;
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
  const initialMode = setupExecutionMode() ?? "local";
  const requireLocal = initialMode !== "cloud";
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
    if (!requireLocal || runtimeReady || runtimeInstallStartedRef.current) return;
    runtimeInstallStartedRef.current = true;
    api
      .installRecommendedRuntime()
      .then((status) => {
        setRuntimeInstallStatus(status);
        return props.refresh?.();
      })
      .catch((caught) => setRuntimeInstallError(formatError(caught)));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [requireLocal, runtimeReady]);

  useEffect(() => {
    if (!requireLocal || runtimeReady) return;
    const interval = window.setInterval(() => {
      void api.runtimeInstallStatus().then(setRuntimeInstallStatus);
    }, 1000);
    return () => window.clearInterval(interval);
  }, [requireLocal, runtimeReady]);

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

  if (!requireLocal) {
    return (
      <div className="setup-step">
        <h1>Connect OpenMindAI Cloud</h1>
        <p className="setup-lede">Cloud mode skips local runtime and model downloads. Connect your account to use Cloud Chat and Coding Agent.</p>
        <CloudAccountSetup />
        <div className="setup-actions">
          <button type="button" className="primary-button" onClick={() => props.onDismiss?.()}>Launch OpenMindAI</button>
        </div>
        <p className="muted">You can sign in after launching, but Cloud features need a verified connection.</p>
      </div>
    );
  }

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
        {initialMode === "auto" ? <CloudAccountSetup /> : null}
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
      <h1>Preparing OpenMindAI Locally</h1>
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

      {initialMode === "auto" ? <CloudAccountSetup /> : null}
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

function CloudAccountSetup() {
  const [connected, setConnected] = useState(false);
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [code, setCode] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    void cloud.status().then((result) => {
      if (active) setConnected(result.connected);
    }).catch(() => undefined);
    return () => { active = false; };
  }, []);

  const signIn = async () => {
    setBusy(true);
    setError(null);
    try {
      const session = await cloud.login(email, password, code || undefined);
      setConnected(session.connected);
      setPassword("");
      setCode("");
    } catch (caught) {
      setError(formatError(caught));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="setup-cloud-account">
      <strong>{connected ? "✓ Cloud account connected" : "Connect Cloud account (optional until launch)"}</strong>
      {!connected ? (
        <form onSubmit={(event) => { event.preventDefault(); void signIn(); }}>
          <label className="setup-field-label">Email
            <input type="email" autoComplete="username" value={email} onChange={(event) => setEmail(event.target.value)} required />
          </label>
          <label className="setup-field-label">Password
            <input type="password" autoComplete="current-password" value={password} onChange={(event) => setPassword(event.target.value)} required />
          </label>
          <label className="setup-field-label">2FA code (if required)
            <input inputMode="numeric" autoComplete="one-time-code" value={code} onChange={(event) => setCode(event.target.value)} />
          </label>
          {error ? <p className="setup-warning" role="alert">{error}</p> : null}
          <button className="primary-button" type="submit" disabled={busy}>{busy ? "Connecting…" : "Sign in to Cloud"}</button>
        </form>
      ) : <p className="muted">Your account session is kept in secure native storage; your password is not saved.</p>}
    </div>
  );
}
