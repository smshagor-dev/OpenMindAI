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
// OCR is the dominant cost. The first scanned pages and a stratified sample
// of the remainder are enough for a preview; remaining pages are disclosed.
const MAX_OCR_PAGES = 16;
const MAX_OCR_ELAPSED_MS = 60_000;
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
  unreadPages: number[];
  warnings: string[];
}

interface PdfOcrPageResult {
  pageNumber: number;
  text: string;
}

export interface PdfExtractionProgress {
  phase: "reading" | "ocr";
  completed: number;
  total: number;
}

export interface PdfExtractionOptions {
  signal?: { readonly aborted: boolean };
  onProgress?: (progress: PdfExtractionProgress) => void;
}

function assertNotCancelled(signal?: PdfExtractionOptions["signal"]) {
  if (signal?.aborted) {
    throw new Error("PDF import cancelled.");
  }
}

function selectOcrPages(scannedPages: number[], limit: number): number[] {
  if (scannedPages.length <= limit) return scannedPages;
  // Prioritize early pages (which receive the first portion of the model's
  // text budget), then sample the remaining document for representative
  // evidence instead of discarding everything after page 16.
  const early = Math.min(8, limit, scannedPages.length);
  const chosen = new Set(scannedPages.slice(0, early));
  const remaining = scannedPages.slice(early);
  for (const page of selectRepresentativePages(remaining, limit - early)) {
    chosen.add(page);
  }
  return [...chosen].sort((a, b) => a - b);
}

export async function extractPdfText(
  file: File,
  options: PdfExtractionOptions = {},
): Promise<PdfExtractionResult> {
  if (file.size === 0) {
    throw new Error("The PDF is empty.");
  }
  if (file.size > MAX_PDF_BYTES) {
    throw new Error("PDF is too large for chat ingestion. Maximum supported size is 32 MB.");
  }

  assertNotCancelled(options.signal);
  const bytes = new Uint8Array(await file.arrayBuffer());
  assertNotCancelled(options.signal);
  const task = getDocument({ data: bytes, useWorkerFetch: false });

  try {
    const pdf = await task.promise;
    assertNotCancelled(options.signal);
    const maxPagesRead = Math.min(pdf.numPages, MAX_PDF_PAGES);
    const pageTexts: string[] = [];
    const scannedPages: number[] = [];
    let selectableChars = 0;
    let truncated = pdf.numPages > maxPagesRead;
    options.onProgress?.({ phase: "reading", completed: 0, total: maxPagesRead });

    // Stop touching later page objects once the text prompt cannot hold
    // additional selectable evidence. Earlier scanned pages remain candidates
    // for a bounded OCR pass because their text precedes later pages.
    for (let pageNumber = 1; pageNumber <= maxPagesRead; pageNumber += 1) {
      assertNotCancelled(options.signal);
      const page = await pdf.getPage(pageNumber);
      assertNotCancelled(options.signal);
      const pageText = await selectablePageText(page);
      assertNotCancelled(options.signal);
      pageTexts.push(pageText);
      selectableChars += pageText.length;
      if (pageText.replace(/\s+/g, "").length < MIN_SELECTABLE_TEXT_CHARS) {
        scannedPages.push(pageNumber);
      }
      options.onProgress?.({ phase: "reading", completed: pageNumber, total: maxPagesRead });
      if (selectableChars >= MAX_EXTRACTED_CHARS) {
        if (pageNumber < pdf.numPages) truncated = true;
        break;
      }
    }
    const pagesRead = pageTexts.length;
    const ocrPages = selectOcrPages(scannedPages, MAX_OCR_PAGES);
    const ocrPageSet = new Set(ocrPages);
    const deferred = scannedPages.filter(pageNumber => !ocrPageSet.has(pageNumber));
    const selectedVisionPages = new Set(selectRepresentativePages(ocrPages, MAX_VISION_PAGES));
    const visionPages: PdfVisionPage[] = [];
    const unreadPages = new Set(ocrPages);
    const startedOcrAt = Date.now();
    let completedOcr = 0;
    let stoppedForTime = false;
    options.onProgress?.({ phase: "ocr", completed: 0, total: ocrPages.length });

    for (let offset = 0; offset < ocrPages.length; offset += OCR_BATCH_SIZE) {
      assertNotCancelled(options.signal);
      if (Date.now() - startedOcrAt >= MAX_OCR_ELAPSED_MS) {
        stoppedForTime = true;
        break;
      }
      const batchNumbers = ocrPages.slice(offset, offset + OCR_BATCH_SIZE);
      const rendered: { pageNumber: number; dataUrl: string }[] = [];
      for (const pageNumber of batchNumbers) {
        assertNotCancelled(options.signal);
        try {
          const page = await pdf.getPage(pageNumber);
          assertNotCancelled(options.signal);
          const dataUrl = await renderPdfPage(page, options.signal);
          rendered.push({ pageNumber, dataUrl });
          if (selectedVisionPages.has(pageNumber)) {
            visionPages.push({ pageNumber, dataUrl, mimeType: "image/jpeg" });
          }
        } catch (error) {
          assertNotCancelled(options.signal);
          // Other scanned pages remain available if a renderer fails.
          void error;
        }
      }
      if (rendered.length) {
        try {
          // Only one small batch exists at a time. Cancellation applies
          // between native OCR batches; the current Tauri invoke is not an
          // interruptible task.
          const ocr = await invoke<PdfOcrPageResult[]>("ocr_pdf_pages", { pages: rendered });
          assertNotCancelled(options.signal);
          const allowed = new Set(rendered.map(page => page.pageNumber));
          for (const result of ocr) {
            if (!allowed.delete(result.pageNumber) || typeof result.text !== "string") continue;
            const visual = result.text.trim();
            if (!visual) continue;
            const existing = pageTexts[result.pageNumber - 1].trim();
            pageTexts[result.pageNumber - 1] = [existing, visual].filter(Boolean).join("\n");
            unreadPages.delete(result.pageNumber);
          }
        } catch (error) {
          assertNotCancelled(options.signal);
          // Preserve text and successful previous OCR batches on failure.
          void error;
        }
      }
      completedOcr += batchNumbers.length;
      options.onProgress?.({
        phase: "ocr",
        completed: completedOcr,
        total: ocrPages.length,
      });
    }
    assertNotCancelled(options.signal);
    const missing = [...unreadPages, ...deferred].sort((a, b) => a - b);
    const ocrFailures = [...unreadPages].sort((a, b) => a - b);
    const warnings = [
      ...(ocrFailures.length ? [
        `OCR incomplete on pages ${ocrFailures.join(", ")}. Only available text is included; image-only content on these pages was not analyzed. Install or repair OpenMindAI Lens for full coverage.`,
      ] : []),
      ...(deferred.length ? [
        `OCR work budget reached: ${deferred.length} scanned page(s) were not analyzed (pages ${deferred.join(", ")}). Split the PDF or import a smaller section to retrieve their contents.`,
      ] : []),
      ...(stoppedForTime ? [
        "PDF OCR time budget reached. Remaining scanned pages were not analyzed; import a smaller section for additional coverage.",
      ] : []),
    ];
    if (missing.length && !pageTexts.some(text => text.trim())) {
      throw new Error(`No readable PDF text is available. ${warnings.join(" ")}`);
    }
    // Do not imply comprehensive visual coverage when OCR was incomplete or
    // bounded. Retained OCR text is still safe for a text-only model.
    if (missing.length) visionPages.length = 0;

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
    if (deferred.length || stoppedForTime) truncated = true;

    return {
      text: chunks.join("").trim(),
      pageCount: pdf.numPages,
      pagesRead,
      truncated,
      visionPages,
      scannedPages,
      unreadPages: missing,
      warnings,
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

async function renderPdfPage(page: PDFPageProxy, signal?: PdfExtractionOptions["signal"]) {
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
  try {
    assertNotCancelled(signal);
    await page.render({ canvas, canvasContext: context, viewport }).promise;
    assertNotCancelled(signal);
    return canvas.toDataURL("image/jpeg", 0.84);
  } finally {
    // Release the decoded bitmap promptly; only the selected, bounded JPEG
    // bytes survive this scope.
    canvas.width = 0;
    canvas.height = 0;
  }
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
