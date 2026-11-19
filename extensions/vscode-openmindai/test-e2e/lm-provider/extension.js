// Test-only language model provider for the OpenMindAI E2E suite.
//
// An isolated VS Code profile has no chat model, and the Chat view does not dispatch
// messages without one. This provider only makes the Chat UI operational; it never answers
// @openmindai requests (the OpenMindAI participant calls its own coding agent). Every reply
// is tagged so a test can detect if it was ever used for an OpenMindAI answer.
const vscode = require("vscode");

const MARKER = "[openmindai-e2e-test-model]";

exports.MARKER = MARKER;

exports.activate = function activate(context) {
  const calls = [];
  context.subscriptions.push(
    vscode.lm.registerLanguageModelChatProvider("openmindai-e2e", {
      provideLanguageModelChatInformation() {
        return [
          {
            id: "openmindai-e2e-echo",
            name: "OpenMindAI E2E test model",
            family: "openmindai-e2e",
            version: "1",
            maxInputTokens: 32768,
            maxOutputTokens: 1024,
            capabilities: {}
          }
        ];
      },
      async provideLanguageModelChatResponse(_model, messages, _options, progress) {
        calls.push(messages.length);
        progress.report(new vscode.LanguageModelTextPart(`${MARKER} test model reply`));
      },
      async provideTokenCount(_model, text) {
        const value = typeof text === "string" ? text : JSON.stringify(text);
        return Math.ceil(value.length / 4);
      }
    })
  );
  return { marker: MARKER, calls: () => calls.slice() };
};

exports.deactivate = function deactivate() {};
