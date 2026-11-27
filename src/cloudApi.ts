import { invoke } from "@tauri-apps/api/core";
import type { Message } from "./types";
import { api as local } from "./api_legacy";
import { describeFailure, ResponsePersistenceError } from "./lib/executionRouting";
import { userMessageDisplay } from "./lib/chat";
import { cloudConnectionPromptReason, promptForCloudConnection } from "./lib/cloudConnectionPrompt";
export type { ExecutionMode } from "./lib/executionRouting";

export type StudioSection = "research-studio" | "3d-studio" | "web-search";
export type CloudConnection = {
  connected: boolean;
  user: { name?: string; user?: { name?: string } } | null;
  message: string | null;
};
export type CloudModel = {
  id: string;
  name?: string;
  available?: boolean;
  active?: boolean;
  operation?: string;
  supports_vision?: boolean;
};

export function cloudModelDisplayName(model: CloudModel): string {
  const identity = `${model.id} ${model.name ?? ""}`.toLowerCase().replace(/[_\s]+/g, "-");
  if (/openmindai[-.]?super|\bsuper\b/.test(identity)) return "OpenMindAI Super";
  if (/openmindai[-.]?ultra|\bultra\b/.test(identity)) return "OpenMindAI Ultra";
  if (/openmindai[-.]?lite|\blite\b/.test(identity)) return "OpenMindAI Lite";
  return model.name?.trim() || model.id;
}
export type StudioJob = {
  id: string;
  status: string;
  output?: unknown;
  result?: unknown;
  error?: string | null;
  asset_id?: number | null;
  events?: unknown[];
};
export type Studio3DDependencyPlan = {
  manifestVersion: string;
  supported: boolean;
  target: { os: string; architecture: string; python: string; abi: string } | null;
  basePython: string;
  state: "ready" | "not_installed" | "repair_required";
  downloadBytes: number;
  requiredFreeBytes: number;
  availableBytes: number;
  managedEnvironmentPath: string;
  packages: Array<{ name: string; version: string; size: number; license: string; reason: string }>;
  installScope: string;
  systemPythonModified: boolean;
  gpuDriverInstall: boolean;
};
export type ResearchProject = {
  id: number;
  title: string;
  status?: string;
  prompt?: string;
  created_at?: string;
  output_language?: string;
  progress?: number;
};
// The hosted research pipeline's detail/event payloads are rendered defensively
// (field names vary by run stage), so these keep an open shape.
export type ResearchEvent = {
  id?: number;
  sequence?: number;
  type?: string;
  status?: string;
  message?: string;
  task?: string;
  agent?: string;
  created_at?: string;
  [key: string]: unknown;
};
export type ResearchDetail = Record<string, unknown> & {
  id?: number;
  title?: string;
  status?: string;
  prompt?: string;
  progress?: number;
};
export type QueuedScene = {
  id: string;
  kind: "scene3d";
  requestKey: string;
  payload: { prompt: string; mode: "object" | "architecture" };
  status: "queued" | "sending" | "review";
  note: string | null;
  createdAt: string;
};

const call = <T>(command: string, args?: Record<string, unknown>) => {
  if (!("__TAURI_INTERNALS__" in window))
    return Promise.reject(new Error("Cloud features require the OpenMindAI desktop application."));
  return invoke<T>(command, args).catch((error: unknown) => {
    if (command !== "cloud_disconnect") {
      const reason = cloudConnectionPromptReason(error);
      if (reason) promptForCloudConnection(reason);
    }
    throw error;
  });
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
    const response = await cloud.request<{ data: CloudModel[] }>("openmindai/models");
    if (!Array.isArray(response.data))
      throw new Error("Cloud returned an invalid API model catalog.");
    // The OpenMindAI developer gateway is OpenAI-compatible; its model list may
    // omit the Mobile capability fields. Keep anything with a usable id that is
    // not explicitly unavailable or marked as a non-chat operation.
    return response.data.filter(
      (model) =>
        typeof model.id === "string" &&
        model.id.trim().length > 0 &&
        model.available !== false &&
        (model.operation === undefined || model.operation === "chat.completions"),
    );
  },
  search: (query: string) =>
    cloud.request<{
      results: Array<{ title?: string; url?: string; snippet?: string; description?: string }>;
    }>("web-search", "POST", { query: query.slice(0, 500) }),
  researchProjects: async (): Promise<ResearchProject[]> => {
    const page = await cloud.request<{ data: ResearchProject[] }>("research/projects");
    return Array.isArray(page.data) ? page.data : [];
  },
  createResearch: (title: string, prompt: string, outputLanguage = "auto") =>
    cloud.request<ResearchProject>("research/projects", "POST", {
      title: title.slice(0, 200),
      prompt: prompt.slice(0, 16000),
      output_language: outputLanguage,
    }),
  startResearch: (id: number) =>
    cloud.request<unknown>("research/projects/" + id + "/runs", "POST", {}),
  researchDetail: (id: number) => cloud.request<ResearchDetail>("research/projects/" + id),
  researchEvents: async (id: number): Promise<ResearchEvent[]> => {
    const page = await cloud.request<{ data?: ResearchEvent[]; events?: ResearchEvent[] }>(
      "research/projects/" + id + "/events",
    );
    if (Array.isArray(page.data)) return page.data;
    if (Array.isArray(page.events)) return page.events;
    return [];
  },
  researchControl: (id: number, action: "pause" | "resume" | "cancel") =>
    cloud.request<unknown>("research/projects/" + id + "/control", "POST", { action }),
  studioEngines: () => cloud.request<Record<string, unknown>>("3d-studio/engines"),
  uploadStudioImage: (name: string, base64: string, preview: string) =>
    cloud.request<{ id: number }>("3d-scenes/uploads", "POST", {
      name,
      data: base64,
      image_preview: preview,
    }),
  imageToMesh: (uploadId: number, prompt = "", requestKey = crypto.randomUUID()) =>
    cloud.request<StudioJob | { asset: { id: number } }>("3d-studio/jobs/image-model", "POST", {
      upload_id: uploadId,
      prompt: prompt.slice(0, 8000),
      request_key: requestKey,
    }),
  studioScene: (
    prompt: string,
    mode: "object" | "architecture",
    requestKey: string = crypto.randomUUID(),
  ) =>
    cloud.request<StudioJob>("3d-studio/jobs/scene", "POST", {
      prompt: prompt.slice(0, 8000),
      mode,
      request_key: requestKey,
    }),
  studioJob: (id: string) => cloud.request<StudioJob>("3d-studio/jobs/" + id),
  downloadMesh: (assetId: number) =>
    call<{ filename: string; data_base64: string }>("cloud_download_mesh", { assetId }),
  startGpu: () => call<{ gpu_active: boolean; gpu?: string; engine?: string }>("local_gpu_start"),
  stopGpu: () => call<void>("local_gpu_stop"),
  studio3dDependencyPlan: () => call<Studio3DDependencyPlan>("studio3d_dependency_plan"),
  installStudio3dDependencies: () => call<Record<string, unknown>>("studio3d_install_dependencies", { approved: true }),
  cancelStudio3dDependencyInstall: () => call<void>("studio3d_cancel_dependency_install"),
  studio3dNeuralCapabilities: () => call<Record<string, unknown>>("studio3d_neural_capabilities"),
  gpuHealth: () => cloud.gpu<{ gpu_active: boolean; gpu?: string; engine?: string }>("health"),
  gpuScene: (
    prompt: string,
    mode: "object" | "architecture",
    requestKey: string = crypto.randomUUID(),
  ) =>
    cloud.gpu<StudioJob>("jobs", "POST", {
      prompt: prompt.slice(0, 8000),
      mode,
      request_key: requestKey,
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
  queueScene: (
    prompt: string,
    mode: "object" | "architecture",
    requestKey: string = crypto.randomUUID(),
  ) =>
    call<QueuedScene[]>("studio_outbox_enqueue", {
      kind: "scene3d",
      requestKey,
      payload: { prompt, mode },
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
    const result =
      target === "cloud"
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
  const target = replaceAssistantId
    ? previous.findIndex((item) => item.id === replaceAssistantId && item.role === "assistant")
    : -1;
  if (replaceAssistantId && target < 0)
    throw new Error("The original assistant response is unavailable for regeneration.");
  const context = target >= 0 ? previous.slice(0, target) : previous;
  const messages: Array<{ role: string; content: string }> = context
    .filter((item) => item.status === "completed" && item.content.trim())
    .slice(-28)
    .map((item) => ({ role: item.role, content: item.content }));
  if (!replaceAssistantId) messages.push({ role: "user", content: prompt });
  else if (!messages.some((message) => message.role === "user"))
    throw new Error("The original user turn is unavailable.");
  // Hosted replies come from OpenMindAI Cloud, so the assistant must not call
  // itself a "local" assistant (that identity belongs to the on-device model).
  if (!messages.some((message) => message.role === "system")) {
    messages.unshift({
      role: "system",
      content:
        "You are OpenMindAI, a cloud-hosted AI assistant running on OpenMindAI Cloud. " +
        "Do not describe yourself as a local or on-device assistant.",
    });
  }
  if (mode === "search" || mode === "research") {
    // Use the app's own web search (keyless DuckDuckGo/Bing, plus configured
    // keys) rather than the Cloud web-search endpoint, which a model-gateway
    // API key cannot reach (HTTP 403). The powerful hosted model then answers
    // grounded on live results end to end.
    try {
      const webMessageId = crypto.randomUUID();
      const searchQuery = userMessageDisplay(prompt).prompt;
      const webContext = await local.webSearchContext(searchQuery, mode, conversationId, webMessageId);
      messages.unshift({
        role: "system",
        content:
          webContext && webContext.trim()
            ? webContext
            : "Live web search returned no usable sources. Answer from your own knowledge and clearly tell the user that current web results could not be retrieved.",
      });
    } catch {
      messages.unshift({
        role: "system",
        content:
          "Live web search is unavailable right now. Answer from your own knowledge and clearly tell the user that current web results could not be retrieved.",
      });
    }
  }
  // The remote request goes first. A network outage must not create a duplicate
  // local user turn or a failed streaming assistant before a local fallback.
  const result = await cloud.request<{ choices: Array<{ message: { content: string } }> }>(
    "openmindai/chat/completions",
    "POST",
    { model, messages, stream: false },
  );
  const response = result.choices?.[0]?.message?.content?.trim();
  if (!response) throw new Error("Cloud returned an empty assistant response.");
  let assistant: Message | null = null;
  try {
    if (!replaceAssistantId) await local.addUserMessage(conversationId, content);
    assistant = replaceAssistantId
      ? await call<Message>("cloud_begin_regeneration", {
          conversationId,
          assistantMessageId: replaceAssistantId,
        })
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
