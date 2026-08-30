import { useEffect, useState } from "react";
import { Cpu, HardDrive, ShieldCheck, Smartphone } from "lucide-react";
import type { HardwareProfile, PortableRootInfo } from "../types";

const MOBILE_ONBOARDING_KEY = "openmindai.mobileOnboardingComplete.v1";

export function MobileSetupWizard(props: {
  hardware?: HardwareProfile | null;
  root?: PortableRootInfo | null;
  onDismiss?: () => void;
}) {
  const [alreadyCompleted] = useState(
    () => window.localStorage.getItem(MOBILE_ONBOARDING_KEY) === "true",
  );

  useEffect(() => {
    if (alreadyCompleted) props.onDismiss?.();
  }, [alreadyCompleted, props]);

  const continueToApp = () => {
    window.localStorage.setItem(MOBILE_ONBOARDING_KEY, "true");
    props.onDismiss?.();
  };

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
            OpenMindAI stores mobile conversations and app data inside Android's private app
            storage. Desktop Full PC and terminal permissions are never enabled on mobile.
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
                <strong>On-device AI runtime</strong>
                <small>
                  {props.hardware
                    ? `${props.hardware.cpu.name} detected. Mobile model compatibility is checked separately.`
                    : "Hardware detection will recommend a compatible mobile model."}
                </small>
              </span>
            </div>
          </div>

          <div className="setup-actions mobile-setup-actions">
            <button type="button" className="primary-button" onClick={continueToApp}>
              Continue to OpenMindAI
            </button>
          </div>
          <p className="mobile-setup-note">
            On-device model execution is enabled only when the native mobile runtime reports that
            the device is compatible. Connected-app and local workspace data remain isolated from
            desktop-only permissions.
          </p>
        </div>
      </div>
    </div>
  );
}
