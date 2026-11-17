import { useEffect, useState, type FormEvent } from "react";
import { cloud, type CloudConnection } from "../cloudApi";
import { formatError } from "../lib/format";

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

export function CloudAccountSection() {
  const [status, setStatus] = useState<CloudConnection | null>(null);
  const [apiKey, setApiKey] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    void cloud
      .status()
      .then((result) => {
        if (active) setStatus(result);
      })
      .catch(() => {
        if (active) setStatus({ connected: false, user: null, message: null });
      });
    return () => {
      active = false;
    };
  }, []);

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
      setStatus({ connected: false, user: null, message: null });
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
        Get your API key from the developer dashboard at <strong>api.openmindai.org</strong>.
      </p>
    </div>
  );
}
