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
    // A malformed decoder page count must not turn an unreadable PDF into an
    // apparently successful empty import or invalid progress metadata.
    if (!Number.isSafeInteger(pdf.numPages) || pdf.numPages < 1) {
      throw new Error("Invalid PDF page count returned by the decoder.");
    }
    const maxPagesRead = Math.min(pdf.numPages, MAX_PDF_PAGES);
    const pageTexts: string[] = [];
    const scannedPages: number[] = [];
    const failedTextPages: number[] = [];
    let selectableChars = 0;
    let truncated = pdf.numPages > maxPagesRead;
    options.onProgress?.({ phase: "reading", completed: 0, total: maxPagesRead });

    // Stop touching later page objects once the text prompt cannot hold
    // additional selectable evidence. Earlier scanned pages remain candidates
    // for a bounded OCR pass because their text precedes later pages.
    for (let pageNumber = 1; pageNumber <= maxPagesRead; pageNumber += 1) {
      assertNotCancelled(options.signal);
      let pageText = "";
      try {
        const page = await pdf.getPage(pageNumber);
        assertNotCancelled(options.signal);
        pageText = await selectablePageText(page);
      } catch (error) {
        assertNotCancelled(options.signal);
        // A damaged page must not discard readable evidence on later pages.
        // Keep its position in pageTexts so page numbering remains stable.
        void error;
        failedTextPages.push(pageNumber);
      }
      assertNotCancelled(options.signal);
      pageTexts.push(pageText);
      selectableChars += pageText.length;
      if (!failedTextPages.includes(pageNumber) && pageText.replace(/\s+/g, "").length < MIN_SELECTABLE_TEXT_CHARS) {
        scannedPages.push(pageNumber);
      }
      options.onProgress?.({ phase: "reading", completed: pageNumber, total: maxPagesRead });
      if (selectableChars >= MAX_EXTRACTED_CHARS) {
        if (pageNumber < pdf.numPages) truncated = true;
        break;
      }
    }
    const pagesRead = pageTexts.length;
    // Selectable text already fills the entire prompt budget: OCR cannot add
    // useful content and would waste rendering/native OCR time on scanned pages.
    const ocrPages = selectableChars >= MAX_EXTRACTED_CHARS
      ? []
      : selectOcrPages(scannedPages, MAX_OCR_PAGES);
    const ocrPageSet = new Set(ocrPages);
    const deferred = scannedPages.filter(pageNumber => !ocrPageSet.has(pageNumber));
    const selectedVisionPages = new Set(selectRepresentativePages(ocrPages, MAX_VISION_PAGES));
    const visionPages: PdfVisionPage[] = [];
    const unreadPages = new Set(ocrPages);
    const startedOcrAt = Date.now();
    let completedOcr = 0;
    let stoppedForTime = false;
    let stoppedForTextBudget = false;
    // Keep OCR work proportional to the maximum text that can enter the prompt.
    // A later OCR batch cannot improve the bounded text preview once full.
    let extractedChars = pageTexts.reduce((sum, text) => sum + text.length, 0);
    options.onProgress?.({ phase: "ocr", completed: 0, total: ocrPages.length });

    for (let offset = 0; offset < ocrPages.length; offset += OCR_BATCH_SIZE) {
      assertNotCancelled(options.signal);
      // Selectable text can fill the prompt before OCR begins. Never render
      // scanned pages or invoke native OCR when none of its output can fit.
      if (extractedChars >= MAX_EXTRACTED_CHARS) {
        stoppedForTextBudget = true;
        break;
      }
      if (Date.now() - startedOcrAt >= MAX_OCR_ELAPSED_MS) {
        stoppedForTime = true;
        break;
      }
      const batchNumbers = ocrPages.slice(offset, offset + OCR_BATCH_SIZE);
      const rendered: { pageNumber: number; dataUrl: string }[] = [];
      for (const pageNumber of batchNumbers) {
        assertNotCancelled(options.signal);
        // Rendering can be expensive even before native OCR starts. Enforce
        // the deadline for each page, not only at batch boundaries.
        if (Date.now() - startedOcrAt >= MAX_OCR_ELAPSED_MS) {
          stoppedForTime = true;
          break;
        }
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
      if (rendered.length && Date.now() - startedOcrAt >= MAX_OCR_ELAPSED_MS) {
        stoppedForTime = true;
      }
      if (rendered.length && !stoppedForTime) {
        try {
          // Only one small batch exists at a time. Cancellation applies
          // between native OCR batches; the current Tauri invoke is not an
          // interruptible task.
          const ocr = await invoke<PdfOcrPageResult[]>("ocr_pdf_pages", { pages: rendered });
          assertNotCancelled(options.signal);
          const allowed = new Set(rendered.map(page => page.pageNumber));
          for (const result of ocr) {
            if (!allowed.delete(result.pageNumber) || typeof result.text !== "string") continue;
            // Bound native OCR output before trimming or Unicode expansion.
            // The native response can be arbitrarily large even when the
            // remaining prompt budget is tiny.
            const remaining = Math.max(0, MAX_EXTRACTED_CHARS - extractedChars);
            if (!remaining) continue;
            const bounded = boundedOcrText(result.text, remaining);
            if (!bounded) continue;
            const existing = pageTexts[result.pageNumber - 1].trim();
            pageTexts[result.pageNumber - 1] = [existing, bounded].filter(Boolean).join("\n");
            extractedChars += bounded.length;
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
      if (stoppedForTime) break;
      if (extractedChars >= MAX_EXTRACTED_CHARS && offset + batchNumbers.length < ocrPages.length) {
        stoppedForTextBudget = true;
        break;
      }
    }
    assertNotCancelled(options.signal);
    const missing = [...new Set([...unreadPages, ...deferred, ...failedTextPages])].sort((a, b) => a - b);
    const ocrFailures = [...unreadPages].sort((a, b) => a - b);
    const warnings = [
      ...(failedTextPages.length ? [
        `PDF text decoding failed on pages ${failedTextPages.join(", ")}. These pages were skipped; the imported document is incomplete.`,
      ] : []),
      ...(ocrFailures.length ? [
        `OCR incomplete on pages ${ocrFailures.join(", ")}. Only available text is included; image-only content on these pages was not analyzed. Install or repair OpenMindAI Lens for full coverage.`,
      ] : []),
      ...(deferred.length ? [
        `OCR work budget reached: ${deferred.length} scanned page(s) were not analyzed (pages ${deferred.join(", ")}). Split the PDF or import a smaller section to retrieve their contents.`,
      ] : []),
      ...(stoppedForTextBudget ? [
        "PDF text budget reached. Later scanned pages were not OCR-analyzed; import a smaller section for additional coverage.",
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
    if (failedTextPages.length || deferred.length || stoppedForTime || stoppedForTextBudget) truncated = true;

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

// Keep memory proportional to the prompt budget, not the native OCR payload.
// Iteration over string code points avoids splitting surrogate pairs. Leading
// whitespace is skipped without materializing a trimmed copy of the full text.
function boundedOcrText(text: string, maxUnits: number): string {
  if (maxUnits <= 0) return "";
  let start = 0;
  let end = 0;
  let used = 0;
  let started = false;
  for (const character of text) {
    if (!started && !character.trim()) {
      start += character.length;
      end = start;
      continue;
    }
    started = true;
    if (used + character.length > maxUnits) break;
    used += character.length;
    end += character.length;
  }
  return text.slice(start, end).trimEnd();
}

async function selectablePageText(page: PDFPageProxy) {
  const textContent = await page.getTextContent();
  const lines: string[] = [];
  let line = "";
  let collected = 0;
  for (const item of textContent.items) {
    if (!("str" in item) || typeof item.str !== "string") continue;
    // A single dense/malformed PDF page must not construct arbitrarily large
    // strings before the caller applies the chat text budget. Slice first so
    // even whitespace normalization cannot allocate the full input string.
    const remaining = MAX_EXTRACTED_CHARS - collected - line.length;
    if (remaining <= 0) break;
    const value = item.str.slice(0, remaining).replace(/\s+/g, " ").trim();
    if (!value) continue;
    const separator = line ? " " : "";
    line += separator + value.slice(0, Math.max(0, remaining - separator.length));
    if (item.hasEOL) {
      const finished = line.trim();
      if (finished) {
        lines.push(finished);
        collected += finished.length + 1;
      }
      line = "";
    }
  }
  if (line.trim() && collected < MAX_EXTRACTED_CHARS) {
    lines.push(line.trim().slice(0, MAX_EXTRACTED_CHARS - collected));
  }
  return lines.filter(Boolean).join("\n").slice(0, MAX_EXTRACTED_CHARS).trim();
}

async function renderPdfPage(page: PDFPageProxy, signal?: PdfExtractionOptions["signal"]) {
  const base = page.getViewport({ scale: 1 });
  // Very large PDF media boxes must not force a minimum scale: a 20,000pt
  // page at scale 0.5 would allocate a 10,000px bitmap despite the cap.
  // Reject invalid dimensions rather than attempting an unbounded canvas.
  if (!Number.isFinite(base.width) || !Number.isFinite(base.height)
      || base.width <= 0 || base.height <= 0) {
    throw new Error("PDF page has invalid dimensions.");
  }
  const scale = Math.min(2, MAX_RENDER_DIMENSION / Math.max(base.width, base.height));
  const viewport = page.getViewport({ scale });
  if (!Number.isFinite(viewport.width) || !Number.isFinite(viewport.height)
      || viewport.width <= 0 || viewport.height <= 0
      || viewport.width > MAX_RENDER_DIMENSION || viewport.height > MAX_RENDER_DIMENSION) {
    throw new Error("Invalid PDF render dimensions.");
  }
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
