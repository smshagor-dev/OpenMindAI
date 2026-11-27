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
  // Backend and imported metadata are not guaranteed to respect TypeScript types.
  if (typeof iso !== "string" || !iso.trim()) return "";
  try {
    const date = new Date(iso);
    if (Number.isNaN(date.getTime())) return "";
    return date.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
  } catch {
    // A malformed timestamp must not take down the chat/history view.
    return "";
  }
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
  // Error reporting must never replace the original failure with another throw.
  // Unknown values can contain throwing accessors, Proxies, or toString methods.
  try {
    if (caught instanceof Error) {
      try {
        const message = caught.message;
        if (typeof message === "string") return message;
      } catch {
        // Fall through to the other safe representations.
      }
    }
    if (typeof caught === "string") return caught;
    if (caught && typeof caught === "object") {
      const record = caught as Record<string, unknown>;
      for (const key of ["message", "error", "reason"]) {
        try {
          const message = record[key];
          if (typeof message === "string") return message;
        } catch {
          // A hostile field must not prevent reading another useful field.
        }
      }
      try {
        const serialized = JSON.stringify(caught);
        if (typeof serialized === "string") return serialized;
      } catch {
        // Cyclic objects and hostile serialization are expected here.
      }
      return "Unknown application error";
    }
    return String(caught);
  } catch {
    // Includes proxies with throwing prototype traps and hostile coercion.
    return "Unknown application error";
  }
}
