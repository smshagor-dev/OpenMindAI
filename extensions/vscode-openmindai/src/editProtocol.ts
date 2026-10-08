import * as fs from "fs";
import * as path from "path";

// Kept free of the vscode module so the edit protocol can be unit tested under plain Node.

/**
 * OpenMindAI edit protocol. Canonical form is one fenced block tagged openmindai-edits:
 *
 *   ```openmindai-edits
 *   {"summary":"short summary","files":[{"path":"relative/path.ext","content":"full file content"}]}
 *   ```
 *
 * For compatibility, a response that consists of nothing but a JSON object strictly matching
 * this schema (optionally wrapped in a single ```json fence) is accepted too.
 */
export const EDIT_FENCE_TAG = "openmindai-edits";
export const EDIT_PLAN_EXAMPLE =
  '{"summary":"what changed","files":[{"path":"relative/path.ext","content":"full file content"}]}';

export const MAX_EDIT_FILES = 50;
export const MAX_EDIT_FILE_CHARS = 2_000_000;
const MAX_PATH_CHARS = 1024;

export interface EditFile {
  path: string;
  content: string;
}

export interface EditPlan {
  summary?: string;
  files: EditFile[];
}

export type EditExtraction =
  | { kind: "none" }
  | { kind: "plan"; format: "fenced" | "bare"; plan: EditPlan; visibleText: string };

export interface EditTarget {
  relative: string;
  absolute: string;
  content: string;
}

const FENCE_PATTERN = new RegExp("```" + EDIT_FENCE_TAG + "[^\\S\\r\\n]*\\r?\\n?([\\s\\S]*?)```", "gi");
const WHOLE_JSON_FENCE = /^```(?:json)?[^\S\r\n]*\r?\n([\s\S]*?)\r?\n?```$/i;
const WINDOWS_DEVICE_NAME = /^(con|prn|aux|nul|com\d|lpt\d)(\..*)?$/i;

/**
 * Finds an edit plan in a model answer.
 * - A fenced openmindai-edits block is an explicit edit request: malformed JSON or schema
 *   violations throw so the user sees why nothing was written.
 * - Bare JSON is only treated as an edit plan when the entire answer is a JSON object that
 *   strictly matches the schema. Anything else, including JSON mixed with prose, is plain text.
 */
export function extractEditPlan(answer: string): EditExtraction {
  const fences = [...answer.matchAll(FENCE_PATTERN)];
  if (fences.length > 1) {
    throw new Error(`OpenMindAI returned ${fences.length} ${EDIT_FENCE_TAG} blocks; expected one. No files were changed.`);
  }
  if (fences.length === 1) {
    let parsed: unknown;
    try {
      parsed = JSON.parse(fences[0][1]);
    } catch (error) {
      throw new Error(`OpenMindAI returned invalid edit JSON: ${error instanceof Error ? error.message : String(error)}`);
    }
    const plan = validateEditPlan(parsed, { allowExtraKeys: true });
    return { kind: "plan", format: "fenced", plan, visibleText: stripEditFences(answer) };
  }

  const bare = bareJsonCandidate(answer);
  if (bare === undefined) return { kind: "none" };
  let parsed: unknown;
  try {
    parsed = JSON.parse(bare);
  } catch {
    return { kind: "none" };
  }
  try {
    return { kind: "plan", format: "bare", plan: validateEditPlan(parsed, { allowExtraKeys: false }), visibleText: "" };
  } catch {
    // Valid JSON that is not an edit plan is an ordinary answer.
    return { kind: "none" };
  }
}

/** Removes edit blocks from text shown to the user, including one left unterminated. */
export function stripEditFences(answer: string): string {
  return answer
    .replace(FENCE_PATTERN, "")
    .replace(new RegExp("```" + EDIT_FENCE_TAG + "[\\s\\S]*$", "i"), "")
    .trim();
}

export const UNREQUESTED_EDIT_NOTE =
  "OpenMindAI suggested file changes, but your message did not ask for any, so nothing was written. Ask for the change explicitly to apply it.";

function bareJsonCandidate(answer: string): string | undefined {
  let candidate = answer.trim();
  const fenced = candidate.match(WHOLE_JSON_FENCE);
  if (fenced) candidate = fenced[1].trim();
  if (!candidate.startsWith("{") || !candidate.endsWith("}")) return undefined;
  return candidate;
}

/**
 * Validates the edit plan schema. Required fields are always strict; unknown keys are only
 * tolerated inside an explicit openmindai-edits fence.
 */
export function validateEditPlan(value: unknown, options: { allowExtraKeys: boolean }): EditPlan {
  if (!isPlainObject(value)) throw new Error("Edit plan must be a JSON object.");
  checkKeys(value, ["summary", "files"], options.allowExtraKeys, "edit plan");
  if (value.summary !== undefined && typeof value.summary !== "string") {
    throw new Error("Edit plan summary must be a string.");
  }
  const files = value.files;
  if (!Array.isArray(files) || files.length === 0) {
    throw new Error("Edit plan must contain a non-empty files array.");
  }
  if (files.length > MAX_EDIT_FILES) {
    throw new Error(`Edit plan changes ${files.length} files; the limit is ${MAX_EDIT_FILES}.`);
  }
  const plan: EditPlan = { files: [] };
  if (typeof value.summary === "string") plan.summary = value.summary;
  for (const [index, file] of files.entries()) {
    if (!isPlainObject(file)) throw new Error(`Edit file #${index + 1} must be an object.`);
    checkKeys(file, ["path", "content"], options.allowExtraKeys, `edit file #${index + 1}`);
    if (typeof file.path !== "string" || !file.path.trim()) {
      throw new Error(`Edit file #${index + 1} needs a non-empty path.`);
    }
    if (typeof file.content !== "string") {
      throw new Error(`Edit file ${file.path} needs string content.`);
    }
    if (file.content.length > MAX_EDIT_FILE_CHARS) {
      throw new Error(`Edit file ${file.path} exceeds ${MAX_EDIT_FILE_CHARS} characters.`);
    }
    plan.files.push({ path: file.path, content: file.content });
  }
  return plan;
}

/**
 * Resolves every file in the plan to a location inside the workspace, or throws without
 * touching the filesystem. Rejects absolute paths, drive letters, UNC paths, parent-directory
 * traversal, .git internals, Windows device names, and duplicate targets.
 */
export function resolveEditTargets(workspaceRoot: string, plan: EditPlan): EditTarget[] {
  const root = path.resolve(workspaceRoot);
  const seen = new Set<string>();
  return plan.files.map((file) => {
    const relative = normalizeEditPath(file.path);
    const absolute = path.resolve(root, ...relative.split("/"));
    const containment = path.relative(root, absolute);
    if (!containment || containment.startsWith("..") || path.isAbsolute(containment)) {
      throw new Error(`Rejected edit outside workspace: ${file.path}`);
    }
    const key = process.platform === "win32" ? absolute.toLowerCase() : absolute;
    if (seen.has(key)) throw new Error(`Edit plan writes ${relative} more than once.`);
    seen.add(key);
    return { relative, absolute, content: file.content };
  });
}

export function normalizeEditPath(rawPath: string): string {
  const trimmed = rawPath.trim();
  if (!trimmed || trimmed.length > MAX_PATH_CHARS) throw new Error(`Rejected unsafe edit path: ${rawPath}`);
  // eslint-disable-next-line no-control-regex
  if (/[\u0000-\u001f]/.test(trimmed)) throw new Error(`Rejected unsafe edit path: ${rawPath}`);
  if (
    /^[\\/]/.test(trimmed) ||
    /^[a-zA-Z]:/.test(trimmed) ||
    path.posix.isAbsolute(trimmed) ||
    path.win32.isAbsolute(trimmed) ||
    trimmed.startsWith("~")
  ) {
    throw new Error(`Rejected absolute edit path: ${rawPath}`);
  }
  const segments = trimmed.split(/[\\/]+/).filter((segment) => segment !== "" && segment !== ".");
  if (!segments.length) throw new Error(`Rejected unsafe edit path: ${rawPath}`);
  for (const segment of segments) {
    if (segment === "..") throw new Error(`Rejected parent-directory traversal in edit path: ${rawPath}`);
    if (segment.toLowerCase() === ".git") throw new Error(`Rejected edit inside .git: ${rawPath}`);
    if (
      segment.includes(":") ||
      /[<>"|?*]/.test(segment) ||
      /[. ]$/.test(segment) ||
      WINDOWS_DEVICE_NAME.test(segment)
    ) {
      throw new Error(`Rejected unsafe edit path: ${rawPath}`);
    }
  }
  return segments.join("/");
}

/**
 * Guards against symlinks or junctions inside the workspace that point elsewhere: the nearest
 * existing ancestor of the target must resolve inside the real workspace root.
 */
export async function assertInsideRealWorkspace(workspaceRoot: string, absolute: string): Promise<void> {
  const realRoot = await fs.promises.realpath(workspaceRoot);
  let current = absolute;
  for (;;) {
    try {
      const real = await fs.promises.realpath(current);
      const containment = path.relative(realRoot, real);
      if (containment.startsWith("..") || path.isAbsolute(containment)) {
        throw new Error(`Rejected edit through a link that leaves the workspace: ${absolute}`);
      }
      return;
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
      const parent = path.dirname(current);
      if (parent === current) return;
      current = parent;
    }
  }
}

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function checkKeys(value: Record<string, unknown>, allowed: string[], allowExtra: boolean, label: string): void {
  if (allowExtra) return;
  const unknown = Object.keys(value).filter((key) => !allowed.includes(key));
  if (unknown.length) throw new Error(`Unexpected ${label} field(s): ${unknown.join(", ")}`);
}

const EDIT_INTENT =
  /\b(create|change|modify|edit|update|add|rename|refactor|fix|write|implement|replace|remove|delete|make)\b/i;

/** Heuristic: the user asked for a workspace change, not an explanation. */
export function looksLikeEditRequest(userText: string): boolean {
  return EDIT_INTENT.test(userText) && !/^\s*(explain|why|what|how|describe)\b/i.test(userText);
}

export const EDIT_REPAIR_PROMPT = [
  "Your previous reply did not use the OpenMindAI edit format, so no files were changed.",
  `Reply again with only one fenced code block tagged ${EDIT_FENCE_TAG} and no other text.`,
  `It must contain JSON in this exact shape: ${EDIT_PLAN_EXAMPLE}`,
  "content is the complete new content of each changed file."
].join(" ");

/**
 * True when an edit request was answered with code but no usable edit plan, so one repair
 * turn asking for the canonical format is worthwhile. A malformed edit block also qualifies.
 */
export function needsEditRepair(userText: string, answer: string): boolean {
  if (!looksLikeEditRequest(userText) || !answer.includes("```")) return false;
  try {
    return extractEditPlan(answer).kind === "none";
  } catch {
    return true;
  }
}
