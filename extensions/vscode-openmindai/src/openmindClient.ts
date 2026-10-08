export type ChatRole = "system" | "user" | "assistant";

export interface ChatMessage {
  role: ChatRole;
  content: string;
}

/**
 * Which request fields an endpoint accepts.
 * coding-agent: the desktop coding-agent endpoint. It resolves the Agent Setup model and owns
 *   model-specific template options, so only standard OpenAI fields are sent.
 * llama-server: a llama.cpp server (desktop runtime from an older desktop build, or a custom
 *   endpoint). Accepts chat_template_kwargs.
 * native-gateway: the fallback Go native interface, which rejects unknown request fields.
 */
export type RequestProfile = "coding-agent" | "llama-server" | "native-gateway";

export interface OpenMindClientOptions {
  endpoint: string;
  model: string;
  requestTimeoutMs: number;
  maxOutputTokens: number;
  apiToken?: string;
  profile?: RequestProfile;
  /**
   * Context window for output budgeting. When omitted for llama-server, the per-slot size
   * reported by /props is used. The coding agent budgets on the desktop side.
   */
  contextSize?: number;
  /** Human-readable description of the target, used in progress messages. */
  label?: string;
  /** Extra request headers, such as the coding-agent session token. */
  extraHeaders?: Record<string, string>;
}

/** Metadata the desktop coding agent attaches to each response. */
export interface CodingAgentRoute {
  route: "coding-agent";
  model?: { id?: string; name?: string; repository?: string | null };
  contextSize?: number;
  promptTokens?: number;
  requestedMaxTokens?: number;
  maxTokens?: number;
}

export interface ChatResult {
  content: string;
  finishReason?: string;
  usage?: { prompt_tokens?: number; completion_tokens?: number; total_tokens?: number };
  /** max_tokens the backend actually used. */
  maxTokens: number;
  route?: CodingAgentRoute;
}

export const DEFAULT_MAX_OUTPUT_TOKENS = 2048;
export const MIN_MAX_OUTPUT_TOKENS = 16;
// Matches the max_tokens ceiling enforced by the OpenMindAI native gateway and worker.
export const MAX_MAX_OUTPUT_TOKENS = 8192;

export const TOKEN_LIMIT_MESSAGE =
  "The model reached the output token limit before producing an answer. Increase openmindai.maxOutputTokens in VS Code settings.";
export const REASONING_ONLY_MESSAGE =
  "The model finished without producing a final answer. Try again, or increase openmindai.maxOutputTokens in VS Code settings.";
export const EMPTY_RESPONSE_MESSAGE =
  "OpenMindAI returned no answer content. Check that the selected model is loaded in the OpenMindAI desktop app and try again.";

/** Tokens kept free for chat-template framing that prompt estimation may miss. */
export const CONTEXT_SAFETY_MARGIN = 64;

const STRICT_FIELD_ERROR =
  /unsupported or invalid native chat fields|unknown field|unrecognized field|chat_template_kwargs/i;

// Endpoints that rejected chat_template_kwargs; later requests skip the extra field.
const strictEndpoints = new Set<string>();
// Per-slot context sizes reported by llama-server /props.
const slotContextCache = new Map<string, number | undefined>();

/** Clamps a user-supplied setting to a usable max_tokens value. */
export function resolveMaxOutputTokens(value: unknown): number {
  const numeric = typeof value === "number" ? value : typeof value === "string" ? Number(value) : NaN;
  if (!Number.isFinite(numeric) || numeric <= 0) return DEFAULT_MAX_OUTPUT_TOKENS;
  return Math.min(MAX_MAX_OUTPUT_TOKENS, Math.max(MIN_MAX_OUTPUT_TOKENS, Math.floor(numeric)));
}

export function buildChatRequestBody(
  model: string,
  messages: ChatMessage[],
  maxOutputTokens: number,
  profile: RequestProfile
): Record<string, unknown> {
  const body: Record<string, unknown> = {
    model,
    messages,
    stream: false,
    max_tokens: resolveMaxOutputTokens(maxOutputTokens)
  };
  if (profile === "llama-server") {
    // llama.cpp hands chat_template_kwargs to the model's Jinja template. The NVIDIA Nemotron 3
    // OpenAgent templates default enable_thinking to true (OpenMindAI Core's Qwen3 template does
    // too), and reasoning shares max_tokens with the answer, so leaving it on can use the whole
    // budget before any final content. Templates that never read the flag ignore it. The
    // coding-agent endpoint applies the same default on the desktop side.
    body.chat_template_kwargs = { enable_thinking: false };
  }
  return body;
}

/** Conservative prompt size estimate (about 3 characters per token plus message framing). */
export function estimatePromptTokens(messages: ChatMessage[]): number {
  return messages.reduce((total, message) => total + Math.ceil(message.content.length / 3) + 8, 0);
}

/**
 * Largest output budget that still fits the context window, capped at the requested value.
 * Returns undefined when the prompt leaves no usable room.
 */
export function fitOutputBudget(requested: number, contextSize: number, promptTokens: number): number | undefined {
  const available = Math.floor(contextSize) - promptTokens - CONTEXT_SAFETY_MARGIN;
  if (available < MIN_MAX_OUTPUT_TOKENS) return undefined;
  return Math.min(resolveMaxOutputTokens(requested), available);
}

export function contextFullMessage(promptTokens: number, contextSize: number): string {
  return `The request (~${promptTokens} prompt tokens) does not fit the model context window (${contextSize} tokens). Reduce openmindai.maxContextCharacters or start a new chat.`;
}

interface ChatCompletionChoice {
  finish_reason?: string | null;
  message?: {
    content?: unknown;
    reasoning_content?: unknown;
  };
  text?: unknown;
}

interface ChatCompletionResponse {
  choices?: ChatCompletionChoice[];
  usage?: ChatResult["usage"];
  openmindai?: CodingAgentRoute;
  error?: {
    message?: string;
  };
}

/**
 * Returns the final assistant answer. reasoning_content is never used as the answer and
 * never included in errors.
 */
export function extractAssistantAnswer(data: unknown): string {
  const response = (data ?? {}) as ChatCompletionResponse;
  if (response.error?.message) {
    throw new Error(response.error.message);
  }
  const choice = Array.isArray(response.choices) ? response.choices[0] : undefined;
  const answer = stripThinkBlocks(contentText(choice?.message?.content) || contentText(choice?.text));
  if (answer) return answer;

  if (choice?.finish_reason === "length") throw new Error(TOKEN_LIMIT_MESSAGE);
  if (contentText(choice?.message?.reasoning_content)) throw new Error(REASONING_ONLY_MESSAGE);
  throw new Error(EMPTY_RESPONSE_MESSAGE);
}

function contentText(value: unknown): string {
  if (typeof value === "string") return value;
  if (Array.isArray(value)) {
    // OpenAI-style content parts: [{ type: "text", text: "..." }]
    return value
      .map((part) =>
        part && typeof part === "object" && typeof (part as { text?: unknown }).text === "string"
          ? (part as { text: string }).text
          : ""
      )
      .join("");
  }
  return "";
}

// Servers without reasoning parsing return thinking inline; keep it out of the answer.
function stripThinkBlocks(text: string): string {
  return text
    .replace(/<think>[\s\S]*?<\/think>/gi, "")
    .replace(/<think>[\s\S]*$/i, "")
    .trim();
}

export class OpenMindClient {
  private readonly endpoint: string;

  constructor(private readonly options: OpenMindClientOptions) {
    const endpoint = options.endpoint.trim().replace(/\/+$/, "");
    if (!/^https?:\/\/(localhost|127\.0\.0\.1|\[::1\])(?::\d+)?(?:\/.*)?$/i.test(endpoint)) {
      throw new Error("OpenMindAI endpoint must be a loopback HTTP(S) URL.");
    }
    this.endpoint = endpoint;
  }

  async health(): Promise<unknown> {
    return this.getJsonWithFallback(["/healthz", "/health"]);
  }

  async ready(): Promise<unknown> {
    return this.getJsonWithFallback(["/readyz", "/v1/models", "/health"]);
  }

  get label(): string {
    return this.options.label ?? "the local OpenMindAI model";
  }

  get profile(): RequestProfile {
    return strictEndpoints.has(this.endpoint) ? "native-gateway" : (this.options.profile ?? "llama-server");
  }

  async chat(messages: ChatMessage[]): Promise<string> {
    return (await this.chatDetailed(messages)).content;
  }

  async chatDetailed(messages: ChatMessage[]): Promise<ChatResult> {
    const profile = this.profile;
    const maxTokens = await this.outputBudget(messages, profile);
    let { response, data } = await this.postChat(messages, profile, maxTokens);

    if (
      profile === "llama-server" &&
      response.status === 400 &&
      STRICT_FIELD_ERROR.test(data.error?.message ?? "")
    ) {
      // A strict server at a manually configured endpoint; resend without extensions.
      strictEndpoints.add(this.endpoint);
      ({ response, data } = await this.postChat(messages, "native-gateway", maxTokens));
    }

    if (!response.ok) {
      throw new Error(data.error?.message ?? `OpenMindAI request failed with HTTP ${response.status}`);
    }
    const choice = Array.isArray(data.choices) ? data.choices[0] : undefined;
    return {
      content: extractAssistantAnswer(data),
      finishReason: choice?.finish_reason ?? undefined,
      usage: data.usage,
      maxTokens: data.openmindai?.maxTokens ?? maxTokens,
      route: data.openmindai?.route === "coding-agent" ? data.openmindai : undefined
    };
  }

  private async outputBudget(messages: ChatMessage[], profile: RequestProfile): Promise<number> {
    const requested = resolveMaxOutputTokens(this.options.maxOutputTokens);
    // The coding agent knows its real context and tokenizer; it fits the budget itself.
    if (profile === "coding-agent") return requested;
    const contextSize =
      this.options.contextSize ?? (profile === "llama-server" ? await this.slotContextSize() : undefined);
    if (!contextSize) return requested;
    const promptTokens = estimatePromptTokens(messages);
    const budget = fitOutputBudget(requested, contextSize, promptTokens);
    if (budget === undefined) throw new Error(contextFullMessage(promptTokens, contextSize));
    return budget;
  }

  /** Per-slot context from llama-server /props; undefined when the server does not report it. */
  private async slotContextSize(): Promise<number | undefined> {
    if (slotContextCache.has(this.endpoint)) return slotContextCache.get(this.endpoint);
    let contextSize: number | undefined;
    try {
      const props = (await this.getJson("/props")) as { default_generation_settings?: { n_ctx?: unknown } };
      const value = props?.default_generation_settings?.n_ctx;
      contextSize = typeof value === "number" && value > 0 ? value : undefined;
    } catch {
      contextSize = undefined;
    }
    slotContextCache.set(this.endpoint, contextSize);
    return contextSize;
  }

  private async postChat(
    messages: ChatMessage[],
    profile: RequestProfile,
    maxTokens: number
  ): Promise<{ response: Response; data: ChatCompletionResponse }> {
    const response = await this.fetchWithTimeout(`${this.endpoint}/v1/chat/completions`, {
      method: "POST",
      headers: this.headers(),
      body: JSON.stringify(buildChatRequestBody(this.options.model, messages, maxTokens, profile))
    });
    const data = (await readJson(response)) as ChatCompletionResponse;
    return { response, data };
  }

  private async getJson(path: string): Promise<unknown> {
    const response = await this.fetchWithTimeout(`${this.endpoint}${path}`, {
      method: "GET",
      headers: this.headers()
    });
    const data = await readJson(response).catch(() => undefined);
    if (!response.ok) {
      throw new Error(`OpenMindAI ${path} failed with HTTP ${response.status}`);
    }
    return data;
  }

  private async getJsonWithFallback(paths: string[]): Promise<unknown> {
    let lastError: unknown;
    for (const path of paths) {
      try {
        return await this.getJson(path);
      } catch (error) {
        lastError = error;
      }
    }
    throw lastError instanceof Error ? lastError : new Error(String(lastError));
  }

  private headers(): Record<string, string> {
    const headers: Record<string, string> = {
      "Content-Type": "application/json",
      "X-OpenMindAI-Client": "vscode",
      ...this.options.extraHeaders
    };
    const token = this.options.apiToken?.trim();
    if (token) {
      headers.Authorization = `Bearer ${token}`;
    }
    return headers;
  }

  private async fetchWithTimeout(url: string, init: RequestInit): Promise<Response> {
    const controller = new AbortController();
    const timeout = setTimeout(() => controller.abort(), this.options.requestTimeoutMs);
    try {
      return await fetch(url, {
        ...init,
        signal: controller.signal
      });
    } catch (error) {
      if (error instanceof Error && error.name === "AbortError") {
        throw new Error(`OpenMindAI request timed out after ${this.options.requestTimeoutMs}ms.`);
      }
      if (error instanceof TypeError) {
        throw new Error("Cannot reach OpenMindAI. Start the OpenMindAI local API and check the endpoint setting.");
      }
      throw error;
    } finally {
      clearTimeout(timeout);
    }
  }
}

async function readJson(response: Response): Promise<unknown> {
  const text = await response.text();
  if (!text.trim()) return {};
  try {
    return JSON.parse(text) as unknown;
  } catch {
    if (!response.ok) {
      return {
        error: {
          message: text.slice(0, 500)
        }
      };
    }
    throw new Error("OpenMindAI returned a non-JSON response.");
  }
}
