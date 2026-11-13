import { api } from "../api";

export const MAX_PROJECT_BINARY_FILE_BYTES = 1_500_000;
const BASE64_CHUNK = 16_384;

/** Original bytes, not a filename/path pointer or decoded-text preview. */
export async function attachProjectBinaryFile(projectId: string, file: File) {
  if (file.size > MAX_PROJECT_BINARY_FILE_BYTES) {
    throw new Error("Binary project attachments must be 1.5 MB or smaller.");
  }
  const bytes = new Uint8Array(await file.arrayBuffer());
  if (bytes.byteLength !== file.size) {
    throw new Error("Project attachment changed during import.");
  }
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
  const binary = globalThis.atob(file.dataBase64);
  if (binary.length !== file.sizeBytes || binary.length > MAX_PROJECT_BINARY_FILE_BYTES) {
    throw new Error("Saved attachment failed size validation.");
  }
  const bytes = Uint8Array.from(binary, (character) => character.charCodeAt(0));
  const blob = new Blob([bytes], { type: file.mimeType || "application/octet-stream" });
  const url = globalThis.URL.createObjectURL(blob);
  const link = document.createElement("a");
  // Names are display labels only. This prevents an older corrupted record
  // from inserting path segments into a browser's suggested download name.
  link.download = [...file.name].map((character) => {
    const code = character.charCodeAt(0);
    return character === "/" || character === "\\" || code < 32 || code === 127
      ? "_"
      : character;
  }).join("") || "project-attachment";
  link.href = url;
  try {
    document.body.appendChild(link);
    link.click();
  } finally {
    link.remove();
    // Delay revocation so the browser/WebView has time to hand off the copy.
    globalThis.setTimeout(() => globalThis.URL.revokeObjectURL(url), 1_000);
  }
}
