// Integration-test harness for the real VS Code UI (npm run test:e2e). Loaded only when VS Code
// runs the extension in Test mode; excluded from the packaged VSIX.
import * as vscode from "vscode";
import { ChatSurfaceObserver, OpenMindChatPanel, OpenMindChatViewProvider, setExtraWebviewScript } from "./chatPanel";
import type { ParticipantOutput } from "./extension";
import type { RuntimeTarget } from "./runtimeDiscovery";

export interface WebviewSnapshot {
  bubbles: Array<{ role: string; text: string }>;
  activity: { phase: string; detail: string };
  busy: boolean;
}

/** Types into the real webview form and reads back its rendered messages. */
export class ChatSurfaceTestHandle implements ChatSurfaceObserver {
  busy = false;
  private readonly pending: Array<(snapshot: WebviewSnapshot) => void> = [];

  constructor(private readonly post: (message: unknown) => void) {}

  submit(text: string): void {
    this.post({ type: "__openmindaiTestSubmit", text });
  }

  snapshot(): Promise<WebviewSnapshot> {
    return new Promise((resolve) => {
      this.pending.push(resolve);
      this.post({ type: "__openmindaiTestSnapshot" });
    });
  }

  receive(message: unknown): boolean {
    const data = message as { type?: string } & Partial<WebviewSnapshot>;
    if (data?.type !== "__openmindaiTestSnapshot") return false;
    const snapshot: WebviewSnapshot = {
      bubbles: data.bubbles ?? [],
      activity: data.activity ?? { phase: "", detail: "" },
      busy: Boolean(data.busy)
    };
    this.pending.splice(0).forEach((resolve) => resolve(snapshot));
    return true;
  }

  busyChanged(busy: boolean): void {
    this.busy = busy;
  }
}

// Runs inside the chat webview: submits through the real form and reports rendered bubbles.
const WEBVIEW_TEST_SCRIPT = `
    window.addEventListener('message', (event) => {
      const message = event.data;
      if (message.type === '__openmindaiTestSubmit') {
        input.value = message.text;
        form.requestSubmit();
      }
      if (message.type === '__openmindaiTestSnapshot') {
        const bubbles = Array.from(messages.querySelectorAll('section.message')).map((section) => ({
          role: (section.querySelector('.role') || {}).textContent || '',
          text: (section.querySelector('.body') || {}).textContent || ''
        }));
        vscode.postMessage({
          type: '__openmindaiTestSnapshot',
          bubbles,
          activity: { phase: activityPhase.textContent || '', detail: activityDetail.textContent || '' },
          busy: send.disabled
        });
      }
    });`;

export interface TestHookDependencies {
  viewProvider: OpenMindChatViewProvider;
  openPanel: () => OpenMindChatPanel;
  activePanel: () => OpenMindChatPanel | undefined;
  runParticipant: (prompt: string, output: ParticipantOutput) => Promise<void>;
  target: () => RuntimeTarget | undefined;
  setEventSink: (sink: (kind: string, text: string) => void) => void;
  setFixConfirm: (confirm: () => Thenable<string | undefined>) => void;
}

function attach(surface: OpenMindChatPanel | OpenMindChatViewProvider): ChatSurfaceTestHandle {
  if (!(surface.observer instanceof ChatSurfaceTestHandle)) {
    surface.observer = new ChatSurfaceTestHandle((message) => surface.postMessage(message));
  }
  return surface.observer as ChatSurfaceTestHandle;
}

export function installTestHooks(deps: TestHookDependencies) {
  setExtraWebviewScript(WEBVIEW_TEST_SCRIPT);
  const log: Array<{ kind: string; text: string }> = [];
  deps.setEventSink((kind, text) => log.push({ kind, text }));
  // Modal dialogs cannot be answered in an extension test host.
  deps.setFixConfirm(() => Promise.resolve("Apply"));
  const viewHandle = attach(deps.viewProvider);
  return {
    viewProvider: {
      get resolved(): boolean {
        return deps.viewProvider.resolved;
      },
      test: viewHandle
    },
    openPanel: () => {
      attach(deps.openPanel());
    },
    activePanel: () => {
      const panel = deps.activePanel();
      return panel ? { test: attach(panel) } : undefined;
    },
    runParticipant: async (prompt: string) => {
      const events: Array<{ kind: string; text: string }> = [];
      await deps.runParticipant(prompt, {
        progress: (text) => events.push({ kind: "progress", text }),
        markdown: (text) => events.push({ kind: "markdown", text })
      });
      return events;
    },
    target: deps.target,
    testLog: () => log.slice(),
    clearTestLog: () => log.splice(0),
    vscodeVersion: vscode.version
  };
}
