import { marked } from "marked";
import hljs from "highlight.js/lib/core";
import python from "highlight.js/lib/languages/python";
import typescript from "highlight.js/lib/languages/typescript";
import javascript from "highlight.js/lib/languages/javascript";
import rust from "highlight.js/lib/languages/rust";
import json from "highlight.js/lib/languages/json";
import bash from "highlight.js/lib/languages/bash";
import xml from "highlight.js/lib/languages/xml";
import css from "highlight.js/lib/languages/css";

hljs.registerLanguage("python", python);
hljs.registerLanguage("py", python);
hljs.registerLanguage("typescript", typescript);
hljs.registerLanguage("ts", typescript);
hljs.registerLanguage("javascript", javascript);
hljs.registerLanguage("js", javascript);
hljs.registerLanguage("rust", rust);
hljs.registerLanguage("rs", rust);
hljs.registerLanguage("json", json);
hljs.registerLanguage("bash", bash);
hljs.registerLanguage("sh", bash);
hljs.registerLanguage("html", xml);
hljs.registerLanguage("xml", xml);
hljs.registerLanguage("css", css);

export type MarkdownBlock =
  | { type: "markdown"; content: string }
  | { type: "code"; content: string; language: string | null };

export function splitMarkdownBlocks(content: string): MarkdownBlock[] {
  const blocks: MarkdownBlock[] = [];
  const fencePattern = /(?:^|\n)(`{2,}|~{3,})[ \t]*([^\n`]*)\n([\s\S]*?)\n\1(?=\n|$)/g;
  let cursor = 0;
  let match: RegExpExecArray | null;

  while ((match = fencePattern.exec(content)) !== null) {
    const startsWithNewline = content[match.index] === "\n";
    const fenceStart = startsWithNewline ? match.index + 1 : match.index;
    const markdownBefore = content.slice(cursor, fenceStart);
    if (markdownBefore.trim()) blocks.push({ type: "markdown", content: markdownBefore });

    blocks.push({
      type: "code",
      language: match[2]?.trim().split(/\s+/)[0] || null,
      content: trimCodeBlock(match[3] ?? ""),
    });
    cursor = fencePattern.lastIndex;
  }

  const rest = content.slice(cursor);
  if (rest.trim() || blocks.length === 0) blocks.push({ type: "markdown", content: rest });
  return blocks;
}

export function renderMarkdown(content: string) {
  return marked.parse(escapeHtml(content), {
    async: false,
    breaks: true,
    gfm: true,
  }) as string;
}

export function highlightCode(code: string, language: string | null) {
  if (language && hljs.getLanguage(language)) {
    return hljs.highlight(code, { language, ignoreIllegals: true }).value;
  }
  return hljs.highlightAuto(code).value;
}

export function normalizeLanguage(language: string | null) {
  if (!language) return null;
  const normalized = language.toLowerCase();
  const aliases: Record<string, string> = {
    py: "python",
    ts: "typescript",
    js: "javascript",
    rs: "rust",
    shell: "bash",
    console: "bash",
  };
  return aliases[normalized] ?? normalized;
}

export function trimCodeBlock(code: string) {
  return code.replace(/^\n+|\s+$/g, "");
}

export function escapeHtml(value: string) {
  return value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}
