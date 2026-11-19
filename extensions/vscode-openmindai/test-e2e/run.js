// Runs the real-UI integration suite in an isolated VS Code Extension Development Host
// against the running OpenMindAI desktop app. It never touches the user's VS Code profile:
// user data and extensions live in temporary directories.
//
// Usage: npm run test:e2e
// Env:   VSCODE_EXECUTABLE   path to Code.exe (default: per-user install)
//        OPENMINDAI_SQLITE   sqlite3 executable, enables the Coding Workspace disabled test
//        OPENMINDAI_DB       desktop database file for that test
const cp = require("node:child_process");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const extensionRoot = path.resolve(__dirname, "..");
const executable =
  process.env.VSCODE_EXECUTABLE ||
  path.join(process.env.LOCALAPPDATA || "", "Programs", "Microsoft VS Code", "Code.exe");
if (!fs.existsSync(executable)) {
  console.error(`VS Code executable not found: ${executable}`);
  process.exit(2);
}

const base = fs.mkdtempSync(path.join(os.tmpdir(), "openmindai-e2e-"));
const workspace = path.join(base, "workspace");
fs.mkdirSync(path.join(workspace, "src"), { recursive: true });
fs.writeFileSync(
  path.join(workspace, "src", "greet.ts"),
  'export function greet(name: string): string {\n  return "Hi " + name;\n}\n'
);
fs.writeFileSync(path.join(workspace, "src", "untouched.ts"), "export const VALUE = 1;\n");
fs.writeFileSync(
  path.join(workspace, "src", "shapes.rs"),
  [
    "pub enum Shape {",
    "    Circle(f64),",
    "    Square(f64),",
    "}",
    "",
    "pub fn area(shape: &Shape) -> f64 {",
    "    match shape {",
    "        Shape::Circle(r) => 3.14 * r * r,",
    "        Shape::Square(s) => s + s,",
    "    }",
    "}",
    ""
  ].join("\n")
);
const report = path.join(base, "report.json");

// Inherited VS Code variables would hand the launch to the user's running instance.
const env = Object.fromEntries(
  Object.entries(process.env).filter(([key]) => !key.startsWith("VSCODE_") && key !== "ELECTRON_RUN_AS_NODE")
);
env.OPENMINDAI_E2E_WORKSPACE = workspace;
env.OPENMINDAI_E2E_REPORT = report;

const args = [
  workspace,
  `--extensionDevelopmentPath=${extensionRoot}`,
  // Test-only chat model so the isolated profile's Chat view can dispatch messages.
  `--extensionDevelopmentPath=${path.join(__dirname, "lm-provider")}`,
  `--extensionTestsPath=${path.join(__dirname, "suite.js")}`,
  `--user-data-dir=${path.join(base, "user-data")}`,
  `--extensions-dir=${path.join(base, "extensions")}`,
  "--disable-workspace-trust",
  "--skip-welcome",
  "--skip-release-notes",
  "--new-window"
];

console.log(`Workspace: ${workspace}`);
console.log(`Report:    ${report}`);
const started = Date.now();
const child = cp.spawn(executable, args, { env, stdio: "inherit" });
child.on("exit", (code) => {
  console.log(`VS Code exited with ${code} after ${Math.round((Date.now() - started) / 1000)}s`);
  if (fs.existsSync(report)) console.log(fs.readFileSync(report, "utf8"));
  process.exit(code ?? 1);
});
