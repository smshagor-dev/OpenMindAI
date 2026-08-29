import { CheckCircle2, Github, Loader2, ShieldAlert, Sparkles, Unplug, X } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { api, type ConnectedAgentPlan } from "../api";
import { CONNECTED_ACTIONS, type GoogleWorkspaceStatus } from "../lib/connectedActions";
import { formatError } from "../lib/format";
import type { GithubAccount } from "../types";
import { ConnectedWorkspace } from "./ConnectedWorkspace";

const MAX_AGENT_STEPS = 8;
const MAX_TRANSCRIPT_CHARS = 11_500;
const MAX_RESULT_FOR_MODEL_CHARS = 9_000;

interface AgentLogEntry {
  id: string;
  kind: "plan" | "tool" | "final" | "error";
  title: string;
  detail: string;
}

interface PendingApproval {
  plan: Extract<ConnectedAgentPlan, { type: "action" }>;
  transcript: string;
}

function plannerCatalog() {
  return JSON.stringify(
    CONNECTED_ACTIONS.map((item) => ({
      provider: item.provider,
      action: item.action,
      write: item.mutating,
      description: item.description,
      paramsExample: item.example,
    })),
  );
}

function appendTranscript(current: string, addition: string) {
  const combined = current ? `${current}\n\n${addition}` : addition;
  if (combined.length <= MAX_TRANSCRIPT_CHARS) return combined;
  return `[Earlier tool transcript truncated]\n${combined.slice(combined.length - MAX_TRANSCRIPT_CHARS)}`;
}

function decodeBase64Text(value: string, urlSafe = false) {
  try {
    const normalized = urlSafe ? value.replace(/-/g, "+").replace(/_/g, "/") : value;
    const padded = normalized.padEnd(Math.ceil(normalized.length / 4) * 4, "=");
    const binary = window.atob(padded);
    const bytes = Uint8Array.from(binary, (character) => character.charCodeAt(0));
    return new window.TextDecoder("utf-8", { fatal: false }).decode(bytes);
  } catch {
    return null;
  }
}

function gmailBody(payload: unknown): string {
  if (!payload || typeof payload !== "object") return "";
  const item = payload as Record<string, unknown>;
  const mimeType = typeof item.mimeType === "string" ? item.mimeType : "";
  const body = item.body && typeof item.body === "object" ? (item.body as Record<string, unknown>) : null;
  const data = body && typeof body.data === "string" ? body.data : null;
  if (data && (mimeType === "text/plain" || mimeType === "text/html" || !mimeType)) {
    const decoded = decodeBase64Text(data, true);
    if (decoded) return decoded;
  }
  const parts = Array.isArray(item.parts) ? item.parts : [];
  const plain = parts.find(
    (part) =>
      part &&
      typeof part === "object" &&
      (part as Record<string, unknown>).mimeType === "text/plain",
  );
  if (plain) {
    const decoded = gmailBody(plain);
    if (decoded) return decoded;
  }
  for (const part of parts) {
    const decoded = gmailBody(part);
    if (decoded) return decoded;
  }
  return "";
}

function simplifyResult(provider: "google" | "github", action: string, output: unknown) {
  if (!output || typeof output !== "object") return output;
  const value = output as Record<string, unknown>;

  if (provider === "github" && action === "file.get") {
    const content = typeof value.content === "string" ? value.content.replace(/\s/g, "") : "";
    const encoding = value.encoding === "base64";
    if (content && encoding) {
      const decoded = decodeBase64Text(content);
      if (decoded !== null) {
        return {
          name: value.name,
          path: value.path,
          sha: value.sha,
          size: value.size,
          htmlUrl: value.html_url,
          content: decoded,
        };
      }
    }
  }

  if (provider === "google" && action === "gmail.get") {
    const payload = value.payload && typeof value.payload === "object"
      ? (value.payload as Record<string, unknown>)
      : null;
    const headers = Array.isArray(payload?.headers) ? payload.headers : [];
    const selectedHeaders: Record<string, string> = {};
    for (const header of headers) {
      if (!header || typeof header !== "object") continue;
      const item = header as Record<string, unknown>;
      const name = typeof item.name === "string" ? item.name : "";
      const headerValue = typeof item.value === "string" ? item.value : "";
      if (["from", "to", "cc", "subject", "date", "message-id", "reply-to"].includes(name.toLowerCase())) {
        selectedHeaders[name] = headerValue;
      }
    }
    return {
      id: value.id,
      threadId: value.threadId,
      labelIds: value.labelIds,
      snippet: value.snippet,
      headers: selectedHeaders,
      bodyText: gmailBody(payload),
    };
  }

  if (provider === "google" && (action === "drive.download" || action === "drive.export")) {
    return {
      fileId: value.fileId,
      sizeBytes: value.sizeBytes,
      note: "Binary content was retrieved successfully but is not injected into the language planner.",
    };
  }

  return output;
}

function resultForModel(provider: "google" | "github", action: string, output: unknown) {
  const serialized = JSON.stringify(simplifyResult(provider, action, output), null, 2);
  if (serialized.length <= MAX_RESULT_FOR_MODEL_CHARS) return serialized;
  return `${serialized.slice(0, MAX_RESULT_FOR_MODEL_CHARS)}\n[Tool result truncated for local planner context]`;
}

export function ConnectedAgentPanel({ onClose }: { onClose: () => void }) {
  const [advancedOpen, setAdvancedOpen] = useState(false);
  const [goal, setGoal] = useState("");
  const [running, setRunning] = useState(false);
  const [logs, setLogs] = useState<AgentLogEntry[]>([]);
  const [pending, setPending] = useState<PendingApproval | null>(null);
  const [google, setGoogle] = useState<GoogleWorkspaceStatus | null>(null);
  const [github, setGithub] = useState<GithubAccount | null>(null);
  const [connectingGoogle, setConnectingGoogle] = useState(false);
  const [connectionLoading, setConnectionLoading] = useState(true);
  const catalog = useMemo(plannerCatalog, []);

  const addLog = (kind: AgentLogEntry["kind"], title: string, detail: string) => {
    setLogs((current) => [
      ...current,
      { id: crypto.randomUUID(), kind, title, detail },
    ]);
  };

  const refreshConnections = async () => {
    setConnectionLoading(true);
    const [googleResult, githubResult] = await Promise.allSettled([
      api.googleWorkspaceStatus(),
      api.githubAccount(),
    ]);
    setGoogle(googleResult.status === "fulfilled" ? googleResult.value : null);
    setGithub(githubResult.status === "fulfilled" ? githubResult.value : null);
    setConnectionLoading(false);
  };

  useEffect(() => {
    void refreshConnections();
  }, []);

  const connectGoogle = async () => {
    setConnectingGoogle(true);
    try {
      setGoogle(await api.connectGoogleWorkspace());
    } catch (caught) {
      addLog("error", "Google connection failed", formatError(caught));
    } finally {
      setConnectingGoogle(false);
    }
  };

  const executeAction = async (
    plan: Extract<ConnectedAgentPlan, { type: "action" }>,
    approved: boolean,
  ) => {
    if (plan.provider === "google") {
      if (!google?.connected) throw new Error("Google Workspace is not connected.");
      return api.executeGoogleWorkspaceAction(plan.action, plan.params, approved);
    }
    if (!github) throw new Error("GitHub is not connected.");
    return api.executeGithubWorkspaceAction(plan.action, plan.params, approved);
  };

  const continueAgent = async (startTranscript: string) => {
    const trimmedGoal = goal.trim();
    if (!trimmedGoal) return;
    setRunning(true);
    let transcript = startTranscript;
    try {
      for (let step = 0; step < MAX_AGENT_STEPS; step += 1) {
        const plan = await api.planConnectedAction(trimmedGoal, transcript, catalog);
        if (plan.type === "final") {
          addLog("final", "Done", plan.message);
          setPending(null);
          return;
        }

        const definition = CONNECTED_ACTIONS.find(
          (item) => item.provider === plan.provider && item.action === plan.action,
        );
        if (!definition) {
          throw new Error(`The local planner requested an unsupported action: ${plan.provider}.${plan.action}`);
        }

        addLog(
          "plan",
          `${definition.mutating ? "Proposed change" : "Reading"}: ${definition.label}`,
          `${plan.reason ?? definition.description}\n\n${JSON.stringify(plan.params, null, 2)}`,
        );

        if (definition.mutating) {
          setPending({ plan, transcript });
          return;
        }

        const output = await executeAction(plan, false);
        const compact = resultForModel(plan.provider, plan.action, output);
        addLog("tool", definition.label, compact);
        transcript = appendTranscript(
          transcript,
          `TOOL ${plan.provider}.${plan.action}\nPARAMS ${JSON.stringify(plan.params)}\nRESULT ${compact}`,
        );
      }
      addLog(
        "error",
        "Step limit reached",
        `OpenMindAI stopped after ${MAX_AGENT_STEPS} connected steps. Refine the request or continue with a more specific goal.`,
      );
    } catch (caught) {
      addLog("error", "Connected Work stopped", formatError(caught));
    } finally {
      setRunning(false);
    }
  };

  const startAgent = async () => {
    if (!goal.trim() || running) return;
    setLogs([]);
    setPending(null);
    await continueAgent("");
  };

  const approvePending = async () => {
    if (!pending || running) return;
    const current = pending;
    const definition = CONNECTED_ACTIONS.find(
      (item) => item.provider === current.plan.provider && item.action === current.plan.action,
    );
    setPending(null);
    setRunning(true);
    try {
      const output = await executeAction(current.plan, true);
      const compact = resultForModel(current.plan.provider, current.plan.action, output);
      addLog("tool", `${definition?.label ?? current.plan.action} · completed`, compact);
      const transcript = appendTranscript(
        current.transcript,
        `APPROVED TOOL ${current.plan.provider}.${current.plan.action}\nPARAMS ${JSON.stringify(current.plan.params)}\nRESULT ${compact}`,
      );
      setRunning(false);
      await continueAgent(transcript);
    } catch (caught) {
      addLog("error", "Approved action failed", formatError(caught));
      setRunning(false);
    }
  };

  const rejectPending = () => {
    if (!pending) return;
    addLog("final", "Change not applied", "You rejected the proposed remote change. No write action was sent.");
    setPending(null);
  };

  if (advancedOpen) {
    return <ConnectedWorkspace onClose={() => setAdvancedOpen(false)} />;
  }

  return (
    <div className="modal-overlay" role="presentation" onClick={onClose}>
      <div
        className="modal-card"
        role="dialog"
        aria-modal="true"
        aria-label="Connected Work agent"
        onClick={(event) => event.stopPropagation()}
        style={{ width: "min(980px, 94vw)", maxWidth: 980, height: "min(820px, 90vh)", overflow: "auto" }}
      >
        <div style={{ display: "flex", justifyContent: "space-between", gap: 16, alignItems: "center" }}>
          <div>
            <h2 style={{ marginBottom: 4, display: "flex", alignItems: "center", gap: 8 }}>
              <Sparkles size={19} /> Connected Work
            </h2>
            <p className="muted" style={{ margin: 0 }}>
              Ask in normal language. Your installed local model plans Google/GitHub steps; remote writes always stop for approval.
            </p>
          </div>
          <button type="button" className="ghost-button" onClick={onClose} title="Close Connected Work">
            <X size={18} />
          </button>
        </div>

        <div className="connector-card" style={{ marginTop: 16 }}>
          {connectionLoading ? (
            <p className="muted"><Loader2 size={14} className="spin" /> Checking connections...</p>
          ) : (
            <div style={{ display: "flex", flexWrap: "wrap", gap: 10, alignItems: "center" }}>
              <span className="tool-menu-badge">
                {google?.connected ? <CheckCircle2 size={13} /> : <ShieldAlert size={13} />}
                Google {google?.connected ? google.email ?? "connected" : google?.configured ? "ready to connect" : "not configured"}
              </span>
              {!google?.connected && google?.configured ? (
                <button
                  type="button"
                  className="ghost-button"
                  onClick={() => void connectGoogle()}
                  disabled={connectingGoogle}
                >
                  {connectingGoogle ? <Loader2 size={14} className="spin" /> : "Connect Google"}
                </button>
              ) : google?.connected ? (
                <button
                  type="button"
                  className="ghost-button"
                  onClick={() => void api.disconnectGoogleWorkspace().then(refreshConnections)}
                >
                  <Unplug size={14} /> Disconnect Google
                </button>
              ) : null}
              <span className="tool-menu-badge">
                <Github size={13} /> GitHub {github ? `@${github.login}` : "not connected"}
              </span>
              <button type="button" className="ghost-button" onClick={() => setAdvancedOpen(true)}>
                Advanced actions
              </button>
            </div>
          )}
        </div>

        <label className="connector-field" style={{ marginTop: 16 }}>
          <span className="muted">What should OpenMindAI do?</span>
          <textarea
            value={goal}
            onChange={(event) => setGoal(event.target.value)}
            placeholder="Examples: Find unread emails from the last 3 days and summarize them. / Check owner/repo failing Actions, inspect the related files and prepare the fix."
            style={{ minHeight: 100, resize: "vertical" }}
            disabled={running || Boolean(pending)}
          />
        </label>
        <div style={{ display: "flex", gap: 8, alignItems: "center", marginTop: 10 }}>
          <button
            type="button"
            className="primary-button"
            onClick={() => void startAgent()}
            disabled={!goal.trim() || running || Boolean(pending)}
          >
            {running ? <Loader2 size={14} className="spin" /> : <Sparkles size={14} />}
            {running ? "Working..." : "Start Work"}
          </button>
          <span className="muted">Read steps can run automatically · writes require approval</span>
        </div>

        {pending ? (
          <div className="connector-card" style={{ marginTop: 16, borderColor: "rgba(245, 158, 11, .45)" }}>
            <div style={{ display: "flex", gap: 9, alignItems: "center" }}>
              <ShieldAlert size={18} />
              <strong>Approval required</strong>
            </div>
            <p className="muted">
              OpenMindAI has stopped before changing remote data. Review the exact operation below.
            </p>
            <pre style={{ whiteSpace: "pre-wrap", wordBreak: "break-word", maxHeight: 250, overflow: "auto" }}>
              {JSON.stringify(
                {
                  provider: pending.plan.provider,
                  action: pending.plan.action,
                  params: pending.plan.params,
                  reason: pending.plan.reason,
                },
                null,
                2,
              )}
            </pre>
            <div style={{ display: "flex", gap: 8, marginTop: 10 }}>
              <button type="button" className="primary-button" onClick={() => void approvePending()}>
                Approve and continue
              </button>
              <button type="button" className="ghost-button" onClick={rejectPending}>
                Reject
              </button>
            </div>
          </div>
        ) : null}

        {logs.length > 0 ? (
          <div style={{ display: "grid", gap: 10, marginTop: 18 }}>
            {logs.map((entry) => (
              <div className="connector-card" key={entry.id}>
                <strong>{entry.title}</strong>
                <pre
                  className={entry.kind === "error" ? "connector-error" : "muted"}
                  style={{ marginBottom: 0, whiteSpace: "pre-wrap", wordBreak: "break-word", maxHeight: 260, overflow: "auto" }}
                >
                  {entry.detail}
                </pre>
              </div>
            ))}
          </div>
        ) : null}
      </div>
    </div>
  );
}
