import { useCallback, useEffect, useMemo, useState } from "react";
import { Cpu, Download, HardDrive, ShieldCheck, Smartphone } from "lucide-react";
import { api } from "../api";
import type {
  DownloadStatus,
  HardwareProfile,
  ModelCatalogStatus,
  ModelRecord,
  PortableRootInfo,
} from "../types";
import { formatBytes, formatError } from "../lib/format";

const MOBILE_ONBOARDING_KEY = "openmindai.mobileOnboardingComplete.v2";
const MOBILE_NANO_ID = "qwen3-06b-q4";
const MOBILE_SWIFT_ID = "qwen3-17b-q4km";
const SWIFT_RECOMMENDED_RAM_BYTES = 8 * 1024 * 1024 * 1024;

function isDownloadActive(status: DownloadStatus | null) {
  return Boolean(
    status &&
      ["queued", "resolving", "downloading", "pausedInterrupted", "verifying"].includes(
        status.state,
      ),
  );
}

export function MobileSetupWizard(props: {
  hardware?: HardwareProfile | null;
  models?: ModelRecord[];
  root?: PortableRootInfo | null;
  refresh?: () => Promise<void>;
  onDismiss?: () => void;
}) {
  const [alreadyCompleted] = useState(
    () => window.localStorage.getItem(MOBILE_ONBOARDING_KEY) === "true",
  );
  const [catalog, setCatalog] = useState<ModelCatalogStatus[]>([]);
  const [catalogLoading, setCatalogLoading] = useState(true);
  const [download, setDownload] = useState<DownloadStatus | null>(null);
  const [error, setError] = useState<string | null>(null);

  const recommendedId =
    (props.hardware?.memory.totalBytes ?? 0) >= SWIFT_RECOMMENDED_RAM_BYTES
      ? MOBILE_SWIFT_ID
      : MOBILE_NANO_ID;

  const recommended = useMemo(
    () =>
      catalog.find((item) => item.entry.id === recommendedId) ??
      catalog.find((item) => item.entry.id === MOBILE_NANO_ID) ??
      null,
    [catalog, recommendedId],
  );

  const installedFromRegistry = useMemo(() => {
    if (!recommended) return false;
    return (props.models ?? []).some(
      (model) =>
        model.sourceRepository === recommended.entry.repo &&
        (model.state === "ready" || model.state === "loaded" || model.state === "installed"),
    );
  }, [props.models, recommended]);
  const recommendedInstalled = Boolean(recommended?.installed || installedFromRegistry);

  const refreshCatalog = useCallback(async () => {
    try {
      const report = await api.checkModelUpdates();
      setCatalog(report.entries);
      setError(null);
    } catch (caught) {
      setError(formatError(caught));
    } finally {
      setCatalogLoading(false);
    }
  }, []);

  useEffect(() => {
    if (alreadyCompleted) {
      props.onDismiss?.();
      return;
    }
    void refreshCatalog();
  }, [alreadyCompleted, props.onDismiss, refreshCatalog]);

  useEffect(() => {
    if (!isDownloadActive(download)) return;
    let cancelled = false;
    const timer = window.setInterval(() => {
      void api
        .modelDownloadStatus()
        .then(async (next) => {
          if (cancelled) return;
          setDownload(next);
          if (next.state === "completed") {
            window.clearInterval(timer);
            await props.refresh?.();
            await refreshCatalog();
          } else if (next.state === "failed" || next.state === "cancelled") {
            window.clearInterval(timer);
            if (next.error) setError(next.error);
          }
        })
        .catch((caught) => {
          if (!cancelled) setError(formatError(caught));
        });
    }, 750);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [download, props.refresh, refreshCatalog]);

  const continueToApp = useCallback(() => {
    window.localStorage.setItem(MOBILE_ONBOARDING_KEY, "true");
    props.onDismiss?.();
  }, [props.onDismiss]);

  const downloadRecommended = useCallback(async () => {
    if (!recommended || !recommended.downloadSupported) return;
    setError(null);
    try {
      const next = await api.downloadCatalogModel(recommended.entry.id);
      setDownload(next);
      if (next.state === "completed") {
        await props.refresh?.();
        await refreshCatalog();
      }
    } catch (caught) {
      setError(formatError(caught));
    }
  }, [props.refresh, recommended, refreshCatalog]);

  const downloadBusy = isDownloadActive(download);
  const percentage = download?.percentage != null ? Math.max(0, Math.min(100, download.percentage)) : null;

  return (
    <div className="setup-wizard mobile-native-setup">
      <div className="setup-card mobile-native-setup-card">
        <div className="setup-step mobile-native-setup-step">
          <div className="mobile-setup-mark" aria-hidden="true">
            <img src="/icon.png" alt="" />
          </div>
          <p className="mobile-setup-kicker">OPENMINDAI MOBILE</p>
          <h1>Your private AI workspace, on Android</h1>
          <p className="setup-lede">
            Conversations, models, and app data stay inside Android-managed private storage.
            Desktop Full PC and terminal permissions are never enabled on mobile.
          </p>

          <div className="mobile-setup-features">
            <div>
              <Smartphone size={18} />
              <span>
                <strong>Mobile-first interface</strong>
                <small>Touch navigation, safe areas, projects and connected apps.</small>
              </span>
            </div>
            <div>
              <HardDrive size={18} />
              <span>
                <strong>Private app storage</strong>
                <small>{props.root?.root ?? "Android-managed local storage"}</small>
              </span>
            </div>
            <div>
              <ShieldCheck size={18} />
              <span>
                <strong>Desktop privileges isolated</strong>
                <small>No Full PC access or unrestricted terminal on Android.</small>
              </span>
            </div>
            <div>
              <Cpu size={18} />
              <span>
                <strong>Hardware-aware local AI</strong>
                <small>
                  {props.hardware
                    ? `${formatBytes(props.hardware.memory.totalBytes)} RAM detected; mobile model sizing is applied automatically.`
                    : "Hardware detection selects a conservative mobile model tier."}
                </small>
              </span>
            </div>
          </div>

          <div className="mobile-home-card mobile-setup-model-card">
            <div className="mobile-home-card-icon model-icon">
              <Download size={18} />
            </div>
            <div className="mobile-home-card-copy">
              <small>RECOMMENDED ON THIS DEVICE</small>
              <strong>
                {catalogLoading ? "Checking mobile models…" : recommended?.entry.name ?? "OpenMindAI Nano"}
              </strong>
              <span>
                {recommended
                  ? `${formatBytes(recommended.entry.sizeBytes)} · ${recommended.entry.quantization}`
                  : "Compact on-device chat model"}
              </span>
            </div>
            {recommendedInstalled ? (
              <span className="mobile-ready-pill">
                <i /> Ready
              </span>
            ) : null}
          </div>

          {downloadBusy ? (
            <div className="mobile-setup-download" aria-live="polite">
              <div>
                <strong>Installing {recommended?.entry.name ?? "mobile AI"}</strong>
                <span>{percentage == null ? download?.state : `${percentage.toFixed(0)}%`}</span>
              </div>
              <progress max={100} value={percentage ?? undefined} />
              <small>
                {formatBytes(download?.downloadedBytes)}
                {download?.totalBytes ? ` of ${formatBytes(download.totalBytes)}` : ""}
              </small>
            </div>
          ) : null}

          {error ? <p className="setup-warning">{error}</p> : null}

          <div className="setup-actions mobile-setup-actions">
            {recommendedInstalled ? (
              <button type="button" className="primary-button" onClick={continueToApp}>
                Continue to OpenMindAI
              </button>
            ) : (
              <>
                <button
                  type="button"
                  className="primary-button"
                  disabled={catalogLoading || downloadBusy || !recommended?.downloadSupported}
                  onClick={() => void downloadRecommended()}
                >
                  {downloadBusy ? "Installing…" : `Download ${recommended?.entry.name ?? "mobile AI"}`}
                </button>
                <button
                  type="button"
                  className="ghost-button"
                  disabled={downloadBusy}
                  onClick={continueToApp}
                >
                  Continue without local model
                </button>
              </>
            )}
          </div>
          <p className="mobile-setup-note">
            OpenMindAI Nano is the conservative default for lower-memory phones. Swift is
            recommended only when the device reports at least 8 GB RAM. Larger desktop models are
            never forced during Android setup.
          </p>
        </div>
      </div>
    </div>
  );
}
