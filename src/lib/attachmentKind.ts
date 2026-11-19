/** Classify supported media using browser-provided MIME before filename hints. */
export type AttachmentMediaKind = "image" | "pdf" | "audio" | "video" | "text" | "binary";

export function classifyAttachmentKind(file: Pick<File, "name" | "type">): AttachmentMediaKind {
  const mime = file.type.toLowerCase().trim();
  // Explicit supported types take precedence over misleading file extensions.
  if (mime.startsWith("image/")) return "image";
  if (mime === "application/pdf") return "pdf";
  if (mime.startsWith("audio/")) return "audio";
  if (mime.startsWith("video/")) return "video";
  if (mime.startsWith("text/")) return "text";

  // Preserve existing extension-based classification for missing, generic,
  // and unsupported MIME values. Downstream decoders validate actual content.
  if (/\.(png|jpe?g|webp)$/i.test(file.name)) return "image";
  if (/\.pdf$/i.test(file.name)) return "pdf";
  if (/\.(wav|mp3|m4a|aac|flac|ogg|opus)$/i.test(file.name)) return "audio";
  if (/\.(mp4|webm|mov|m4v|mpeg|mpg)$/i.test(file.name)) return "video";
  if (/\.(md|txt|json|csv|ts|tsx|js|jsx|rs|py|html|css|sql|toml|yaml|yml)$/i.test(file.name)) return "text";
  return "binary";
}
