import fs from "node:fs";
import path from "node:path";

const root = process.cwd();

function readText(relativePath) {
  return fs.readFileSync(path.join(root, relativePath), "utf8");
}

function readJson(relativePath) {
  return JSON.parse(readText(relativePath));
}

function fail(message) {
  throw new Error(`Version consistency check failed: ${message}`);
}

function readCargoPackageVersion() {
  const cargoToml = readText(path.join("src-tauri", "Cargo.toml"));
  let inPackageSection = false;
  for (const rawLine of cargoToml.split(/\r?\n/)) {
    const line = rawLine.trim();
    if (/^\[[^\]]+\]$/.test(line)) {
      inPackageSection = line === "[package]";
      continue;
    }
    if (inPackageSection) {
      const match = line.match(/^version\s*=\s*"([^"]+)"\s*$/);
      if (match) return match[1];
    }
  }
  fail("unable to locate package version in src-tauri/Cargo.toml");
}

function readCargoLockVersion() {
  const cargoLock = readText(path.join("src-tauri", "Cargo.lock"));
  const block = cargoLock
    .split("[[package]]")
    .find((candidate) => /(?:^|\r?\n)name = "open-mind-ai"(?:\r?\n|$)/.test(candidate));
  const match = block?.match(/(?:^|\r?\n)version = "([^"]+)"(?:\r?\n|$)/);
  if (!match) fail("unable to locate open-mind-ai version in src-tauri/Cargo.lock");
  return match[1];
}

function readMarkerVersion() {
  const match = readText("openmindai.marker").match(/^version=(.+)$/m);
  if (!match) fail("openmindai.marker does not contain version=");
  return match[1].trim();
}

function readLauncherVersion(relativePath) {
  const match = readText(relativePath).match(/^(?:rem |# )Version:\s*([^\r\n]+)$/m);
  if (!match) fail(`${relativePath} does not contain a Version header`);
  return match[1].trim();
}

function readRequestedTag() {
  const index = process.argv.indexOf("--tag");
  if (index === -1) return null;
  const value = process.argv[index + 1];
  if (!value || value.startsWith("--")) fail("--tag requires a value such as v3.0.0");
  return value;
}

const packageJson = readJson("package.json");
const packageLock = readJson("package-lock.json");
const tauriConfig = readJson(path.join("src-tauri", "tauri.conf.json"));
const versions = new Map([
  ["package.json", packageJson.version],
  ["package-lock.json", packageLock.version],
  ["package-lock.json packages['']", packageLock.packages?.[""]?.version],
  ["src-tauri/tauri.conf.json", tauriConfig.version],
  ["src-tauri/Cargo.toml", readCargoPackageVersion()],
  ["src-tauri/Cargo.lock", readCargoLockVersion()],
  ["openmindai.marker", readMarkerVersion()],
  ["OpenMindAI-Setup.bat", readLauncherVersion("OpenMindAI-Setup.bat")],
  ["OpenMindAI-Setup.command", readLauncherVersion("OpenMindAI-Setup.command")],
  ["openmindai-setup.sh", readLauncherVersion("openmindai-setup.sh")],
]);

for (const [source, version] of versions) {
  if (typeof version !== "string" || !/^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$/.test(version)) {
    fail(`${source} does not contain a valid semantic version: ${String(version)}`);
  }
}

const canonicalVersion = packageJson.version;
const mismatches = [...versions.entries()].filter(([, version]) => version !== canonicalVersion);
if (mismatches.length > 0) {
  fail(`version mismatch: package.json=${canonicalVersion}; ${mismatches.map(([source, version]) => `${source}=${version}`).join(", ")}`);
}

const expectedTag = `v${canonicalVersion}`;
const releaseNotesRelativePath = path.join(".github", "releases", `${expectedTag}.txt`);
const releaseNotesPath = path.join(root, releaseNotesRelativePath);
if (!fs.existsSync(releaseNotesPath)) fail(`missing release notes: ${releaseNotesRelativePath}`);
const releaseNotes = fs.readFileSync(releaseNotesPath, "utf8");
if (!releaseNotes.includes(`# OpenMindAI ${expectedTag}`)) fail("release notes title does not match synchronized version");
if (/\b(?:TODO|TBD|PLACEHOLDER)\b/i.test(releaseNotes)) fail("release notes contain unfinished placeholder text");

const readme = readText("README.md");
if (!readme.includes(`**Current source version: ${expectedTag}`)) {
  fail("README.md current source version does not match synchronized version");
}

const requestedTag = readRequestedTag();
if (requestedTag !== null && requestedTag !== expectedTag) {
  fail(`release tag ${requestedTag} does not match synchronized app version ${expectedTag}`);
}

console.log(`Version consistency OK: ${canonicalVersion}${requestedTag ? ` (${requestedTag})` : ""}; manifests, locks, marker, launchers, README, and release notes are synchronized.`);
