#!/usr/bin/env node
// Temporary, fail-closed gate for upstream ZIP advisories that cannot yet be
// eliminated from the transitive TTS/Candle and DOCX dependency graph.
// See docs/security-transitive-zip-exceptions.md for the security rationale.
import { readFileSync } from "node:fs";

function fail(message) {
  console.error(`Dependency audit guard failed: ${message}`);
  process.exitCode = 1;
}
const lock = readFileSync("src-tauri/Cargo.lock", "utf8");
const manifest = readFileSync("src-tauri/Cargo.toml", "utf8");
const entries = lock.split(/(?=^\[\[package\]\])/m).map((text) => {
  const name = text.match(/^name = "([^"]+)"$/m)?.[1];
  const version = text.match(/^version = "([^"]+)"$/m)?.[1];
  const deps = text.match(/^dependencies = \[\n([\s\S]*?)^\]/m)?.[1] ?? "";
  return { name, version, deps: [...deps.matchAll(/^\s*"([^"]+)"[,]?$/gm)].map((x) => x[1]) };
}).filter((entry) => entry.name);
function hasExact(name, version) {
  return entries.some((entry) => entry.name === name && entry.version === version);
}
if (!hasExact("quinn-proto", "0.11.19")) fail("expected patched quinn-proto 0.11.19");
if (!hasExact("rustls", "0.23.46")) fail("expected patched rustls 0.23.46");
if (!hasExact("zip", "9.0.4")) fail("direct ZIP must be patched 9.0.4");
if (!/\bzip\s*=\s*"9\.0\.4"/.test(manifest))
  fail("direct ZIP must require patched 9.0.4");
if (!/tauri-plugin-updater\s*=\s*\{[^\n]*default-features\s*=\s*false/.test(manifest))
  fail("updater legacy ZIP feature must remain disabled");
if (entries.filter((x) => x.name === "tauri-plugin-updater").some((x) =>
  x.deps.some((d) => d === "zip" || d.startsWith("zip "))))
  fail("legacy updater ZIP transitive dependency was re-enabled");

const allowed = new Map([
  ["7.2.0", ["any-tts@0.1.3", "candle-core@0.10.2"]],
  ["8.6.0", ["docx-rs@0.4.22"]],
]);
for (const item of entries.filter((x) => x.name === "zip")) {
  if (item.version === "9.0.4") continue;
  const owners = entries.filter((p) =>
    p.deps.some((d) => d === `zip ${item.version}` ||
      (d === "zip" && entries.filter((x) => x.name === "zip").length === 1)))
    .map((p) => `${p.name}@${p.version}`).sort();
  const expected = (allowed.get(item.version) ?? []).slice().sort();
  if (!expected.length || JSON.stringify(owners) !== JSON.stringify(expected))
    fail(`unreviewed legacy ZIP ${item.version} consumers: ${owners.join(", ")}`);
}
for (const version of allowed.keys()) {
  if (!hasExact("zip", version)) {
    // Removal is welcome, but never silently replace with another unreviewed old version.
    continue;
  }
}
const expiry = Date.UTC(2026, 10, 17);
if (Date.now() >= expiry) fail("temporary upstream ZIP exception expired 2026-11-17");
if (process.exitCode) process.exit(process.exitCode);
console.log("Temporary transitive ZIP exception is confined to three reviewed upstream dependencies.");
