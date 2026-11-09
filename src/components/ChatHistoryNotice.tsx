import { useState } from "react";
import { formatError } from "../lib/format";

interface Props {
  onAcknowledge: () => Promise<void>;
  onOpenPrivacy: () => void;
}

export function ChatHistoryNotice({ onAcknowledge, onOpenPrivacy }: Props) {
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function acknowledge() {
    if (saving) return;
    setSaving(true);
    setError(null);
    try {
      await onAcknowledge();
    } catch (caught) {
      setError(formatError(caught));
    } finally {
      setSaving(false);
    }
  }

  return (
    <section className="history-storage-notice" role="alert" aria-label="Chat history storage notice">
      <div>
        <strong>Your previous history setting did not stop saving</strong>
        <p>
          The earlier off setting was not supported. Conversations, attachments and generated
          files are stored on this device. This version does not offer temporary chats, and
          existing saved data has not been removed.
        </p>
        {error ? <p className="history-storage-notice-error">Could not dismiss this notice: {error}</p> : null}
      </div>
      <div className="history-storage-notice-actions">
        <button type="button" onClick={onOpenPrivacy}>Privacy settings</button>
        <button type="button" disabled={saving} onClick={() => void acknowledge()}>
          {saving ? "Saving…" : "I understand"}
        </button>
      </div>
    </section>
  );
}
