import type {
  Artifact,
  Message,
  StreamChunkEvent,
  StreamDoneEvent,
  StreamStartedEvent,
  WebResearchProgressEvent,
} from "../types";

type Owner = string | null;
type Change<T> = T | ((current: T) => T);
export interface ChatActivity {
  typing: boolean;
  searching: boolean;
  researching: boolean;
  detail: string | null;
}
export interface ConversationRun {
  request: number | null;
  streamingId: string | null;
  activity: ChatActivity;
  webProgress: WebResearchProgressEvent | null;
  error: string | null;
  live: Message[];
}
export interface RequestLease {
  readonly token: number;
  conversationId: Owner;
  streamId: string | null;
}
interface Snapshot {
  activeId: Owner;
  epoch: number;
  messages: Message[];
  artifacts: Artifact[];
  runs: ReadonlyMap<Owner, ConversationRun>;
}
type Collection = "messages" | "artifacts";
interface LoadTicket {
  owner: string;
  epoch: number;
  sequence: number;
  revision: number;
}
const idleActivity = (): ChatActivity => ({
  typing: false,
  searching: false,
  researching: false,
  detail: null,
});
const idleRun = (): ConversationRun => ({
  request: null,
  streamingId: null,
  activity: idleActivity(),
  webProgress: null,
  error: null,
  live: [],
});
const resolve = <T>(change: Change<T>, current: T): T =>
  typeof change === "function" ? (change as (value: T) => T)(current) : change;

/** One visible history, with independently owned background generation state. */
export class ConversationState {
  private snapshot: Snapshot = {
    activeId: null,
    epoch: 0,
    messages: [],
    artifacts: [],
    runs: new Map(),
  };
  private listeners = new Set<() => void>();
  private requestSequence = 0;
  private requests = new Map<number, RequestLease>();
  private loadSequence = { messages: 0, artifacts: 0 };
  private revision = { messages: 0, artifacts: 0 };
  private touched = { messages: new Map<string, number>(), artifacts: new Map<string, number>() };
  private terminal = new Map<string, Map<string, Message["status"]>>();

  getSnapshot = () => this.snapshot;
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };
  private publish(next: Snapshot) {
    this.snapshot = next;
    this.listeners.forEach((listener) => listener());
  }
  run(owner: Owner): ConversationRun {
    return this.snapshot.runs.get(owner) ?? idleRun();
  }
  isBusy(owner: Owner) {
    const run = this.run(owner);
    return run.request !== null || run.streamingId !== null;
  }
  anyBusy() {
    return [...this.snapshot.runs.keys()].some((owner) => this.isBusy(owner));
  }
  async cancelGeneration(owner: Owner, cancel: (conversationId: string) => Promise<void>) {
    if (owner && this.run(owner).streamingId) await cancel(owner);
  }
  cancelActiveGeneration(cancel: (conversationId: string) => Promise<void>) {
    return this.cancelGeneration(this.snapshot.activeId, cancel);
  }
  captureView() {
    return { owner: this.snapshot.activeId, epoch: this.snapshot.epoch };
  }
  isCurrent(view: { owner: Owner; epoch: number }) {
    return this.snapshot.activeId === view.owner && this.snapshot.epoch === view.epoch;
  }

  selectConversation = (change: Change<Owner>) => {
    const activeId = resolve(change, this.snapshot.activeId);
    if (activeId === this.snapshot.activeId) return;
    this.touched = { messages: new Map(), artifacts: new Map() };
    this.revision = { messages: 0, artifacts: 0 };
    this.publish({
      ...this.snapshot,
      activeId,
      epoch: this.snapshot.epoch + 1,
      messages: this.run(activeId).live,
      artifacts: [],
    });
  };

  updateRun(owner: Owner, change: (current: ConversationRun) => ConversationRun) {
    const runs = new Map(this.snapshot.runs);
    runs.set(owner, change(this.run(owner)));
    this.publish({ ...this.snapshot, runs });
  }
  scope(owner: Owner) {
    return {
      setMessages: (change: Change<Message[]>) => this.setMessages(owner, change),
      setArtifacts: (change: Change<Artifact[]>) => this.setArtifacts(owner, change),
      setActivity: (change: Change<ChatActivity>) =>
        this.updateRun(owner, (run) => ({ ...run, activity: resolve(change, run.activity) })),
      setWebProgress: (change: Change<WebResearchProgressEvent | null>) =>
        this.updateRun(owner, (run) => {
          const webProgress = resolve(change, run.webProgress);
          if (
            webProgress &&
            (webProgress.conversationId !== owner ||
              this.terminal.get(webProgress.conversationId)?.has(webProgress.messageId) ||
              (run.streamingId && run.streamingId !== webProgress.messageId))
          )
            return run;
          return { ...run, webProgress };
        }),
      setError: (error: string | null) => this.updateRun(owner, (run) => ({ ...run, error })),
    };
  }
  private markChanges<T extends { id: string }>(kind: Collection, previous: T[], next: T[]) {
    const revision = ++this.revision[kind];
    const before = new Map(previous.map((item) => [item.id, item]));
    const after = new Map(next.map((item) => [item.id, item]));
    for (const [id, item] of before)
      if (after.get(id) !== item) this.touched[kind].set(id, revision);
    for (const [id, item] of after)
      if (before.get(id) !== item) this.touched[kind].set(id, revision);
  }
  setMessages(owner: Owner, change: Change<Message[]>) {
    if (owner !== this.snapshot.activeId) return;
    const messages = resolve(change, this.snapshot.messages).filter(
      (item) => item.conversationId === owner && item.role !== "system",
    );
    this.markChanges("messages", this.snapshot.messages, messages);
    this.publish({ ...this.snapshot, messages });
  }
  setArtifacts(owner: Owner, change: Change<Artifact[]>) {
    if (owner !== this.snapshot.activeId) return;
    const artifacts = resolve(change, this.snapshot.artifacts).filter(
      (item) => item.conversationId === owner,
    );
    this.markChanges("artifacts", this.snapshot.artifacts, artifacts);
    this.publish({ ...this.snapshot, artifacts });
  }

  private beginLoad(owner: string, kind: Collection): LoadTicket | null {
    if (owner !== this.snapshot.activeId) return null;
    return {
      owner,
      epoch: this.snapshot.epoch,
      sequence: ++this.loadSequence[kind],
      revision: this.revision[kind],
    };
  }
  private currentLoad(ticket: LoadTicket, kind: Collection) {
    return (
      this.isCurrent({ owner: ticket.owner, epoch: ticket.epoch }) &&
      ticket.sequence === this.loadSequence[kind]
    );
  }
  private mergeLoad<T extends { id: string; conversationId: string }>(
    kind: Collection,
    ticket: LoadTicket,
    incoming: T[],
    current: T[],
  ): T[] {
    const rows = new Map(
      incoming.filter((row) => row.conversationId === ticket.owner).map((row) => [row.id, row]),
    );
    const latest = new Map(current.map((row) => [row.id, row]));
    // Events/local edits after this read started win, including deletions.
    for (const [id, revision] of this.touched[kind]) {
      if (revision <= ticket.revision) continue;
      const value = latest.get(id);
      if (value) rows.set(id, value);
      else rows.delete(id);
    }
    if (kind === "messages" && this.run(ticket.owner).request !== null) {
      for (const row of current) if (row.id.startsWith("local-")) rows.set(row.id, row);
    }
    return [...rows.values()];
  }
  async loadMessages(owner: string, fetch: (id: string) => Promise<Message[]>) {
    const ticket = this.beginLoad(owner, "messages");
    if (!ticket) return;
    try {
      const incoming = await fetch(owner);
      if (!this.currentLoad(ticket, "messages")) return;
      const messages = this.mergeLoad("messages", ticket, incoming, this.snapshot.messages)
        .filter((row) => row.role !== "system")
        .filter(
          (row) =>
            !row.id.startsWith("local-") ||
            !incoming.some(
              (native) =>
                native.conversationId === owner &&
                native.role === "user" &&
                native.content === row.content &&
                Date.parse(native.createdAt) >= Date.parse(row.createdAt) - 250,
            ),
        )
        .map((row) =>
          this.terminal.get(owner)?.has(row.id)
            ? { ...row, status: this.terminal.get(owner)!.get(row.id)! }
            : row,
        )
        .sort((a, b) => a.createdAt.localeCompare(b.createdAt));
      this.publish({ ...this.snapshot, messages });
      const runningId = this.run(owner).streamingId;
      const streaming = runningId
        ? messages.find((row) => row.id === runningId && row.status === "streaming")
        : messages
            .slice()
            .reverse()
            .find((row) => row.status === "streaming");
      if (streaming) {
        const user = messages
          .slice(0, messages.indexOf(streaming))
          .reverse()
          .find((row) => row.role === "user");
        this.updateRun(owner, (run) => ({
          ...run,
          streamingId: streaming.id,
          live: user ? [user, streaming] : [streaming],
          activity: { ...run.activity, typing: true },
        }));
      }
    } catch (error) {
      if (this.currentLoad(ticket, "messages")) throw error;
    }
  }
  async loadArtifacts(owner: string, fetch: (id: string) => Promise<Artifact[]>) {
    const ticket = this.beginLoad(owner, "artifacts");
    if (!ticket) return;
    try {
      const incoming = await fetch(owner);
      if (!this.currentLoad(ticket, "artifacts")) return;
      const artifacts = this.mergeLoad("artifacts", ticket, incoming, this.snapshot.artifacts);
      this.publish({ ...this.snapshot, artifacts });
    } catch (error) {
      if (this.currentLoad(ticket, "artifacts")) throw error;
    }
  }

  beginRequest(owner: Owner): RequestLease | null {
    // Keep the existing single foreground request policy for the shared runtime.
    if (this.anyBusy()) return null;
    const lease = { token: ++this.requestSequence, conversationId: owner, streamId: null };
    this.requests.set(lease.token, lease);
    this.updateRun(owner, () => ({ ...idleRun(), request: lease.token }));
    return lease;
  }
  assignRequest(lease: RequestLease, owner: string) {
    if (this.requests.get(lease.token) !== lease) return;
    const prior = this.run(lease.conversationId);
    this.updateRun(lease.conversationId, (run) => ({ ...run, request: null }));
    lease.conversationId = owner;
    this.updateRun(owner, (run) => ({ ...run, request: lease.token, error: prior.error }));
  }
  finishRequest(lease: RequestLease) {
    if (this.requests.get(lease.token) !== lease) return;
    this.requests.delete(lease.token);
    this.updateRun(lease.conversationId, (run) => {
      if (run.request !== lease.token) return run;
      if (run.streamingId && run.streamingId !== lease.streamId) return { ...run, request: null };
      return {
        ...run,
        request: null,
        streamingId: null,
        activity: idleActivity(),
        webProgress: null,
        live: [],
      };
    });
  }
  startStream(event: StreamStartedEvent) {
    const owner = event.conversationId;
    if (
      event.user.conversationId !== owner ||
      event.assistant.conversationId !== owner ||
      this.terminal.get(owner)?.has(event.assistant.id)
    )
      return false;
    const run = this.run(owner);
    if (
      run.streamingId === event.assistant.id &&
      run.live.some((row) => row.id === event.assistant.id)
    )
      return false;
    const knownAssistant =
      this.snapshot.activeId === owner
        ? this.snapshot.messages.find((row) => row.id === event.assistant.id)
        : undefined;
    const assistant = knownAssistant ?? event.assistant;
    const lease = this.requests.get(this.run(owner).request ?? -1);
    if (lease && !lease.streamId) lease.streamId = event.assistant.id;
    this.updateRun(owner, (run) => ({
      ...run,
      streamingId: assistant.id,
      live: [event.user, assistant],
      webProgress: run.webProgress?.messageId === assistant.id ? run.webProgress : null,
      activity: { ...run.activity, typing: true, detail: null },
    }));
    this.setMessages(owner, (rows) => [
      ...rows.filter(
        (row) =>
          row.id !== event.user.id &&
          row.id !== event.assistant.id &&
          !(row.id.startsWith("local-") && row.content === event.user.content),
      ),
      event.user,
      assistant,
    ]);
    return true;
  }
  streamChunk(event: StreamChunkEvent) {
    if (this.run(event.conversationId).streamingId !== event.messageId) return;
    const append = (rows: Message[]) =>
      rows.map((row) =>
        row.id === event.messageId && row.status === "streaming"
          ? { ...row, content: row.content + event.chunk }
          : row,
      );
    this.updateRun(event.conversationId, (run) => ({ ...run, live: append(run.live) }));
    this.setMessages(event.conversationId, append);
  }
  finishStream(event: StreamDoneEvent) {
    const terminal = this.terminal.get(event.conversationId) ?? new Map();
    terminal.set(event.messageId, event.status);
    while (terminal.size > 32) terminal.delete(terminal.keys().next().value!);
    this.terminal.set(event.conversationId, terminal);
    this.setMessages(event.conversationId, (rows) =>
      rows.map((row) => (row.id === event.messageId ? { ...row, status: event.status } : row)),
    );
    if (this.run(event.conversationId).streamingId !== event.messageId) return;
    this.updateRun(event.conversationId, (run) => ({
      ...run,
      streamingId: null,
      live: [],
      activity: idleActivity(),
      webProgress: run.webProgress?.messageId === event.messageId ? null : run.webProgress,
    }));
  }
}
