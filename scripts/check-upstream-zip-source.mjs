#!/usr/bin/env node
// Temporary source-level gate for ZIP 7 consumers. This checks the exact
// registry crates Cargo resolved, not just an upstream default branch.
// Extraction from untrusted archives must not be introduced unnoticed while
// RUSTSEC-2026-0367/0368/0369 remain temporarily ignored.
import assert from "node:assert/strict";
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";

const EXPECTED = [
  ["any-tts", "0.1.3"],
  ["candle-core", "0.10.2"],
];

function sourceFiles(directory) {
  const files = [];
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const full = join(directory, entry.name);
    if (entry.isDirectory()) files.push(...sourceFiles(full));
    else if (entry.isFile() && entry.name.endsWith(".rs")) files.push(full);
  }
  return files;
}

// A deliberately conservative source search, not a Rust parser or a proof
// that archive input is safe. Flag extraction APIs even if the receiver has
// been renamed, and any fully-qualified ZipArchive extraction call.
function forbiddenExtraction(text) {
  return /\.\s*extract(?:_unchecked|_unsafe)?\s*\(/m.test(text)
    || /\bZipArchive\s*::\s*extract(?:_unchecked|_unsafe)?\s*\(/m.test(text);
}

function selfTest() {
  assert.equal(forbiddenExtraction("let x = archive.by_name(\"data.pkl\")?;"), false);
  assert.equal(forbiddenExtraction("zip.extract(path)?;"), true);
  assert.equal(forbiddenExtraction("zip . extract\n (destination)?;"), true);
  assert.equal(forbiddenExtraction("ZipArchive::extract(&mut archive, path)?;"), true);
  assert.equal(forbiddenExtraction("archive.extract_unchecked(target)?;"), true);
  console.log("ZIP consumer source gate self-tests passed.");
}

selfTest();
const metadataPath = process.argv[2];
if (!metadataPath) {
  console.error("Usage: node scripts/check-upstream-zip-source.mjs cargo-metadata.json");
  process.exit(2);
}
const metadata = JSON.parse(readFileSync(metadataPath, "utf8"));
const packages = metadata.packages ?? [];
let violations = 0;
for (const [name, version] of EXPECTED) {
  const matches = packages.filter(
    (pkg) => pkg.name === name && pkg.version === version &&
      typeof pkg.source === "string" && pkg.source.startsWith("registry+"),
  );
  if (matches.length !== 1) {
    console.error(`Expected exactly one registry package ${name}@${version}; found ${matches.length}`);
    violations++;
    continue;
  }

  const packageRoot = resolve(dirname(matches[0].manifest_path));
  const src = join(packageRoot, "src");
  if (!existsSync(src)) {
    console.error(`Dependency source missing: ${src}`);
    violations++;
    continue;
  }
  let zipReferences = 0;
  let filesInspected = 0;
  for (const file of sourceFiles(src)) {
    filesInspected++;
    const contents = readFileSync(file, "utf8");
    if (/\bZipArchive\b/.test(contents)) zipReferences++;
    if (forbiddenExtraction(contents)) {
      console.error(`Archive extraction API found in ${name}@${version}: ${file}`);
      violations++;
    }
  }
  if (zipReferences === 0) {
    console.error(`No ZipArchive references found in ${name}@${version}; review ZIP use and update this gate`);
    violations++;
  }
  console.log(`${name}@${version}: checked ${filesInspected} Rust files; ${zipReferences} refer to ZipArchive.`);
}
if (violations) {
  console.error(`ZIP source inspection failed with ${violations} issue(s).`);
  process.exit(1);
}
console.log("Pinned ZIP 7 consumers have no extraction API calls matched by this conservative source gate.");
