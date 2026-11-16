#!/usr/bin/env node
// Fail-closed verifier for the ZIP 9-only production dependency graph.
// Legacy ZIP versions are always a failing build; there are no audit ignores.
import assert from "node:assert/strict";
import { existsSync, lstatSync, readFileSync, realpathSync, readdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const EXPECTED = new Map([["any-tts", "0.1.3"], ["candle-core", "0.10.2"]]);
const PATCHED_ZIP = "9.0.4";
const LOCK_PATH = join(ROOT, "src-tauri", "Cargo.lock");
const MANIFEST_PATH = join(ROOT, "src-tauri", "Cargo.toml");

function die(message) { throw new Error(message); }

function readPackages(text) {
  const packages = [];
  for (const block of text.split(/(?=^\[\[package\]\]\s*$)/m)) {
    const name = block.match(/^name = "([^"]+)"$/m)?.[1];
    if (!name) continue;
    const version = block.match(/^version = "([^"]+)"$/m)?.[1];
    const source = block.match(/^source = "([^"]+)"$/m)?.[1] ?? null;
    const body = block.match(/^dependencies = \[\n([\s\S]*?)^\]/m)?.[1] ?? "";
    const deps = [...body.matchAll(/^\s*"([^"]+)"[,]?$/gm)].map((m) => m[1]);
    packages.push({ name, version, source, deps });
  }
  return packages;
}

function patchedSourcePaths(manifestText) {
  const lines = manifestText.split(/\r?\n/);
  const positions = lines.flatMap((line, index) =>
    line.trim() === "[patch.crates-io]" ? [index] : []);
  if (!positions.length) return null;
  if (positions.length !== 1) die("Duplicate crates.io patch sections");
  const paths = new Map();
  for (const raw of lines.slice(positions[0] + 1)) {
    const line = raw.trim();
    if (line.startsWith("[")) break;
    if (!line || line.startsWith("#")) continue;
    const m = line.match(/^([a-z][a-z0-9-]*)\s*=\s*\{\s*path\s*=\s*"([^"]+)"\s*\}\s*$/);
    if (!m || paths.has(m[1])) die("Unexpected or duplicate crates.io patch entry");
    paths.set(m[1], m[2]);
  }
  return paths;
}

function classify(packages, manifest) {
  const hasExact = (name, version) =>
    packages.filter((p) => p.name === name && p.version === version).length === 1;
  if (!hasExact("quinn-proto", "0.11.19")) die("Patched quinn-proto version missing");
  if (!hasExact("rustls", "0.23.46")) die("Patched rustls version missing");
  if (!/^zip\s*=\s*"9\.0\.4"\s*$/m.test(manifest))
    die("Direct ZIP requirement must stay exactly 9.0.4");
  if (!/^tauri-plugin-updater\s*=\s*\{[^\n]*default-features\s*=\s*false/m.test(manifest))
    die("Updater must keep default features disabled to prevent legacy ZIP");
  if (packages.some((p) => p.name === "docx-rs"))
    die("Legacy docx-rs must not be reintroduced");
  const updater = packages.filter((p) => p.name === "tauri-plugin-updater");
  if (updater.some((p) => p.deps.some((dep) => dep === "zip" || dep.startsWith("zip "))))
    die("Updater unexpectedly depends on ZIP");
  const zips = packages.filter((p) => p.name === "zip").map((p) => p.version);
  if (zips.length !== 1 || zips[0] !== PATCHED_ZIP)
    die("Production Cargo.lock must contain exclusively one zip@" + PATCHED_ZIP +
      " (found " + zips.join(", ") + ")");
  for (const [name, version] of EXPECTED) {
    const matches = packages.filter((p) => p.name === name && p.version === version);
    if (matches.length !== 1) die("Expected exactly one " + name + "@" + version);
  }
  const paths = patchedSourcePaths(manifest);
  if (!paths || paths.size !== EXPECTED.size) die("Missing/unexpected path-pinned vendor patches");
  for (const [name, version] of EXPECTED) {
    if (paths.get(name) !== '../vendor/' + name + '-' + version)
      die("Unreviewed source patch path for " + name);
    const p = packages.find((item) => item.name === name);
    if (p.source !== null) die("ZIP 9 migrated " + name + " is not a local source");
  }
  return "remediated";
}

function scanRustSources(directory) {
  let files = 0, zipUses = 0;
  const walk = (path) => {
    for (const entry of readdirSync(path, { withFileTypes: true })) {
      const full = join(path, entry.name);
      if (entry.isSymbolicLink()) die("Unexpected source symlink: " + full);
      if (entry.isDirectory()) walk(full);
      if (!entry.isFile() || !entry.name.endsWith(".rs")) continue;
      files++;
      const source = readFileSync(full, "utf8");
      if (/\bZipArchive\b/.test(source)) zipUses++;
      if (/\.\s*extract(?:_unchecked|_unsafe)?\s*\(/m.test(source) ||
          /\bZipArchive\s*::\s*extract(?:_unchecked|_unsafe)?\s*\(/m.test(source))
        die("Unreviewed archive extraction API in " + full);
    }
  };
  walk(directory);
  if (!files || !zipUses) die("Missing expected ZIP-reading implementation in " + directory);
}

function inspectMigratedMetadata(metadataText) {
  const metadata = JSON.parse(metadataText);
  for (const [name, version] of EXPECTED) {
    const matches = (metadata.packages ?? []).filter((p) => p.name === name);
    if (matches.length !== 1 || matches[0].version !== version ||
        matches[0].source !== null)
      die("Expected one local, exact-version production source for " + name);
    const expectedRoot = resolve(ROOT, "vendor", name + "-" + version);
    if (!existsSync(expectedRoot) || lstatSync(expectedRoot).isSymbolicLink())
      die("Missing or symlinked vendored source for " + name);
    if (realpathSync(expectedRoot) !== expectedRoot)
      die("Vendored source escapes expected canonical path for " + name);
    if (resolve(dirname(matches[0].manifest_path)) !== expectedRoot)
      die("Cargo metadata source path mismatch for " + name);
    if (!existsSync(join(expectedRoot, "src")))
      die("Missing vendored crate src/ for " + name);
    scanRustSources(join(expectedRoot, "src"));
    console.log("Inspected local source " + name + "@" + version);
  }
}

function selfTest() {
  const manifest =
    '[package]\nname="fixture"\n' +
    'zip = "9.0.4"\n' +
    'tauri-plugin-updater = { version = "2.10.1", default-features = false }\n';
  const registry = (name, version, zip = "7.2.0") =>
    '[[package]]\nname = "' + name + '"\nversion = "' + version + '"\nsource = "registry+https://github.com/rust-lang/crates.io-index"\n' +
    'dependencies = [\n "zip ' + zip + '",\n]\n';
  const patchedCore = registry("quinn-proto", "0.11.19") +
    registry("rustls", "0.23.46");
  const legacy = readPackages(patchedCore + registry("zip", "7.2.0") +
    registry("zip", "9.0.4") +
    registry("any-tts", "0.1.3") + registry("candle-core", "0.10.2"));
  assert.throws(() => classify(legacy, manifest));
  const patch = '\n[patch.crates-io]\n' +
    'any-tts = { path = "../vendor/any-tts-0.1.3" }\n' +
    'candle-core = { path = "../vendor/candle-core-0.10.2" }\n';
  const upgraded = readPackages(patchedCore + registry("zip", "9.0.4") +
    '[[package]]\nname = "any-tts"\nversion = "0.1.3"\n' +
    '[[package]]\nname = "candle-core"\nversion = "0.10.2"\n');
  assert.equal(classify(upgraded, manifest + patch), "remediated");
  for (const [pkgs, toml] of [
    [legacy, manifest], [legacy, manifest + patch], [upgraded, manifest],
    [upgraded, manifest + patch.replace("any-tts-0.1.3", "outside")],
    [upgraded.concat([{ name: "zip", version: "8.6.0" }]), manifest + patch],
    [upgraded.concat([{ name: "zip", version: "7.2.0" }]), manifest + patch],
    [upgraded, manifest.replace('zip = "9.0.4"', 'zip = "8.6.0"') + patch],
    [upgraded, manifest.replace("default-features = false", "default-features = true") + patch],
    [upgraded.filter((p) => p.name !== "rustls"), manifest + patch],
  ]) assert.throws(() => classify(pkgs, toml));
  console.log("ZIP 9-only production dependency guard self-tests PASSED");
}

try {
  if (process.argv.includes("--self-test")) {
    selfTest();
  } else {
    const packages = readPackages(readFileSync(LOCK_PATH, "utf8"));
    const mode = classify(packages, readFileSync(MANIFEST_PATH, "utf8"));
    if (process.argv.includes("--metadata")) {
      const index = process.argv.indexOf("--metadata");
      const path = process.argv[index + 1];
      if (!path) die("--metadata requires a Cargo metadata JSON path");
      inspectMigratedMetadata(readFileSync(path, "utf8"));
    } else {
      console.log("Verified production ZIP graph: " + mode + ", no legacy advisory ignores");
    }
  }
} catch (error) {
  console.error("ZIP dependency audit selector FAILED: " + error.message);
  process.exitCode = 1;
}
