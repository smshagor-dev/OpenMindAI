/** Resolve a Lens image MIME without trusting an extension over a declared type. */
export function visionMimeType(file: Pick<File, "name" | "type">): "image/png" | "image/jpeg" | "image/webp" | null {
  const declared = file.type.trim().toLowerCase();
  if (declared === "image/png" || declared === "image/jpeg" || declared === "image/webp") return declared;
  // Only missing or generic MIME types may be inferred from the filename.
  if (declared && declared !== "application/octet-stream") return null;
  if (/\.png$/i.test(file.name)) return "image/png";
  if (/\.jpe?g$/i.test(file.name)) return "image/jpeg";
  if (/\.webp$/i.test(file.name)) return "image/webp";
  return null;
}
