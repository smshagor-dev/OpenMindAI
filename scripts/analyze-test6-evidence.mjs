import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const repo = path.resolve(scriptDir, "..");
const pack = process.argv[2] ?? "C:/Users/Shahanur Islam/Downloads/OpenMindAI_Test6_Real_Research_Evidence_Pack";
const output = process.argv[3] ?? path.join(repo, "artifacts/acceptance/test6-2026-11-23/evidence-analysis");
const manifest = JSON.parse(fs.readFileSync(path.join(pack, "evidence_manifest.json"), "utf8"));
fs.mkdirSync(output, { recursive: true });

function sha256(bytes) {
  return crypto.createHash("sha256").update(bytes).digest("hex");
}

function parseCsv(text) {
  const rows = [];
  let row = [];
  let cell = "";
  let quoted = false;
  for (let i = 0; i < text.length; i += 1) {
    const char = text[i];
    if (char === '"' && quoted && text[i + 1] === '"') {
      cell += '"';
      i += 1;
    } else if (char === '"') {
      quoted = !quoted;
    } else if (char === "," && !quoted) {
      row.push(cell);
      cell = "";
    } else if ((char === "\n" || char === "\r") && !quoted) {
      if (char === "\r" && text[i + 1] === "\n") i += 1;
      row.push(cell);
      cell = "";
      if (row.some((value) => value.length)) rows.push(row);
      row = [];
    } else {
      cell += char;
    }
  }
  if (quoted) throw new Error("CSV contains an unterminated quoted field.");
  if (cell.length || row.length) {
    row.push(cell);
    if (row.some((value) => value.length)) rows.push(row);
  }
  if (rows.length < 2) throw new Error("CSV has no data rows.");
  const headers = rows[0].map((header) => header.replace(/^\uFEFF/, ""));
  return rows.slice(1).map((values) => Object.fromEntries(headers.map((header, index) => [header, values[index] ?? ""])));
}

function quantile(values, probability) {
  const sorted = [...values].sort((a, b) => a - b);
  const position = (sorted.length - 1) * probability;
  const lower = Math.floor(position);
  const fraction = position - lower;
  return sorted[lower] + (sorted[Math.min(lower + 1, sorted.length - 1)] - sorted[lower]) * fraction;
}

const files = manifest.files.map((entry) => {
  const absolutePath = path.join(pack, ...entry.path.split("/"));
  const bytes = fs.readFileSync(absolutePath);
  const actualHash = sha256(bytes);
  if (bytes.length !== entry.bytes || actualHash !== entry.sha256) {
    throw new Error(`Manifest mismatch for ${entry.path}: ${bytes.length} bytes, SHA-256 ${actualHash}`);
  }
  return { path: entry.path, bytes: bytes.length, sha256: actualHash };
});

const methods = {};
for (const [key, folder, expectedMethod] of [
  ["fedavg", "fedavg", "fedavg"],
  ["per_fedavg", "per_fedavg", "per_fedavg"],
]) {
  const methodRoot = path.join(pack, folder);
  const request = JSON.parse(fs.readFileSync(path.join(methodRoot, "request.json"), "utf8"));
  const result = JSON.parse(fs.readFileSync(path.join(methodRoot, "result.json"), "utf8"));
  const clients = parseCsv(fs.readFileSync(path.join(methodRoot, "clients_deployed.csv"), "utf8"));
  const globalClients = parseCsv(fs.readFileSync(path.join(methodRoot, "clients_global.csv"), "utf8"));
  const rounds = parseCsv(fs.readFileSync(path.join(methodRoot, "rounds.csv"), "utf8"));
  if (request.method !== expectedMethod || request.dataset !== "MNIST" || request.partition !== "iid" || request.seed !== 11 || request.rounds !== 50) {
    throw new Error(`Unexpected run settings for ${folder}: ${JSON.stringify(request)}`);
  }
  if (result.request.method !== expectedMethod || result.request.train_partition_hash !== request.train_partition_hash || result.request.test_partition_hash !== request.test_partition_hash) {
    throw new Error(`Request/result provenance mismatch for ${folder}`);
  }
  if (result.training_uses_test_split !== false || result.evidence_class !== "official_torchvision_heldout") {
    throw new Error(`Unexpected evaluation provenance for ${folder}`);
  }
  if (clients.length !== 20 || rounds.length !== 50) throw new Error(`Unexpected row count for ${folder}`);
  const totalCorrect = clients.reduce((sum, client) => sum + Number(client.n_correct), 0);
  const totalTest = clients.reduce((sum, client) => sum + Number(client.n_test), 0);
  const accuracies = clients.map((client) => Number(client.n_correct) / Number(client.n_test));
  const microAccuracy = totalCorrect / totalTest;
  const globalMicroAccuracy = globalClients.reduce((sum, client) => sum + Number(client.n_correct), 0)
    / globalClients.reduce((sum, client) => sum + Number(client.n_test), 0);
  if (Math.abs(microAccuracy - result.metrics.micro_accuracy) > 1e-12) {
    throw new Error(`Recomputed micro-accuracy differs from result.json for ${folder}`);
  }
  if (Math.abs(globalMicroAccuracy - result.global_reference_metrics.micro_accuracy) > 1e-12) {
    throw new Error(`Recomputed global-reference accuracy differs from result.json for ${folder}`);
  }
  methods[key] = {
    method: request.method,
    evaluation: result.primary_evaluation,
    dataset: request.dataset,
    partition: request.partition,
    seed: request.seed,
    rounds: rounds.length,
    clientCount: clients.length,
    correct: totalCorrect,
    heldout: totalTest,
    microAccuracy,
    meanClientAccuracy: accuracies.reduce((sum, value) => sum + value, 0) / accuracies.length,
    p10ClientAccuracyType7: quantile(accuracies, 0.1),
    worstClientAccuracy: Math.min(...accuracies),
    reportedMicroAccuracy: result.metrics.micro_accuracy,
    globalReferenceAccuracy: globalMicroAccuracy,
    deployedVsGlobalAccuracyDifference: microAccuracy - globalMicroAccuracy,
    trainPartitionHash: request.train_partition_hash,
    testPartitionHash: request.test_partition_hash,
    sourceCommitSha: request.source_commit_sha,
    clients: clients.map((client) => ({
      clientId: Number(client.client_id),
      correct: Number(client.n_correct),
      heldout: Number(client.n_test),
      accuracy: Number(client.n_correct) / Number(client.n_test),
      reportedAccuracy: Number(client.accuracy),
    })),
  };
}

if (methods.fedavg.trainPartitionHash !== methods.per_fedavg.trainPartitionHash || methods.fedavg.testPartitionHash !== methods.per_fedavg.testPartitionHash) {
  throw new Error("The matched methods do not share identical train and test partition hashes.");
}
const perClient = methods.fedavg.clients.map((fedavgClient, index) => {
  const personalizedClient = methods.per_fedavg.clients[index];
  if (fedavgClient.clientId !== personalizedClient.clientId || fedavgClient.heldout !== personalizedClient.heldout) {
    throw new Error(`Client alignment mismatch at index ${index}`);
  }
  return {
    clientId: fedavgClient.clientId,
    fedavgAccuracy: fedavgClient.accuracy,
    perFedavgAccuracy: personalizedClient.accuracy,
    difference: personalizedClient.accuracy - fedavgClient.accuracy,
  };
});
const analysis = {
  scope: "One matched MNIST IID training seed (11); 20 clients, 50 rounds; descriptive comparison only.",
  manifestFiles: files,
  methods,
  comparison: {
    matchedTrainAndTestPartitionHashes: true,
    seedPairs: 1,
    fedavgMinusPerFedavgMicroAccuracy: methods.fedavg.microAccuracy - methods.per_fedavg.microAccuracy,
    fedavgMinusPerFedavgPercentagePoints: 100 * (methods.fedavg.microAccuracy - methods.per_fedavg.microAccuracy),
    perClientOutcomes: perClient,
    inferentialStatistics: "Not calculated: one matched training-seed pair; clients are not independent seed replicates.",
    roundsCsvUse: "Describes selected-client counts and cumulative optimizer steps; no per-round accuracy was present or plotted.",
  },
};

const tableRows = [methods.fedavg, methods.per_fedavg].map((method) => [
  method.method,
  method.evaluation,
  method.clients.length,
  method.rounds,
  method.correct,
  method.heldout,
  method.microAccuracy.toFixed(6),
  method.meanClientAccuracy.toFixed(6),
  method.p10ClientAccuracyType7.toFixed(6),
  method.worstClientAccuracy.toFixed(6),
]);
const table = ["method,evaluation,clients,rounds,correct,heldout,micro_accuracy,mean_client_accuracy,p10_type7,worst_client_accuracy", ...tableRows.map((row) => row.join(","))].join("\n") + "\n";
fs.writeFileSync(path.join(output, "table-1-heldout-metrics.csv"), table);
fs.writeFileSync(path.join(output, "analysis.json"), `${JSON.stringify(analysis, null, 2)}\n`);

const width = 900;
const height = 600;
const left = 100;
const right = 850;
const xMin = 0.94;
const xMax = 1.0;
const x = (value) => left + ((value - xMin) / (xMax - xMin)) * (right - left);
const ticks = [0.94, 0.95, 0.96, 0.97, 0.98, 0.99, 1.0];
const clientRows = perClient.map((client, index) => {
  const y = 110 + index * 20;
  return `<text x="38" y="${y + 4}" font-size="12">${client.clientId}</text><line x1="${x(client.fedavgAccuracy).toFixed(1)}" y1="${y - 3}" x2="${x(client.perFedavgAccuracy).toFixed(1)}" y2="${y + 3}" stroke="#8993a5" stroke-width="1"/><circle cx="${x(client.fedavgAccuracy).toFixed(1)}" cy="${y - 3}" r="4" fill="#2563eb"/><circle cx="${x(client.perFedavgAccuracy).toFixed(1)}" cy="${y + 3}" r="4" fill="#dc2626"/>`;
}).join("\n");
const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}">
<rect width="100%" height="100%" fill="white"/><text x="38" y="36" font-family="Arial,sans-serif" font-size="20" font-weight="700">Held-out accuracy by client</text>
<text x="38" y="59" font-family="Arial,sans-serif" font-size="12" fill="#475569">Matched MNIST/IID partition, seed 11; descriptive client outcomes (not independent replicates)</text>
${ticks.map((tick) => `<line x1="${x(tick).toFixed(1)}" y1="84" x2="${x(tick).toFixed(1)}" y2="520" stroke="#e2e8f0"/><text x="${x(tick).toFixed(1)}" y="542" text-anchor="middle" font-family="Arial,sans-serif" font-size="11">${(tick * 100).toFixed(0)}%</text>`).join("\n")}
${clientRows}
<circle cx="235" cy="578" r="4" fill="#2563eb"/><text x="245" y="582" font-family="Arial,sans-serif" font-size="12">FedAvg deployed (shared global)</text><circle cx="485" cy="578" r="4" fill="#dc2626"/><text x="495" y="582" font-family="Arial,sans-serif" font-size="12">Per-FedAvg deployed (personalized)</text>
<text x="475" y="565" text-anchor="middle" font-family="Arial,sans-serif" font-size="12">Held-out client accuracy</text></svg>`;
fs.writeFileSync(path.join(output, "figure-1-client-accuracy.svg"), svg);
const analyzerBytes = fs.readFileSync(fileURLToPath(import.meta.url));
const outputNames = ["analysis.json", "table-1-heldout-metrics.csv", "figure-1-client-accuracy.svg", "article-draft.md"];
const runManifest = {
  runSettings: {
    methodFiles: ["fedavg/clients_deployed.csv", "fedavg/clients_global.csv", "fedavg/request.json", "fedavg/result.json", "fedavg/rounds.csv", "per_fedavg/clients_deployed.csv", "per_fedavg/clients_global.csv", "per_fedavg/request.json", "per_fedavg/result.json", "per_fedavg/rounds.csv"],
    primaryOutcome: "sum(n_correct) / sum(n_test) from each method's clients_deployed.csv",
    p10Convention: "Type 7 linear interpolation over per-client n_correct / n_test values",
    inference: "Descriptive only; one matched seed pair",
  },
  sourceManifestSha256: sha256(fs.readFileSync(path.join(pack, "evidence_manifest.json"))),
  analyzerSha256: sha256(analyzerBytes),
  outputs: outputNames.filter((name) => fs.existsSync(path.join(output, name))).map((name) => ({
    path: name,
    sha256: sha256(fs.readFileSync(path.join(output, name))),
    bytes: fs.statSync(path.join(output, name)).size,
  })),
};
fs.writeFileSync(path.join(output, "run-manifest.json"), `${JSON.stringify(runManifest, null, 2)}\n`);
console.log(JSON.stringify({ output, methodMetrics: Object.fromEntries(Object.entries(methods).map(([key, value]) => [key, { evaluation: value.evaluation, correct: value.correct, heldout: value.heldout, microAccuracy: value.microAccuracy, meanClientAccuracy: value.meanClientAccuracy, p10ClientAccuracyType7: value.p10ClientAccuracyType7, worstClientAccuracy: value.worstClientAccuracy, reportedMicroAccuracy: value.reportedMicroAccuracy, globalReferenceAccuracy: value.globalReferenceAccuracy }])), fileCount: files.length, figure: "figure-1-client-accuracy.svg", table: "table-1-heldout-metrics.csv" }, null, 2));
