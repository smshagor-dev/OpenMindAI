import {
  ChatBackend,
  ChatMessage,
  ChatResult,
  EMPTY_RESPONSE_MESSAGE,
  contextFullMessage,
  estimatePromptTokens,
  extractAssistantAnswer,
  fitOutputBudget,
  resolveMaxOutputTokens
} from "./openmindClient";

/** Default OpenMindAI Cloud developer gateway (OpenAI-compatible, API-key auth). */
export const DEFAULT_CLOUD_ENDPOINT = "https://cloud.openmindai.org/api/v1/";

export interface CloudClientOptions {
  /** Gateway base URL. Trailing slash optional; normalized by the client. */
  endpoint: string;
  /** Bearer API key issued at api.openmindai.org. */
  apiKey: string;
  /** Model id sent to the gateway (for example an OpenMindAI hosted model). */
  model: string;
  requestTimeoutMs: number;
  maxOutputTokens: number;
  /** Optional context window for output budgeting. Hosted models are large, so usually omitted. */
  contextSize?: number;
  /** Human-readable description used in progress messages. */
  label?: string;
}

export interface CloudModel {
  id: string;
  name?: string;
}

/**
 * Normalizes a cloud gateway base URL to one ending in a single slash. An empty or "auto"
 * value falls back to {@link DEFAULT_CLOUD_ENDPOINT}.
 */
export function normalizeCloudEndpoint(value: string | undefined): string {
  const trimmed = (value ?? "").trim();
  if (!trimmed || trimmed.toLowerCase() === "auto") return DEFAULT_CLOUD_ENDPOINT;
  return trimmed.replace(/\/+$/, "") + "/";
}

/**
 * Validates a normalized cloud base URL. Real cloud traffic must use HTTPS so the API key
 * is never sent in clear text; a loopback host may use HTTP for a self-hosted gateway or tests.
 */
export function assertCloudEndpoint(base: string): URL {
  let url: URL;
  try {
    url = new URL(base);
  } catch {
    throw new Error("OpenMindAI Cloud endpoint must be a valid URL.");
  }
  const loopback = /^(localhost|127\.0\.0\.1|\[::1\]|::1)$/i.test(url.hostname);
  if (url.protocol !== "https:" && !(url.protocol === "http:" && loopback)) {
    throw new Error("OpenMindAI Cloud endpoint must use HTTPS.");
  }
  return url;
}

interface ChatCompletionResponse {
  choices?: Array<{ finish_reason?: string | null; message?: { content?: unknown } }>;
  usage?: ChatResult["usage"];
  error?: { message?: string };
  message?: string;
}

/**
 * Talks to the OpenMindAI Cloud developer gateway (OpenAI-compatible) with a Bearer API key.
 * Mirrors the local OpenMindClient so the chat surfaces, the participant, and the editor
 * commands can target hosted models without branching on transport.
 */
export class CloudClient implements ChatBackend {
  private readonly base: string;
  private readonly apiKey: string;

  constructor(private readonly options: CloudClientOptions) {
    this.base = normalizeCloudEndpoint(options.endpoint);
    assertCloudEndpoint(this.base);
    this.apiKey = (options.apiKey ?? "").trim();
    if (!this.apiKey) {
      throw new Error('OpenMindAI Cloud API key is not set. Run "OpenMindAI: Set Cloud API Key".');
    }
  }

  get label(): string {
    return this.options.label ?? `OpenMindAI Cloud (${this.options.model})`;
  }

  async health(): Promise<unknown> {
    return this.listModels();
  }

  async ready(): Promise<unknown> {
    return this.listModels();
  }

  /** Returns the chat-capable models the gateway reports for this key. */
  async listModels(): Promise<CloudModel[]> {
    const data = (await this.getJson("openmindai/models")) as { data?: unknown };
    const rows = Array.isArray(data?.data) ? data.data : [];
    return rows
      .filter(
        (row): row is { id: string; name?: string } =>
          Boolean(row) &&
          typeof (row as { id?: unknown }).id === "string" &&
          (row as { id: string }).id.trim().length > 0
      )
      .map((row) => ({ id: row.id, name: typeof row.name === "string" ? row.name : undefined }));
  }

  async chat(messages: ChatMessage[]): Promise<string> {
    return (await this.chatDetailed(messages)).content;
  }

  async chatDetailed(messages: ChatMessage[]): Promise<ChatResult> {
    const maxTokens = this.outputBudget(messages);
    const response = await this.fetchWithTimeout(this.url("openmindai/chat/completions"), {
      method: "POST",
      headers: this.headers(),
      // The gateway is OpenAI-compatible and rejects unknown fields; send standard fields only.
      body: JSON.stringify({ model: this.options.model, messages, stream: false, max_tokens: maxTokens })
    });
    const data = (await readJson(response)) as ChatCompletionResponse;
    if (!response.ok) {
      throw new Error(
        data.error?.message ?? data.message ?? `OpenMindAI Cloud request failed with HTTP ${response.status}`
      );
    }
    const choice = Array.isArray(data.choices) ? data.choices[0] : undefined;
    return {
      content: extractAssistantAnswer(data),
      finishReason: choice?.finish_reason ?? undefined,
      usage: data.usage,
      maxTokens
    };
  }

  private outputBudget(messages: ChatMessage[]): number {
    const requested = resolveMaxOutputTokens(this.options.maxOutputTokens);
    const contextSize = this.options.contextSize;
    if (!contextSize) return requested;
    const promptTokens = estimatePromptTokens(messages);
    const budget = fitOutputBudget(requested, contextSize, promptTokens);
    if (budget === undefined) throw new Error(contextFullMessage(promptTokens, contextSize));
    return budget;
  }

  private url(path: string): string {
    return this.base + path.replace(/^\/+/, "");
  }

  private async getJson(path: string): Promise<unknown> {
    const response = await this.fetchWithTimeout(this.url(path), { method: "GET", headers: this.headers() });
    const data = (await readJson(response).catch(() => undefined)) as ChatCompletionResponse | undefined;
    if (!response.ok) {
      const detail = data?.error?.message ?? data?.message;
      throw new Error(detail ?? `OpenMindAI Cloud ${path} failed with HTTP ${response.status}`);
    }
    return data ?? {};
  }

  private headers(): Record<string, string> {
    return {
      "Content-Type": "application/json",
      Accept: "application/json",
      Authorization: `Bearer ${this.apiKey}`,
      "X-OpenMindAI-Client": "vscode"
    };
  }

  private async fetchWithTimeout(url: string, init: RequestInit): Promise<Response> {
    const controller = new AbortController();
    const timeout = setTimeout(() => controller.abort(), this.options.requestTimeoutMs);
    try {
      return await fetch(url, { ...init, signal: controller.signal });
    } catch (error) {
      if (error instanceof Error && error.name === "AbortError") {
        throw new Error(`OpenMindAI Cloud request timed out after ${this.options.requestTimeoutMs}ms.`);
      }
      if (error instanceof TypeError) {
        throw new Error(
          "Cannot reach OpenMindAI Cloud. Check your internet connection and the cloud endpoint setting."
        );
      }
      throw error;
    } finally {
      clearTimeout(timeout);
    }
  }
}

/** One-shot model listing for the model picker, without constructing a full chat client. */
export async function fetchCloudModels(
  endpoint: string,
  apiKey: string,
  requestTimeoutMs = 20000
): Promise<CloudModel[]> {
  return new CloudClient({
    endpoint,
    apiKey,
    model: "models-probe",
    requestTimeoutMs,
    maxOutputTokens: 0
  }).listModels();
}

async function readJson(response: Response): Promise<unknown> {
  const text = await response.text();
  if (!text.trim()) return {};
  try {
    return JSON.parse(text) as unknown;
  } catch {
    if (!response.ok) {
      return { error: { message: text.slice(0, 500) } };
    }
    throw new Error("OpenMindAI Cloud returned a non-JSON response.");
  }
}

// Re-export so callers that import the Cloud transport also see the shared empty-answer message.
export { EMPTY_RESPONSE_MESSAGE };
