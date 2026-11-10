// The local model sees bounded, named excerpts, never silently skipped documents.
export const MAX_SINGLE_ATTACHMENT_CONTEXT_CHARS = 8_000;
export const MAX_TOTAL_ATTACHMENT_CONTEXT_CHARS = 16_000;
const TRUNCATION_NOTE = "\n[Attachment excerpt truncated; omitted text is NOT available to the model]";
const GAP = "\n[... intervening text omitted ...]\n";
const OVERALL_NOTE = "[Attachment context limit: only excerpts were sent; omitted text was not analyzed. Ask for a specific passage or attach fewer files.]";

export interface AttachmentContextSource {
  name: string;
  header: string;
  content: string | null;
}
export interface AttachmentContextItem {
  name: string;
  providedChars: number;
  originalChars: number;
  truncated: boolean;
}
export interface AttachmentContextPlan {
  budgets: number[];
  items: AttachmentContextItem[];
  truncatedCount: number;
  omittedCount: number;
}

/** Distribute prompt context fairly. Reserve one header and warning capacity per file. */
export function planAttachmentContext(sources: AttachmentContextSource[]): AttachmentContextPlan {
  const joinedHeaders = sources.reduce((used, source) => used + source.header.length + (source.content ? 1 : 0), 0)
    + Math.max(0, sources.length - 1) * 2;
  // Keep space for the final disclosure, even if every file needs its maximum share.
  let left = Math.max(0, MAX_TOTAL_ATTACHMENT_CONTEXT_CHARS - joinedHeaders - OVERALL_NOTE.length - 2);
  const wanted = sources.map(source => Math.min(MAX_SINGLE_ATTACHMENT_CONTEXT_CHARS, source.content?.length ?? 0));
  const budgets = sources.map(() => 0);
  let live = wanted.map((count, index) => count > 0 ? index : -1).filter(index => index >= 0);
  while (left > 0 && live.length > 0) {
    const share = Math.max(1, Math.floor(left / live.length));
    for (const index of live) {
      const added = Math.min(share, wanted[index] - budgets[index], left);
      budgets[index] += added;
      left -= added;
      if (left === 0) break;
    }
    live = live.filter(index => budgets[index] < wanted[index]);
  }
  const items = sources.map((source, index) => ({
    name: source.name,
    providedChars: budgets[index],
    originalChars: source.content?.length ?? 0,
    truncated: (source.content?.length ?? 0) > budgets[index],
  }));
  return {
    budgets,
    items,
    truncatedCount: items.filter(item => item.truncated).length,
    omittedCount: items.filter(item => item.originalChars > 0 && item.providedChars === 0).length,
  };
}

function relevantPosition(source: string, prompt: string, from: number): number {
  const terms = [...new Set((prompt.toLowerCase().normalize("NFKC").match(/[\p{L}\p{N}]{4,}/gu) ?? [])
    .filter(term => !["please", "about", "these", "those", "document", "explain", "summarize", "what", "which", "from", "with"].includes(term)))].slice(0, 16);
  if (!terms.length) return Math.floor(source.length / 2);
  const lower = source.toLowerCase().normalize("NFKC");
  let best = -1;
  for (const term of terms) {
    const at = lower.indexOf(term, from);
    if (at >= 0 && (best < 0 || at < best)) best = at;
  }
  return best >= 0 ? best : Math.floor(source.length / 2);
}

/** Includes start, a query-relevant mid-document window, and the ending. */
export function selectAttachmentExcerpt(content: string, limit: number, prompt: string): string {
  if (limit <= 0) return "";
  if (content.length <= limit) return content;
  if (limit <= TRUNCATION_NOTE.length + GAP.length * 2 + 12) {
    if (limit <= TRUNCATION_NOTE.length) return TRUNCATION_NOTE.slice(0, limit);
    return content.slice(0, limit - TRUNCATION_NOTE.length) + TRUNCATION_NOTE;
  }
  const usable = limit - TRUNCATION_NOTE.length - GAP.length * 2;
  const head = Math.max(1, Math.floor(usable * 0.35));
  const tail = Math.max(1, Math.floor(usable * 0.25));
  const middle = usable - head - tail;
  const hit = relevantPosition(content, prompt, head);
  const start = Math.max(head, Math.min(content.length - tail - middle, hit - Math.floor(middle / 2)));
  const text = content.slice(0, head) + GAP + content.slice(start, start + middle) + GAP + content.slice(-tail) + TRUNCATION_NOTE;
  return text.slice(0, limit);
}

export function serializeAttachmentContext(sources: AttachmentContextSource[], prompt: string) {
  const plan = planAttachmentContext(sources);
  const parts = sources.map((source, index) => {
    const excerpt = selectAttachmentExcerpt(source.content ?? "", plan.budgets[index], prompt);
    return excerpt ? source.header + "\n" + excerpt : source.header;
  });
  if (plan.truncatedCount || plan.omittedCount) parts.push(OVERALL_NOTE);
  const text = parts.join("\n\n");
  return { text, plan };
}
