const IMAGE_NOUNS =
  "image|photo|photograph|picture|artwork|graphic|poster|logo|thumbnail|illustration|wallpaper|banner|icon";

const IMAGE_ACTION_INTENT = new RegExp(
  `\\b(?:create|make|generate|design|render|draw|paint|illustrate)\\s+(?:(?:an?|the|me|my|some|new)\\s+){0,2}(?:${IMAGE_NOUNS})\\b`,
  "i",
);
const IMAGE_NOUN_INTENT = new RegExp(
  `\\b(?:poster|logo|thumbnail|illustration|wallpaper|banner)\\b`,
  "i",
);
const IMAGE_REFUSAL =
  /(?:\bi(?:'m| am)?\s+sorry\b|\b(?:cannot|can't|unable to)\b[^.\n]{0,80}\b(?:generate|create|make|render)\b[^.\n]{0,40}\b(?:image|photo|picture|art)\b|\b(?:text[- ]only|do not have the ability)\b)/i;

export function isImageGenerationIntent(prompt: string) {
  const normalized = prompt.trim();
  return IMAGE_ACTION_INTENT.test(normalized) || IMAGE_NOUN_INTENT.test(normalized);
}

export function plainImageRequest(persistedUserContent: string) {
  const content = persistedUserContent.trim();
  if (!content.startsWith("[Mode: Image Creation]")) return content;
  const separator = content.indexOf("\n\n");
  return separator >= 0 ? content.slice(separator + 2).trim() : "";
}

export function imageRendererPrompt(assistantContent: string, persistedUserContent: string) {
  const enhanced = assistantContent.trim();
  const original = plainImageRequest(persistedUserContent);
  if (!enhanced || IMAGE_REFUSAL.test(enhanced)) return original;
  return enhanced;
}
