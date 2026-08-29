import { CheckCircle2, Github, Loader2, ShieldAlert, Unplug, X } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { api } from "../api";
import type { GithubAccount } from "../types";
import {
  CONNECTED_ACTIONS,
  type ConnectedProvider,
  type GoogleWorkspaceStatus,
} from "../lib/connectedActions";
import { formatError } from "../lib/format";

export function ConnectedWorkspace({ onClose }: { onClose: () => void }) {
  const [provider, setProvider] = useState<ConnectedProvider>("google");
  const [google, setGoogle] = useState<GoogleWorkspaceStatus | null>(null);
  const [github, setGithub] = useState<GithubAccount | null>(null);
  const [loadingConnections, setLoadingConnections] = useState(true);
  const [connectingGoogle, setConnectingGoogle] = useState(false);
  const [filter, setFilter] = useState("");
  const providerActions = useMemo(
    () =>
      CONNECTED_ACTIONS.filter(
        (item) =>
          item.provider === provider &&
          (!filter.trim() ||
            `${item.label} ${item.description} ${item.action}`
              .toLowerCase()
              .includes(filter.trim().toLowerCase())),
      ),
    [filter, provider],
  );
  const [action, setAction] = useState("gmail.search");
  const definition =
    CONNECTED_ACTIONS.find((item) => item.provider === provider && item.action === action) ??
    providerActions[0] ??
    null;
  const [paramsText, setParamsText] = useState(
    JSON.stringify(CONNECTED_ACTIONS[0]?.example ?? {}, null, 2),
  );
  const [approved, setApproved] = useState(false);
  const [running, setRunning] = useState(false);
  const [result, setResult] = useState<string>("");
  const [error, setError] = useState<string | null>(null);

  const refreshConnections = async () => {
    setLoadingConnections(true);
    const [googleResult, githubResult] = await Promise.allSettled([
      api.googleWorkspaceStatus(),
      api.githubAccount(),
    ]);
    setGoogle(googleResult.status === "fulfilled" ? googleResult.value : null);
    setGithub(githubResult.status === "fulfilled" ? githubResult.value : null);
    setLoadingConnections(false);
  };

  useEffect(() => {
    void refreshConnections();
  }, []);

  useEffect(() => {
    const first = CONNECTED_ACTIONS.find((item) => item.provider === provider);
    if (!first) return;
    setAction(first.action);
    setParamsText(JSON.stringify(first.example, null, 2));
    setApproved(false);
    setResult("");
    setError(null);
  }, [provider]);

  useEffect(() => {
    if (!definition) return;
    setParamsText(JSON.stringify(definition.example, null, 2));
    setApproved(false);
    setResult("");
    setError(null);
  }, [definition]);

  const connectGoogle = async () => {
    setConnectingGoogle(true);
    setError(null);
    try {
      setGoogle(await api.connectGoogleWorkspace());
    } catch (caught) {
      setError(formatError(caught));
    } finally {
      setConnectingGoogle(false);
    }
  };

  const disconnectGoogle = async () => {
    setError(null);
    try {
      await api.disconnectGoogleWorkspace();
      await refreshConnections();
    } catch (caught) {
      setError(formatError(caught));
    }
  };

  const runAction = async () => {
    if (!definition) return;
    setError(null);
    setResult("");
    let params: Record<string, unknown>;
    try {
      const parsed: unknown = JSON.parse(paramsText || "{}");
      if (!parsed || Array.isArray(parsed) || typeof parsed !== "object") {
        throw new Error("Parameters must be a JSON object.");
      }
      params = parsed as Record<string, unknown>;
    } catch (caught) {
      setError(formatError(caught));
      return;
    }
    if (definition.mutating && !approved) {
      setError("Approve this remote change before running it.");
      return;
    }
    setRunning(true);
    try {
      const output =
        provider === "google"
          ? await api.executeGoogleWorkspaceAction(definition.action, params, approved)
          : await api.executeGithubWorkspaceAction(definition.action, params, approved);
      const serialized = JSON.stringify(output, null, 2);
      setResult(
        serialized.length > 150_000
          ? `${serialized.slice(0, 150_000)}\n\n[Result display truncated at 150,000 characters]`
          : serialized,
      );
    } catch (caught) {
      setError(formatError(caught));
    } finally {
      setRunning(false);
    }
  };

  const connectionReady = provider === "google" ? Boolean(google?.connected) : Boolean(github);

  return (
    <div className="modal-overlay" role="presentation" onClick={onClose}>
      <div
        className="modal-card"
        role="dialog"
        aria-modal="true"
        aria-label="Connected Work"
        onClick={(event) => event.stopPropagation()}
        style={{ width: "min(1100px, 94vw)", maxWidth: 1100, height: "min(820px, 90vh)", overflow: "auto" }}
      >
        <div style={{ display: "flex", justifyContent: "space-between", gap: 16, alignItems: "center" }}>
          <div>
            <h2 style={{ marginBottom: 4 }}>Connected Work</h2>
            <p className="muted" style={{ margin: 0 }}>
              Google Workspace and GitHub actions run from OpenMindAI. Local chat remains independent when offline.
            </p>
          </div>
          <button type="button" className="ghost-button" onClick={onClose} title="Close Connected Work">
            <X size={18} />
          </button>
        </div>

        <div style={{ display: "flex", gap: 8, marginTop: 18 }}>
          <button
            type="button"
            className={provider === "google" ? "primary-button" : "ghost-button"}
            onClick={() => setProvider("google")}
          >
            Google Workspace
          </button>
          <button
            type="button"
            className={provider === "github" ? "primary-button" : "ghost-button"}
            onClick={() => setProvider("github")}
          >
            <Github size={15} /> GitHub
          </button>
        </div>

        <div className="connector-card" style={{ marginTop: 16 }}>
          {loadingConnections ? (
            <p className="muted"><Loader2 size={14} className="spin" /> Checking connections...</p>
          ) : provider === "google" ? (
            <div style={{ display: "flex", justifyContent: "space-between", gap: 16, alignItems: "center" }}>
              <div>
                <strong>{google?.connected ? "Google connected" : google?.configured ? "Google OAuth configured" : "Google OAuth not configured"}</strong>
                <p className="muted" style={{ margin: "4px 0 0" }}>
                  {google?.connected
                    ? google.email ?? "Connected Google account"
                    : google?.configured
                      ? "Sign in to grant Gmail, Drive, Calendar and Contacts access."
                      : "First save a Google Desktop OAuth Client ID and Client Secret in Settings → Connections."}
                </p>
              </div>
              {google?.connected ? (
                <button type="button" className="ghost-button" onClick={() => void disconnectGoogle()}>
                  <Unplug size={14} /> Disconnect
                </button>
              ) : (
                <button
                  type="button"
                  className="primary-button"
                  onClick={() => void connectGoogle()}
                  disabled={!google?.configured || connectingGoogle}
                >
                  {connectingGoogle ? <Loader2 size={14} className="spin" /> : "Connect Google"}
                </button>
              )}
            </div>
          ) : (
            <div>
              <strong>{github ? `GitHub connected as ${github.login}` : "GitHub not connected"}</strong>
              <p className="muted" style={{ margin: "4px 0 0" }}>
                {github
                  ? "Available actions depend on the repository permissions granted to the saved token."
                  : "Connect a GitHub token in Settings → Connections. Give only the repository permissions you need."}
              </p>
            </div>
          )}
        </div>

        <div style={{ display: "grid", gridTemplateColumns: "minmax(260px, 0.85fr) minmax(360px, 1.5fr)", gap: 18, marginTop: 18 }}>
          <section>
            <label className="connector-field">
              <span className="muted">Find action</span>
              <input value={filter} onChange={(event) => setFilter(event.target.value)} placeholder="Search actions..." />
            </label>
            <div style={{ display: "grid", gap: 6, maxHeight: 480, overflow: "auto", marginTop: 10 }}>
              {providerActions.map((item) => (
                <button
                  type="button"
                  key={`${item.provider}:${item.action}`}
                  className={definition?.action === item.action ? "primary-button" : "ghost-button"}
                  onClick={() => setAction(item.action)}
                  style={{ justifyContent: "flex-start", textAlign: "left", height: "auto", padding: "9px 11px" }}
                >
                  <span>{item.label}</span>
                </button>
              ))}
            </div>
          </section>

          <section>
            {definition ? (
              <>
                <div style={{ display: "flex", justifyContent: "space-between", gap: 12, alignItems: "flex-start" }}>
                  <div>
                    <h3 style={{ margin: 0 }}>{definition.label}</h3>
                    <p className="muted" style={{ marginTop: 5 }}>{definition.description}</p>
                    <code>{definition.action}</code>
                  </div>
                  {definition.mutating ? (
                    <span className="tool-menu-badge"><ShieldAlert size={13} /> Write</span>
                  ) : (
                    <span className="tool-menu-badge"><CheckCircle2 size={13} /> Read</span>
                  )}
                </div>

                <label className="connector-field" style={{ marginTop: 14 }}>
                  <span className="muted">Parameters (JSON)</span>
                  <textarea
                    value={paramsText}
                    onChange={(event) => setParamsText(event.target.value)}
                    spellCheck={false}
                    style={{ minHeight: 210, resize: "vertical", fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace" }}
                  />
                </label>

                {definition.mutating ? (
                  <label style={{ display: "flex", gap: 9, alignItems: "flex-start", marginTop: 12 }}>
                    <input type="checkbox" checked={approved} onChange={(event) => setApproved(event.target.checked)} />
                    <span>
                      I approve this remote change. OpenMindAI will send this operation to {provider === "google" ? "Google" : "GitHub"}.
                    </span>
                  </label>
                ) : null}

                <button
                  type="button"
                  className="primary-button"
                  onClick={() => void runAction()}
                  disabled={running || !connectionReady || (definition.mutating && !approved)}
                  style={{ marginTop: 14 }}
                >
                  {running ? <Loader2 size={14} className="spin" /> : "Run action"}
                </button>

                {error ? <p className="connector-error" style={{ marginTop: 12 }}>{error}</p> : null}
                {result ? (
                  <pre
                    style={{
                      marginTop: 14,
                      padding: 14,
                      maxHeight: 290,
                      overflow: "auto",
                      whiteSpace: "pre-wrap",
                      wordBreak: "break-word",
                      borderRadius: 10,
                      background: "rgba(0,0,0,.22)",
                    }}
                  >
                    {result}
                  </pre>
                ) : null}
              </>
            ) : (
              <p className="muted">No matching actions.</p>
            )}
          </section>
        </div>
      </div>
    </div>
  );
}
