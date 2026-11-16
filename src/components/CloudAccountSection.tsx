import { useEffect, useState, type FormEvent } from "react";
import { cloud, type CloudConnection } from "../cloudApi";
import { formatError } from "../lib/format";

/**
 * Cloud account panel shown in Settings → Cloud. Reuses the existing
 * email/password (+ optional verification code) sign-in that the backend
 * already supports via `cloud_login`. Registration and in-app OTP delivery are
 * not wired yet (the cloud server API for them is not part of this client), so
 * account creation points to the web for now.
 */
function accountName(status: CloudConnection | null): string | null {
  const user = status?.user as { name?: string; user?: { name?: string } } | null;
  return user?.name ?? user?.user?.name ?? null;
}

export function CloudAccountSection() {
  const [status, setStatus] = useState<CloudConnection | null>(null);
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [code, setCode] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    void cloud
      .status()
      .then((result) => { if (active) setStatus(result); })
      .catch(() => { if (active) setStatus({ connected: false, user: null, message: null }); });
    return () => { active = false; };
  }, []);

  const signIn = async (event: FormEvent) => {
    event.preventDefault();
    setBusy(true);
    setError(null);
    setNotice(null);
    try {
      const session = await cloud.login(email.trim(), password, code.trim() || undefined);
      setStatus(session);
      setPassword("");
      setCode("");
      setNotice("Signed in to OpenMindAI Cloud.");
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
          Your session is stored in secure native storage; your password is never saved.
          Cloud unlocks hosted chat models, web search, Research Studio and 3D Studio. Local AI keeps working without an account.
        </p>
        {notice ? <p className="cloud-account-note" role="status">{notice}</p> : null}
        {error ? <p className="setup-warning" role="alert">{error}</p> : null}
        <button type="button" className="cloud-account-disconnect" disabled={busy} onClick={() => void disconnect()}>
          {busy ? "Working…" : "Disconnect"}
        </button>
      </div>
    );
  }

  return (
    <div className="cloud-account">
      <p className="muted">
        Sign in to OpenMindAI Cloud to use hosted models, web search, Research Studio and 3D Studio.
        Local AI keeps working without an account.
      </p>
      {status?.message ? <p className="cloud-account-note" role="status">{status.message}</p> : null}
      <form className="cloud-account-form" onSubmit={signIn}>
        <label className="setup-field-label">
          Email
          <input type="email" autoComplete="username" value={email} onChange={(event) => setEmail(event.target.value)} required />
        </label>
        <label className="setup-field-label">
          Password
          <input type="password" autoComplete="current-password" value={password} onChange={(event) => setPassword(event.target.value)} required />
        </label>
        <label className="setup-field-label">
          Verification code (if required)
          <input inputMode="numeric" autoComplete="one-time-code" value={code} onChange={(event) => setCode(event.target.value)} placeholder="6-digit code" />
        </label>
        {error ? <p className="setup-warning" role="alert">{error}</p> : null}
        {notice ? <p className="cloud-account-note" role="status">{notice}</p> : null}
        <button type="submit" className="primary-button" disabled={busy}>
          {busy ? "Signing in…" : "Sign in to Cloud"}
        </button>
      </form>
      <p className="muted cloud-account-register">
        Don&apos;t have an account yet? Create one at <strong>cloud.openmindai.org</strong>.
        In-app registration and email verification are coming soon.
      </p>
    </div>
  );
}
