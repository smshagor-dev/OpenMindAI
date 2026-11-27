import test from "node:test";
import assert from "node:assert/strict";
import { studio3dProgress } from "../src/lib/studio3dProgress.ts";

test("local checkpoints reflect actual pipeline stages and do not reach 100 before mesh is ready", () => {
  const stages = ["queued","scheduled","understanding-prompt","planning-scene","reviewing-prompt-fidelity","refining-scene","geometry-generation","worker-running","worker-completed","output-validation","published"];
  const values = stages.map(stage => studio3dProgress("local",stage,false).percent);
  assert.deepEqual(values, [0,5,10,30,50,50,65,65,85,85,95]);
  assert.equal(studio3dProgress("local","published",true).percent,100);
});
test("Cloud progress stays at a checkpoint until provider completion", () => {
  assert.equal(studio3dProgress("cloud","provider-queued",false).percent,20);
  assert.equal(studio3dProgress("cloud","provider-running",false).percent,20);
  assert.equal(studio3dProgress("cloud","provider-completed",false).percent,80);
  assert.equal(studio3dProgress("cloud","published",false).percent,95);
});
test("unknown progress cannot imply success", () => {
  assert.equal(studio3dProgress("local","unexpected",false).percent,0);
  assert.equal(studio3dProgress("cloud","provider-failed",false).percent,0);
});
