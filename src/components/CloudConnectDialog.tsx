import { X } from "lucide-react";
import type { CloudConnection } from "../cloudApi";
import { CloudAccountSection } from "./CloudAccountSection";

export function CloudConnectDialog(props: {
  reason: string;
  onClose: () => void;
  onConnectionChange: (connection: CloudConnection) => void;
}) {
  return (
    <div className="modal-overlay cloud-connect-overlay" role="presentation" onClick={props.onClose}>
      <section className="modal-card cloud-connect-dialog" role="dialog" aria-modal="true" aria-labelledby="cloud-connect-title" onClick={(event) => event.stopPropagation()}>
        <button type="button" className="cloud-connect-close" aria-label="Close Cloud connection dialog" onClick={props.onClose}><X size={18} /></button>
        <h2 id="cloud-connect-title">Connect OpenMindAI Cloud</h2>
        <p className="muted" role="status">{props.reason}</p>
        <CloudAccountSection onConnectionChange={props.onConnectionChange} />
      </section>
    </div>
  );
}
