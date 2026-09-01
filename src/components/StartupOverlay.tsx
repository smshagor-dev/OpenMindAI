import { invoke } from "@tauri-apps/api/core";
import { Check, Cpu, Database, HardDrive, LoaderCircle, MessageSquare, Sparkles, TriangleAlert } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { api } from "../api";
import type { LlamaRuntimeStatus } from "../types";

type StepState = "pending" | "loading" | "done" | "warning";

type StartupStep = {
  id: string;
  label: string;
  detail: string;
  state: StepState;
};

const INITIAL_STEPS: StartupStep[] = [
  {
    id: "workspace",
    label: "Loading your workspace",
    detail: "Opening your private local data and preferences",
    state: "pending",
  },
  {
    id: "conversations",
    label: "Loading your conversations",
    detail: "Restoring recent chats and local history",
    state: "pending",
  },
  {
    id: "hardware",
    label: "Detecting your hardware",
    detail: "Preparing the best CPU and GPU path",
    state: "pending",
  },
  {
    id: "runtime",
    label: "Preparing your AI engine",
    detail: "Checking the local llama.cpp runtime",
    state: "pending",
  },
  {
    id: "model",
    label: "Loading your local AI model",
    detail: "Preparing OpenMindAI Core for fast first response",
    state: "pending",
  },
];

const ICONS = [HardDrive, MessageSquare, Cpu, Database, Sparkles];

export function StartupOverlay() {
  const [steps, setSteps] = useState<StartupStep[]>(INITIAL_STEPS);
  const [bootstrapComplete, setBootstrapComplete] = useState(false);
  const [appReady, setAppReady] = useState(false);
  const [visible, setVisible] = useState(true);

  const completed = useMemo(
    () => steps.filter((step) => step.state === "done" || step.state === "warning").length,
    [steps],
  );
  const progress = Math.round((completed / steps.length) * 100);
  const activeStep = steps.find((step) => step.state === "loading") ?? steps.find((step) => step.state === "pending");

  useEffect(() => {
    const detectAppReady = () => {
      const ready = Boolean(document.querySelector(".app-shell, .setup-wizard"));
      if (ready) setAppReady(true);
      return ready;
    };
    if (detectAppReady()) return;

    const observer = new MutationObserver(() => {
      if (detectAppReady()) observer.disconnect();
    });
    observer.observe(document.body, { childList: true, subtree: true });
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    let cancelled = false;

    const updateStep = (id: string, state: StepState, detail?: string) => {
      if (cancelled) return;
      setSteps((current) =>
        current.map((step) =>
          step.id === id ? { ...step, state, detail: detail ?? step.detail } : step,
        ),
      );
    };

    const run = async () => {
      updateStep("workspace", "loading");
      const workspace = await Promise.allSettled([
        api.root(),
        api.preferences(),
        api.userProfile(),
        api.installationStatus(),
      ]);
      const installation = workspace[3].status === "fulfilled" ? workspace[3].value : null;
      updateStep(
        "workspace",
        workspace.some((result) => result.status === "rejected") ? "warning" : "done",
        workspace.some((result) => result.status === "rejected")
          ? "Workspace opened; one optional setting will retry after startup"
          : "Your private workspace is ready",
      );

      updateStep("conversations", "loading");
      try {
        await api.conversations();
        updateStep("conversations", "done", "Conversation history is ready");
      } catch {
        updateStep("conversations", "warning", "Chat history will retry in the workspace");
      }

      updateStep("hardware", "loading");
      const hardware = await Promise.allSettled([api.hardware(), api.performance()]);
      updateStep(
        "hardware",
        hardware.some((result) => result.status === "rejected") ? "warning" : "done",
        hardware.some((result) => result.status === "rejected")
          ? "Hardware profile will finish in the background"
          : "CPU and GPU profile is ready",
      );

      updateStep("runtime", "loading");
      try {
        const runtime = await api.runtimeStatus();
        updateStep(
          "runtime",
          runtime.available ? "done" : "warning",
          runtime.available ? "Local AI engine is ready" : "AI engine setup is required",
        );
      } catch {
        updateStep("runtime", "warning", "AI engine will retry when needed");
      }

      if (installation?.setupRequired) {
        updateStep("model", "warning", "Complete setup to prepare OpenMindAI Core");
        if (!cancelled) setBootstrapComplete(true);
        return;
      }

      updateStep("model", "loading");
      try {
        const status = await invoke<LlamaRuntimeStatus>("prepare_default_chat_runtime");
        updateStep(
          "model",
          "done",
          status.backend
            ? `OpenMindAI Core is ready on ${status.backend.toUpperCase()}`
            : "OpenMindAI Core is ready",
        );
      } catch {
        // Do not trap the user on the loader if a model/runtime is missing.
        // The normal chat path still reports the actionable error and can retry.
        updateStep("model", "warning", "Core will finish preparing on your first message");
      }

      if (!cancelled) setBootstrapComplete(true);
    };

    void run();
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    if (!bootstrapComplete || !appReady) return;
    const timer = window.setTimeout(() => setVisible(false), 220);
    return () => window.clearTimeout(timer);
  }, [appReady, bootstrapComplete]);

  if (!visible) return null;

  return (
    <div className={`startup-overlay${bootstrapComplete && appReady ? " startup-overlay-ready" : ""}`}>
      <div className="startup-panel" role="status" aria-live="polite">
        <div className="startup-brand">
          <img src="/icon.png" alt="" />
          <div>
            <strong>OpenMindAI</strong>
            <span>Your private AI workspace</span>
          </div>
        </div>

        <div className="startup-headline">
          <h1>{bootstrapComplete ? "OpenMindAI is ready" : activeStep?.label ?? "Starting OpenMindAI"}</h1>
          <p>
            {bootstrapComplete
              ? "Your local workspace and default AI are prepared."
              : activeStep?.detail ?? "Preparing your local workspace..."}
          </p>
        </div>

        <div className="startup-progress" aria-label={`Startup ${progress}% complete`}>
          <div className="startup-progress-track">
            <span style={{ width: `${progress}%` }} />
          </div>
          <small>{progress}%</small>
        </div>

        <div className="startup-steps">
          {steps.map((step, index) => {
            const Icon = ICONS[index];
            return (
              <div className={`startup-step startup-step-${step.state}`} key={step.id}>
                <span className="startup-step-icon">
                  <Icon size={16} />
                </span>
                <span className="startup-step-copy">
                  <strong>{step.label}</strong>
                  <small>{step.detail}</small>
                </span>
                <span className="startup-step-state" aria-hidden="true">
                  {step.state === "loading" ? (
                    <LoaderCircle className="startup-spin" size={17} />
                  ) : step.state === "done" ? (
                    <Check size={17} />
                  ) : step.state === "warning" ? (
                    <TriangleAlert size={16} />
                  ) : (
                    <span className="startup-step-dot" />
                  )}
                </span>
              </div>
            );
          })}
        </div>

        <p className="startup-local-note">Everything is prepared locally on this computer.</p>
      </div>
    </div>
  );
}
