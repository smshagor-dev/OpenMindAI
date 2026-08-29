/// <reference types="vite/client" />

import { GlobalWorkerOptions, getDocument } from "pdfjs-dist";
import type { PDFPageProxy } from "pdfjs-dist";
import pdfWorkerUrl from "pdfjs-dist/build/pdf.worker.min.mjs?url";

GlobalWorkerOptions.workerSrc = pdfWorkerUrl;

const MAX_PDF_BYTES = 32 * 1024 * 1024;
const MAX_PDF_PAGES = 250;
const MAX_EXTRACTED_CHARS = 10_000;
const MAX_VISION_PAGES = 4;
const MIN_SELECTABLE_TEXT_CHARS = 24;
const MAX_RENDER_DIMENSION = 1440;

export interface PdfVisionPage {
  pageNumber: number;
  dataUrl: string;
  mimeType: "image/jpeg";
}

export interface PdfExtractionResult {
  text: string;
  pageCount: number;
  pagesRead: number;
  truncated: boolean;
  visionPages: PdfVisionPage[];
  scannedPages: number[];
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
    const scannedPages: number[] = [];
    let currentLength = 0;
    let truncated = document.numPages > pagesRead;

    for (let pageNumber = 1; pageNumber <= pagesRead; pageNumber += 1) {
      const page = await document.getPage(pageNumber);
      const textContent = await page.getTextContent();
      const lines: string[] = [];
      let line = "";

      for (const item of textContent.items) {
        if (!("str" in item)) continue;
        const value = item.str.replace(/\s+/g, " ").trim();
        if (!value) continue;
        line += `${line ? " " : ""}${value}`;
        if (item.hasEOL) {
          lines.push(line.trim());
          line = "";
        }
      }
      if (line.trim()) lines.push(line.trim());

      const pageText = lines.filter(Boolean).join("\n").trim();
      if (pageText.replace(/\s+/g, "").length < MIN_SELECTABLE_TEXT_CHARS) {
        scannedPages.push(pageNumber);
      }
      if (!pageText) continue;

      const pageChunk = `\n\n--- Page ${pageNumber} ---\n${pageText}`;
      const remaining = MAX_EXTRACTED_CHARS - currentLength;
      if (remaining <= 0) {
        truncated = true;
        continue;
      }
      if (pageChunk.length > remaining) {
        chunks.push(pageChunk.slice(0, remaining));
        currentLength += remaining;
        truncated = true;
        continue;
      }
      chunks.push(pageChunk);
      currentLength += pageChunk.length;
    }

    const selectedVisionPages = selectRepresentativePages(scannedPages, MAX_VISION_PAGES);
    const visionPages: PdfVisionPage[] = [];
    for (const pageNumber of selectedVisionPages) {
      const page = await document.getPage(pageNumber);
      visionPages.push({
        pageNumber,
        dataUrl: await renderPdfPage(page),
        mimeType: "image/jpeg",
      });
    }

    return {
      text: chunks.join("").trim(),
      pageCount: document.numPages,
      pagesRead,
      truncated,
      visionPages,
      scannedPages,
    };
  } finally {
    await task.destroy().catch(() => undefined);
  }
}

async function renderPdfPage(page: PDFPageProxy) {
  const base = page.getViewport({ scale: 1 });
  const scale = Math.min(2, MAX_RENDER_DIMENSION / Math.max(base.width, base.height));
  const viewport = page.getViewport({ scale: Math.max(0.5, scale) });
  const canvas = document.createElement("canvas");
  canvas.width = Math.max(1, Math.round(viewport.width));
  canvas.height = Math.max(1, Math.round(viewport.height));
  const context = canvas.getContext("2d", { alpha: false });
  if (!context) {
    throw new Error("Could not render a scanned PDF page for local vision.");
  }
  context.fillStyle = "#ffffff";
  context.fillRect(0, 0, canvas.width, canvas.height);
  await page.render({ canvas, canvasContext: context, viewport }).promise;
  return canvas.toDataURL("image/jpeg", 0.84);
}

function selectRepresentativePages(pageNumbers: number[], limit: number) {
  if (pageNumbers.length <= limit) return pageNumbers;
  if (limit <= 1) return [pageNumbers[0]];
  const selected = new Set<number>();
  for (let index = 0; index < limit; index += 1) {
    const position = Math.round((index * (pageNumbers.length - 1)) / (limit - 1));
    selected.add(pageNumbers[position]);
  }
  return Array.from(selected).slice(0, limit);
}
