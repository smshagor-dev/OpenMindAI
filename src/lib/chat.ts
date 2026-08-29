import type { Message } from "../types";
import { formatBytes } from "./format";
import { extractPdfText } from "./pdf";

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
  | "vision";

export interface AttachmentDraft {
  id: string;
  name: string;
  size: number;
  type: string;
  kind: "text" | "image" | "pdf" | "binary";
  contentPreview: string | null;
  mediaDataUrl: string | null;
  mediaMimeType: "image/png" | "image/jpeg" | null;
}

export interface InferenceMediaDraft {
  kind: "image";
  name: string;
  mimeType: "image/png" | "image/jpeg";
  dataUrl: string;
}

export interface UserMessageDisplay {
  prompt: string;
  attachmentNames: string[];
  displayText: string;
}

const MAX_VISION_IMAGE_INPUT_BYTES = 16 * 1024 * 1024;
const MAX_VISION_IMAGE_DATA_URL_CHARS = 6_000_000;
const MAX_VISION_IMAGE_DIMENSION = 2048;
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

export async function readAttachment(file: File): Promise<AttachmentDraft> {
  const maxPreviewBytes = 1024 * 1024;
  const kind = attachmentKind(file);
  const isTextLike =
    file.type.startsWith("text/") ||
    /\.(md|txt|json|csv|ts|tsx|js|jsx|rs|py|html|css|sql|toml|yaml|yml)$/i.test(file.name);

  let contentPreview: string | null = null;
  let mediaDataUrl: string | null = null;
  let mediaMimeType: "image/png" | "image/jpeg" | null = null;
  if (isTextLike && file.size <= maxPreviewBytes) {
    contentPreview = await file.text();
  } else if (kind === "image") {
    const encoded = await encodeVisionImage(file);
    mediaDataUrl = encoded.dataUrl;
    mediaMimeType = encoded.mimeType;
    contentPreview = `[Image attached: ${file.name}. Optimized image bytes are supplied only to the current local vision request and are not stored in chat history.]`;
  } else if (kind === "pdf") {
    try {
      const extracted = await extractPdfText(file);
      const status = [
        `${extracted.pageCount} page${extracted.pageCount === 1 ? "" : "s"}`,
        `${extracted.pagesRead} processed`,
        extracted.truncated ? "text truncated to fit local chat context" : null,
      ]
        .filter(Boolean)
        .join(", ");
      contentPreview = `[PDF text extracted locally: ${status}. Page markers below correspond to the original PDF.]\n\n${extracted.text}`;
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      contentPreview = `[PDF attached: ${file.name}. Local PDF extraction could not produce usable text: ${message} Do not claim to have read inaccessible pages. If this is a scanned/image-only PDF, use OCR/vision once OpenMindAI Lens is available.]`;
    }
  }

  return {
    id: crypto.randomUUID(),
    name: file.name,
    size: file.size,
    type: file.type || "unknown",
    kind,
    contentPreview,
    mediaDataUrl,
    mediaMimeType,
  };
}

export function attachmentMedia(attachments: AttachmentDraft[]): InferenceMediaDraft[] {
  return attachments.flatMap((attachment) => {
    if (attachment.kind !== "image" || !attachment.mediaDataUrl || !attachment.mediaMimeType) {
      return [];
    }
    return [
      {
        kind: "image" as const,
        name: attachment.name,
        mimeType: attachment.mediaMimeType,
        dataUrl: attachment.mediaDataUrl,
      },
    ];
  });
}

export function buildMessageContent(
  prompt: string,
  attachments: AttachmentDraft[],
  mode: ChatMode,
) {
  const modePrefix = modeInstruction(mode);
  if (attachments.length === 0) return [modePrefix, prompt].filter(Boolean).join("\n\n");
  const attachmentText = attachments
    .map((attachment) => {
      const header = `[Attachment: ${attachment.name}, ${attachment.kind}, ${formatBytes(attachment.size)}, ${attachment.type}]`;
      return attachment.contentPreview ? `${header}\n${attachment.contentPreview}` : header;
    })
    .join("\n\n");
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
      /^\[Attachment:\s*(.+?),\s*(?:text|image|pdf|binary),\s*[^,\]]+,\s*[^\]]+\]$/gm,
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

export function inferChatMode(prompt: string, attachments: AttachmentDraft[]): ChatMode {
  const text = prompt.toLowerCase();
  if (attachments.some((attachment) => attachment.kind === "image")) return "vision";
  if (attachments.some((attachment) => attachment.kind === "pdf")) return "pdf";
  if (/\b(web|search|google|latest|today|current|news|source|sources|verify)\b/.test(text))
    return "search";
  if (
    /\b(research|deep research|analyze deeply|investigate|market study|literature review)\b/.test(
      text,
    )
  )
    return "research";
  if (/\b(pdf|export pdf|make a pdf|pdf ready)\b/.test(text)) return "pdf";
  if (/\b(document|write a doc|report|proposal|resume|cv|letter|contract|outline)\b/.test(text))
    return "document";
  if (
    /\b(create image|make image|generate image|draw|poster|logo|thumbnail|illustration)\b/.test(
      text,
    )
  )
    return "image";
  if (/\b(create video|make video|generate video|video clip|animation|animate)\b/.test(text))
    return "video";
  if (
    /\b(create voice|make voice|generate voice|voiceover|voice over|narration|text to speech|tts)\b/.test(
      text,
    )
  )
    return "voice";
  if (/\b(think|reason|solve|debug|step by step|carefully)\b/.test(text)) return "thinking";
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
        "Use live web evidence supplied by the local retrieval backend when available. Cite current factual claims with the numbered sources provided by that backend. If retrieval fails, state that clearly and do not fabricate current sources.",
      ].join("\n");
    case "research":
      return [
        "[Mode: Deep Research]",
        "Use the live evidence supplied by the local research retrieval backend, cross-check claims across sources, distinguish evidence from inference, surface uncertainty, and cite numbered sources. If live retrieval fails, say so rather than inventing evidence.",
      ].join("\n");
    case "thinking":
      return "[Mode: Think carefully before answering. Give the final answer clearly, with concise reasoning.]";
    case "document":
      return "[Mode: Document Writer]\nCreate polished document-ready content with title, sections, clear formatting, and final copy the user can paste into a document.";
    case "pdf":
      return [
        "[Mode: PDF]",
        "If an attached PDF includes locally extracted text, answer from that text and use the supplied Page markers when page-specific attribution helps. Never claim to have read pages whose text was unavailable. If the request is to create/export a PDF instead, produce clean PDF-ready content with headings, concise paragraphs, and tables where useful.",
      ].join("\n");
    case "image":
      return "[Mode: Image Creation]\nCreate a concise production-quality prompt for the connected local image renderer, including subject, composition, style, lighting, camera/framing, colors, and useful negative constraints. Do not claim rendering succeeded until the artifact pipeline reports a ready image.";
    case "video":
      return "[Mode: Video Creation]\nWrite only the final positive visual prompt for the local video renderer. Describe subject, environment, action, scene progression, camera motion, lighting, composition, and visual style in natural prose. Do not add headings, markdown, negative prompts, duration, aspect-ratio recommendations, explanations, or claims that rendering already succeeded; the local runtime controls those settings separately.";
    case "voice":
      return "[Mode: Voice Creation]\nWrite only the final words that should be spoken aloud. Do not include headings, voice-style metadata, stage directions, markdown, or explanations. Keep punctuation natural for text-to-speech. The local voice runtime will synthesize this exact response after generation completes.";
    case "vision":
      return "[Mode: Image/Vision Review]\nAnalyze only the image bytes supplied to the current local Lens request. Never invent visual details from attachment metadata. If Lens rejects or cannot access the image, say so clearly.";
    default:
      return "";
  }
}