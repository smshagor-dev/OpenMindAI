import {
  Boxes,
  CheckCircle2,
  Cloud,
  FileText,
  Loader2,
  Mail,
  MessageSquare,
  PlugZap,
  Trash2,
  Unplug,
} from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { api } from "../api";
import type { ConnectedProvider, IntegrationStatus } from "../lib/connectedActions";
import { formatError } from "../lib/format";

type EcosystemProvider = Exclude<ConnectedProvider, "google" | "github">;
type FormState = Record<string, string>;

const providerOrder: EcosystemProvider[] = ["microsoft", "slack", "notion", "dropbox", "mcp"];

const defaults: Record<EcosystemProvider, FormState> = {
  microsoft: {
    clientId: "",
    tenant: "common",
    redirectUri: "http://localhost:17894/oauth/microsoft",
  },
  slack: {
    clientId: "",
    redirectUri: "http://localhost:17895/oauth/slack",
    botScopes:
      "channels:read,channels:history,groups:read,groups:history,im:read,im:history,mpim:read,mpim:history,chat:write,reactions:write,users:read",
    userScopes: "search:read",
  },
  notion: {
    clientId: "",
    redirectUri: "http://localhost:17896/oauth/notion",
  },
  dropbox: {
    appKey: "",
    redirectUri: "http://localhost:17897/oauth/dropbox",
  },
  mcp: {
    name: "My MCP server",
    endpoint: "",
  },
};

const providerMeta: Record<
  EcosystemProvider,
  {
    title: string;
    description: string;
    fields: Array<{ key: string; label: string; placeholder?: string }>;
    clientSecret: boolean;
    tokenLabel: string;
    oauth: boolean;
  }
> = {
  microsoft: {
    title: "Microsoft 365",
    description: "Outlook Mail, OneDrive, Calendar and Contacts through delegated Microsoft Graph access.",
    fields: [
      { key: "clientId", label: "Application (client) ID", placeholder: "xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx" },
      { key: "tenant", label: "Tenant", placeholder: "common" },
      { key: "redirectUri", label: "Desktop redirect URI" },
    ],
    clientSecret: false,
    tokenLabel: "Optional access token (advanced)",
    oauth: true,
  },
  slack: {
    title: "Slack",
    description: "Channels, history, threads, search, users, messages and reactions through the Slack Web API.",
    fields: [
      { key: "clientId", label: "Slack app Client ID" },
      { key: "redirectUri", label: "OAuth redirect URI" },
      { key: "botScopes", label: "Bot scopes" },
      { key: "userScopes", label: "User scopes" },
    ],
    clientSecret: true,
    tokenLabel: "Optional bot/user token (xoxb-/xoxp-)",
    oauth: true,
  },
  notion: {
    title: "Notion",
    description: "Search pages, read blocks/data sources/comments, and create or update workspace content.",
    fields: [
      { key: "clientId", label: "Notion OAuth Client ID" },
      { key: "redirectUri", label: "OAuth redirect URI" },
    ],
    clientSecret: true,
    tokenLabel: "Optional internal/personal integration token",
    oauth: true,
  },
  dropbox: {
    title: "Dropbox",
    description: "List, search, download, upload, move and delete Dropbox files with PKCE OAuth.",
    fields: [
      { key: "appKey", label: "Dropbox App key" },
      { key: "redirectUri", label: "OAuth redirect URI" },
    ],
    clientSecret: false,
    tokenLabel: "Optional Dropbox access token",
    oauth: true,
  },
  mcp: {
    title: "MCP Server",
    description: "Connect any compatible remote MCP server for tools, resources and prompts.",
    fields: [
      { key: "name", label: "Display name", placeholder: "Company MCP" },
      { key: "endpoint", label: "Streamable HTTP endpoint", placeholder: "https://mcp.example.com/mcp" },
    ],
    clientSecret: false,
    tokenLabel: "Optional bearer token",
    oauth: false,
  },
};

function ProviderIcon({ provider }: { provider: EcosystemProvider }) {
  if (provider === "microsoft") return <Mail size={18} />;
  if (provider === "slack") return <MessageSquare size={18} />;
  if (provider === "notion") return <FileText size={18} />;
  if (provider === "dropbox") return <Cloud size={18} />;
  return <Boxes size={18} />;
}

function statusText(status: IntegrationStatus | null | undefined) {
  if (status === undefined) return "Checking...";
  if (!status) return "Unavailable";
  if (status.connected) {
    return status.accountLabel ? `Connected · ${status.accountLabel}` : "Connected";
  }
  return status.configured ? "Configured · not connected" : "Not configured";
}

export function AdditionalConnectors() {
  const [statuses, setStatuses] = useState<Partial<Record<EcosystemProvider, IntegrationStatus | null>>>({});
  const [forms, setForms] = useState<Record<EcosystemProvider, FormState>>(() => ({
    microsoft: { ...defaults.microsoft },
    slack: { ...defaults.slack },
    notion: { ...defaults.notion },
    dropbox: { ...defaults.dropbox },
    mcp: { ...defaults.mcp },
  }));
  const [clientSecrets, setClientSecrets] = useState<Partial<Record<EcosystemProvider, string>>>({});
  const [tokens, setTokens] = useState<Partial<Record<EcosystemProvider, string>>>({});
  const [busy, setBusy] = useState<string | null>(null);
  const [errors, setErrors] = useState<Partial<Record<EcosystemProvider, string>>>({});

  const loadProvider = async (provider: EcosystemProvider) => {
    try {
      const status = await api.integrationStatus(provider);
      setStatuses((current) => ({ ...current, [provider]: status }));
      setForms((current) => {
        const next = { ...current[provider] };
        for (const [key, value] of Object.entries(status.config ?? {})) {
          if (typeof value === "string") next[key] = value;
        }
        return { ...current, [provider]: next };
      });
    } catch {
      setStatuses((current) => ({ ...current, [provider]: null }));
    }
  };

  useEffect(() => {
    for (const provider of providerOrder) void loadProvider(provider);
  }, []);

  const updateField = (provider: EcosystemProvider, key: string, value: string) => {
    setForms((current) => ({
      ...current,
      [provider]: { ...current[provider], [key]: value },
    }));
  };

  const setProviderError = (provider: EcosystemProvider, error: unknown) => {
    setErrors((current) => ({ ...current, [provider]: formatError(error) }));
  };

  const clearProviderError = (provider: EcosystemProvider) => {
    setErrors((current) => ({ ...current, [provider]: undefined }));
  };

  const save = async (provider: EcosystemProvider) => {
    setBusy(`${provider}:save`);
    clearProviderError(provider);
    try {
      const secret = providerMeta[provider].clientSecret ? clientSecrets[provider] : undefined;
      const status = await api.saveIntegrationConfig(
        provider,
        forms[provider],
        secret?.trim() ? secret.trim() : undefined,
      );
      setStatuses((current) => ({ ...current, [provider]: status }));
      setClientSecrets((current) => ({ ...current, [provider]: "" }));
    } catch (error) {
      setProviderError(provider, error);
    } finally {
      setBusy(null);
    }
  };

  const connect = async (provider: EcosystemProvider, useToken: boolean) => {
    setBusy(`${provider}:connect`);
    clearProviderError(provider);
    try {
      const token = useToken ? tokens[provider]?.trim() : undefined;
      if (useToken && !token) throw new Error("Enter a token first.");
      const status = await api.connectIntegration(provider, token);
      setStatuses((current) => ({ ...current, [provider]: status }));
      setTokens((current) => ({ ...current, [provider]: "" }));
    } catch (error) {
      setProviderError(provider, error);
    } finally {
      setBusy(null);
    }
  };

  const disconnect = async (provider: EcosystemProvider) => {
    setBusy(`${provider}:disconnect`);
    clearProviderError(provider);
    try {
      await api.disconnectIntegration(provider);
      await loadProvider(provider);
    } catch (error) {
      setProviderError(provider, error);
    } finally {
      setBusy(null);
    }
  };

  const clear = async (provider: EcosystemProvider) => {
    setBusy(`${provider}:clear`);
    clearProviderError(provider);
    try {
      await api.clearIntegrationConfig(provider);
      setStatuses((current) => ({ ...current, [provider]: null }));
      setForms((current) => ({ ...current, [provider]: { ...defaults[provider] } }));
      setClientSecrets((current) => ({ ...current, [provider]: "" }));
      setTokens((current) => ({ ...current, [provider]: "" }));
    } catch (error) {
      setProviderError(provider, error);
    } finally {
      setBusy(null);
    }
  };

  const connectedCount = useMemo(
    () => providerOrder.filter((provider) => statuses[provider]?.connected).length,
    [statuses],
  );

  return (
    <>
      <div style={{ margin: "22px 0 10px" }}>
        <strong>More connected apps</strong>
        <p className="muted" style={{ margin: "4px 0 0" }}>
          {connectedCount} of {providerOrder.length} additional providers connected. Tokens and OAuth secrets stay in
          the operating system credential store.
        </p>
      </div>

      {providerOrder.map((provider) => {
        const meta = providerMeta[provider];
        const status = statuses[provider];
        const providerBusy = busy?.startsWith(`${provider}:`) ?? false;
        return (
          <div className="connector-card" key={provider}>
            <div className="connector-card-header">
              <span className="connector-icon">
                <ProviderIcon provider={provider} />
              </span>
              <div className="connector-card-title">
                <strong>{meta.title}</strong>
                <span className="muted">{statusText(status)}</span>
              </div>
              {status?.connected ? (
                <button type="button" className="ghost-button" onClick={() => void disconnect(provider)} disabled={providerBusy}>
                  <Unplug size={14} /> Disconnect
                </button>
              ) : null}
            </div>

            <p className="muted connector-hint">{meta.description}</p>

            {meta.fields.map((field) => (
              <label className="connector-field" key={field.key}>
                <span className="muted">{field.label}</span>
                <input
                  type="text"
                  placeholder={field.placeholder}
                  value={forms[provider][field.key] ?? ""}
                  onChange={(event) => updateField(provider, field.key, event.target.value)}
                />
              </label>
            ))}

            {meta.clientSecret ? (
              <label className="connector-field">
                <span className="muted">OAuth Client Secret</span>
                <input
                  type="password"
                  placeholder={status?.hasSecret ? "Saved — enter a new value only to replace it" : "Client secret"}
                  value={clientSecrets[provider] ?? ""}
                  onChange={(event) =>
                    setClientSecrets((current) => ({ ...current, [provider]: event.target.value }))
                  }
                />
              </label>
            ) : null}

            <div className="connector-connect-row" style={{ flexWrap: "wrap" }}>
              <button type="button" className="primary-button" onClick={() => void save(provider)} disabled={providerBusy}>
                {busy === `${provider}:save` ? <Loader2 size={14} className="spin" /> : "Save setup"}
              </button>
              {!status?.connected && status?.configured ? (
                <button type="button" className="ghost-button" onClick={() => void connect(provider, false)} disabled={providerBusy}>
                  {busy === `${provider}:connect` ? (
                    <Loader2 size={14} className="spin" />
                  ) : meta.oauth ? (
                    <><PlugZap size={14} /> Connect with OAuth</>
                  ) : (
                    <><CheckCircle2 size={14} /> Test connection</>
                  )}
                </button>
              ) : null}
              {status ? (
                <button type="button" className="ghost-button" onClick={() => void clear(provider)} disabled={providerBusy}>
                  <Trash2 size={14} /> Clear
                </button>
              ) : null}
            </div>

            <div className="connector-connect-row" style={{ marginTop: 10 }}>
              <input
                type="password"
                placeholder={meta.tokenLabel}
                value={tokens[provider] ?? ""}
                onChange={(event) => setTokens((current) => ({ ...current, [provider]: event.target.value }))}
                onKeyDown={(event) => {
                  if (event.key === "Enter") void connect(provider, true);
                }}
              />
              <button
                type="button"
                className="ghost-button"
                onClick={() => void connect(provider, true)}
                disabled={providerBusy || !(tokens[provider]?.trim())}
              >
                Connect token
              </button>
            </div>

            {errors[provider] ? <p className="connector-error">{errors[provider]}</p> : null}

            <p className="muted connector-hint">
              {provider === "microsoft"
                ? "Register a Mobile and desktop application in Microsoft Entra, add the exact localhost redirect URI above, and grant delegated User.Read, Mail.ReadWrite, Mail.Send, Files.ReadWrite, Calendars.ReadWrite and Contacts.Read permissions."
                : provider === "slack"
                  ? "Create a Slack app, add the exact redirect URI, configure the requested bot/user scopes, then install it to a workspace. Direct bot/user tokens are also supported for private use."
                  : provider === "notion"
                    ? "Create a public Notion connection for OAuth or use an internal/personal token. Notion only exposes pages and data sources explicitly shared with the connection."
                    : provider === "dropbox"
                      ? "Create a Dropbox app, add the exact redirect URI, choose the file scopes you need, then connect with PKCE. A generated access token can also be used for testing/private use."
                      : "Use an HTTPS Streamable HTTP MCP endpoint (HTTP is accepted only for localhost). Tool calls always require explicit approval in Connected Work."}
            </p>
          </div>
        );
      })}
    </>
  );
}
