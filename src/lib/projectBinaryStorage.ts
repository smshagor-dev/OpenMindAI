import { api } from "../api";

export const MAX_PROJECT_BINARY_FILE_BYTES = 1_500_000;
const BASE64_CHUNK = 16_384;

/** Original bytes, not a filename/path pointer or decoded-text preview. */
export async function attachProjectBinaryFile(projectId: string, file: File) {
  // Reject untrusted metadata before reading or base64-encoding attachment bytes.
  // A Unicode filename may occupy up to four UTF-8 bytes per character.
  if (typeof file.name !== "string" || file.name.length > 255 || !file.name.trim() ||
      new TextEncoder().encode(file.name).byteLength > 255) {
    throw new Error("Binary attachment filename must be nonempty and at most 255 UTF-8 bytes.");
  }
  if (typeof file.type !== "string" || file.type.length > 255 ||
      /[^\x20-\x7e]/.test(file.type)) {
    throw new Error("Binary attachment MIME type must be printable ASCII and at most 255 bytes.");
  }
  if (!Number.isSafeInteger(file.size) || file.size < 0 ||
      file.size > MAX_PROJECT_BINARY_FILE_BYTES) {
    throw new Error("Binary project attachments must have a valid size of 1.5 MB or smaller.");
  }
  const buffer = await file.arrayBuffer();
  // Validate the returned buffer before constructing a view or base64-encoding it.
  // File-like sources may report an incorrect size and return far more bytes.
  if (!(buffer instanceof ArrayBuffer) || buffer.byteLength !== file.size ||
      buffer.byteLength > MAX_PROJECT_BINARY_FILE_BYTES) {
    throw new Error("Project attachment changed during import.");
  }
  const bytes = new Uint8Array(buffer);
  // Use small chunks to avoid JS argument-spread limits for 1.5 MB buffers.
  const segments: string[] = [];
  for (let offset = 0; offset < bytes.length; offset += BASE64_CHUNK) {
    segments.push(String.fromCharCode(...bytes.subarray(offset, offset + BASE64_CHUNK)));
  }
  const dataBase64 = globalThis.btoa(segments.join(""));
  return api.addProjectBinaryFile(
    projectId,
    file.name,
    bytes.byteLength,
    file.type || null,
    dataBase64,
  );
}

/** A requested, verified, project-scoped copy. Never use stored names as paths. */
export async function saveProjectBinaryCopy(projectId: string, fileId: string) {
  const file = await api.readProjectBinaryFile(projectId, fileId);
  if (!file || typeof file !== "object") {
    throw new Error("Saved attachment has invalid metadata.");
  }
  // Bound untrusted encoded data before allocating decoded bytes in the WebView.
  if (!Number.isSafeInteger(file.sizeBytes) || file.sizeBytes < 0 ||
      file.sizeBytes > MAX_PROJECT_BINARY_FILE_BYTES) {
    throw new Error("Saved attachment failed size validation.");
  }
  // Validate IPC metadata before decoding or creating a browser URL.
  // Long legacy names remain compatible: the filename loop below reads
  // at most 255 code points and does not allocate the entire name.
  if (typeof file.dataBase64 !== "string" ||
      typeof file.name !== "string" ||
      (file.mimeType !== null &&
        (typeof file.mimeType !== "string" || file.mimeType.length > 255 ||
          /[^\x20-\x7e]/.test(file.mimeType)))) {
    throw new Error("Saved attachment has invalid metadata.");
  }
  const expectedEncodedLength = 4 * Math.ceil(file.sizeBytes / 3);
  if (file.dataBase64.length !== expectedEncodedLength ||
      !/^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/.test(file.dataBase64)) {
    throw new Error("Saved attachment has invalid base64 encoding.");
  }
  // RFC 4648 padding bits must be zero. Otherwise distinct encoded records can
  // silently decode to the same bytes, hiding corruption in legacy metadata.
  const alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
  const encoded = file.dataBase64;
  const hasTwoPads = encoded.endsWith("==");
  const hasOnePad = !hasTwoPads && encoded.endsWith("=");
  if ((hasTwoPads && (alphabet.indexOf(encoded[encoded.length - 3]) & 15) !== 0) ||
      (hasOnePad && (alphabet.indexOf(encoded[encoded.length - 2]) & 3) !== 0)) {
    throw new Error("Saved attachment has noncanonical base64 padding.");
  }
  const binary = globalThis.atob(file.dataBase64);
  if (binary.length !== file.sizeBytes || binary.length > MAX_PROJECT_BINARY_FILE_BYTES) {
    throw new Error("Saved attachment failed size validation.");
  }
  const bytes = Uint8Array.from(binary, (character) => character.charCodeAt(0));
  const blob = new Blob([bytes], { type: file.mimeType || "application/octet-stream" });
  const url = globalThis.URL.createObjectURL(blob);
  let link: HTMLAnchorElement | undefined;
  let downloadStarted = false;
  try {
    link = document.createElement("a");
    // Names are display labels only. Never interpret stored names as paths.
    const safeName: string[] = [];
    for (const character of file.name) {
      if (safeName.length === 255) break;
      const code = character.charCodeAt(0);
      safeName.push(character === "/" || character === "\\" || code < 32 || code === 127 ||
        code === 0x061c || code === 0x200e || code === 0x200f ||
        (code >= 0x202a && code <= 0x202e) ||
        (code >= 0x2066 && code <= 0x2069) ||
        code === 0x2028 || code === 0x2029
        ? "_"
        : character);
    }
    link.download = safeName.join("") || "project-attachment";
    link.href = url;
    document.body.appendChild(link);
    link.click();
    downloadStarted = true;
  } finally {
    try {
      link?.remove();
    } finally {
      if (downloadStarted) {
        // Allow the browser/WebView to hand off the download before revoking.
        try {
          globalThis.setTimeout(() => globalThis.URL.revokeObjectURL(url), 1_000);
        } catch {
          globalThis.URL.revokeObjectURL(url);
        }
      } else {
        // Failed DOM setup/click must not retain a URL to attachment bytes.
        globalThis.URL.revokeObjectURL(url);
      }
    }
  }
}
