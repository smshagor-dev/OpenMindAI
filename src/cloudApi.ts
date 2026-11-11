import { invoke } from "@tauri-apps/api/core";
import type { Message } from "./types";
import { api as local } from "./api_legacy";
import { describeFailure, ResponsePersistenceError } from "./lib/executionRouting";
export type { ExecutionMode } from "./lib/executionRouting";

export type StudioSection = "research-studio" | "3d-studio" | "web-search";
export type CloudConnection = {
  connected: boolean;
  user: { name?: string; user?: { name?: string } } | null;
  message: string | null;
};
export type CloudModel = { id: string; name?: string; available?: boolean; active?: boolean; operation?: string; supports_vision?: boolean };
export type StudioJob = {
  id: string;
  status: string;
  output?: unknown;
  result?: unknown;
  error?: string | null;
  asset_id?: number | null;
  events?: unknown[];
};
export type ResearchProject = { id: number; title: string; status?: string; prompt?: string };
export type QueuedScene = {
  id: string; kind: "scene3d"; requestKey: string;
  payload: { prompt: string; mode: "object" | "architecture" };
  status: "queued" | "sending" | "review";
  note: string | null; createdAt: string;
};

const call = <T>(command: string, args?: Record<string, unknown>) => {
  if (!("__TAURI_INTERNALS__" in window))
    return Promise.reject(new Error("Cloud features require the OpenMindAI desktop application."));
  return invoke<T>(command, args);
};

let oneTimeOutboxRecovery: Promise<void> | null = null;

export const cloud = {
  status: () => call<CloudConnection>("cloud_status"),
  login: (email: string, password: string, code?: string) =>
    call<CloudConnection>("cloud_login", { email, password, code: code || null }),
  connect: (token: string) => call<CloudConnection>("cloud_connect", { token }),
  disconnect: () => call<void>("cloud_disconnect"),
  request: <T>(path: string, method = "GET", payload?: unknown) =>
    call<T>("cloud_request", { path, method, payload: payload ?? null }),
  gpu: <T>(path: string, method = "GET", payload?: unknown) =>
    call<T>("local_gpu_request", { path, method, payload: payload ?? null }),
  models: async () => {
    const response = await cloud.request<{ data: CloudModel[] }>("mobile/models");
    if (!Array.isArray(response.data)) throw new Error("Cloud returned an invalid API model catalog.");
    // Match Mobile's server-side capability filters. A listed-but-unavailable
    // model must never appear selectable or be submitted for chat.
    return response.data.filter((model) =>
      typeof model.id === "string" && model.id.trim().length > 0 &&
      model.available === true && model.operation === "chat.completions"
    );
  },
  search: (query: string) =>
    cloud.request<{ results: Array<{ title?: string; url?: string; snippet?: string; description?: string }> }>(
      "web-search", "POST", { query: query.slice(0, 500) },
    ),
  researchProjects: async (): Promise<ResearchProject[]> => {
    const page = await cloud.request<{ data: ResearchProject[] }>("research/projects");
    return Array.isArray(page.data) ? page.data : [];
  },
  createResearch: (title: string, prompt: string) =>
    cloud.request<ResearchProject>("research/projects", "POST", {
      title: title.slice(0, 200), prompt: prompt.slice(0, 16000), output_language: "auto",
    }),
  startResearch: (id: number) =>
    cloud.request<unknown>("research/projects/" + id + "/runs", "POST", {}),
  researchDetail: (id: number) =>
    cloud.request<Record<string, unknown>>("research/projects/" + id),
  researchControl: (id: number, action: "pause" | "resume" | "cancel") =>
    cloud.request<unknown>("research/projects/" + id + "/control", "POST", { action }),
  studioEngines: () => cloud.request<Record<string, unknown>>("3d-studio/engines"),
  uploadStudioImage: (name: string, base64: string, preview: string) =>
    cloud.request<{ id: number }>("3d-scenes/uploads", "POST", {
      name, data: base64, image_preview: preview,
    }),
  imageToMesh: (uploadId: number) =>
    cloud.request<StudioJob | { asset: { id: number } }>("3d-studio/jobs/image-model", "POST", {
      upload_id: uploadId, request_key: crypto.randomUUID(),
    }),
  studioScene: (prompt: string, mode: "object" | "architecture", requestKey: string = crypto.randomUUID()) =>
    cloud.request<StudioJob>("3d-studio/jobs/scene", "POST", {
      prompt: prompt.slice(0, 8000), mode, request_key: requestKey,
    }),
  studioJob: (id: string) => cloud.request<StudioJob>("3d-studio/jobs/" + id),
  downloadMesh: (assetId: number) =>
    call<{ filename: string; data_base64: string }>("cloud_download_mesh", { assetId }),
  startGpu: () => call<{ gpu_active: boolean; gpu?: string; engine?: string }>("local_gpu_start"),
  stopGpu: () => call<void>("local_gpu_stop"),
  gpuHealth: () => cloud.gpu<{ gpu_active: boolean; gpu?: string; engine?: string }>("health"),
  gpuScene: (prompt: string, mode: "object" | "architecture", requestKey: string = crypto.randomUUID()) =>
    cloud.gpu<StudioJob>("jobs", "POST", {
      prompt: prompt.slice(0, 8000), mode, request_key: requestKey,
    }),
  gpuJob: (id: string) => cloud.gpu<StudioJob>("jobs/" + id),
  gpuMesh: (id: string) =>
    cloud.gpu<{ filename: string; data_base64: string }>("jobs/" + id + "/mesh"),
  outboxRecover: () => {
    if (!oneTimeOutboxRecovery)
      oneTimeOutboxRecovery = call<void>("studio_outbox_recover").catch((error: unknown) => {
        oneTimeOutboxRecovery = null;
        throw error;
      });
    return oneTimeOutboxRecovery;
  },
  outboxList: () => call<QueuedScene[]>("studio_outbox_list"),
  queueScene: (prompt: string, mode: "object" | "architecture", requestKey: string = crypto.randomUUID()) =>
    call<QueuedScene[]>("studio_outbox_enqueue", {
      kind: "scene3d", requestKey, payload: { prompt, mode },
    }),
  outboxClaim: (id: string) => call<boolean>("studio_outbox_claim", { id }),
  outboxFinish: (id: string, status: "completed" | "review", note?: string) =>
    call<void>("studio_outbox_finish", { id, status, note: note ?? null }),
  outboxRetry: (id: string) => call<void>("studio_outbox_retry", { id }),
};

/**
 * Outbox requests use stable idempotency keys. A failed POST becomes "review",
 * never silently replayed after a crash or ambiguous network timeout.
 */
export async function submitQueuedScene(
  item: QueuedScene,
  target: "local" | "cloud",
): Promise<StudioJob | null> {
  if (!(await cloud.outboxClaim(item.id))) return null;
  try {
    const result = target === "cloud"
      ? await cloud.studioScene(item.payload.prompt, item.payload.mode, item.requestKey)
      : await cloud.gpuScene(item.payload.prompt, item.payload.mode, item.requestKey);
    await cloud.outboxFinish(item.id, "completed", result.id);
    return result;
  } catch (caught) {
    await cloud.outboxFinish(item.id, "review", describeFailure(caught)).catch(() => undefined);
    throw caught;
  }
}

// Hosted completions share the user's existing local conversation/history storage.
// A hosted response is never misrepresented as a local GPU response.
export async function hostedChat(
  conversationId: string,
  content: string,
  model: string,
  mode: string,
  replaceAssistantId?: string,
): Promise<Message> {
  if (!model) throw new Error("Choose an available Cloud model before sending.");
  const previous = await local.messages(conversationId);
  const prompt = content.trim();
  if (!prompt) throw new Error("Cloud prompt is empty.");
  const target = replaceAssistantId ? previous.findIndex((item) =>
    item.id === replaceAssistantId && item.role === "assistant") : -1;
  if (replaceAssistantId && target < 0)
    throw new Error("The original assistant response is unavailable for regeneration.");
  const context = target >= 0 ? previous.slice(0, target) : previous;
  const messages: Array<{ role: string; content: string }> =
    context.filter((item) => item.status === "completed" && item.content.trim())
      .slice(-28).map((item) => ({ role: item.role, content: item.content }));
  if (!replaceAssistantId) messages.push({ role: "user", content: prompt });
  else if (!messages.some((message) => message.role === "user"))
    throw new Error("The original user turn is unavailable.");
  if (mode === "search" || mode === "research") {
    const found = await cloud.search(prompt.slice(0, 500));
    const context = (found.results ?? []).slice(0, 8).map((item, index) =>
      "[" + (index + 1) + "] " + (item.title ?? "Result") + " " + (item.url ?? "") +
      "\n" + (item.snippet ?? item.description ?? ""),
    ).join("\n");
    if (!context) throw new Error("Cloud web search returned no sources. No answer was fabricated.");
    messages.unshift({ role: "system", content:
      "Answer using these retrieved sources. Cite the numbered source URLs. State uncertainty explicitly.\n" + context });
  }
  // The remote request goes first. A network outage must not create a duplicate
  // local user turn or a failed streaming assistant before a local fallback.
  const result = await cloud.request<{ choices: Array<{ message: { content: string } }> }>(
    "mobile/chat/completions", "POST", { model, messages, stream: false },
  );
  const response = result.choices?.[0]?.message?.content?.trim();
  if (!response) throw new Error("Cloud returned an empty assistant response.");
  let assistant: Message | null = null;
  try {
    if (!replaceAssistantId) await local.addUserMessage(conversationId, content);
    assistant = replaceAssistantId
      ? await call<Message>("cloud_begin_regeneration", { conversationId, assistantMessageId: replaceAssistantId })
      : await local.createStreamingAssistantMessage(conversationId);
    await local.appendMessageChunk(assistant.id, response);
    await local.completeMessage(assistant.id, "completed");
    return { ...assistant, content: response, status: "completed", modelId: model };
  } catch (error) {
    // Never invoke another engine after the remote reply has succeeded.
    if (assistant) await local.completeMessage(assistant.id, "failed").catch(() => undefined);
    throw new ResponsePersistenceError(error);
  }
}
