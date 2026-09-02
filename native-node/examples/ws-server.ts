import { WebSocketServer } from "ws";
// `npm run build` in native-node generates index.js/index.d.ts.
// eslint-disable-next-line @typescript-eslint/no-var-requires
const { NativeLlama } = require("../index.js") as {
  NativeLlama: new (
    modelPath: string,
    options?: { baseContextTokens?: number; gpuLayers?: number },
  ) => {
    generate(
      prompt: string,
      systemPrompt: string | undefined,
      options: { temperature?: number; topP?: number; maxTokens?: number } | undefined,
      events: (kind: "token" | "done" | "error", data: string) => void,
    ): void;
  };
};

const modelPath = process.env.OPENMINDAI_MODEL_PATH;
if (!modelPath) {
  throw new Error("OPENMINDAI_MODEL_PATH is required");
}

const engine = new NativeLlama(modelPath, {
  baseContextTokens: 8192,
  gpuLayers: -1,
});

const wss = new WebSocketServer({ port: 8787 });

wss.on("connection", (socket) => {
  socket.on("message", (raw) => {
    const request = JSON.parse(raw.toString()) as {
      prompt: string;
      systemPrompt?: string;
      temperature?: number;
      topP?: number;
      maxTokens?: number;
    };

    engine.generate(
      request.prompt,
      request.systemPrompt,
      {
        temperature: request.temperature,
        topP: request.topP,
        maxTokens: request.maxTokens,
      },
      (kind, data) => {
        if (socket.readyState === socket.OPEN) {
          socket.send(JSON.stringify({ kind, data }));
        }
      },
    );
  });
});

console.log("OpenMindAI native token WebSocket listening on ws://127.0.0.1:8787");
