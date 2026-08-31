import { useCallback, useEffect, useState } from "react";
import { Cpu, Download, HardDrive, ShieldCheck, Smartphone } from "lucide-react";
import { api, type MobileModelRecommendation } from "../api";
import type { DownloadStatus, HardwareProfile, PortableRootInfo } from "../types";
import { formatBytes, formatError } from "../lib/format";

const MOBILE_ONBOARDING_KEY = "openmindai.mobileOnboardingComplete.v1";

export function MobileSetupWizard(props: {
  hardware?: HardwareProfile | null;
  root?: PortableRootInfo | null;
  onDismiss?: () => void;
}) {
  const [alreadyCompleted] = useState(
    () => window.localStorage.getItem(MOBILE_ONBOARDING_KEY) === "true",
  );
  const [recommendation, setRecommendation] = useState<MobileModelRecommendation | null>(null);
  const [download, setDownload] = useState<DownloadStatus | null>(null);
  const [loadingRecommendation, setLoadingRecommendation] = useState(true);
  const [installing, setInstalling] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const loadRecommendation = useCallback(async () => {
    setLoadingRecommendation(true);
    try {
      setRecommendation(await api.mobileModelRecommendation());
      setError(null);
    } catch (caught) {
      setError(formatError(caught));
    } finally {
      setLoadingRecommendation(false);
    }
  }, []);

  useEffect(() => {
    if (alreadyCompleted) {
      props.onDismiss?.();
      return;
    }
    void loadRecommendation();
  }, [alreadyCompleted, loadRecommendation, props.onDismiss]);

  useEffect(() => {
    if (!installing) return;
    let cancelled = false;

    const refreshDownload = () => {
      void api
        .modelDownloadStatus()
        .then((status) => {
          if (!cancelled) setDownload(status);
        })
        .catch(() => undefined);
    };

    refreshDownload();
    const timer = window.setInterval(refreshDownload, 750);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [installing]);

  const installRecommendedModel = useCallback(async () => {
    if (!recommendation || installing) return;
    setInstalling(true);
    setDownload(null);
    setError(null);
    try {
      const status = await api.downloadCatalogModel(recommendation.modelId);
      setDownload(status);
      await loadRecommendation();
    } catch (caught) {
      setError(formatError(caught));
    } finally {
      setInstalling(false);
    }
  }, [installing, loadRecommendation, recommendation]);

  const continueToApp = () => {
    window.localStorage.setItem(MOBILE_ONBOARDING_KEY, "true");
    props.onDismiss?.();
  };

  const modelReady = recommendation?.installed ?? false;
  const downloadPercent = download?.percentage;

  return (
    <div className="setup-wizard mobile-native-setup">
      <div className="setup-card mobile-native-setup-card">
        <div className="setup-step mobile-native-setup-step">
          <div className="mobile-setup-mark" aria-hidden="true">
            <img src="/icon.png" alt="" />
          </div>
          <p className="mobile-setup-kicker">OPENMINDAI MOBILE</p>
          <h1>Your private AI workspace, on mobile</h1>
          <p className="setup-lede">
            OpenMindAI stores mobile conversations, models, and app data inside the operating
            system&apos;s private app storage. Desktop Full PC and terminal permissions are never
            enabled on mobile.
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
                <small>{props.root?.root ?? "OS-managed private app storage"}</small>
              </span>
            </div>
            <div>
              <ShieldCheck size={18} />
              <span>
                <strong>Desktop privileges isolated</strong>
                <small>No Full PC access or unrestricted terminal on mobile.</small>
              </span>
            </div>
            <div>
              <Cpu size={18} />
              <span>
                <strong>Device-aware local AI</strong>
                <small>
                  {props.hardware
                    ? `${formatBytes(props.hardware.memory.totalBytes)} RAM detected.`
                    : "Hardware detection is selecting a safe local model tier."}
                </small>
              </span>
            </div>
          </div>

          <div className="mobile-model-recommendation" aria-live="polite">
            <div className="mobile-model-recommendation-heading">
              <Download size={18} />
              <span>
                <strong>Recommended local model</strong>
                <small>Selected for this device&apos;s memory budget.</small>
              </span>
            </div>

            {loadingRecommendation ? <p className="muted">Checking device model tier…</p> : null}

            {recommendation ? (
              <div className="mobile-model-recommendation-body">
                <p>
                  <strong>{recommendation.name}</strong> · {recommendation.quantization} · {formatBytes(recommendation.sizeBytes)}
                </p>
                <p className="muted">{recommendation.reason}</p>
                <p className={modelReady ? "setup-success" : "muted"}>
                  {modelReady
                    ? "Installed and ready for native on-device inference."
                    : "Not installed yet. You can download it now or continue and install it later."}
                </p>
                {downloadPercent != null && installing ? (
                  <p className="muted">Download progress: {Math.round(downloadPercent)}%</p>
                ) : null}
                {!modelReady ? (
                  <button
                    type="button"
                    className="ghost-button"
                    disabled={installing}
                    onClick={() => void installRecommendedModel()}
                  >
                    {installing ? "Installing local model…" : `Download ${recommendation.name}`}
                  </button>
                ) : null}
              </div>
            ) : null}
          </div>

          {error ? <p className="setup-warning">{error}</p> : null}

          <div className="setup-actions mobile-setup-actions">
            <button type="button" className="primary-button" onClick={continueToApp}>
              {modelReady ? "Start Local AI" : "Continue to OpenMindAI"}
            </button>
          </div>
          <p className="mobile-setup-note">
            A local model is optional during onboarding. You can still open OpenMindAI and use
            connected features, then install the recommended model later. Local generation never
            enables desktop-only filesystem or terminal privileges.
          </p>
        </div>
      </div>
    </div>
  );
}
