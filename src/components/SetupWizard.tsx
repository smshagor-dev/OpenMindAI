import type {
  HardwareProfile,
  ModelRecord,
  PendingSetup,
  PortableRootInfo,
  RuntimeInventory,
} from "../types";
import { isLikelyNativeMobile } from "../lib/platform";
import { MobileSetupWizard } from "./MobileSetupWizard";
import { SetupWizard as DesktopSetupWizard } from "./SetupWizardDesktop";

export function SetupWizard(props: {
  mode: "full" | "preparing";
  hardware?: HardwareProfile | null;
  models?: ModelRecord[];
  runtime?: RuntimeInventory | null;
  root?: PortableRootInfo | null;
  pending?: PendingSetup | null;
  refresh?: () => Promise<void>;
  onDismiss?: () => void;
}) {
  if (isLikelyNativeMobile()) {
    return (
      <MobileSetupWizard
        hardware={props.hardware ?? null}
        root={props.root ?? null}
        onDismiss={props.onDismiss}
      />
    );
  }

  return <DesktopSetupWizard {...props} />;
}
