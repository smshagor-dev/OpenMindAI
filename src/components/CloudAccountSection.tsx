import { useCallback, useEffect, useState, type FormEvent } from "react";
import { cloud, cloudModelDisplayName, type CloudConnection, type CloudModel } from "../cloudApi";
import { formatError } from "../lib/format";
import { api } from "../api_legacy";

/**
 * Cloud account panel shown in Settings → Cloud. Connection uses a developer
 * API key (issued at api.openmindai.org) rather than an email/password login:
 * the key is validated and stored by the existing `cloud_connect` backend
 * command, which keeps it in secure native storage and authenticates every
 * hosted request. No password or OTP handling is needed.
 */
function accountName(status: CloudConnection | null): string | null {
  const user = status?.user as { name?: string; user?: { name?: string } } | null;
  return user?.name ?? user?.user?.name ?? null;
}

export function CloudAccountSection(props: {
  onConnectionChange: (connection: CloudConnection) => void;
}) {
  const { onConnectionChange } = props;
  const [status, setStatus] = useState<CloudConnection | null>(null);
  const [apiKey, setApiKey] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [models, setModels] = useState<CloudModel[] | null>(null);

  const refreshModels = useCallback(async () => {
    setModels(null);
    try {
      setModels(await cloud.models());
    } catch {
      setModels([]);
    }
  }, []);

  useEffect(() => {
    let active = true;
    void cloud
      .status()
      .then((result) => {
        if (active) {
          setStatus(result);
          onConnectionChange(result);
          if (result.connected) void refreshModels();
          else setModels([]);
        }
      })
      .catch(() => {
        if (active) {
          const disconnected = { connected: false, user: null, message: null };
          setStatus(disconnected);
          setModels([]);
          onConnectionChange(disconnected);
        }
      });
    return () => {
      active = false;
    };
  }, [onConnectionChange, refreshModels]);

  const connect = async (event: FormEvent) => {
    event.preventDefault();
    const key = apiKey.trim();
    if (!key) return;
    setBusy(true);
    setError(null);
    setNotice(null);
    try {
      const session = await cloud.connect(key);
      setStatus(session);
      onConnectionChange(session);
      void refreshModels();
      setApiKey("");
      setNotice("Connected to OpenMindAI Cloud.");
    } catch (caught) {
      setError(formatError(caught));
    } finally {
      setBusy(false);
    }
  };

  const disconnect = async () => {
    setBusy(true);
    setError(null);
    setNotice(null);
    try {
      await cloud.disconnect();
      const disconnected = { connected: false, user: null, message: null };
      setStatus(disconnected);
      setModels([]);
      onConnectionChange(disconnected);
      setNotice("Disconnected from Cloud.");
    } catch (caught) {
      setError(formatError(caught));
    } finally {
      setBusy(false);
    }
  };

  if (status?.connected) {
    const name = accountName(status);
    return (
      <div className="cloud-account">
        <div className="cloud-account-head connected">
          <span className="studio-dot good" />
          <div>
            <strong>Connected to OpenMindAI Cloud</strong>
            {name ? <p className="muted">Signed in as {name}</p> : null}
          </div>
        </div>
        <p className="muted">
          Your API key is stored in secure native storage. Cloud unlocks hosted chat models, web
          search, Research Studio and 3D Studio. Local AI keeps working without it.
        </p>
        <div className="cloud-model-catalog" aria-live="polite">
          {models === null ? (
            <p className="muted">Checking available hosted chat models…</p>
          ) : models.length === 0 ? (
            <p className="muted">No hosted chat models are currently available.</p>
          ) : (
            <>
              <strong>Available hosted chat models ({models.length})</strong>
              <ul>
                {models.map((model) => {
                  const displayName = cloudModelDisplayName(model);
                  return (
                    <li key={model.id}>
                      <span>{displayName}</span>
                      {model.id !== displayName ? <small title="Provider model ID">{model.id}</small> : null}
                    </li>
                  );
                })}
              </ul>
            </>
          )}
        </div>
        {notice ? (
          <p className="cloud-account-note" role="status">
            {notice}
          </p>
        ) : null}
        {error ? (
          <p className="setup-warning" role="alert">
            {error}
          </p>
        ) : null}
        <button
          type="button"
          className="cloud-account-disconnect"
          disabled={busy}
          onClick={() => void disconnect()}
        >
          {busy ? "Working…" : "Disconnect"}
        </button>
      </div>
    );
  }

  return (
    <div className="cloud-account">
      <p className="muted">
        Connect OpenMindAI Cloud with your developer API key to use hosted models, web search,
        Research Studio and 3D Studio. Local AI keeps working without it.
      </p>
      {status?.message ? (
        <p className="cloud-account-note" role="status">
          {status.message}
        </p>
      ) : null}
      <form className="cloud-account-form" onSubmit={connect}>
        <label className="setup-field-label">
          API key
          <input
            type="password"
            autoComplete="off"
            spellCheck={false}
            value={apiKey}
            onChange={(event) => setApiKey(event.target.value)}
            placeholder="Paste your OpenMindAI API key"
            required
          />
        </label>
        {error ? (
          <p className="setup-warning" role="alert">
            {error}
          </p>
        ) : null}
        {notice ? (
          <p className="cloud-account-note" role="status">
            {notice}
          </p>
        ) : null}
        <button type="submit" className="primary-button" disabled={busy || !apiKey.trim()}>
          {busy ? "Connecting…" : "Connect Cloud"}
        </button>
      </form>
      <p className="muted cloud-account-register">
        Get your API key from the developer dashboard at{" "}
        <a href="https://api.openmindai.org" onClick={(event) => { event.preventDefault(); void api.openExternalUrl("https://api.openmindai.org"); }}>api.openmindai.org</a>.
      </p>
    </div>
  );
}
