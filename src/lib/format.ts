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

const MAX_ERROR_DISPLAY_UNITS = 2_048;

function boundErrorDisplay(message: string): string {
  if (message.length <= MAX_ERROR_DISPLAY_UNITS) return message;
  let end = MAX_ERROR_DISPLAY_UNITS - 1;
  // Keep the truncation suffix within the limit and avoid lone high surrogates.
  const last = message.charCodeAt(end - 1);
  if (last >= 0xD800 && last <= 0xDBFF) end -= 1;
  return `${message.slice(0, end)}…`;
}

export function formatError(caught: unknown): string {
  return boundErrorDisplay(formatErrorUnbounded(caught));
}

function formatErrorUnbounded(caught: unknown): string {
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
