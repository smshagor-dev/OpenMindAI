/// <reference types="vite/client" />

import { invoke } from "@tauri-apps/api/core";
import { GlobalWorkerOptions, getDocument } from "pdfjs-dist";
import type { PDFPageProxy } from "pdfjs-dist";
import pdfWorkerUrl from "pdfjs-dist/build/pdf.worker.min.mjs?url";

GlobalWorkerOptions.workerSrc = pdfWorkerUrl;

const MAX_PDF_BYTES = 32 * 1024 * 1024;
const MAX_PDF_PAGES = 250;
const MAX_EXTRACTED_CHARS = 20_000;
const MAX_VISION_PAGES = 4;
const OCR_BATCH_SIZE = 4;
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

interface PdfOcrPageResult {
  pageNumber: number;
  text: string;
}

export async function extractPdfText(file: File): Promise<PdfExtractionResult> {
  if (file.size === 0) {
    throw new Error("The PDF is empty.");
  }
  if (file.size > MAX_PDF_BYTES) {
    throw new Error("PDF is too large for chat ingestion. Maximum supported size is 32 MB.");
  }

  const bytes = new Uint8Array(await file.arrayBuffer());
  const task = getDocument({ data: bytes, useWorkerFetch: false });

  try {
    const pdf = await task.promise;
    const pagesRead = Math.min(pdf.numPages, MAX_PDF_PAGES);
    const pageTexts = new Array<string>(pagesRead).fill("");
    const scannedPages: number[] = [];
    let truncated = pdf.numPages > pagesRead;

    for (let pageNumber = 1; pageNumber <= pagesRead; pageNumber += 1) {
      const page = await pdf.getPage(pageNumber);
      const pageText = await selectablePageText(page);
      pageTexts[pageNumber - 1] = pageText;
      if (pageText.replace(/\s+/g, "").length < MIN_SELECTABLE_TEXT_CHARS) {
        scannedPages.push(pageNumber);
      }
    }

    const selectedVisionPages = new Set(selectRepresentativePages(scannedPages, MAX_VISION_PAGES));
    const visionPages: PdfVisionPage[] = [];

    for (let offset = 0; offset < scannedPages.length; offset += OCR_BATCH_SIZE) {
      const batchNumbers = scannedPages.slice(offset, offset + OCR_BATCH_SIZE);
      const rendered = await Promise.all(
        batchNumbers.map(async (pageNumber) => {
          const page = await pdf.getPage(pageNumber);
          const dataUrl = await renderPdfPage(page);
          if (selectedVisionPages.has(pageNumber)) {
            visionPages.push({ pageNumber, dataUrl, mimeType: "image/jpeg" });
          }
          return { pageNumber, dataUrl };
        }),
      );

      let ocr: PdfOcrPageResult[];
      try {
        ocr = await invoke<PdfOcrPageResult[]>("ocr_pdf_pages", { pages: rendered });
      } catch (error) {
        const detail = error instanceof Error ? error.message : String(error);
        throw new Error(
          `OpenMindAI Lens could not read scanned PDF pages ${batchNumbers.join(", ")}: ${detail}`,
        );
      }
      for (const result of ocr) {
        if (result.pageNumber < 1 || result.pageNumber > pagesRead) continue;
        const existing = pageTexts[result.pageNumber - 1].trim();
        const visual = result.text.trim();
        pageTexts[result.pageNumber - 1] = [existing, visual].filter(Boolean).join("\n");
      }
    }

    visionPages.sort((left, right) => left.pageNumber - right.pageNumber);
    const chunks: string[] = [];
    let currentLength = 0;
    for (let index = 0; index < pageTexts.length; index += 1) {
      const pageText = pageTexts[index].trim();
      if (!pageText) continue;
      const pageChunk = `${chunks.length ? "\n\n" : ""}--- Page ${index + 1} ---\n${pageText}`;
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

    return {
      text: chunks.join("").trim(),
      pageCount: pdf.numPages,
      pagesRead,
      truncated,
      visionPages,
      scannedPages,
    };
  } finally {
    await task.destroy().catch(() => undefined);
  }
}

async function selectablePageText(page: PDFPageProxy) {
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
  return lines.filter(Boolean).join("\n").trim();
}

async function renderPdfPage(page: PDFPageProxy) {
  const base = page.getViewport({ scale: 1 });
  const scale = Math.min(2, MAX_RENDER_DIMENSION / Math.max(base.width, base.height));
  const viewport = page.getViewport({ scale: Math.max(0.5, scale) });
  const canvas = document.createElement("canvas");
  canvas.width = Math.max(1, Math.round(viewport.width));
  canvas.height = Math.max(1, Math.round(viewport.height));
  const context = canvas.getContext("2d", { alpha: false });
  if (!context) throw new Error("Could not render a scanned PDF page for local vision.");
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
