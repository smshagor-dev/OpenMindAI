import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";
import { getDocument, OPS } from "pdfjs-dist/legacy/build/pdf.mjs";
import { createCanvas } from "@napi-rs/canvas";

const [pdfPath, outDir, reportPath] = process.argv.slice(2);
if (!pdfPath || !outDir || !reportPath) throw new Error("Usage: node inspect-test6-3c-pdf.mjs <pdf> <output-dir> <report.json>");
const data = new Uint8Array(fs.readFileSync(pdfPath));
const pdf = await getDocument({ data, useSystemFonts: false, isEvalSupported: false }).promise;
const pages = [];
for (let number = 1; number <= pdf.numPages; number++) {
  const page = await pdf.getPage(number);
  const text = await page.getTextContent();
  const words = text.items.filter(item => "str" in item).map(item => item.str).join(" ");
  const ops = await page.getOperatorList();
  const imageOps = ops.fnArray.map((fn, index) => ({ fn, index })).filter(item => item.fn === OPS.paintImageXObject || item.fn === OPS.paintImageXObjectRepeat || item.fn === OPS.paintInlineImageXObject);
  const viewport = page.getViewport({ scale: 1.5 });
  const canvas = createCanvas(Math.ceil(viewport.width), Math.ceil(viewport.height));
  await page.render({ canvasContext: canvas.getContext("2d"), viewport }).promise;
  const png = canvas.toBuffer("image/png");
  const renderPath = path.join(outDir, `native-figure-pdf-page-${number}.png`);
  fs.mkdirSync(outDir, { recursive: true });
  fs.writeFileSync(renderPath, png);
  pages.push({ number, text: words, imageXObjectCount: imageOps.length, imageOperatorIndexes: imageOps.map(op => op.index), renderPath, renderSha256: crypto.createHash("sha256").update(png).digest("hex") });
}
const imagePages = pages.filter(page => page.imageXObjectCount > 0);
const report = { pdfPath, fileSha256: crypto.createHash("sha256").update(fs.readFileSync(pdfPath)).digest("hex"), pageCount: pdf.numPages, totalImageXObjects: pages.reduce((sum, page) => sum + page.imageXObjectCount, 0), pages: pages.map(({ number, text, imageXObjectCount, imageOperatorIndexes, renderPath, renderSha256 }) => ({ number, text, imageXObjectCount, imageOperatorIndexes, renderPath, renderSha256 })), figurePageHasCaption: imagePages.some(page => page.text.includes("Figure 1. Held-out client accuracy by round")) };
fs.writeFileSync(reportPath, `${JSON.stringify(report, null, 2)}\n`);
console.log(JSON.stringify(report, null, 2));
if (!report.totalImageXObjects || !report.figurePageHasCaption) process.exitCode = 1;
