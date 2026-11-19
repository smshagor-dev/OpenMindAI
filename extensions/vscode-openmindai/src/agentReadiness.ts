import { CodingAgentInfo, codingAgentHeaders } from "./runtimeDiscovery";

// Kept free of the vscode module so readiness handling can be unit tested under plain Node.

/** Slightly above the desktop's 300 s startup bound so the desktop reports the timeout. */
export const AGENT_READY_TIMEOUT_MS = 330_000;
export const AGENT_READY_POLL_MS = 1_000;

export const DISABLED_MESSAGE =
  "Coding Workspace is disabled in OpenMindAI Settings -> Agent Setup, so the VS Code coding agent is unavailable. Enable it there and try again.";

export interface ReadinessOptions {
  timeoutMs?: number;
  pollMs?: number;
  sleep?: (ms: number) => Promise<void>;
  now?: () => number;
  fetchJson?: (url: string, method: "GET" | "POST", headers: Record<string, string>) => Promise<unknown>;
}

/** Display name used in progress text, e.g. "OpenAgent Lite". */
export function agentName(info: CodingAgentInfo | undefined): string {
  return info?.model?.name ?? "the coding agent";
}

export function loadingMessage(info: CodingAgentInfo): string {
  const seconds = Math.round((info.runtime?.elapsedMs ?? 0) / 1000);
  return `OpenMindAI is loading ${agentName(info)}…${seconds > 0 ? ` ${seconds}s` : ""}`;
}

/**
 * Makes sure the Agent Setup coding agent is loaded before a request is sent, so a slow cold
 * start shows up as progress instead of a failed or timed-out AI request. Starts the load if
 * the agent is stopped, then polls the desktop status until it is ready.
 */
export async function ensureCodingAgentReady(
  endpoint: string,
  token: string | undefined,
  onProgress: (message: string) => void,
  options: ReadinessOptions = {}
): Promise<CodingAgentInfo> {
  const timeoutMs = options.timeoutMs ?? AGENT_READY_TIMEOUT_MS;
  const pollMs = options.pollMs ?? AGENT_READY_POLL_MS;
  const sleep = options.sleep ?? ((ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms)));
  const now = options.now ?? Date.now;
  const fetchJson = options.fetchJson ?? defaultFetchJson;
  const headers = codingAgentHeaders(token);
  const statusUrl = `${endpoint}/openmindai/coding-agent`;

  const started = now();
  let info = (await fetchJson(statusUrl, "GET", headers)) as CodingAgentInfo;
  let startRequested = false;
  for (;;) {
    assertUsable(info);
    const state = info.runtime?.state;
    if (state === "ready" || state === undefined) return info;
    if (state === "error" && startRequested) {
      throw new Error(`${agentName(info)} could not be loaded: ${info.runtime?.error ?? "unknown error"}`);
    }
    if ((state === "stopped" || state === "error") && !startRequested) {
      startRequested = true;
      onProgress(`Starting ${agentName(info)}…`);
      info = (await fetchJson(`${statusUrl}/start`, "POST", headers)) as CodingAgentInfo;
      continue;
    }
    if (state === "starting" || state === "loadingModel") onProgress(loadingMessage(info));
    if (now() - started >= timeoutMs) {
      throw new Error(
        `${agentName(info)} is still loading after ${Math.round(timeoutMs / 1000)} seconds. Check the OpenMindAI desktop app (Settings -> Agent Setup).`
      );
    }
    await sleep(pollMs);
    info = (await fetchJson(statusUrl, "GET", headers)) as CodingAgentInfo;
  }
}

function assertUsable(info: CodingAgentInfo): void {
  if (info?.configured === false || info?.runtime?.state === "notInstalled") {
    throw new Error(
      info.message ??
        "No OpenAgent coding model is installed. Download one in OpenMindAI Settings -> Agent Setup."
    );
  }
  if (info?.codingEnabled === false || info?.runtime?.state === "disabled") {
    throw new Error(DISABLED_MESSAGE);
  }
}

async function defaultFetchJson(url: string, method: "GET" | "POST", headers: Record<string, string>): Promise<unknown> {
  const response = await fetch(url, { method, headers });
  const text = await response.text();
  const data = text.trim() ? (JSON.parse(text) as { error?: { message?: string } }) : {};
  if (!response.ok) {
    throw new Error(data.error?.message ?? `OpenMindAI coding agent status failed with HTTP ${response.status}`);
  }
  return data;
}
