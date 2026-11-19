/** Identify a serialized attachment block without hiding literal user text on malformed markers. */
const ATTACHMENT_HEADER = /^\[Attachment:\s*(.+?),\s*(text|image|pdf|audio|video|binary),\s*[^,\]\r\n]+,\s*[^\]\r\n]+\]$/;

export function parseDisplayedAttachmentSection(visible: string): {
  start: number;
  names: string[];
} {
  let offset = 0;
  while (offset < visible.length) {
    const start = visible.startsWith("[Attachment:", offset)
      ? offset
      : visible.indexOf("\n\n[Attachment:", offset);
    if (start < 0) break;
    const headerStart = visible.startsWith("[Attachment:", start) ? start : start + 2;
    const end = visible.indexOf("\n", headerStart);
    const firstLine = visible.slice(headerStart, end < 0 ? undefined : end).replace(/\r$/, "");
    if (ATTACHMENT_HEADER.test(firstLine)) {
      const payload = visible.slice(headerStart);
      const names = Array.from(
        payload.matchAll(/^\[Attachment:\s*(.+?),\s*(?:text|image|pdf|audio|video|binary),\s*[^,\]\r\n]+,\s*[^\]\r\n]+\]$/gm),
        (match) => match[1].trim(),
      );
      return { start, names };
    }
    offset = headerStart + "[Attachment:".length;
  }
  return { start: -1, names: [] };
}
