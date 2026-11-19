/** Only explicit requests to produce a file trigger automatic document export.
 * Reading, analyzing or summarizing an attached document must never create one.
 */
export function requestedDocumentArtifact(prompt: string): "pdf" | "docx" | null {
  const normalized = prompt.trim().toLowerCase().replace(/^(?:please\s+|(?:can|could|would) you (?:please )?)/, "");
  if (!/^(?:create|make|generate|write|prepare|export|save)\b/.test(normalized)) return null;
  // Require an actual output format, not a topic mentioning PDFs or Word.
  if (/\b(?:as|to|into|in|a|an)\s+(?:a\s+)?(?:formatted\s+)?pdf\b|\bpdf\s+(?:file|document|report)\b/.test(normalized)) return "pdf";
  if (/\b(?:docx|\.docx)\b|\b(?:word|microsoft word)\s+(?:file|document)\b/.test(normalized)) return "docx";
  return null;
}
