import { Check, ChevronDown, ChevronRight, Chrome, Github, Loader2, Unplug } from "lucide-react";
import { useEffect, useState } from "react";
import { api } from "../api";
import type { GithubAccount, GithubIssue, GithubRepo, GoogleCredentialsStatus } from "../types";
import type { GoogleWorkspaceStatus } from "../lib/connectedActions";
import { formatError } from "../lib/format";
import { AdditionalConnectors } from "./AdditionalConnectors";

export function Connectors() {
  const [account, setAccount] = useState<GithubAccount | null | undefined>(undefined);
  const [tokenInput, setTokenInput] = useState("");
  const [connecting, setConnecting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [repos, setRepos] = useState<GithubRepo[] | null>(null);
  const [reposLoading, setReposLoading] = useState(false);
  const [expandedRepo, setExpandedRepo] = useState<string | null>(null);
  const [issuesByRepo, setIssuesByRepo] = useState<Record<string, GithubIssue[]>>({});
  const [issuesLoading, setIssuesLoading] = useState<string | null>(null);

  const [googleStatus, setGoogleStatus] = useState<GoogleCredentialsStatus | null | undefined>(undefined);
  const [googleWorkspace, setGoogleWorkspace] = useState<GoogleWorkspaceStatus | null>(null);
  const [clientIdInput, setClientIdInput] = useState("");
  const [clientSecretInput, setClientSecretInput] = useState("");
  const [googleSaving, setGoogleSaving] = useState(false);
  const [googleConnecting, setGoogleConnecting] = useState(false);
  const [googleSaved, setGoogleSaved] = useState(false);
  const [googleError, setGoogleError] = useState<string | null>(null);

  const refreshGoogle = async () => {
    const [credentials, workspace] = await Promise.allSettled([
      api.googleCredentials(),
      api.googleWorkspaceStatus(),
    ]);
    const credentialStatus = credentials.status === "fulfilled" ? credentials.value : null;
    setGoogleStatus(credentialStatus);
    if (credentialStatus) setClientIdInput(credentialStatus.clientId);
    setGoogleWorkspace(workspace.status === "fulfilled" ? workspace.value : null);
  };

  useEffect(() => {
    api
      .githubAccount()
      .then(setAccount)
      .catch(() => setAccount(null));
    void refreshGoogle();
  }, []);

  const saveGoogleCredentials = async () => {
    if (!clientIdInput.trim() || !clientSecretInput.trim()) return;
    setGoogleSaving(true);
    setGoogleError(null);
    setGoogleSaved(false);
    try {
      const status = await api.saveGoogleCredentials(clientIdInput.trim(), clientSecretInput.trim());
      setGoogleStatus(status);
      setClientSecretInput("");
      setGoogleSaved(true);
      await refreshGoogle();
    } catch (caught) {
      setGoogleError(formatError(caught));
    } finally {
      setGoogleSaving(false);
    }
  };

  const connectGoogle = async () => {
    setGoogleConnecting(true);
    setGoogleError(null);
    try {
      setGoogleWorkspace(await api.connectGoogleWorkspace());
    } catch (caught) {
      setGoogleError(formatError(caught));
    } finally {
      setGoogleConnecting(false);
    }
  };

  const disconnectGoogle = async () => {
    setGoogleError(null);
    try {
      await api.disconnectGoogleWorkspace();
      await refreshGoogle();
    } catch (caught) {
      setGoogleError(formatError(caught));
    }
  };

  const clearGoogleCredentials = async () => {
    if (googleWorkspace?.connected) await api.disconnectGoogleWorkspace();
    await api.clearGoogleCredentials();
    setGoogleStatus(null);
    setGoogleWorkspace(null);
    setClientIdInput("");
    setClientSecretInput("");
    setGoogleSaved(false);
  };

  const connect = async () => {
    if (!tokenInput.trim()) return;
    setConnecting(true);
    setError(null);
    try {
      const connected = await api.saveGithubToken(tokenInput.trim());
      setAccount(connected);
      setTokenInput("");
    } catch (caught) {
      setError(formatError(caught));
    } finally {
      setConnecting(false);
    }
  };

  const disconnect = async () => {
    await api.disconnectGithub();
    setAccount(null);
    setRepos(null);
    setIssuesByRepo({});
    setExpandedRepo(null);
  };

  const loadRepos = async () => {
    setReposLoading(true);
    setError(null);
    try {
      setRepos(await api.listGithubRepos());
    } catch (caught) {
      setError(formatError(caught));
    } finally {
      setReposLoading(false);
    }
  };

  const toggleRepo = async (fullName: string) => {
    if (expandedRepo === fullName) {
      setExpandedRepo(null);
      return;
    }
    setExpandedRepo(fullName);
    if (issuesByRepo[fullName]) return;
    setIssuesLoading(fullName);
    try {
      const issues = await api.listGithubIssues(fullName);
      setIssuesByRepo((current) => ({ ...current, [fullName]: issues }));
    } catch (caught) {
      setError(formatError(caught));
    } finally {
      setIssuesLoading(null);
    }
  };

  return (
    <>
      <div className="connector-card">
        <div className="connector-card-header">
          <span className="connector-icon">
            <Github size={18} />
          </span>
          <div className="connector-card-title">
            <strong>GitHub</strong>
            <span className="muted">
              {account === undefined ? "Checking..." : account ? `Connected as ${account.login}` : "Not connected"}
            </span>
          </div>
          {account ? (
            <button type="button" className="ghost-button" onClick={disconnect}>
              Disconnect
            </button>
          ) : null}
        </div>

        {!account ? (
          <div className="connector-connect-row">
            <input
              type="password"
              placeholder="Paste a GitHub personal access token"
              value={tokenInput}
              onChange={(event) => setTokenInput(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter") void connect();
              }}
            />
            <button
              type="button"
              className="primary-button"
              onClick={connect}
              disabled={connecting || !tokenInput.trim()}
            >
              {connecting ? <Loader2 size={14} className="spin" /> : "Connect"}
            </button>
          </div>
        ) : null}

        {!account ? (
          <p className="muted connector-hint">
            Prefer a fine-grained token restricted to the repositories OpenMindAI should manage. Grant Contents,
            Pull requests, Issues, Actions/Workflows and Releases permissions only when you want those features.
            The token is stored in the operating system credential store; remote writes still require explicit
            approval inside Work mode.
          </p>
        ) : (
          <p className="muted connector-hint">
            Open <strong>Work → GitHub</strong> to read or modify repository files, create branches and multi-file
            commits, manage issues and pull requests, inspect or control Actions, and manage tags/releases. GitHub
            will reject any action that exceeds the saved token&apos;s permissions.
          </p>
        )}

        {error ? <p className="connector-error">{error}</p> : null}

        {account ? (
          <div className="connector-body">
            <button type="button" className="ghost-button" onClick={loadRepos} disabled={reposLoading}>
              {reposLoading ? <Loader2 size={14} className="spin" /> : repos ? "Refresh repositories" : "Load repositories"}
            </button>
            {repos ? (
              <div className="connector-repo-list">
                {repos.length === 0 ? <p className="muted">No repositories found.</p> : null}
                {repos.map((repo) => (
                  <div className="connector-repo" key={repo.id}>
                    <button type="button" className="connector-repo-row" onClick={() => void toggleRepo(repo.fullName)}>
                      {expandedRepo === repo.fullName ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
                      <span className="connector-repo-name">{repo.fullName}</span>
                      <span className="muted">
                        {repo.private ? "Private" : "Public"} · {repo.stargazersCount}★
                      </span>
                    </button>
                    {expandedRepo === repo.fullName ? (
                      <div className="connector-issue-list">
                        {issuesLoading === repo.fullName ? (
                          <p className="muted">Loading issues...</p>
                        ) : (issuesByRepo[repo.fullName] ?? []).length === 0 ? (
                          <p className="muted">No open issues.</p>
                        ) : (
                          (issuesByRepo[repo.fullName] ?? []).map((issue) => (
                            <a
                              className="connector-issue"
                              key={issue.id}
                              href={issue.htmlUrl}
                              target="_blank"
                              rel="noreferrer"
                            >
                              <span className="muted">#{issue.number}</span>
                              <span>{issue.title}</span>
                              {issue.isPullRequest ? <span className="tool-menu-badge">PR</span> : null}
                            </a>
                          ))
                        )}
                      </div>
                    ) : null}
                  </div>
                ))}
              </div>
            ) : null}
          </div>
        ) : null}
      </div>

      <div className="connector-card">
        <div className="connector-card-header">
          <span className="connector-icon">
            <Chrome size={18} />
          </span>
          <div className="connector-card-title">
            <strong>Google Workspace</strong>
            <span className="muted">
              {googleStatus === undefined
                ? "Checking..."
                : googleWorkspace?.connected
                  ? `Connected${googleWorkspace.email ? ` as ${googleWorkspace.email}` : ""}`
                  : googleStatus
                    ? `OAuth configured · ${googleStatus.hasSecret ? "Secret saved" : "No secret saved"}`
                    : "Not configured"}
            </span>
          </div>
          {googleWorkspace?.connected ? (
            <button type="button" className="ghost-button" onClick={() => void disconnectGoogle()}>
              <Unplug size={14} /> Disconnect
            </button>
          ) : googleStatus ? (
            <button type="button" className="ghost-button" onClick={clearGoogleCredentials}>
              Clear
            </button>
          ) : null}
        </div>

        <label className="connector-field">
          <span className="muted">Desktop OAuth Client ID</span>
          <input
            type="text"
            placeholder="xxxxxxxx.apps.googleusercontent.com"
            value={clientIdInput}
            onChange={(event) => {
              setClientIdInput(event.target.value);
              setGoogleSaved(false);
            }}
          />
        </label>
        <label className="connector-field">
          <span className="muted">Client Secret</span>
          <input
            type="password"
            placeholder={googleStatus?.hasSecret ? "Saved — enter a new value to replace it" : "GOCSPX-..."}
            value={clientSecretInput}
            onChange={(event) => {
              setClientSecretInput(event.target.value);
              setGoogleSaved(false);
            }}
          />
        </label>

        <div className="connector-connect-row">
          <button
            type="button"
            className="primary-button"
            onClick={saveGoogleCredentials}
            disabled={googleSaving || !clientIdInput.trim() || !clientSecretInput.trim()}
          >
            {googleSaving ? <Loader2 size={14} className="spin" /> : googleSaved ? <Check size={14} /> : "Save OAuth app"}
          </button>
          {!googleWorkspace?.connected ? (
            <button
              type="button"
              className="ghost-button"
              onClick={() => void connectGoogle()}
              disabled={!googleStatus?.hasSecret || googleConnecting}
            >
              {googleConnecting ? <Loader2 size={14} className="spin" /> : "Connect Google account"}
            </button>
          ) : null}
        </div>

        {googleError ? <p className="connector-error">{googleError}</p> : null}

        <p className="muted connector-hint">
          Create a <strong>Desktop app</strong> OAuth client in Google Cloud, enable Gmail, Drive, Calendar and People
          APIs, then save the client credentials here. Connect opens Google&apos;s consent page with PKCE/state
          protection and a localhost callback. OAuth access/refresh tokens and the client secret are stored in the
          operating system credential store, not chat history. Use <strong>Work → Google Workspace</strong> for
          Gmail, Drive, Calendar and Contacts actions.
        </p>
      </div>

      <AdditionalConnectors />
    </>
  );
}
