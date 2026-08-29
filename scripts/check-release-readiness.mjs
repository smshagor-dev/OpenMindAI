import { Buffer } from "node:buffer";
import fs from "node:fs";
import path from "node:path";

const root = process.cwd();

function fail(message) {
  throw new Error(`Release readiness check failed: ${message}`);
}

function readJson(relativePath) {
  return JSON.parse(fs.readFileSync(path.join(root, relativePath), "utf8"));
}

const tauri = readJson(path.join("src-tauri", "tauri.conf.json"));
const bundle = tauri.bundle ?? {};
const updater = tauri.plugins?.updater ?? {};

if (bundle.createUpdaterArtifacts !== true) {
  fail("bundle.createUpdaterArtifacts must be true for Tauri v2 updater releases");
}

const targets = Array.isArray(bundle.targets) ? bundle.targets : [bundle.targets].filter(Boolean);
if (!targets.includes("nsis")) {
  fail("Windows NSIS must remain an enabled bundle target for the configured updater channel");
}

if (bundle.windows?.nsis?.installMode !== "currentUser") {
  fail("Windows NSIS installMode must remain currentUser so passive updates do not require elevation");
}

if (updater.windows?.installMode !== "passive") {
  fail("plugins.updater.windows.installMode must remain passive");
}

if (!Array.isArray(updater.endpoints) || updater.endpoints.length === 0) {
  fail("at least one updater endpoint is required");
}

for (const endpoint of updater.endpoints) {
  if (typeof endpoint !== "string" || !endpoint.startsWith("https://")) {
    fail(`updater endpoint must use HTTPS: ${String(endpoint)}`);
  }
}

const githubLatestEndpoint = updater.endpoints.find((endpoint) =>
  endpoint.endsWith("/releases/latest/download/latest.json"),
);
if (!githubLatestEndpoint) {
  fail("static GitHub updater channel must point to releases/latest/download/latest.json");
}

const publicKey = updater.pubkey;
if (typeof publicKey !== "string" || publicKey.trim() === "") {
  fail("plugins.updater.pubkey is required");
}
if (!/^[A-Za-z0-9+/]+={0,2}$/.test(publicKey) || publicKey.length % 4 !== 0) {
  fail("plugins.updater.pubkey must be canonical base64");
}

let decodedKey;
try {
  decodedKey = Buffer.from(publicKey, "base64").toString("utf8").trim();
} catch {
  fail("plugins.updater.pubkey could not be decoded");
}

const keyLines = decodedKey.split(/\r?\n/).filter(Boolean);
if (
  keyLines.length !== 2 ||
  !/^untrusted comment: minisign public key: [0-9A-F]{16}$/i.test(keyLines[0]) ||
  !/^RW[A-Za-z0-9+/]{54}$/.test(keyLines[1])
) {
  fail("plugins.updater.pubkey is not a structurally valid minisign public key");
}

if (/placeholder|example|replace|changeme/i.test(decodedKey)) {
  fail("plugins.updater.pubkey appears to be a placeholder");
}

console.log(
  `Release readiness config OK: NSIS updater enabled, HTTPS latest.json endpoint configured, minisign public key ${keyLines[0].split(": ").at(-1)} validated structurally.`,
);
