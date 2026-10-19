const IMAGE_NOUNS =
  "image|photo|photograph|picture|artwork|graphic|poster|logo|thumbnail|illustration|wallpaper|banner|icon";

const IMAGE_ACTION_INTENT = new RegExp(
  `\\b(?:create|make|generate|design|render|draw|paint|illustrate|sketch)\\s+(?:(?:[a-z-]+\\s+){0,7})(?:${IMAGE_NOUNS})\\b`,
  "i",
);
const IMAGE_NOUN_INTENT =
  /^(?:(?:an?|the|my)\s+)?(?:poster|logo|thumbnail|illustration|wallpaper|banner)\s+(?:for|of|showing|with)\b/i;
const FREEFORM_DRAW_INTENT =
  /\b(?:draw|sketch|paint|illustrate)\s+(?!(?:(?:me|us|an?|the|my|some)\s+){0,2}(?:conclusion|attention|parallel|inference|comparison)\b)(?:(?:me|us|an?|the|my|some)\s+){0,2}[a-z][a-z -]{1,80}/i;
const BANGLA_IMAGE_INTENT =
  /(?=.*(?:ছবি|ইমেজ|ফটো|চিত্র|পোস্টার|লোগো|আইকন|থাম্বনেইল|আঁকো|আঁকুন))(?=.*(?:তৈরি|বানাও|বানিয়ে|বানিয়ে|করো|করুন|আঁকো|আঁকুন|দাও|দেখাও|জেনারেট))/u;
const RUSSIAN_IMAGE_INTENT =
  /(?:создай|создайте|сгенерируй|сгенерируйте|нарисуй|нарисуйте|изобрази|изобразите|сделай|сделайте)\s+(?:(?:мне|пожалуйста|красивую|красивый|новую|новый|одну)\s+){0,4}(?:изображени[ея]|картинк[уаие]|фото|рисунок|логотип|постер|баннер|кот[а-я]*)/iu;
const IMAGE_REFUSAL =
  /(?:\bi(?:'m| am)?\s+sorry\b|\b(?:cannot|can't|unable to)\b[^.\n]{0,80}\b(?:generate|create|make|render)\b[^.\n]{0,40}\b(?:image|photo|picture|art)\b|\b(?:text[- ]only|do not have the ability)\b)/i;

/** The image renderer works independently of the chat model. */
export const DIRECT_IMAGE_MESSAGE = "Image request submitted to the local renderer. See the image artifact below.";

export function isImageGenerationIntent(prompt: string) {
  const normalized = prompt.trim().normalize("NFKC");
  return (
    IMAGE_ACTION_INTENT.test(normalized) ||
    IMAGE_NOUN_INTENT.test(normalized) ||
    FREEFORM_DRAW_INTENT.test(normalized) ||
    BANGLA_IMAGE_INTENT.test(normalized) ||
    RUSSIAN_IMAGE_INTENT.test(normalized)
  );
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
  if (!enhanced || enhanced === DIRECT_IMAGE_MESSAGE || IMAGE_REFUSAL.test(enhanced)) return original;
  return enhanced;
}

/** Persist an image request without sending it to a llama.cpp text model. */
export async function persistDirectImageRequest<T extends { id: string }>(
  conversationId: string,
  userContent: string,
  persistence: {
    addUserMessage: (id: string, content: string) => Promise<unknown>;
    createStreamingAssistantMessage: (id: string) => Promise<T>;
    appendMessageChunk: (id: string, chunk: string) => Promise<unknown>;
    completeMessage: (id: string, status: "completed" | "failed") => Promise<unknown>;
  },
): Promise<T> {
  await persistence.addUserMessage(conversationId, userContent);
  const assistant = await persistence.createStreamingAssistantMessage(conversationId);
  try {
    await persistence.appendMessageChunk(assistant.id, DIRECT_IMAGE_MESSAGE);
    await persistence.completeMessage(assistant.id, "completed");
  } catch (error) {
    await persistence.completeMessage(assistant.id, "failed").catch(() => undefined);
    throw error;
  }
  return assistant;
}
