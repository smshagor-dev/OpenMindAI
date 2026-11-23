export function formatBytes(value?: number | null) {
  if (!value) return value === 0 ? "0 B" : undefined;
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

const sanitizeErrorDisplay = (value: string): string =>
  value.replace(/[\u0000-\u0008\u000B\u000C\u000E-\u001F\u007F-\u009F\u202A-\u202E\u2066-\u2069]/g, "");

export function formatError(caught: unknown): string {
  if (caught instanceof Error) return sanitizeErrorDisplay(caught.message);
  if (typeof caught === "string") return sanitizeErrorDisplay(caught);
  if (caught && typeof caught === "object") {
    const record = caught as Record<string, unknown>;
    const message = record.message ?? record.error ?? record.reason;
    if (typeof message === "string") return sanitizeErrorDisplay(message);
    try {
      return sanitizeErrorDisplay(JSON.stringify(caught));
    } catch {
      return "Unknown application error";
    }
  }
  return sanitizeErrorDisplay(String(caught));
}
