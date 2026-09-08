import { useCallback, useEffect, useMemo, useState } from "react";
import {
  CheckCircle2,
  CircleAlert,
  Clock3,
  Gauge,
  History,
  LoaderCircle,
  PauseCircle,
  Play,
  RefreshCw,
  RotateCcw,
  ShieldCheck,
  XCircle,
} from "lucide-react";
import {
  api,
  type CodingApproval,
  type CodingRunSnapshot,
  type OpenAgentRun,
} from "../api";
import { formatError, formatTime } from "../lib/format";
import "../coding-workspace.css";

export function CodingRunTimeline(props: { conversationIds: string[] }) {
  const [runs, setRuns] = useState<OpenAgentRun[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [snapshot, setSnapshot] = useState<CodingRunSnapshot | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    if (!props.conversationIds.length) {
      setRuns([]);
      setSelectedId(null);
      setSnapshot(null);
      return;
    }
    const groups = await Promise.all(
      props.conversationIds.slice(-20).map((id) => api.listOpenAgentRuns(id, 8)),
    );
    const byId = new Map<string, OpenAgentRun>();
    for (const group of groups) {
      for (const run of group) byId.set(run.id, run);
    }
    const next = Array.from(byId.values())
      .sort((left, right) => right.updatedAt.localeCompare(left.updatedAt))
      .slice(0, 24);
    setRuns(next);
    setSelectedId((current) =>
      current && next.some((run) => run.id === current) ? current : next[0]?.id ?? null,
    );
  }, [props.conversationIds]);

  useEffect(() => {
    void refresh().catch((caught) => setError(formatError(caught)));
  }, [refresh]);

  useEffect(() => {
    if (!selectedId) {
      setSnapshot(null);
      return;
    }
    let alive = true;
    void api
      .codingRunSnapshot(selectedId)
      .then((value) => {
        if (alive) setSnapshot(value);
      })
      .catch((caught) => {
        if (alive) setError(formatError(caught));
      });
    return () => {
      alive = false;
    };
  }, [selectedId]);

  const selectedRun = useMemo(
    () => runs.find((run) => run.id === selectedId) ?? null,
    [runs, selectedId],
  );

  const mutate = async (key: string, action: () => Promise<unknown>) => {
    if (busy) return;
    setBusy(key);
    setError(null);
    try {
      await action();
      await refresh();
      if (selectedId) setSnapshot(await api.codingRunSnapshot(selectedId));
    } catch (caught) {
      setError(formatError(caught));
    } finally {
      setBusy(null);
    }
  };

  const decide = (approval: CodingApproval, approved: boolean) =>
    mutate(`${approved ? "approve" : "reject"}-${approval.id}`, () =>
      approved ? api.approveCodingAction(approval.id) : api.rejectCodingAction(approval.id),
    );

  if (!runs.length) {
    return (
      <section className="coding-timeline coding-timeline-empty">
        <History size={17} />
        <span>
          <strong>Coding run timeline</strong>
          <small>Plans, validations, approvals, budgets, and recovery controls appear here after a coding task starts.</small>
        </span>
      </section>
    );
  }

  return (
    <section className="coding-timeline">
      <header className="coding-timeline-head">
        <div>
          <span className="cg-work-eyebrow">Execution evidence</span>
          <strong>Coding run timeline</strong>
        </div>
        <button type="button" disabled={busy !== null} onClick={() => void refresh()} title="Refresh timeline">
          <RefreshCw size={14} className={busy === "refresh" ? "spin" : ""} /> Refresh
        </button>
      </header>

      {error ? <button className="coding-error" onClick={() => setError(null)}>{error}</button> : null}

      <div className="coding-run-tabs">
        {runs.slice(0, 8).map((run) => (
          <button
            type="button"
            key={run.id}
            className={run.id === selectedId ? "active" : ""}
            onClick={() => setSelectedId(run.id)}
          >
            {statusIcon(run.status)}
            <span>
              <strong>{run.status}</strong>
              <small>{formatTime(run.updatedAt)}</small>
            </span>
          </button>
        ))}
      </div>

      {selectedRun && snapshot ? (
        <div className="coding-run-detail">
          <div className="coding-run-summary">
            <span><Clock3 size={14} /> Step {selectedRun.currentStep}/{selectedRun.maxSteps}</span>
            <span><ShieldCheck size={14} /> Validation {selectedRun.validationStatus.replaceAll("_", " ")}</span>
            <span><Gauge size={14} /> {metricSummary(snapshot)}</span>
          </div>

          {snapshot.plan ? (
            <div className="coding-plan-card">
              <div className="coding-card-title">
                <strong>Plan revision {snapshot.plan.revision}</strong>
                <small>{snapshot.plan.reason}</small>
              </div>
              <ol>
                {snapshot.plan.steps.map((step) => (
                  <li key={step.id} data-status={step.status}>
                    <span>{step.title}</span><small>{step.status}</small>
                  </li>
                ))}
              </ol>
            </div>
          ) : null}

          {snapshot.approvals.some((approval) => approval.status === "pending") ? (
            <div className="coding-approval-stack">
              {snapshot.approvals
                .filter((approval) => approval.status === "pending")
                .map((approval) => (
                  <div className="coding-approval-card" key={approval.id}>
                    <div>
                      <CircleAlert size={16} />
                      <span>
                        <strong>Approval required · {approval.tool}</strong>
                        <small>{approval.reason}</small>
                      </span>
                    </div>
                    <div className="button-row">
                      <button
                        type="button"
                        disabled={busy !== null}
                        onClick={() => void decide(approval, true)}
                      >
                        {busy === `approve-${approval.id}` ? <LoaderCircle className="spin" size={14} /> : <CheckCircle2 size={14} />}
                        Approve exact action
                      </button>
                      <button
                        type="button"
                        disabled={busy !== null}
                        onClick={() => void decide(approval, false)}
                      >
                        <XCircle size={14} /> Reject
                      </button>
                    </div>
                  </div>
                ))}
            </div>
          ) : null}

          <div className="coding-timeline-events">
            {snapshot.events.slice(-20).reverse().map((event) => (
              <div key={event.id}>
                <span className="coding-event-dot" data-kind={event.kind} />
                <span>
                  <strong>{event.label}</strong>
                  <small>{event.kind} · {formatTime(event.createdAt)}</small>
                </span>
              </div>
            ))}
          </div>

          <div className="coding-recovery-row">
            {selectedRun.status === "interrupted" ? (
              <button
                type="button"
                disabled={busy !== null}
                onClick={() => void mutate("resume", () => api.resumeCodingRun(selectedRun.id))}
              >
                {busy === "resume" ? <LoaderCircle className="spin" size={14} /> : <Play size={14} />}
                Resume safely
              </button>
            ) : null}
            {snapshot.details.checkpoints.length ? (
              <button
                type="button"
                disabled={busy !== null || selectedRun.status === "running"}
                onClick={() => {
                  const checkpoint = [...snapshot.details.checkpoints]
                    .reverse()
                    .find((item) => item.kind === "before_mutation");
                  if (checkpoint) {
                    void mutate("restore", () => api.restoreOpenAgentCheckpoint(checkpoint.id));
                  }
                }}
              >
                {busy === "restore" ? <LoaderCircle className="spin" size={14} /> : <RotateCcw size={14} />}
                Restore latest safe checkpoint
              </button>
            ) : null}
          </div>
        </div>
      ) : null}
    </section>
  );
}

function statusIcon(status: OpenAgentRun["status"]) {
  if (status === "completed") return <CheckCircle2 size={14} />;
  if (status === "running") return <LoaderCircle className="spin" size={14} />;
  if (status === "interrupted") return <PauseCircle size={14} />;
  return <XCircle size={14} />;
}

function metricSummary(snapshot: CodingRunSnapshot) {
  const metrics = snapshot.metrics;
  if (!metrics) return "metrics unavailable";
  const tokens = metrics.promptTokens + metrics.completionTokens;
  const runtimeSeconds = Math.round((metrics.modelMs + metrics.toolMs) / 1000);
  return `${tokens.toLocaleString()} tokens · ${runtimeSeconds}s measured · $0 local`;
}
