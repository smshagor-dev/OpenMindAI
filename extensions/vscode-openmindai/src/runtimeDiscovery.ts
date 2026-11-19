import * as cp from "child_process";
import * as fs from "fs";
import * as http from "http";
import * as os from "os";
import * as path from "path";

// Kept free of the vscode module so discovery can be unit tested under plain Node.

export const NATIVE_INTERFACE_ENDPOINT = "http://127.0.0.1:11435";
export const CODING_AGENT_SERVICE = "openmindai-coding-agent";
export const CODING_AGENT_DESCRIPTOR_FILE = "coding-agent-endpoint.json";
const LOOPBACK_ENDPOINT = /^http:\/\/(127\.0\.0\.1|localhost|\[::1\]):\d+$/i;

/** Where the request goes; decides which request fields are safe to send. */
export type RuntimeKind = "coding-agent" | "llama-server" | "native-gateway" | "custom";

export type CodingAgentState =
  | "disabled"
  | "notInstalled"
  | "stopped"
  | "starting"
  | "loadingModel"
  | "ready"
  | "error";

/** Live runtime state the desktop reports for the coding agent. */
export interface CodingAgentRuntime {
  state: CodingAgentState;
  elapsedMs?: number | null;
  lastStartupMs?: number | null;
  error?: string | null;
  message?: string | null;
  effectiveContext?: number | null;
  runtimePlacement?: "shared" | "dedicated" | null;
}

/** The coding agent selected in the desktop app's Settings -> Agent Setup. */
export interface CodingAgentInfo {
  service?: string;
  instanceId?: string;
  configured: boolean;
  codingEnabled?: boolean;
  message?: string | null;
  model?: {
    id: string;
    name: string;
    displayName?: string;
    family?: string | null;
    repository?: string | null;
    quantization?: string | null;
  } | null;
  contextSize?: number | null;
  maxOutputTokens?: number;
  runtime?: CodingAgentRuntime;
}

export interface RuntimeTarget {
  endpoint: string;
  kind: RuntimeKind;
  /** Present for kind "coding-agent". */
  agent?: CodingAgentInfo;
  /** Per-session token for the coding-agent endpoint, read from the desktop descriptor. */
  token?: string;
  /** Model the endpoint reports as loaded, for kinds that bypass Agent Setup. */
  loadedModel?: string;
}

/** Headers every request to the coding-agent endpoint must carry. */
export function codingAgentHeaders(token?: string): Record<string, string> {
  const headers: Record<string, string> = { "X-OpenMindAI-Client": "vscode" };
  if (token) headers["X-OpenMindAI-Token"] = token;
  return headers;
}

/** True when a process with this id exists (EPERM still means it exists). */
export function processAlive(pid: unknown): boolean {
  if (typeof pid !== "number" || !Number.isInteger(pid) || pid <= 0) return false;
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    return (error as NodeJS.ErrnoException).code === "EPERM";
  }
}

/**
 * Candidate locations of the descriptor the desktop app writes next to its installation
 * pointer (Rust: dirs::config_local_dir()/OpenMindAI).
 */
export function codingAgentDescriptorPaths(
  env: NodeJS.ProcessEnv = process.env,
  platform: NodeJS.Platform = process.platform,
  home: string = os.homedir()
): string[] {
  const paths: string[] = [];
  const override = env.OPENMINDAI_CODING_AGENT_DESCRIPTOR?.trim();
  if (override) paths.push(override);
  let configDir: string | undefined;
  if (platform === "win32") {
    configDir = env.LOCALAPPDATA || path.join(home, "AppData", "Local");
  } else if (platform === "darwin") {
    configDir = path.join(home, "Library", "Application Support");
  } else {
    configDir = env.XDG_CONFIG_HOME || path.join(home, ".config");
  }
  paths.push(path.join(configDir, "OpenMindAI", CODING_AGENT_DESCRIPTOR_FILE));
  return paths;
}

/**
 * Reads the descriptor and confirms the advertised endpoint is the live coding agent:
 * the endpoint must be loopback, the owning process must exist, and the endpoint must
 * accept the session token and report the descriptor's instance id. A stale descriptor
 * therefore never leads to an unrelated process that later took the same port.
 */
export async function findCodingAgentEndpoint(
  descriptorPaths: string[] = codingAgentDescriptorPaths(),
  probe: JsonProbe = requestJson,
  isAlive: (pid: unknown) => boolean = processAlive
): Promise<RuntimeTarget | undefined> {
  for (const descriptorPath of descriptorPaths) {
    let descriptor: { service?: unknown; endpoint?: unknown; pid?: unknown; instanceId?: unknown; token?: unknown };
    try {
      descriptor = JSON.parse(fs.readFileSync(descriptorPath, "utf8"));
    } catch {
      continue;
    }
    if (descriptor.service !== CODING_AGENT_SERVICE || typeof descriptor.endpoint !== "string") continue;
    const endpoint = descriptor.endpoint.trim().replace(/\/+$/, "");
    if (!LOOPBACK_ENDPOINT.test(endpoint)) continue;
    if (descriptor.pid !== undefined && !isAlive(descriptor.pid)) continue;
    const token = typeof descriptor.token === "string" ? descriptor.token : undefined;
    try {
      const info = (await probe(`${endpoint}/openmindai/coding-agent`, 5000, codingAgentHeaders(token))) as CodingAgentInfo;
      if (info?.service !== CODING_AGENT_SERVICE) continue;
      if (typeof descriptor.instanceId === "string" && info.instanceId !== descriptor.instanceId) continue;
      return { endpoint, kind: "coding-agent", agent: info, token };
    } catch {
      // Desktop app not running; the descriptor is stale.
    }
  }
  return undefined;
}

/**
 * True when a descriptor names a running desktop process. Then a failed probe means the
 * coding agent is busy or restarting, not absent, and callers must not fall back to a
 * model outside Agent Setup.
 */
export function hasLiveCodingAgentDescriptor(
  descriptorPaths: string[] = codingAgentDescriptorPaths(),
  isAlive: (pid: unknown) => boolean = processAlive
): boolean {
  return descriptorPaths.some((descriptorPath) => {
    try {
      const descriptor = JSON.parse(fs.readFileSync(descriptorPath, "utf8"));
      return (
        descriptor.service === CODING_AGENT_SERVICE &&
        typeof descriptor.endpoint === "string" &&
        LOOPBACK_ENDPOINT.test(descriptor.endpoint.trim().replace(/\/+$/, "")) &&
        isAlive(descriptor.pid)
      );
    } catch {
      return false;
    }
  });
}

/** Display name of the model a llama.cpp-style /v1/models reports, if any. */
export function loadedModelName(models: unknown): string | undefined {
  const data = (models as { data?: Array<{ id?: unknown }> } | undefined)?.data;
  const id = Array.isArray(data) ? data[0]?.id : undefined;
  if (typeof id !== "string" || !id.trim()) return undefined;
  return id.replace(/\\/g, "/").split("/").pop();
}

export function describeTarget(target: RuntimeTarget): string {
  if (target.kind === "coding-agent") {
    const model = target.agent?.model;
    if (!model) return "OpenMindAI coding agent";
    const detail = model.displayName ?? model.repository;
    return `OpenMindAI coding agent: ${model.name}${detail ? ` (${detail})` : ""}`;
  }
  if (target.kind === "llama-server") {
    return `desktop llama-server${target.loadedModel ? ` with ${target.loadedModel} loaded` : ""} (bypasses Agent Setup)`;
  }
  if (target.kind === "native-gateway") return "native interface fallback";
  return `custom endpoint ${target.endpoint}`;
}

export type JsonProbe = (url: string, timeoutMs: number, headers?: Record<string, string>) => Promise<unknown>;

export function isNativeInterfaceEndpoint(endpoint: string): boolean {
  const normalized = endpoint.trim().replace(/\/+$/, "").toLowerCase();
  return /^https?:\/\/(localhost|127\.0\.0\.1|\[::1\]):11435$/.test(normalized);
}

export function parseListeningPorts(stdout: string): number[] {
  return stdout
    .split(/\r?\n/)
    .map((line) => Number(line.trim()))
    .filter((port) => Number.isInteger(port) && port > 0);
}

/** Returns the first port that answers like an OpenMindAI desktop llama-server. */
export async function findDesktopRuntimeEndpoint(
  ports: number[],
  probe: JsonProbe = requestJson
): Promise<string | undefined> {
  for (const port of ports) {
    const endpoint = `http://127.0.0.1:${port}`;
    try {
      await probe(`${endpoint}/health`, 1000);
      await probe(`${endpoint}/v1/models`, 1500);
      return endpoint;
    } catch {
      // Keep looking; other local llama-server instances may not be OpenMindAI's.
    }
  }
  return undefined;
}

export async function listeningPortsByProcessName(processName: string): Promise<number[]> {
  return new Promise((resolve) => {
    const escapedName = processName.replace(/'/g, "''");
    const command = [
      "$connections = Get-NetTCPConnection -LocalAddress 127.0.0.1 -State Listen -ErrorAction SilentlyContinue;",
      "$connections | ForEach-Object {",
      "  $process = Get-Process -Id $_.OwningProcess -ErrorAction SilentlyContinue;",
      `  if ($process -and $process.ProcessName -eq '${escapedName}') { $_.LocalPort }`,
      "}"
    ].join(" ");
    cp.exec(`powershell -NoProfile -Command "${command}"`, (error, stdout) => {
      if (error) {
        resolve([]);
        return;
      }
      resolve(parseListeningPorts(stdout));
    });
  });
}

export function requestJson(url: string, timeoutMs: number, headers?: Record<string, string>): Promise<unknown> {
  return new Promise((resolve, reject) => {
    const options = { timeout: timeoutMs, headers: { "X-OpenMindAI-Client": "vscode", ...headers } };
    const request = http.get(url, options, (response) => {
      let body = "";
      response.setEncoding("utf8");
      response.on("data", (chunk) => {
        body += chunk;
      });
      response.on("end", () => {
        if (!response.statusCode || response.statusCode < 200 || response.statusCode >= 300) {
          reject(new Error(`HTTP ${response.statusCode}: ${body}`));
          return;
        }
        try {
          resolve(body ? JSON.parse(body) : {});
        } catch (error) {
          reject(error);
        }
      });
    });
    request.on("timeout", () => {
      request.destroy(new Error("request timed out"));
    });
    request.on("error", reject);
  });
}
