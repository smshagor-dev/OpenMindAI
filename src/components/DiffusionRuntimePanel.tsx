import { useCallback, useEffect, useState } from "react";
import { Download, LoaderCircle, RefreshCw } from "lucide-react";
import { api } from "../api";
import type { DiffusionRuntimeStatus } from "../types";
import { formatBytes, formatError } from "../lib/format";

const BUSY_STATES = new Set(["resolving", "downloading", "verifying", "extracting"]);

export function DiffusionRuntimePanel() {
  const [status, setStatus] = useState<DiffusionRuntimeStatus | null>(null);
  const [installing, setInstalling] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const check = useCallback(async () => {
    try {
      setStatus(await api.diffusionRuntimeStatus());
    } catch (caught) {
      setError(formatError(caught));
    }
  }, []);

  useEffect(() => {
    void check();
  }, [check]);

  useEffect(() => {
    if (!installing && !BUSY_STATES.has(status?.state ?? "")) return;
    const interval = window.setInterval(() => void check(), 900);
    return () => window.clearInterval(interval);
  }, [installing, status?.state, check]);

  const install = async () => {
    if (installing) return;
    setError(null);
    setInstalling(true);
    try {
      setStatus(await api.installDiffusionRuntime());
    } catch (caught) {
      setError(formatError(caught));
    } finally {
      setInstalling(false);
      void check();
    }
  };

  const busy = installing || BUSY_STATES.has(status?.state ?? "");
  const installed = status?.state === "ready";
  const unavailable = status?.state === "unsupported" || status?.state === "browserPreview";

  return (
    <div className="sub-panel">
      <h4>Image generation runtime</h4>
      <p className="muted">
        SDXL image generation uses stable-diffusion.cpp. It has its own runtime and does not
        require the llama.cpp chat server. The SDXL model must be downloaded separately under Models.
      </p>
      <InfoLabel label="Status" value={status?.state ?? "Checking…"} />
      <InfoLabel label="Backend" value={status?.backend ?? "Not selected"} />
      <InfoLabel label="Version" value={status?.version ?? "Not installed"} />
      <p className="muted" role="status" aria-live="polite">{status?.detail}</p>

      {busy ? (
        <div className="setup-progress">
          <progress
            aria-label="Image renderer download progress"
            max={100}
            value={status?.percentage ?? undefined}
          />
          <p className="muted">
            {status?.state ?? "resolving"}
            {status?.percentage != null ? ` · ${Math.round(status.percentage)}%` : ""}
            {status?.totalBytes != null
              ? ` · ${formatBytes(status.downloadedBytes)} / ${formatBytes(status.totalBytes)}`
              : ""}
          </p>
        </div>
      ) : null}

      {error ? <p className="setup-warning" role="alert">{error}</p> : null}
      <div className="button-row">
        <button type="button" onClick={() => void check()} disabled={busy}>
          <RefreshCw size={16} /> Check status
        </button>
        <button type="button" onClick={() => void install()} disabled={busy || unavailable || installed}>
          {busy ? <LoaderCircle className="spin" size={16} /> : <Download size={16} />}
          {busy ? "Installing image renderer…" : "Install / repair image renderer"}
        </button>
      </div>
      <p className="muted">
        A missing or damaged renderer can be reinstalled here. Downloads are verified before use.
        Installation may need internet access. Closing Settings does not cancel an ongoing download.
      </p>
    </div>
  );
}

function InfoLabel({ label, value }: { label: string; value: string }) {
  return <p className="muted"><strong>{label}:</strong> {value}</p>;
}
