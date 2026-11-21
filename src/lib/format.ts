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

export function formatError(caught: unknown): string {
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
