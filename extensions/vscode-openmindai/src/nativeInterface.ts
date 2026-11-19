import * as cp from "child_process";
import * as fs from "fs";
import * as path from "path";
import * as vscode from "vscode";
import {
  NATIVE_INTERFACE_ENDPOINT,
  RuntimeTarget,
  describeTarget,
  findCodingAgentEndpoint,
  findDesktopRuntimeEndpoint,
  hasLiveCodingAgentDescriptor,
  listeningPortsByProcessName,
  loadedModelName,
  requestJson
} from "./runtimeDiscovery";

export class NativeInterfaceManager {
  private process: cp.ChildProcess | undefined;

  constructor(private readonly output: vscode.OutputChannel) {}

  /**
   * Finds where requests should go, in order of preference:
   * 1. the desktop coding agent (Settings -> Agent Setup),
   * 2. a desktop llama-server from an older desktop build (reported as bypassing Agent Setup),
   * 3. the native interface fallback.
   */
  async ensureRunning(): Promise<RuntimeTarget | undefined> {
    const config = vscode.workspace.getConfiguration("openmindai");
    const codingAgent = await findCodingAgentEndpoint();
    if (codingAgent) return this.announce(codingAgent);
    if (hasLiveCodingAgentDescriptor()) {
      throw new Error(
        "The OpenMindAI desktop app is running, but its coding agent did not respond. Try again in a moment."
      );
    }

    const desktopTarget = await discoverDesktopRuntime();
    if (desktopTarget) return this.announce(desktopTarget);

    if (await endpointReady()) return this.announce({ endpoint: NATIVE_INTERFACE_ENDPOINT, kind: "native-gateway" });
    if (!config.get<boolean>("autoStartNativeInterface", false)) return undefined;

    const root = await this.detectRoot();
    if (!root) return undefined;
    return this.announce({ endpoint: await this.start(root), kind: "native-gateway" });
  }

  private lastAnnounced = "";

  private announce(target: RuntimeTarget): RuntimeTarget {
    const description = `${describeTarget(target)} at ${target.endpoint}`;
    if (description !== this.lastAnnounced) {
      this.lastAnnounced = description;
      this.output.appendLine(`Using ${description}`);
      if (target.kind === "llama-server") {
        this.output.appendLine(
          "The desktop coding-agent endpoint was not found, so requests use whichever model the desktop runtime has loaded instead of the Agent Setup coding agent. Update the OpenMindAI desktop app to route VS Code through Agent Setup."
        );
      }
    }
    return target;
  }

  async start(root?: string): Promise<string> {
    const resolvedRoot = root ?? (await this.detectRoot());
    if (!resolvedRoot) {
      throw new Error("OpenMindAI root was not found. Set openmindai.nativeInterfaceRoot.");
    }

    const apiExe = path.join(resolvedRoot, "native-vulkan-artifact", "openmind-api.exe");
    const workerExe = path.join(resolvedRoot, "native-vulkan-artifact", "openmind-native-worker.exe");
    const runtimeDir = path.join(resolvedRoot, "native-vulkan-artifact");
    const logsDir = path.join(resolvedRoot, "logs");

    for (const required of [apiExe, workerExe, runtimeDir]) {
      if (!fs.existsSync(required)) {
        throw new Error(`Native interface file is missing: ${required}`);
      }
    }

    fs.mkdirSync(logsDir, { recursive: true });
    const registryPath = await this.writeRegistry(resolvedRoot, logsDir);
    const outPath = path.join(logsDir, "openmind-api-vscode.out.log");
    const errPath = path.join(logsDir, "openmind-api-vscode.err.log");
    const out = fs.openSync(outPath, "a");
    const err = fs.openSync(errPath, "a");

    this.output.appendLine(`Starting OpenMindAI native interface: ${apiExe}`);
    this.process = cp.spawn(apiExe, [], {
      cwd: path.dirname(apiExe),
      detached: true,
      windowsHide: true,
      stdio: ["ignore", out, err],
      env: {
        ...process.env,
        OPENMINDAI_API_BACKEND: "native",
        OPENMINDAI_NATIVE_WORKER: workerExe,
        OPENMINDAI_NATIVE_MODELS: registryPath,
        OPENMINDAI_API_GENERATION_TIMEOUT: "120s",
        PATH: `${runtimeDir}${path.delimiter}${process.env.PATH ?? ""}`
      }
    });
    this.process.unref();

    await waitForReady(20000);
    return NATIVE_INTERFACE_ENDPOINT;
  }

  async stop(): Promise<void> {
    const owners = await owningProcesses(11435);
    for (const pid of owners) {
      try {
        process.kill(pid);
        this.output.appendLine(`Stopped OpenMindAI native interface process ${pid}.`);
      } catch (error) {
        this.output.appendLine(`Could not stop process ${pid}: ${String(error)}`);
      }
    }
  }

  async build(): Promise<void> {
    const root = await this.detectRoot();
    if (!root) {
      throw new Error("OpenMindAI root was not found. Set openmindai.nativeInterfaceRoot.");
    }
    const serviceDir = path.join(root, "services", "inference-api");
    const outputExe = path.join(root, "native-vulkan-artifact", "openmind-api.exe");
    if (!fs.existsSync(path.join(serviceDir, "cmd", "openmind-api", "main.go"))) {
      throw new Error(`Inference API source was not found: ${serviceDir}`);
    }
    fs.mkdirSync(path.dirname(outputExe), { recursive: true });
    await runProcess("go", ["build", "-o", outputExe, "./cmd/openmind-api"], serviceDir, this.output);
  }

  private async detectRoot(): Promise<string | undefined> {
    const configured = vscode.workspace
      .getConfiguration("openmindai")
      .get<string>("nativeInterfaceRoot", "")
      .trim();
    if (configured) return configured;

    for (const folder of vscode.workspace.workspaceFolders ?? []) {
      let current = folder.uri.fsPath;
      for (let i = 0; i < 5; i += 1) {
        if (
          fs.existsSync(path.join(current, "services", "inference-api", "cmd", "openmind-api", "main.go")) &&
          fs.existsSync(path.join(current, "native-vulkan-artifact"))
        ) {
          return current;
        }
        const parent = path.dirname(current);
        if (parent === current) break;
        current = parent;
      }
    }
    return undefined;
  }

  private async writeRegistry(root: string, logsDir: string): Promise<string> {
    const config = vscode.workspace.getConfiguration("openmindai");
    const modelId = config.get<string>("model", "openmind-local");
    const modelPath = await this.findModel(root);
    const registryPath = path.join(logsDir, "openmind-native-models.json");
    const registry = {
      [modelId]: {
        path: modelPath,
        gpu_layers: config.get<number>("nativeGpuLayers", 0),
        context_size: config.get<number>("nativeContextSize", 1024)
      }
    };
    fs.writeFileSync(registryPath, JSON.stringify(registry, null, 2), { encoding: "utf8" });
    return registryPath;
  }

  private async findModel(root: string): Promise<string> {
    const configured = vscode.workspace
      .getConfiguration("openmindai")
      .get<string>("nativeModelPath", "")
      .trim();
    if (configured) return configured;

    const smokeModel = path.join(root, "native-smoke-model.gguf");
    if (fs.existsSync(smokeModel)) return smokeModel;

    const modelsDir = path.join(root, "models");
    const found = findFirstGguf(modelsDir);
    if (found) return found;

    throw new Error("No GGUF model found. Set openmindai.nativeModelPath.");
  }
}

async function endpointReady(): Promise<boolean> {
  try {
    await requestJson(`${NATIVE_INTERFACE_ENDPOINT}/readyz`, 2000);
    return true;
  } catch {
    return false;
  }
}

async function discoverDesktopRuntime(): Promise<RuntimeTarget | undefined> {
  if (process.platform !== "win32") return undefined;
  const endpoint = await findDesktopRuntimeEndpoint(await listeningPortsByProcessName("llama-server"));
  if (!endpoint) return undefined;
  const loadedModel = await requestJson(`${endpoint}/v1/models`, 1500).then(loadedModelName, () => undefined);
  return { endpoint, kind: "llama-server", loadedModel };
}

async function waitForReady(timeoutMs: number): Promise<void> {
  const started = Date.now();
  let lastError = "";
  while (Date.now() - started < timeoutMs) {
    try {
      await requestJson(`${NATIVE_INTERFACE_ENDPOINT}/readyz`, 2000);
      return;
    } catch (error) {
      lastError = error instanceof Error ? error.message : String(error);
      await new Promise((resolve) => setTimeout(resolve, 500));
    }
  }
  throw new Error(`Native interface did not become ready: ${lastError}`);
}

async function owningProcesses(port: number): Promise<number[]> {
  if (process.platform !== "win32") return [];
  return new Promise((resolve) => {
    cp.exec(`powershell -NoProfile -Command "Get-NetTCPConnection -LocalPort ${port} -ErrorAction SilentlyContinue | Select-Object -ExpandProperty OwningProcess -Unique"`, (error, stdout) => {
      if (error) {
        resolve([]);
        return;
      }
      resolve(
        stdout
          .split(/\r?\n/)
          .map((line) => Number(line.trim()))
          .filter((pid) => Number.isInteger(pid) && pid > 0)
      );
    });
  });
}

function runProcess(
  command: string,
  args: string[],
  cwd: string,
  output: vscode.OutputChannel
): Promise<void> {
  return new Promise((resolve, reject) => {
    output.appendLine(`Running ${command} ${args.join(" ")}`);
    const child = cp.spawn(command, args, { cwd, windowsHide: true });
    child.stdout.on("data", (chunk) => output.append(chunk.toString()));
    child.stderr.on("data", (chunk) => output.append(chunk.toString()));
    child.on("error", reject);
    child.on("exit", (code) => {
      if (code === 0) resolve();
      else reject(new Error(`${command} exited with code ${code}`));
    });
  });
}

function findFirstGguf(root: string): string | undefined {
  if (!fs.existsSync(root)) return undefined;
  const entries = fs.readdirSync(root, { withFileTypes: true });
  for (const entry of entries) {
    const fullPath = path.join(root, entry.name);
    if (entry.isFile() && entry.name.toLowerCase().endsWith(".gguf")) return fullPath;
    if (entry.isDirectory()) {
      const found = findFirstGguf(fullPath);
      if (found) return found;
    }
  }
  return undefined;
}
