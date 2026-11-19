const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { test } = require("node:test");

const {
  EDIT_FENCE_TAG,
  MAX_EDIT_FILES,
  assertInsideRealWorkspace,
  EDIT_REPAIR_PROMPT,
  UNREQUESTED_EDIT_NOTE,
  extractEditPlan,
  looksLikeEditRequest,
  needsEditRepair,
  normalizeEditPath,
  resolveEditTargets,
  stripEditFences,
  validateEditPlan
} = require("../dist/editProtocol.js");
const { SYSTEM_PROMPT_SOURCE } = (() => {
  // chatPanel.ts imports vscode, so check its prompt text at the source level.
  const source = fs.readFileSync(path.join(__dirname, "..", "src", "chatPanel.ts"), "utf8");
  return { SYSTEM_PROMPT_SOURCE: source };
})();

const plan = {
  summary: "Add math helper",
  files: [{ path: "src/math.ts", content: "export function add(a: number, b: number): number {\n  return a + b;\n}\n" }]
};
const fenced = (json) => "```" + EDIT_FENCE_TAG + "\n" + json + "\n```";
const workspace = path.resolve(os.tmpdir(), "openmindai-edit-workspace");

test("1. valid fenced edit JSON is extracted and the fence is hidden from chat text", () => {
  const answer = `Here is the change.\n\n${fenced(JSON.stringify(plan))}`;
  const result = extractEditPlan(answer);
  assert.equal(result.kind, "plan");
  assert.equal(result.format, "fenced");
  assert.deepEqual(result.plan, plan);
  assert.equal(result.visibleText, "Here is the change.");
  assert.equal(stripEditFences(answer), "Here is the change.");
});

test("2. valid bare edit JSON (whole answer) is accepted with no raw JSON shown", () => {
  for (const answer of [
    JSON.stringify(plan),
    `\n  ${JSON.stringify(plan, null, 2)}\n`,
    "```json\n" + JSON.stringify(plan, null, 2) + "\n```"
  ]) {
    const result = extractEditPlan(answer);
    assert.equal(result.kind, "plan", answer);
    assert.equal(result.format, "bare");
    assert.deepEqual(result.plan, plan);
    assert.equal(result.visibleText, "");
  }
});

test("3. arbitrary JSON is not treated as an edit", () => {
  for (const answer of [
    '{"name":"openmindai","version":"1.0.0"}',
    '{"files":"not-an-array"}',
    '{"files":[]}',
    '{"files":[{"path":"a.ts"}]}',
    '{"files":[{"path":"a.ts","content":"x","mode":"0777"}]}',
    '{"summary":"x","files":[{"path":"a.ts","content":"x"}],"command":"rm -rf /"}',
    JSON.stringify([plan]) // the schema is an object; bare arrays are not accepted
  ]) {
    assert.deepEqual(extractEditPlan(answer), { kind: "none" }, answer);
  }
});

test("4. malformed fenced edit JSON throws instead of guessing", () => {
  assert.throws(() => extractEditPlan(fenced('{"files":[{"path":"a.ts",')), /invalid edit JSON/);
  assert.throws(() => extractEditPlan(fenced('{"files":[{"path":"a.ts"}]}')), /string content/);
  assert.throws(() => extractEditPlan(fenced('{"files":[]}')), /non-empty files/);
  assert.throws(
    () => extractEditPlan(`${fenced(JSON.stringify(plan))}\n${fenced(JSON.stringify(plan))}`),
    /expected one/
  );
  // Malformed bare JSON is just text.
  assert.deepEqual(extractEditPlan('{"files":[{"path":"a.ts",'), { kind: "none" });
});

test("5. JSON mixed with explanatory prose is shown as text, not applied", () => {
  for (const answer of [
    `Here you go: ${JSON.stringify(plan)}`,
    `${JSON.stringify(plan)}\nLet me know if you need more.`,
    `Sure!\n\n\`\`\`json\n${JSON.stringify(plan)}\n\`\`\``
  ]) {
    assert.deepEqual(extractEditPlan(answer), { kind: "none" }, answer);
  }
});

test("6. path traversal is rejected", () => {
  for (const bad of ["../../something", "src/../../outside.ts", "..\\..\\x.ts", "a/../../b"]) {
    assert.throws(() => normalizeEditPath(bad), /traversal|unsafe/, bad);
    assert.throws(() => resolveEditTargets(workspace, { files: [{ path: bad, content: "" }] }), undefined, bad);
  }
});

test("7. absolute and outside-workspace paths are rejected", () => {
  for (const bad of [
    "C:\\outside-workspace\\file",
    "C:/outside/file",
    "c:relative-to-drive.txt",
    "/etc/passwd",
    "\\\\server\\share\\file",
    "~/secrets.txt"
  ]) {
    assert.throws(() => normalizeEditPath(bad), /absolute/i, bad);
  }
  for (const bad of [".git/config", "sub/.GIT/hooks/pre-commit", "NUL", "con.txt", "a:stream", "trail.", "bad|name", ""]) {
    assert.throws(() => normalizeEditPath(bad), undefined, bad);
  }
});

test("8. valid workspace file edits resolve inside the workspace", () => {
  const [target] = resolveEditTargets(workspace, {
    files: [{ path: "./src\\nested/file.ts", content: "x" }]
  });
  assert.equal(target.relative, "src/nested/file.ts");
  assert.equal(target.absolute, path.join(workspace, "src", "nested", "file.ts"));
  assert.equal(target.content, "x");
  assert.equal(normalizeEditPath(".github/workflows/ci.yml"), ".github/workflows/ci.yml");
});

test("9. create-file edits for new paths are valid plans", () => {
  const result = extractEditPlan(fenced(JSON.stringify(plan)));
  const [target] = resolveEditTargets(workspace, result.plan);
  assert.equal(target.relative, "src/math.ts");
  assert.match(target.content, /export function add/);
});

test("10. multiple edits are supported and duplicate targets rejected", () => {
  const multi = {
    summary: "two files",
    files: [
      { path: "a.ts", content: "a" },
      { path: "b/c.ts", content: "c" }
    ]
  };
  assert.deepEqual(
    resolveEditTargets(workspace, extractEditPlan(JSON.stringify(multi)).plan).map((t) => t.relative),
    ["a.ts", "b/c.ts"]
  );
  assert.throws(
    () => resolveEditTargets(workspace, { files: [{ path: "a.ts", content: "1" }, { path: "./a.ts", content: "2" }] }),
    /more than once/
  );
  const tooMany = { files: Array.from({ length: MAX_EDIT_FILES + 1 }, (_, i) => ({ path: `f${i}.ts`, content: "" })) };
  assert.throws(() => validateEditPlan(tooMany, { allowExtraKeys: true }), /limit/);
});

test("11. normal non-edit responses (including normal JSON) never produce edits", () => {
  for (const answer of [
    "A Rust enum is a sum type; a struct is a product type.",
    '{"answer":"42"}',
    "```json\n{\"status\":\"ok\"}\n```",
    "Use `{ files: [] }` when nothing changes."
  ]) {
    assert.deepEqual(extractEditPlan(answer), { kind: "none" }, answer);
  }
});

test("12-14. empty, reasoning-only, and token-limit answers are rejected before edit parsing", () => {
  // These are covered end-to-end in openmindClient.test.js: extractAssistantAnswer throws, so
  // no answer text ever reaches extractEditPlan. An empty string is never an edit plan.
  assert.deepEqual(extractEditPlan(""), { kind: "none" });
  assert.deepEqual(extractEditPlan("   \n"), { kind: "none" });
});

test("fenced plans tolerate extra metadata keys, bare plans do not", () => {
  const withExtra = { ...plan, note: "x" };
  assert.equal(extractEditPlan(fenced(JSON.stringify(withExtra))).kind, "plan");
  assert.deepEqual(extractEditPlan(JSON.stringify(withExtra)), { kind: "none" });
});

test("links that leave the workspace are rejected", async () => {
  const base = fs.mkdtempSync(path.join(os.tmpdir(), "openmindai-link-"));
  const inside = path.join(base, "ws");
  const outside = path.join(base, "outside");
  fs.mkdirSync(inside);
  fs.mkdirSync(outside);
  try {
    await assertInsideRealWorkspace(inside, path.join(inside, "src", "new.ts"));
    let linked = false;
    try {
      fs.symlinkSync(outside, path.join(inside, "escape"), "junction");
      linked = true;
    } catch {
      // Creating links may need privileges; the containment check above still ran.
    }
    if (linked) {
      await assert.rejects(
        assertInsideRealWorkspace(inside, path.join(inside, "escape", "file.ts")),
        /leaves the workspace/
      );
    }
  } finally {
    fs.rmSync(base, { recursive: true, force: true });
  }
});

test("system prompt makes the fenced payload canonical and forbids surrounding prose", () => {
  assert.match(SYSTEM_PROMPT_SOURCE, /exactly one fenced code block tagged \$\{EDIT_FENCE_TAG\} and nothing else/);
  assert.match(SYSTEM_PROMPT_SOURCE, /no explanation before or after it/);
  assert.match(SYSTEM_PROMPT_SOURCE, /never output an edit block/);
  assert.doesNotMatch(SYSTEM_PROMPT_SOURCE, /Qwen/);
});

test("edit repair is requested only for edit requests answered with code but no plan", () => {
  const codeOnly = "```typescript\nexport function greet(name: string) { return `Hello, ${name}!`; }\n```";
  assert.equal(needsEditRepair("Change greet in src/greet.ts to use a template literal", codeOnly), true);
  // Already a valid plan, prose without code, or a question: no repair turn.
  assert.equal(needsEditRepair("Create src/math.ts", fenced(JSON.stringify(plan))), false);
  assert.equal(needsEditRepair("Create src/math.ts", JSON.stringify(plan)), false);
  assert.equal(needsEditRepair("Change greet", "I cannot see that file."), false);
  assert.equal(needsEditRepair("Explain how to add a field to a struct", codeOnly), false);
  // A malformed edit block is worth one repair.
  assert.equal(needsEditRepair("Update src/a.ts", fenced('{"files":[{"path":"a.ts"}]}')), true);
  assert.equal(looksLikeEditRequest("What does this function do?"), false);
  assert.match(EDIT_REPAIR_PROMPT, /openmindai-edits/);
  // The repair prompt itself never relaxes validation: its answer goes through extractEditPlan.
  assert.deepEqual(extractEditPlan(codeOnly), { kind: "none" });
});

test("edit blocks never reach displayed text, even when left unterminated", () => {
  const unterminated =
    "A generic is a type parameter.\n\n```openmindai-edits\n{\"summary\":\"x\",\"files\":[{\"path\":\"a.ts\",\"content\":\"export const a = 1;";
  assert.equal(stripEditFences(unterminated), "A generic is a type parameter.");
  assert.deepEqual(extractEditPlan(unterminated), { kind: "none" });
  const closed = `Text first.\n${fenced(JSON.stringify(plan))}\nText after.`;
  assert.equal(stripEditFences(closed), "Text first.\n\nText after.");
});

test("questions are not edit requests, so suggested edits are not applied", () => {
  for (const question of [
    "Explain what a TypeScript generic is in two sentences.",
    "What is the difference between let and const in TypeScript?",
    "Say hello."
  ]) {
    assert.equal(looksLikeEditRequest(question), false, question);
  }
  for (const request of [
    "Create src/math.ts with an exported multiply(a: number, b: number) function.",
    "Change greet in src/greet.ts so it returns a template literal."
  ]) {
    assert.equal(looksLikeEditRequest(request), true, request);
  }
  assert.match(UNREQUESTED_EDIT_NOTE, /nothing was written/);
});
