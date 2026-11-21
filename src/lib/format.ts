export function formatBytes(value?: number | null) {
  if (value == null) return undefined;
  // Never present corrupted or untrusted byte counts as valid storage usage.
  if (!Number.isFinite(value) || value < 0) return "Unknown size";
  if (value === 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let size = value;
  let index = 0;
  while (size >= 1024 && index < units.length - 1) {
    size /= 1024;
    index += 1;
  }
  return `${size.toFixed(index === 0 ? 0 : 1)} ${units[index]}`;
}

export function formatTime(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "";
  return date.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
}

export function formatError(caught: unknown): string {
  if (caught instanceof Error) return caught.message;
  if (typeof caught === "string") return caught;
  if (caught && typeof caught === "object") {
    const record = caught as Record<string, unknown>;
    const message = record.message ?? record.error ?? record.reason;
    if (typeof message === "string") return message;
    try {
      return JSON.stringify(caught);
    } catch {
      return "Unknown application error";
    }
  }
  return String(caught);
}
