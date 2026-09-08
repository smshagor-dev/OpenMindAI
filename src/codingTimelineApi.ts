import { api, type OpenAgentRunDetails } from "./api";

export type CodingPlanStep = {
  id: string;
  title: string;
  status: string;
};

export type CodingPlan = {
  revision: number;
  reason: string;
  steps: CodingPlanStep[];
};

export type CodingEvent = {
  id: string;
  runId: string;
  sequence: number;
  kind: string;
  label: string;
  detailJson: string | null;
  createdAt: string;
};

export type CodingApproval = {
  id: string;
  runId: string;
  stepId: string | null;
  actionHash: string;
  tool: string;
  actionJson: string;
  reason: string;
  status: "pending" | "approved" | "rejected" | "consumed";
  requestedAt: string;
  decidedAt: string | null;
  consumedAt: string | null;
};

export type CodingMetrics = {
  runId: string;
  promptTokens: number;
  completionTokens: number;
  modelMs: number;
  toolMs: number;
  toolCalls: number;
  workerCalls: number;
  localCostMicros: number;
  hardwareJson: string;
  budgetStopReason: string | null;
  updatedAt: string;
};

export type CodingRunSnapshot = {
  details: OpenAgentRunDetails;
  plan: CodingPlan | null;
  events: CodingEvent[];
  approvals: CodingApproval[];
  metrics: CodingMetrics | null;
  parentRunId: string | null;
};

function legacyEvents(details: OpenAgentRunDetails): CodingEvent[] {
  return details.steps.map((step, index) => ({
    id: step.id,
    runId: step.runId,
    sequence: index + 1,
    kind: "tool",
    label: `${step.tool} ${step.status}`,
    detailJson: step.error ?? step.resultSummary,
    createdAt: step.completedAt ?? step.startedAt,
  }));
}

async function codingRunSnapshot(runId: string): Promise<CodingRunSnapshot | null> {
  const details = await api.openAgentRunDetails(runId);
  if (!details) return null;
  return {
    details,
    plan: null,
    events: legacyEvents(details),
    approvals: [],
    metrics: null,
    parentRunId: null,
  };
}

async function resumeCodingRun(runId: string) {
  const details = await api.openAgentRunDetails(runId);
  if (!details) throw new Error("Coding run not found.");
  return api.sendChatMessage(details.run.conversationId, details.run.goal, "chat");
}

async function unsupportedApproval(): Promise<never> {
  throw new Error("Exact coding approvals are unavailable until the durable coding-control backend is active.");
}

export const codingTimelineApi = {
  codingRunSnapshot,
  resumeCodingRun,
  approveCodingAction: (_approvalId: string) => unsupportedApproval(),
  rejectCodingAction: (_approvalId: string) => unsupportedApproval(),
};
