import type { Message } from "../types";
import { formatBytes } from "./format";
import { analyzeVideoFile, transcribeAudioFile } from "./media";
import { extractPdfText, type PdfExtractionOptions, type PdfExtractionProgress } from "./pdf";
import { isImageGenerationIntent } from "./imageGeneration";
import {
  MAX_SINGLE_ATTACHMENT_CONTEXT_CHARS,
  planAttachmentContext,
  serializeAttachmentContext,
  type AttachmentContextSource,
} from "./attachmentContext";

export type ChatMode =
  | "chat"
  | "search"
  | "research"
  | "thinking"
  | "document"
  | "pdf"
  | "image"
  | "video"
  | "voice"
  | "sound"
  | "vision";

export interface InferenceMediaDraft {
  kind: "image";
  name: string;
  mimeType: "image/png" | "image/jpeg";
  dataUrl: string;
}

export interface AttachmentDraft {
  id: string;
  name: string;
  size: number;
  type: string;
  kind: "text" | "image" | "pdf" | "audio" | "video" | "binary";
  warning?: string;
  contentPreview: string | null;
  mediaDataUrl: string | null;
  mediaMimeType: "image/png" | "image/jpeg" | null;
  mediaItems: InferenceMediaDraft[];
}

export interface UserMessageDisplay {
  prompt: string;
  attachmentNames: string[];
  displayText: string;
}

const MAX_TEXT_ATTACHMENT_BYTES = 1024 * 1024;
const MAX_VISION_IMAGE_INPUT_BYTES = 16 * 1024 * 1024;
const MAX_VISION_IMAGE_DATA_URL_CHARS = 6_000_000;
const MAX_VISION_IMAGE_DIMENSION = 2048;
const MAX_INFERENCE_MEDIA_ITEMS = 4;
const SUPPORTED_VISION_IMAGE_TYPES = new Set(["image/jpeg", "image/png", "image/webp"]);
const CHAT_MODES: ChatMode[] = [
  "search",
  "research",
  "thinking",
  "document",
  "pdf",
  "image",
  "video",
  "voice",
  "sound",
  "vision",
];

function visionMimeType(file: File) {
  const declared = file.type.toLowerCase();
  if (SUPPORTED_VISION_IMAGE_TYPES.has(declared)) return declared;
  if (/\.png$/i.test(file.name)) return "image/png";
  if (/\.jpe?g$/i.test(file.name)) return "image/jpeg";
  if (/\.webp$/i.test(file.name)) return "image/webp";
  return null;
}

async function encodeVisionImage(file: File) {
  const mimeType = visionMimeType(file);
  if (!mimeType) {
    throw new Error("OpenMindAI Lens currently accepts PNG, JPEG, and WebP images.");
  }
  if (file.size > MAX_VISION_IMAGE_INPUT_BYTES) {
    throw new Error("Image is too large for local vision. Use an image smaller than 16 MB.");
  }

  const objectUrl = window.URL.createObjectURL(file);
  try {
    const image = document.createElement("img");
    await new Promise<void>((resolve, reject) => {
      image.onload = () => resolve();
      image.onerror = () => reject(new Error("Could not decode the selected image."));
      image.src = objectUrl;
    });

    const scale = Math.min(
      1,
      MAX_VISION_IMAGE_DIMENSION / Math.max(image.naturalWidth, image.naturalHeight, 1),
    );
    const width = Math.max(1, Math.round(image.naturalWidth * scale));
    const height = Math.max(1, Math.round(image.naturalHeight * scale));
    const canvas = document.createElement("canvas");
    canvas.width = width;
    canvas.height = height;
    const context = canvas.getContext("2d");
    if (!context) throw new Error("Could not prepare the image for local vision.");
    context.drawImage(image, 0, 0, width, height);

    let mediaMimeType: "image/png" | "image/jpeg" =
      mimeType === "image/png" ? "image/png" : "image/jpeg";
    let dataUrl =
      mediaMimeType === "image/png"
        ? canvas.toDataURL("image/png")
        : canvas.toDataURL("image/jpeg", 0.88);

    if (dataUrl.length > MAX_VISION_IMAGE_DATA_URL_CHARS) {
      const flattened = document.createElement("canvas");
      flattened.width = width;
      flattened.height = height;
      const flattenedContext = flattened.getContext("2d");
      if (!flattenedContext) throw new Error("Could not compress the image for local vision.");
      flattenedContext.fillStyle = "#ffffff";
      flattenedContext.fillRect(0, 0, width, height);
      flattenedContext.drawImage(canvas, 0, 0);
      mediaMimeType = "image/jpeg";
      dataUrl = flattened.toDataURL("image/jpeg", 0.82);
    }

    if (dataUrl.length > MAX_VISION_IMAGE_DATA_URL_CHARS) {
      throw new Error("Image remains too large after local optimization. Resize it and try again.");
    }

    return { dataUrl, mimeType: mediaMimeType };
  } finally {
    window.URL.revokeObjectURL(objectUrl);
  }
}

export function settledValue<T>(result: PromiseSettledResult<unknown>): T | null {
  return result.status === "fulfilled" ? (result.value as T) : null;
}

export function mergeStreamingSnapshot(current: Message[], snapshot: Message[]) {
  const currentById = new Map(current.map((message) => [message.id, message]));
  return snapshot.map((next) => {
    const existing = currentById.get(next.id);
    if (!existing) return next;
    const content =
      existing.status === "streaming" && existing.content.length > next.content.length
        ? existing.content
        : next.content;
    return { ...next, content };
  });
}

export interface AttachmentReadOptions {
  signal?: PdfExtractionOptions["signal"];
  onPdfProgress?: (progress: PdfExtractionProgress) => void;
}

export async function readAttachment(
  file: File,
  options: AttachmentReadOptions = {},
): Promise<AttachmentDraft> {
  const kind = attachmentKind(file);
  const isTextLike =
    file.type.startsWith("text/") ||
    /\.(md|txt|json|csv|ts|tsx|js|jsx|rs|py|html|css|sql|toml|yaml|yml)$/i.test(file.name);

  if (kind === "binary") {
    throw new Error(
      "This file type cannot be read by local chat. Attach text/code, PDF, image, audio, or video files.",
    );
  }
  if (isTextLike && file.size > MAX_TEXT_ATTACHMENT_BYTES) {
    throw new Error("Text and code attachments must be 1 MB or smaller for local chat context.");
  }

  let warning: string | undefined;
  let contentPreview: string | null = null;
  let mediaDataUrl: string | null = null;
  let mediaMimeType: "image/png" | "image/jpeg" | null = null;
  let mediaItems: InferenceMediaDraft[] = [];

  if (isTextLike) {
    // Retain the full bounded source until the user's query is known. Selecting
    // only the first 8,000 characters here permanently loses deep evidence.
    contentPreview = await file.text();
    if (contentPreview.length > MAX_SINGLE_ATTACHMENT_CONTEXT_CHARS) {
      warning = "Long text file: OpenMindAI will send selected excerpts (8,000 characters per file, 16,000 total). Unselected text is not analyzed.";
    }
  } else if (kind === "image") {
    const encoded = await encodeVisionImage(file);
    mediaDataUrl = encoded.dataUrl;
    mediaMimeType = encoded.mimeType;
    mediaItems = [
      {
        kind: "image",
        name: file.name,
        mimeType: encoded.mimeType,
        dataUrl: encoded.dataUrl,
      },
    ];
    contentPreview = `[Image attached: ${file.name}. The actual optimized image bytes are supplied to OpenMindAI Lens for this request.]`;
  } else if (kind === "pdf") {
    const extracted = await extractPdfText(file, {
      signal: options.signal,
      onProgress: options.onPdfProgress,
    });
    mediaItems = extracted.visionPages.map((page) => ({
      kind: "image",
      name: `${file.name} - page ${page.pageNumber}`,
      mimeType: page.mimeType,
      dataUrl: page.dataUrl,
    }));
    const status = [
      `${extracted.pageCount} page${extracted.pageCount === 1 ? "" : "s"}`,
      `${extracted.pagesRead} processed`,
      extracted.scannedPages.length
        ? `${extracted.scannedPages.length} image/scanned page(s) detected`
        : null,
      extracted.visionPages.length
        ? `${extracted.visionPages.length} representative page(s) sent to Lens`
        : null,
      extracted.truncated ? "text or scanned-page coverage limited for local chat context" : null,
    ]
      .filter(Boolean)
      .join(", ");
    const body = extracted.text
      ? extracted.text
      : "[No reliable selectable text was found. Use the supplied rendered PDF page images for visual/OCR analysis.]";
    contentPreview = `[${extracted.unreadPages.length ? "PDF text-only" : "PDF processed locally"}: ${status}. Page markers below correspond to the original PDF.]\n${extracted.warnings.join("\n")}\n${body}`;
    warning = [
      ...extracted.warnings,
      ...(extracted.truncated
        ? ["PDF text extraction was truncated before chat-context excerpt selection."]
        : []),
    ].join(" ") || undefined;
  } else if (kind === "audio") {
    const transcript = await transcribeAudioFile(file);
    const language = transcript.language ? `, language ${transcript.language}` : "";
    contentPreview = `[Audio transcribed locally with OpenMindAI Hear: ${transcript.durationSeconds.toFixed(1)} seconds${language}]\n\n${transcript.text}`;
  } else if (kind === "video") {
    const analysis = await analyzeVideoFile(file);
    mediaItems = analysis.frames;
    const transcript = analysis.transcript?.text
      ? `\n\n--- Audio transcript ---\n${analysis.transcript.text}`
      : "\n\n[No local audio transcript was available; analyze the supplied representative video frames.]";
    contentPreview = `[Video processed locally: ${analysis.durationSeconds.toFixed(1)} seconds, ${analysis.frames.length} representative frame(s) sent to OpenMindAI Lens.]${transcript}`;
  }

  return {
    id: crypto.randomUUID(),
    name: file.name,
    size: file.size,
    type: file.type || "unknown",
    kind,
    contentPreview,
    warning,
    mediaDataUrl,
    mediaMimeType,
    mediaItems,
  };
}

export function attachmentMedia(attachments: AttachmentDraft[]): InferenceMediaDraft[] {
  return attachments
    .flatMap((attachment) => attachment.mediaItems)
    .slice(0, MAX_INFERENCE_MEDIA_ITEMS);
}

function attachmentContextSources(attachments: AttachmentDraft[]): AttachmentContextSource[] {
  return attachments.map((attachment) => {
    // Prevent file metadata from injecting fake attachment headers or prompt lines.
    const safeName = attachment.name.replace(/[\r\n,]/g, " ").split("[").join(" ").split("]").join(" ").trim().slice(0, 160) || "unnamed";
    const safeType = attachment.type.replace(/[^a-zA-Z0-9.+/-]/g, "").slice(0, 80);
    return {
      name: attachment.name,
      header: `[Attachment: ${safeName}, ${attachment.kind}, ${formatBytes(attachment.size)}, ${safeType}]`,
      content: attachment.contentPreview,
    };
  });
}

export function attachmentContextStats(attachments: AttachmentDraft[]) {
  return planAttachmentContext(attachmentContextSources(attachments));
}

export function buildMessageContent(
  prompt: string,
  attachments: AttachmentDraft[],
  mode: ChatMode,
) {
  const modePrefix = modeInstruction(mode);
  if (attachments.length === 0) return [modePrefix, prompt].filter(Boolean).join("\n\n");

  const { text: attachmentText } = serializeAttachmentContext(
    attachmentContextSources(attachments),
    prompt,
  );
  return [modePrefix, prompt, attachmentText].filter(Boolean).join("\n\n");
}

/**
 * Converts the persisted inference payload back into what the user actually
 * typed. Mode instructions and extracted attachment text are implementation
 * context for the local model; they must never be exposed by chat bubbles,
 * copy actions, or edit/resend.
 */
export function userMessageDisplay(content: string): UserMessageDisplay {
  let visible = content.trim();

  for (const mode of CHAT_MODES) {
    const instruction = modeInstruction(mode);
    if (!instruction) continue;
    if (visible === instruction) {
      visible = "";
      break;
    }
    const prefix = `${instruction}\n\n`;
    if (visible.startsWith(prefix)) {
      visible = visible.slice(prefix.length).trimStart();
      break;
    }
  }

  const attachmentStart = visible.startsWith("[Attachment:")
    ? 0
    : visible.indexOf("\n\n[Attachment:");
  const attachmentPayload = attachmentStart >= 0 ? visible.slice(attachmentStart).trim() : "";
  const prompt = (attachmentStart >= 0 ? visible.slice(0, attachmentStart) : visible).trim();
  const attachmentNames = Array.from(
    attachmentPayload.matchAll(
      /^\[Attachment:\s*(.+?),\s*(?:text|image|pdf|audio|video|binary),\s*[^,\]]+,\s*[^\]]+\]$/gm,
    ),
    (match) => match[1].trim(),
  );
  const attachmentSummary = attachmentNames.length
    ? `Attached: ${attachmentNames.join(", ")}`
    : attachmentPayload
      ? "Attached file"
      : "";

  return {
    prompt,
    attachmentNames,
    displayText: [prompt, attachmentSummary].filter(Boolean).join("\n\n"),
  };
}

// Only explicit output commands outrank attachment analysis and topic keywords.
// Keep explanatory/search prose out of this fast path.
function explicitMediaMode(text: string): ChatMode | null {
  const command = text.replace(/^(?:please\s+|(?:can|could|would) you (?:please )?)/, "");
  const action = /^(create|make|generate|design|render|draw|paint|illustrate|sketch|animate)\b\s*(.*)/.exec(command);
  if (action) {
    const object = /\b(image|photo|photograph|picture|artwork|graphic|poster|logo|thumbnail|illustration|wallpaper|banner|icon|video|animation|voiceover|voice|narration|speech|tts|music|sound|sfx|soundscape|ambience|pdf|document|report|proposal|resume|cv|letter|contract|outline|function|script|code|tutorial|guide|explanation|comparison|summary)\b/.exec(action[2]);
    if (object && action[2].slice(0, object.index).trim().split(/\s+/).length <= 7) {
      const noun = object[1];
      if (/^(video|animation)$/.test(noun)) return "video";
      if (/^(voiceover|voice|narration|speech|tts)$/.test(noun)) return "voice";
      if (/^(music|sound|sfx|soundscape|ambience)$/.test(noun)) return "sound";
      if (/^(image|photo|photograph|picture|artwork|graphic|poster|logo|thumbnail|illustration|wallpaper|banner|icon)$/.test(noun) && isImageGenerationIntent(command)) return "image";
      return null;
    }
    if (/^(draw|paint|illustrate|sketch)$/.test(action[1]) && isImageGenerationIntent(command)) return "image";
  }
  if (/^(?:пожалуйста[, ]+)?(?:создай|создайте|сгенерируй|сгенерируйте|сделай|сделайте|нарисуй|нарисуйте)\s+/u.test(command)) {
    const object = /(?:изображени[ея]|картинк[уаие]|фото|рисунок|логотип|постер|баннер|видео|анимаци[юя]|озвучк[уа]|голос|речь|музык[уа]|звук|отчёт|отчет|документ|код|функци[юя])/u.exec(command);
    if (object) {
      if (/^(видео|анимаци)/u.test(object[0])) return "video";
      if (/^(озвучк|голос|речь)/u.test(object[0])) return "voice";
      if (/^(музык|звук)/u.test(object[0])) return "sound";
      if (/^(изображени|картинк|фото|рисунок|логотип|постер|баннер)/u.test(object[0]) && isImageGenerationIntent(command)) return "image";
      return null;
    }
  }
  if (!/(কীভাবে|কিভাবে|ব্যাখ্যা|সম্পর্কে)/u.test(command) &&
      /(?:তৈরি\s*(?:করো|করুন|করে\s*দাও)|বানাও|বানান|জেনারেট\s*(?:করো|করুন)|আঁকো|আঁকুন)/u.test(command)) {
    const object = Array.from(command.matchAll(/(?:ছবি|ইমেজ|ফটো|চিত্র|পোস্টার|লোগো|ভিডিও|অ্যানিমেশন|ভয়েসওভার|ভয়েসওভার|ভয়েস|ভয়েস|কণ্ঠ|বর্ণনা|মিউজিক|সঙ্গীত|সংগীত|সাউন্ড|শব্দ|রিপোর্ট|ডকুমেন্ট|কোড|ফাংশন)/gu)).find((match) => !/^(?:র|ের)(?:\s|$)/u.test(command.slice((match.index ?? 0) + match[0].length)));
    if (object) {
      if (/^(ভিডিও|অ্যানিমেশন)/u.test(object[0])) return "video";
      if (/^(ভয়েস|ভয়েস|কণ্ঠ|বর্ণনা)/u.test(object[0])) return "voice";
      if (/^(মিউজিক|সঙ্গীত|সংগীত|সাউন্ড|শব্দ)/u.test(object[0])) return "sound";
      if (/^(ছবি|ইমেজ|ফটো|চিত্র|পোস্টার|লোগো)/u.test(object[0]) && isImageGenerationIntent(command)) return "image";
    }
  }
  return null;
}

export function inferChatMode(prompt: string, attachments: AttachmentDraft[]): ChatMode {
  const text = prompt.trim().toLowerCase().normalize("NFKC");
  const explicitOutput = explicitMediaMode(text);
  if (explicitOutput) return explicitOutput;
  if (attachments.some((attachment) => attachment.mediaItems.length > 0)) return "vision";
  if (attachments.some((attachment) => attachment.kind === "pdf")) return "pdf";
  // Prioritize explicit research intent over generic words such as "search".
  if (
    /\b(deep research|analyze deeply|investigate|market study|literature review)\b/.test(text) ||
    /(গভীর গবেষণা|গবেষণা কর|বিস্তারিত গবেষণা|গবেষণা করে|углублённое исследование|глубокое исследование|исследуй подробно)/u.test(text)
  )
    return "research";
  if (
    /\b(web|search|google|latest|today|current|news|sources?|verify|recent|up.to.date|look up|browse|online)\b/.test(text) ||
    /(খুঁজে|খোঁজ|সার্চ|সার্চ কর|ওয়েব|ওয়েব|ইন্টারনেট|আজকের|আজকে|সর্বশেষ|নতুন খবর|যাচাই কর|অনলাইনে|তথ্য বের কর|খবর|гугл|поиск|найди|поищи|интернет|сегодня|последние новости|проверь|актуальн|онлайн)/u.test(text)
  )
    return "search";
  if (/\b(pdf|export pdf|make a pdf|pdf ready)\b/.test(text)) return "pdf";
  if (/\b(document|write a doc|report|proposal|resume|cv|letter|contract|outline)\b/.test(text))
    return "document";
  if (isImageGenerationIntent(text)) return "image";
  if (/\b(create video|make video|generate video|video clip|animation|animate)\b/.test(text))
    return "video";
  if (
    /\b(create voice|make voice|generate voice|voiceover|voice over|narration|text to speech|tts)\b/.test(
      text,
    )
  )
    return "voice";
  if (
    /\b(generate music|create music|make music|sound effect|sound effects|sfx|soundscape|ambience|ambient sound|background music|notification sound)\b/.test(
      text,
    )
  )
    return "sound";
  if (/\b(think deeply|reason deeply|reason carefully|step by step|show your reasoning)\b/.test(text))
    return "thinking";
  return "chat";
}

export function isUntitledConversation(title: string) {
  const normalized = title.trim().toLowerCase();
  return (
    !normalized ||
    normalized === "new conversation" ||
    normalized === "new chat" ||
    normalized.startsWith("untitled")
  );
}

export function titleFromPrompt(prompt: string) {
  const cleaned = prompt
    .replace(/```[\s\S]*?```/g, " code ")
    .replace(/\s+/g, " ")
    .replace(/^[#>*\-\s]+/, "")
    .trim();
  if (!cleaned) return "New chat";
  const words = cleaned.split(" ").slice(0, 8).join(" ");
  const title = words.length > 54 ? `${words.slice(0, 51).trim()}...` : words;
  return title.charAt(0).toUpperCase() + title.slice(1);
}

export function attachmentKind(file: File): AttachmentDraft["kind"] {
  if (file.type.startsWith("image/") || /\.(png|jpe?g|webp)$/i.test(file.name)) return "image";
  if (file.type === "application/pdf" || /\.pdf$/i.test(file.name)) return "pdf";
  if (file.type.startsWith("audio/") || /\.(wav|mp3|m4a|aac|flac|ogg|opus)$/i.test(file.name)) {
    return "audio";
  }
  if (file.type.startsWith("video/") || /\.(mp4|webm|mov|m4v|mpeg|mpg)$/i.test(file.name)) {
    return "video";
  }
  if (
    file.type.startsWith("text/") ||
    /\.(md|txt|json|csv|ts|tsx|js|jsx|rs|py|html|css|sql|toml|yaml|yml)$/i.test(file.name)
  ) {
    return "text";
  }
  return "binary";
}

export function modeInstruction(mode: ChatMode) {
  switch (mode) {
    case "search":
      return [
        "[Mode: Web Search]",
        "Use the local live web retrieval evidence when available. Cite each current factual claim with the exact numbered source [n], and end with a References section containing clickable Markdown links to the matching URLs. Distinguish full-page evidence from snippets. If retrieval fails, say so clearly and do not fabricate sources or verification.",
      ].join("\n");
    case "research":
      return [
        "[Mode: Deep Research]",
        "Produce an evidence-driven research brief from retrieved full webpages and search snippets. Evaluate each key claim against independent domains, explicitly identify contradictions and limitations, label snippet-only evidence, cite the exact [n] markers, and include clickable Markdown links to every cited URL in a References section. Do not claim independent verification where it was unavailable, and never invent citations.",
      ].join("\n");
    case "thinking":
      return "[Mode: Think carefully before answering. Give the final answer clearly, with concise reasoning.]";
    case "document":
      return "[Mode: Document Writer]\nCreate polished document-ready content with title, sections, clear formatting, and final copy the user can paste into a document.";
    case "pdf":
      return [
        "[Mode: PDF]",
        "Answer from the locally extracted PDF text. Use supplied Page markers when page-specific attribution helps. If visual page images are supplied, use them for scanned/image-only content and OCR-style reading. Distinguish unavailable pages from analyzed pages instead of inventing content. If the request is to create/export a PDF, produce clean PDF-ready content.",
      ].join("\n");
    case "image":
      return "[Mode: Image Creation]\nCreate a concise production-quality prompt for the connected local image renderer, including subject, composition, style, lighting, camera/framing, colors, and useful negative constraints. Do not claim rendering succeeded until the artifact pipeline reports a ready image.";
    case "video":
      return "[Mode: Video Creation]\nWrite only the final positive visual prompt for the local video renderer. Describe subject, environment, action, scene progression, camera motion, lighting, composition, and visual style in natural prose. Do not add headings, markdown, negative prompts, duration, aspect-ratio recommendations, explanations, or claims that rendering already succeeded; the local runtime controls those settings separately.";
    case "voice":
      return "[Mode: Voice Creation]\nWrite only the final words that should be spoken aloud. Do not include headings, voice-style metadata, stage directions, markdown, or explanations. Keep punctuation natural for text-to-speech. The local voice runtime will synthesize this exact response after generation completes.";
    case "sound":
      return "[Mode: Music/SFX Creation]\nWrite one concise generation description for the local Soundscape engine. Preserve requested mood, genre, sound source, tempo/BPM, duration, environment, and whether the request is music, ambience, or a UI/SFX sound. Do not claim audio already exists.";
    case "vision":
      return "[Mode: Multimodal Vision Review]\nAnalyze only the image bytes supplied to the current local Lens request together with any locally extracted PDF text, video timestamps, or audio transcript included in the message. Treat rendered PDF pages and video keyframes as sampled visual evidence, not a guarantee that every frame/page was inspected. Never invent visual details from filenames or metadata.";
    default:
      return "";
  }
}
