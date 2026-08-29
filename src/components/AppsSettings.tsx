import { useEffect, useMemo, useState } from "react";
import {
  Boxes,
  CheckCircle2,
  Cloud,
  Github,
  Loader2,
  Mail,
  MessageSquare,
  PlugZap,
  Unplug,
} from "lucide-react";
import { api } from "../api";
import type { GithubAccount, GoogleCredentialsStatus } from "../types";
import type {
  ConnectedProvider,
  GoogleWorkspaceStatus,
  IntegrationStatus,
} from "../lib/connectedActions";
import { formatError } from "../lib/format";

type EcosystemProvider = Exclude<ConnectedProvider, "google" | "github">;
type FormState = Record<string, string>;

const providerOrder: EcosystemProvider[] = ["microsoft", "slack", "notion", "dropbox", "mcp"];

const providerMeta: Record<
  EcosystemProvider,
  {
    title: string;
    description: string;
    fields: Array<{ key: string; label: string; placeholder?: string }>;
    defaults: FormState;
    secret: boolean;
  }
> = {
  microsoft: {
    title: "Microsoft 365",
    description: "Mail, OneDrive, calendar, and contacts.",
    fields: [
      { key: "clientId", label: "Application (client) ID" },
      { key: "tenant", label: "Tenant", placeholder: "common" },
      { key: "redirectUri", label: "Desktop redirect URI" },
    ],
    defaults: {
      clientId: "",
      tenant: "common",
      redirectUri: "http://localhost:17894/oauth/microsoft",
    },
    secret: false,
  },
  slack: {
    title: "Slack",
    description: "Channels, threads, search, and messages.",
    fields: [
      { key: "clientId", label: "Slack app Client ID" },
      { key: "redirectUri", label: "OAuth redirect URI" },
      { key: "botScopes", label: "Bot scopes" },
      { key: "userScopes", label: "User scopes" },
    ],
    defaults: {
      clientId: "",
      redirectUri: "http://localhost:17895/oauth/slack",
      botScopes:
        "channels:read,channels:history,groups:read,groups:history,im:read,im:history,mpim:read,mpim:history,chat:write,reactions:write,users:read",
      userScopes: "search:read",
    },
    secret: true,
  },
  notion: {
    title: "Notion",
    description: "Search, read, create, and update workspace content.",
    fields: [
      { key: "clientId", label: "Notion OAuth Client ID" },
      { key: "redirectUri", label: "OAuth redirect URI" },
    ],
    defaults: { clientId: "", redirectUri: "http://localhost:17896/oauth/notion" },
    secret: true,
  },
  dropbox: {
    title: "Dropbox",
    description: "Search, download, upload, move, and delete files.",
    fields: [
      { key: "appKey", label: "Dropbox App key" },
      { key: "redirectUri", label: "OAuth redirect URI" },
    ],
    defaults: { appKey: "", redirectUri: "http://localhost:17897/oauth/dropbox" },
    secret: false,
  },
  mcp: {
    title: "MCP Server",
    description: "Use tools and resources from a compatible MCP server.",
    fields: [
      { key: "name", label: "Display name", placeholder: "Company MCP" },
      {
        key: "endpoint",
        label: "Streamable HTTP endpoint",
        placeholder: "https://mcp.example.com/mcp",
      },
    ],
    defaults: { name: "My MCP server", endpoint: "" },
    secret: false,
  },
};

function ProviderIcon({ provider }: { provider: EcosystemProvider }) {
  if (provider === "microsoft") return <Mail size={19} />;
  if (provider === "slack") return <MessageSquare size={19} />;
  if (provider === "notion") return <Boxes size={19} />;
  if (provider === "dropbox") return <Cloud size={19} />;
  return <PlugZap size={19} />;
}

function statusLabel(connected: boolean, configured: boolean, account?: string | null) {
  if (connected) return account ? `Connected · ${account}` : "Connected";
  if (configured) return "Ready to connect";
  return "Not connected";
}

export function AppsSettings() {
  const [github, setGithub] = useState<GithubAccount | null | undefined>(undefined);
  const [githubToken, setGithubToken] = useState("");
  const [githubSetup, setGithubSetup] = useState(false);
  const [googleCredentials, setGoogleCredentials] = useState<
    GoogleCredentialsStatus | null | undefined
  >(undefined);
  const [googleWorkspace, setGoogleWorkspace] = useState<GoogleWorkspaceStatus | null>(null);
  const [googleSetup, setGoogleSetup] = useState(false);
  const [googleClientId, setGoogleClientId] = useState("");
  const [googleClientSecret, setGoogleClientSecret] = useState("");
  const [statuses, setStatuses] = useState<
    Partial<Record<EcosystemProvider, IntegrationStatus | null>>
  >({});
  const [expandedProvider, setExpandedProvider] = useState<EcosystemProvider | null>(null);
  const [forms, setForms] = useState<Record<EcosystemProvider, FormState>>(() => ({
    microsoft: { ...providerMeta.microsoft.defaults },
    slack: { ...providerMeta.slack.defaults },
    notion: { ...providerMeta.notion.defaults },
    dropbox: { ...providerMeta.dropbox.defaults },
    mcp: { ...providerMeta.mcp.defaults },
  }));
  const [secrets, setSecrets] = useState<Partial<Record<EcosystemProvider, string>>>({});
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = async () => {
    const [githubResult, credentialsResult, workspaceResult, ...integrationResults] =
      await Promise.allSettled([
        api.githubAccount(),
        api.googleCredentials(),
        api.googleWorkspaceStatus(),
        ...providerOrder.map((provider) => api.integrationStatus(provider)),
      ]);
    setGithub(githubResult.status === "fulfilled" ? githubResult.value : null);
    const credentials = credentialsResult.status === "fulfilled" ? credentialsResult.value : null;
    setGoogleCredentials(credentials);
    setGoogleClientId(credentials?.clientId ?? "");
    setGoogleWorkspace(workspaceResult.status === "fulfilled" ? workspaceResult.value : null);
    integrationResults.forEach((result, index) => {
      const provider = providerOrder[index];
      if (result.status !== "fulfilled") {
        setStatuses((current) => ({ ...current, [provider]: null }));
        return;
      }
      setStatuses((current) => ({ ...current, [provider]: result.value }));
      setForms((current) => {
        const next = { ...current[provider] };
        for (const [key, value] of Object.entries(result.value.config ?? {})) {
          if (typeof value === "string") next[key] = value;
        }
        return { ...current, [provider]: next };
      });
    });
  };

  useEffect(() => {
    void refresh();
  }, []);

  const connectedCount = useMemo(
    () =>
      Number(Boolean(github)) +
      Number(Boolean(googleWorkspace?.connected)) +
      providerOrder.filter((provider) => statuses[provider]?.connected).length,
    [github, googleWorkspace?.connected, statuses],
  );

  const run = async (key: string, action: () => Promise<void>) => {
    if (busy) return;
    setBusy(key);
    setError(null);
    try {
      await action();
    } catch (caught) {
      setError(formatError(caught));
    } finally {
      setBusy(null);
    }
  };

  const connectGithub = () =>
    run("github", async () => {
      if (!githubToken.trim()) {
        setGithubSetup(true);
        return;
      }
      setGithub(await api.saveGithubToken(githubToken.trim()));
      setGithubToken("");
      setGithubSetup(false);
    });

  const connectGoogle = () =>
    run("google", async () => {
      if (!googleCredentials?.hasSecret) {
        setGoogleSetup(true);
        return;
      }
      setGoogleWorkspace(await api.connectGoogleWorkspace());
    });

  const saveGoogleSetup = () =>
    run("google-setup", async () => {
      if (!googleClientId.trim() || !googleClientSecret.trim()) return;
      const credentials = await api.saveGoogleCredentials(
        googleClientId.trim(),
        googleClientSecret.trim(),
      );
      setGoogleCredentials(credentials);
      setGoogleClientSecret("");
      setGoogleWorkspace(await api.connectGoogleWorkspace());
      setGoogleSetup(false);
    });

  const connectProvider = (provider: EcosystemProvider) =>
    run(`${provider}:connect`, async () => {
      const status = statuses[provider];
      if (!status?.configured) {
        setExpandedProvider(provider);
        return;
      }
      const connected = await api.connectIntegration(provider);
      setStatuses((current) => ({ ...current, [provider]: connected }));
    });

  const saveProvider = (provider: EcosystemProvider) =>
    run(`${provider}:save`, async () => {
      const status = await api.saveIntegrationConfig(
        provider,
        forms[provider],
        providerMeta[provider].secret && secrets[provider]?.trim()
          ? secrets[provider]?.trim()
          : undefined,
      );
      setStatuses((current) => ({ ...current, [provider]: status }));
      setSecrets((current) => ({ ...current, [provider]: "" }));
      setExpandedProvider(null);
    });

  return (
    <div className="apps-settings">
      <div className="apps-settings-intro">
        <div>
          <span className="tools-eyebrow">Connected apps</span>
          <h3>Apps work inside your conversations</h3>
          <p>
            Connect once, then ask naturally in Chat or Project Work. OpenMindAI chooses the
            relevant app and action internally—there is no provider picker, raw action console, or
            JSON form in Work.
          </p>
        </div>
        <span className="apps-connected-count">
          <CheckCircle2 size={14} /> {connectedCount} connected
        </span>
      </div>

      {error ? (
        <button type="button" className="error-banner apps-error" onClick={() => setError(null)}>
          {error}
        </button>
      ) : null}

      <div className="apps-grid">
        <article className="app-card">
          <div className="app-card-main">
            <span className="app-card-icon">
              <Github size={19} />
            </span>
            <div>
              <strong>GitHub</strong>
              <p>Repositories, issues, pull requests, Actions, and releases.</p>
            </div>
          </div>
          <div className="app-card-actions">
            <span className={github ? "app-status connected" : "app-status"}>
              {github === undefined
                ? "Checking…"
                : github
                  ? `Connected · ${github.login}`
                  : "Not connected"}
            </span>
            {github ? (
              <button
                type="button"
                className="ghost-button"
                disabled={Boolean(busy)}
                onClick={() =>
                  void run("github-disconnect", async () => {
                    await api.disconnectGithub();
                    setGithub(null);
                  })
                }
              >
                <Unplug size={14} /> Disconnect
              </button>
            ) : (
              <button
                type="button"
                className="primary-button"
                disabled={Boolean(busy)}
                onClick={() => {
                  setGithubSetup(true);
                }}
              >
                <PlugZap size={14} /> Connect
              </button>
            )}
          </div>
          {githubSetup && !github ? (
            <div className="app-setup-panel">
              <p>
                OpenMindAI currently uses a GitHub token stored in your operating-system credential
                store. This setup is only for connecting the app; repository actions stay internal
                to Chat/Work.
              </p>
              <input
                type="password"
                value={githubToken}
                onChange={(event) => setGithubToken(event.target.value)}
                placeholder="Fine-grained GitHub token"
              />
              <div className="button-row">
                <button
                  type="button"
                  className="primary-button"
                  disabled={!githubToken.trim() || Boolean(busy)}
                  onClick={() => void connectGithub()}
                >
                  {busy === "github" ? <Loader2 size={14} className="spin" /> : "Connect GitHub"}
                </button>
                <button
                  type="button"
                  className="ghost-button"
                  onClick={() => setGithubSetup(false)}
                >
                  Cancel
                </button>
              </div>
            </div>
          ) : null}
        </article>

        <article className="app-card">
          <div className="app-card-main">
            <span className="app-card-icon">
              <Mail size={19} />
            </span>
            <div>
              <strong>Google Workspace</strong>
              <p>Gmail, Drive, Calendar, and Contacts.</p>
            </div>
          </div>
          <div className="app-card-actions">
            <span className={googleWorkspace?.connected ? "app-status connected" : "app-status"}>
              {statusLabel(
                Boolean(googleWorkspace?.connected),
                Boolean(googleCredentials),
                googleWorkspace?.email,
              )}
            </span>
            {googleWorkspace?.connected ? (
              <button
                type="button"
                className="ghost-button"
                disabled={Boolean(busy)}
                onClick={() =>
                  void run("google-disconnect", async () => {
                    await api.disconnectGoogleWorkspace();
                    setGoogleWorkspace(null);
                  })
                }
              >
                <Unplug size={14} /> Disconnect
              </button>
            ) : (
              <button
                type="button"
                className="primary-button"
                disabled={Boolean(busy)}
                onClick={() => void connectGoogle()}
              >
                <PlugZap size={14} /> Connect
              </button>
            )}
          </div>
          {googleSetup && !googleWorkspace?.connected ? (
            <div className="app-setup-panel">
              <p>
                This self-hosted desktop build needs a one-time Google Desktop OAuth client.
                Credentials are stored in the operating-system credential store and are never shown
                in Work.
              </p>
              <input
                type="text"
                value={googleClientId}
                onChange={(event) => setGoogleClientId(event.target.value)}
                placeholder="Desktop OAuth Client ID"
              />
              <input
                type="password"
                value={googleClientSecret}
                onChange={(event) => setGoogleClientSecret(event.target.value)}
                placeholder={
                  googleCredentials?.hasSecret ? "Saved — enter only to replace" : "Client Secret"
                }
              />
              <div className="button-row">
                <button
                  type="button"
                  className="primary-button"
                  disabled={!googleClientId.trim() || !googleClientSecret.trim() || Boolean(busy)}
                  onClick={() => void saveGoogleSetup()}
                >
                  {busy === "google-setup" ? (
                    <Loader2 size={14} className="spin" />
                  ) : (
                    "Save & connect"
                  )}
                </button>
                <button
                  type="button"
                  className="ghost-button"
                  onClick={() => setGoogleSetup(false)}
                >
                  Cancel
                </button>
              </div>
            </div>
          ) : null}
        </article>

        {providerOrder.map((provider) => {
          const meta = providerMeta[provider];
          const status = statuses[provider];
          const expanded = expandedProvider === provider;
          return (
            <article className="app-card" key={provider}>
              <div className="app-card-main">
                <span className="app-card-icon">
                  <ProviderIcon provider={provider} />
                </span>
                <div>
                  <strong>{meta.title}</strong>
                  <p>{meta.description}</p>
                </div>
              </div>
              <div className="app-card-actions">
                <span className={status?.connected ? "app-status connected" : "app-status"}>
                  {statusLabel(
                    Boolean(status?.connected),
                    Boolean(status?.configured),
                    status?.accountLabel,
                  )}
                </span>
                {status?.connected ? (
                  <button
                    type="button"
                    className="ghost-button"
                    disabled={Boolean(busy)}
                    onClick={() =>
                      void run(`${provider}:disconnect`, async () => {
                        await api.disconnectIntegration(provider);
                        setStatuses((current) => ({
                          ...current,
                          [provider]: { ...status, connected: false },
                        }));
                      })
                    }
                  >
                    <Unplug size={14} /> Disconnect
                  </button>
                ) : (
                  <button
                    type="button"
                    className="primary-button"
                    disabled={Boolean(busy)}
                    onClick={() => void connectProvider(provider)}
                  >
                    <PlugZap size={14} /> {status?.configured ? "Connect" : "Set up"}
                  </button>
                )}
              </div>
              {expanded ? (
                <div className="app-setup-panel">
                  <p>
                    Provider setup is kept here in Settings. Once connected, OpenMindAI uses it
                    internally from natural-language requests.
                  </p>
                  {meta.fields.map((field) => (
                    <label key={field.key}>
                      <span>{field.label}</span>
                      <input
                        type="text"
                        placeholder={field.placeholder}
                        value={forms[provider][field.key] ?? ""}
                        onChange={(event) =>
                          setForms((current) => ({
                            ...current,
                            [provider]: { ...current[provider], [field.key]: event.target.value },
                          }))
                        }
                      />
                    </label>
                  ))}
                  {meta.secret ? (
                    <label>
                      <span>OAuth Client Secret</span>
                      <input
                        type="password"
                        value={secrets[provider] ?? ""}
                        onChange={(event) =>
                          setSecrets((current) => ({ ...current, [provider]: event.target.value }))
                        }
                        placeholder={
                          status?.hasSecret ? "Saved — enter only to replace" : "Client secret"
                        }
                      />
                    </label>
                  ) : null}
                  <div className="button-row">
                    <button
                      type="button"
                      className="primary-button"
                      disabled={Boolean(busy)}
                      onClick={() => void saveProvider(provider)}
                    >
                      {busy === `${provider}:save` ? (
                        <Loader2 size={14} className="spin" />
                      ) : (
                        "Save setup"
                      )}
                    </button>
                    <button
                      type="button"
                      className="ghost-button"
                      onClick={() => setExpandedProvider(null)}
                    >
                      Cancel
                    </button>
                  </div>
                </div>
              ) : null}
            </article>
          );
        })}
      </div>

      <p className="apps-privacy-note">
        Read actions may run as part of a request when the app is connected. Remote changes remain
        subject to the backend approval and permission guards. Credentials stay out of chat history.
      </p>
    </div>
  );
}
