import { useEffect, useRef } from "react";

export function ConfirmDialog(props: {
  open: boolean;
  title: string;
  description?: string;
  confirmLabel?: string;
  cancelLabel?: string;
  danger?: boolean;
  busy?: boolean;
  error?: string | null;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  const confirmRef = useRef<HTMLButtonElement | null>(null);

  useEffect(() => {
    if (props.open) confirmRef.current?.focus();
  }, [props.open]);

  useEffect(() => {
    if (!props.open) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        if (!props.busy) props.onCancel();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [props]);

  if (!props.open) return null;

  return (
    <div className="modal-overlay" role="presentation" onClick={() => { if (!props.busy) props.onCancel(); }}>
      <div
        className="modal-card"
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="confirm-dialog-title"
        onClick={(event) => event.stopPropagation()}
      >
        <h2 id="confirm-dialog-title">{props.title}</h2>
        {props.description ? <p className="muted">{props.description}</p> : null}
        {props.error ? <p className="setup-warning" role="alert">{props.error}</p> : null}
        <div className="modal-actions">
          <button type="button" disabled={props.busy} className="ghost-button" onClick={() => { if (!props.busy) props.onCancel(); }}>
            {props.cancelLabel ?? "Cancel"}
          </button>
          <button
            type="button"
            disabled={props.busy}
            ref={confirmRef}
            className={props.danger ? "danger-button" : "primary-button"}
            onClick={props.onConfirm}
          >
            {props.confirmLabel ?? "Confirm"}
          </button>
        </div>
      </div>
    </div>
  );
}
