/// <reference types="vite/client" />

import { GlobalWorkerOptions, getDocument } from "pdfjs-dist";
import pdfWorkerUrl from "pdfjs-dist/build/pdf.worker.min.mjs?url";

GlobalWorkerOptions.workerSrc = pdfWorkerUrl;

const MAX_PDF_BYTES = 32 * 1024 * 1024;
const MAX_PDF_PAGES = 250;
// The local chat backend currently enforces an initial ~24k-character text
// context budget. Keep one PDF well below that ceiling so the prompt, recent
// history, profile/project instructions, and other attachments still have
// room instead of accepting a huge extraction that is guaranteed to fail at
// inference time.
const MAX_EXTRACTED_CHARS = 10_000;

export interface PdfExtractionResult {
  text: string;
  pageCount: number;
  pagesRead: number;
  truncated: boolean;
}

export async function extractPdfText(file: File): Promise<PdfExtractionResult> {
  if (file.size === 0) {
    throw new Error("The PDF is empty.");
  }
  if (file.size > MAX_PDF_BYTES) {
    throw new Error("PDF is too large for chat ingestion. Maximum supported size is 32 MB.");
  }

  const bytes = new Uint8Array(await file.arrayBuffer());
  const task = getDocument({
    data: bytes,
    useWorkerFetch: false,
  });

  try {
    const document = await task.promise;
    const pagesRead = Math.min(document.numPages, MAX_PDF_PAGES);
    const chunks: string[] = [];
    let currentLength = 0;
    let truncated = document.numPages > pagesRead;

    for (let pageNumber = 1; pageNumber <= pagesRead; pageNumber += 1) {
      const page = await document.getPage(pageNumber);
      const textContent = await page.getTextContent();
      const lines: string[] = [];
      let line = "";

      for (const item of textContent.items) {
        if (!("str" in item)) continue;
        const value = item.str.replace(/\s+/g, " ");
        if (!value) continue;
        line += `${line ? " " : ""}${value}`;
        if (item.hasEOL) {
          lines.push(line.trim());
          line = "";
        }
      }
      if (line.trim()) lines.push(line.trim());

      const pageText = lines.filter(Boolean).join("\n").trim();
      if (!pageText) continue;
      const pageChunk = `\n\n--- Page ${pageNumber} ---\n${pageText}`;
      const remaining = MAX_EXTRACTED_CHARS - currentLength;
      if (remaining <= 0) {
        truncated = true;
        break;
      }
      if (pageChunk.length > remaining) {
        chunks.push(pageChunk.slice(0, remaining));
        currentLength += remaining;
        truncated = true;
        break;
      }
      chunks.push(pageChunk);
      currentLength += pageChunk.length;
    }

    const text = chunks.join("").trim();
    if (!text) {
      throw new Error(
        "No selectable text was found in this PDF. It may be scanned or image-only and requires OCR/vision.",
      );
    }

    return {
      text,
      pageCount: document.numPages,
      pagesRead,
      truncated,
    };
  } finally {
    await task.destroy().catch(() => undefined);
  }
}
