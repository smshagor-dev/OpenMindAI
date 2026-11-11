/** One capability model for chat and Studio: route availability, never invent parity. */
export type ExecutionMode = "auto" | "local" | "cloud";
export type ExecutionTask = "chat" | "vision" | "search" | "research" | "scene3d" | "image-to-3d";
export type ExecutionTarget = "cloud" | "local" | "deferred";

export interface RouteInput {
  preference: ExecutionMode;
  task: ExecutionTask;
  online: boolean;
  cloudConnected: boolean;
  cloudModelReady?: boolean;
  localAvailable?: boolean;
  forceLocal?: boolean;
}

export interface RouteDecision {
  target: ExecutionTarget;
  reason: string;
  offline: boolean;
}

/** Local image-to-3D is not a reference-conditioned ML model. Never silently substitute a primitive. */
export function selectExecutionRoute(input: RouteInput): RouteDecision {
  const offline = !input.online;
  const localSupported = input.task !== "image-to-3d" && input.localAvailable !== false;
  if (input.forceLocal) {
    return localSupported
      ? { target: "local", offline: true, reason: "Your prompt explicitly requires private/local-only processing." }
      : { target: "deferred", offline: true, reason: "This task has no equivalent installed local capability; it will not be sent to Cloud." };
  }
  const remoteReady = input.online && input.cloudConnected &&
    (input.task !== "chat" && input.task !== "vision" && input.task !== "search" ||
      input.cloudModelReady !== false);
  if (input.preference === "local" && localSupported) {
    return { target: "local", offline, reason: offline
      ? "Local mode. Internet is unavailable; live web sources cannot be fetched."
      : "Local mode selected." };
  }
  if (input.preference !== "local" && remoteReady) {
    return { target: "cloud", offline: false, reason: "Connected to OpenMindAI Cloud." };
  }
  if (localSupported) {
    return { target: "local", offline, reason: offline
      ? "Internet unavailable. Using the local engine without live web access."
      : "Cloud unavailable or unsupported; using the local engine." };
  }
  if (input.preference === "local" && remoteReady) {
    return { target: "cloud", offline: false, reason:
      "Reference-conditioned image-to-3D is only available on Cloud; using Cloud." };
  }
  return { target: "deferred", offline, reason:
    "Reference-conditioned 3D requires Cloud. No equivalent local image-to-mesh model is installed." };
}

/**
 * API model discovery is a private, authenticated Cloud feature.
 * Never show cached hosted models when signed out, offline or using Local.
 */
/** Foreground Cloud-health probing, aligned with Mobile's bounded backoff. */
export function cloudProbeDelayMs(
  connected: boolean,
  consecutiveFailures: number,
  jitter = 1,
): number {
  if (connected) return 60_000;
  const attempts = Math.max(1, Math.floor(consecutiveFailures));
  const seconds = Math.min(60, 5 * 2 ** Math.min(4, attempts - 1));
  return Math.round(seconds * 1000 * Math.max(0.8, Math.min(1.2, jitter)));
}

/** Mirrors Mobile's explicit offline/local-only privacy override. */
export function requestsLocalOnly(prompt: string): boolean {
  return /\b(offline[ -]only|local[ -]only|on[ -]device only|without internet|no cloud|do not (use|send|upload|share).{0,25}(cloud|internet|server)|don['’]?t (use|send|upload|share).{0,25}(cloud|internet|server)|only (use )?(local|offline))\b|শুধু\s*(লোকাল|অফলাইন)|ইন্টারনেট\s*ছা[ড়ড]া|ক্লাউডে\s*পাঠা(?:বে\s*না|বেন\s*না)|без\s+интернета|только\s+локально|не\s+отправл.{0,20}облак/i.test(prompt);
}

export function shouldShowCloudModels(
  mode: ExecutionMode,
  online: boolean,
  signedIn: boolean,
  availableModelCount: number,
): boolean {
  return mode !== "local" && online && signedIn && availableModelCount > 0;
}

export function describeFailure(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  if (error && typeof error === "object" && "message" in error)
    return String((error as { message: unknown }).message);
  return "Unknown request failure";
}

/** Safe to fall back on these Cloud errors; never retry a POST on the remote server. */
export function isRecoverableCloudFailure(error: unknown): boolean {
  const message = describeFailure(error);
  // Mobile also switches to installed models after a Cloud session expires.
  if (/\bHTTP\s+401\b/i.test(message)) return true;
  if (/\bHTTP\s+(400|401|403|404|405|409|413|415|422)\b/i.test(message) ||
      /invalid|unauthori[sz]ed|forbidden|validation|permission|user-session token|store failed|persist|quota invalid/i.test(message))
    return false;
  return /\bHTTP\s+(408|425|429|500|502|503|504|520|522|524)\b/i.test(message) ||
    /unreachable|connection (?:lost|refused|reset)|offline|no internet|network(?:error| failure)?|timeout|timed out|failed to fetch|cloud api request failed|cloud returned an empty assistant response|server returned non-json response|temporarily unavailable|service unavailable/i.test(message);
}

export class ResponsePersistenceError extends Error {
  constructor(cause: unknown) {
    super("Cloud responded, but saving the reply failed: " + describeFailure(cause));
    this.name = "ResponsePersistenceError";
  }
}

/**
 * Cloud responses are persisted only after success, so a transport failure does
 * not leave duplicate local user/assistant turns. Only Cloud -> Local failover:
 * local inference may already have begun, so do not automatically replay it.
 */
export async function runWithFallback<T>(
  decision: RouteDecision,
  remote: () => Promise<T>,
  local: () => Promise<T>,
  onFallback?: (reason: string) => void,
): Promise<{ result: T; target: "cloud" | "local"; fallback: boolean }> {
  if (decision.target === "deferred") throw new Error(decision.reason);
  if (decision.target === "local") {
    return { result: await local(), target: "local", fallback: false };
  }
  try {
    return { result: await remote(), target: "cloud", fallback: false };
  } catch (error) {
    if (error instanceof ResponsePersistenceError || !isRecoverableCloudFailure(error))
      throw error;
    onFallback?.("Cloud request failed (" + describeFailure(error) + "). Continuing with the local engine.");
    return { result: await local(), target: "local", fallback: true };
  }
}

export function offlineSourceNotice(prompt: string): string {
  return "[Offline mode: The internet is unavailable. Answer from the installed local model and supplied local context only. Do not claim that live web search or current-source verification occurred. Explicitly say when current information cannot be verified.]\n\n" + prompt;
}
