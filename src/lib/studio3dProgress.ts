// Percentages represent completed pipeline checkpoints, not estimated LLM tokens,
// wall-clock time, or provider-specific GPU progress. Unknown stages stay at 0.
export type Studio3dProgress = { percent: number; label: string };
const LOCAL: Record<string, Studio3dProgress> = {
  preparing: { percent: 0, label: "Preparing your request" },
  queued: { percent: 0, label: "Queued for generation" },
  scheduled: { percent: 5, label: "Preparing local engine" },
  "local-model-inference": { percent: 10, label: "Loading local AI model" },
  "understanding-prompt": { percent: 10, label: "Understanding your prompt" },
  "planning-scene": { percent: 30, label: "Planning scene geometry" },
  "reviewing-prompt-fidelity": { percent: 50, label: "Checking prompt fidelity" },
  "refining-scene": { percent: 50, label: "Refining missing details" },
  "geometry-generation": { percent: 65, label: "Creating editable geometry" },
  "worker-queued": { percent: 65, label: "Geometry queued" },
  "worker-running": { percent: 65, label: "Building 3D mesh" },
  "worker-completed": { percent: 85, label: "Mesh built; validating output" },
  "output-validation": { percent: 85, label: "Validating and saving GLB" },
  published: { percent: 95, label: "Loading saved 3D model" },
};
const CLOUD: Record<string, Studio3dProgress> = {
  preparing: { percent: 0, label: "Preparing your request" },
  queued: { percent: 0, label: "Queued for generation" },
  "provider-submission": { percent: 10, label: "Submitting to Cloud engine" },
  "provider-queued": { percent: 20, label: "Queued on Cloud" },
  "provider-pending": { percent: 20, label: "Queued on Cloud" },
  "provider-running": { percent: 20, label: "Cloud is generating your model" },
  "provider-processing": { percent: 20, label: "Cloud is generating your model" },
  "provider-completed": { percent: 80, label: "Downloading and validating result" },
  "provider-complete": { percent: 80, label: "Downloading and validating result" },
  "provider-done": { percent: 80, label: "Downloading and validating result" },
  "provider-success": { percent: 80, label: "Downloading and validating result" },
  published: { percent: 95, label: "Loading saved 3D model" },
};
export function studio3dProgress(route: string, stage: string, meshReady: boolean): Studio3dProgress {
  if (meshReady) return { percent: 100, label: "3D model ready" };
  const stages = route === "cloud" ? CLOUD : LOCAL;
  return stages[stage] ?? { percent: 0, label: "Waiting for generation status" };
}
