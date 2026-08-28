import fs from "node:fs";
import path from "node:path";

const root = process.cwd();

function readJson(relativePath) {
  const absolutePath = path.join(root, relativePath);
  return JSON.parse(fs.readFileSync(absolutePath, "utf8"));
}

function readCargoPackageVersion() {
  const cargoToml = fs.readFileSync(path.join(root, "src-tauri", "Cargo.toml"), "utf8");
  const packageSection = cargoToml.match(/^\[package\]\s*([\s\S]*?)(?=^\[|\Z)/m);
  if (!packageSection) {
    throw new Error("Unable to locate [package] in src-tauri/Cargo.toml");
  }
  const versionMatch = packageSection[1].match(/^version\s*=\s*"([^"]+)"\s*$/m);
  if (!versionMatch) {
    throw new Error("Unable to locate package version in src-tauri/Cargo.toml");
  }
  return versionMatch[1];
}

function readRequestedTag() {
  const index = process.argv.indexOf("--tag");
  if (index === -1) {
    return null;
  }
  const value = process.argv[index + 1];
  if (!value || value.startsWith("--")) {
    throw new Error("--tag requires a value such as v2.0.0");
  }
  return value;
}

const packageJson = readJson("package.json");
const packageLock = readJson("package-lock.json");
const tauriConfig = readJson(path.join("src-tauri", "tauri.conf.json"));
const cargoVersion = readCargoPackageVersion();

const versions = new Map([
  ["package.json", packageJson.version],
  ["package-lock.json", packageLock.version],
  ["package-lock.json packages['']", packageLock.packages?.[""]?.version],
  ["src-tauri/tauri.conf.json", tauriConfig.version],
  ["src-tauri/Cargo.toml", cargoVersion],
]);

for (const [source, version] of versions) {
  if (typeof version !== "string" || version.trim() === "") {
    throw new Error(`${source} does not contain a valid version`);
  }
}

const canonicalVersion = packageJson.version;
const mismatches = [...versions.entries()].filter(([, version]) => version !== canonicalVersion);
if (mismatches.length > 0) {
  const details = mismatches.map(([source, version]) => `${source}=${version}`).join(", ");
  throw new Error(`Version mismatch: package.json=${canonicalVersion}; ${details}`);
}

const requestedTag = readRequestedTag();
if (requestedTag !== null) {
  const expectedTag = `v${canonicalVersion}`;
  if (requestedTag !== expectedTag) {
    throw new Error(`Release tag ${requestedTag} does not match synchronized app version ${expectedTag}`);
  }
}

console.log(`Version consistency OK: ${canonicalVersion}${requestedTag ? ` (${requestedTag})` : ""}`);
