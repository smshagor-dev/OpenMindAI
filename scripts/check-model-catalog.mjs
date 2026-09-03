import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const catalogPath = path.join(root, "src-tauri", "model-catalog.json");
const catalog = JSON.parse(fs.readFileSync(catalogPath, "utf8"));

if (!Number.isInteger(catalog.catalogVersion) || catalog.catalogVersion < 7) {
  throw new Error("model catalog version must be at least 7");
}
if (!Array.isArray(catalog.models) || catalog.models.length === 0) {
  throw new Error("model catalog must contain models");
}

const ids = new Set();
const destinations = new Set();
let requiredCount = 0;

for (const model of catalog.models) {
  if (!model.id || ids.has(model.id)) throw new Error(`duplicate or missing model id: ${model.id}`);
  ids.add(model.id);

  if (typeof model.name !== "string" || !model.name.startsWith("OpenMindAI")) {
    throw new Error(`model ${model.id} must use an OpenMindAI display name`);
  }
  if (model.required) requiredCount += 1;

  if (model.download) {
    const destination = model.download.destinationDir;
    if (!destination || destinations.has(destination)) {
      throw new Error(`duplicate or missing destination for ${model.id}: ${destination}`);
    }
    destinations.add(destination);
    if (!model.download.filenamePattern) {
      throw new Error(`missing filename pattern for ${model.id}`);
    }
  }
}

if (requiredCount !== 1) {
  throw new Error(`expected exactly one required model, found ${requiredCount}`);
}

const expected = new Map([
  ["gpt-oss-20b-mxfp4", "OpenMindAI Forge"],
  ["gpt-oss-120b-mxfp4", "OpenMindAI Forge Max"],
  ["gemma4-e2b-q4", "OpenMindAI Flash"],
  ["gemma4-e4b-q4", "OpenMindAI Flash Plus"],
  ["gemma4-12b-q4", "OpenMindAI Vision"],
  ["gemma4-26b-a4b-q4", "OpenMindAI Vision Pro"],
  ["gemma4-31b-q4", "OpenMindAI Vision Max"],
  ["nemotron3-nano-4b-q4km", "OpenMindAI Agent Lite"],
  ["nemotron3-nano-30b-a3b-q4km", "OpenMindAI Agent"],
  ["nemotron35-lightning-30b-a3b-q4", "OpenMindAI Agent Lightning"],
  ["nemotron3-super-120b-q4k", "OpenMindAI Agent Pro"],
]);

for (const [id, name] of expected) {
  const model = catalog.models.find((candidate) => candidate.id === id);
  if (!model) throw new Error(`missing expected open model: ${id}`);
  if (model.name !== name) throw new Error(`${id} must be displayed as ${name}`);
}

console.log(`Model catalog OK: ${catalog.models.length} models, version ${catalog.catalogVersion}.`);
