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
  if (isTextLike && file.size <= maxPreviewBytes) {
    contentPreview = await file.text();
  } else if (kind === "image") {
    contentPreview = `[Image attached: ${file.name}. Exact visual analysis requires the local OpenMindAI Lens vision runtime. Do not infer unseen image content from metadata alone.]`;
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
  };
}

export function buildMessageContent(prompt: string, attachments: AttachmentDraft[], mode: ChatMode) {
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

export function inferChatMode(prompt: string, attachments: AttachmentDraft[]): ChatMode {
  const text = prompt.toLowerCase();
  if (attachments.some((attachment) => attachment.kind === "image")) return "vision";
  if (attachments.some((attachment) => attachment.kind === "pdf")) return "pdf";
  if (/\b(web|search|google|latest|today|current|news|source|sources|verify)\b/.test(text)) return "search";
  if (/\b(research|deep research|analyze deeply|investigate|market study|literature review)\b/.test(text)) return "research";
  if (/\b(pdf|export pdf|make a pdf|pdf ready)\b/.test(text)) return "pdf";
  if (/\b(document|write a doc|report|proposal|resume|cv|letter|contract|outline)\b/.test(text)) return "document";
  if (/\b(create image|make image|generate image|draw|poster|logo|thumbnail|illustration)\b/.test(text)) return "image";
  if (/\b(create video|make video|generate video|video clip|animation|animate)\b/.test(text)) return "video";
  if (/\b(create voice|make voice|generate voice|voiceover|voice over|narration|text to speech|tts)\b/.test(text)) return "voice";
  if (/\b(think|reason|solve|debug|step by step|carefully)\b/.test(text)) return "thinking";
  return "chat";
}

export function isUntitledConversation(title: string) {
  const normalized = title.trim().toLowerCase();
  return !normalized || normalized === "new conversation" || normalized === "new chat" || normalized.startsWith("untitled");
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
  if (file.type.startsWith("image/")) return "image";
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
      return "[Mode: Image Creation]\nCreate a production-quality image generation prompt with subject, composition, style, lighting, camera/framing, colors, negative prompt, and recommended aspect ratio. Do not claim an image file was generated unless an image generator is connected.";
    case "video":
      return "[Mode: Video Creation]\nCreate a production-quality video generation prompt with subject, scene progression, camera motion, timing, lighting, style, negative prompt, and recommended duration/aspect ratio. Do not claim a video file was generated unless a video generator is connected.";
    case "voice":
      return "[Mode: Voice Creation]\nCreate a production-quality voice generation prompt or script with voice style, pacing, emotion, pronunciation notes, format, and final narration copy. Do not claim audio was generated unless a voice generator is connected.";
    case "vision":
      return "[Mode: Image/Vision Review]\nUse the local vision runtime when the attached image has been made available to it. Never invent visual details from file metadata. If the vision runtime is unavailable, say so clearly.";
    default:
      return "";
  }
}
