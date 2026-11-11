import test from "node:test";
import assert from "node:assert/strict";
import {
  selectExecutionRoute, isRecoverableCloudFailure, runWithFallback,
  offlineSourceNotice, ResponsePersistenceError,
} from "../src/lib/executionRouting.ts";

test("auto routing prefers Cloud when authenticated and online", () => {
  assert.equal(selectExecutionRoute({preference:"auto",task:"chat",online:true,cloudConnected:true,cloudModelReady:true}).target,"cloud");
  assert.equal(selectExecutionRoute({preference:"auto",task:"research",online:true,cloudConnected:true}).target,"cloud");
  assert.equal(selectExecutionRoute({preference:"auto",task:"scene3d",online:true,cloudConnected:true}).target,"cloud");
});

test("offline or disconnected Cloud always routes supported work locally", () => {
  for (const task of ["chat","vision","search","research","scene3d"]) {
    const offline = selectExecutionRoute({preference:"auto",task,online:false,cloudConnected:true});
    assert.equal(offline.target,"local",task);
    assert.equal(offline.offline,true);
    assert.equal(selectExecutionRoute({preference:"cloud",task,online:true,cloudConnected:false}).target,"local");
  }
});

test("explicit Local preference retains local processing when online", () => {
  assert.equal(selectExecutionRoute({preference:"local",task:"chat",online:true,cloudConnected:true}).target,"local");
  assert.equal(selectExecutionRoute({preference:"auto",task:"chat",online:true,cloudConnected:true,cloudModelReady:false}).target,"local");
});

test("never claim local image-to-mesh parity for an uploaded reference", () => {
  assert.equal(selectExecutionRoute({preference:"auto",task:"image-to-3d",online:false,cloudConnected:true}).target,"deferred");
  assert.equal(selectExecutionRoute({preference:"local",task:"image-to-3d",online:true,cloudConnected:false}).target,"deferred");
  assert.equal(selectExecutionRoute({preference:"local",task:"image-to-3d",online:true,cloudConnected:true}).target,"cloud");
});

test("Cloud network failures permit one local fallback without remote replay", async () => {
  let cloudCalls=0, localCalls=0;
  const decision=selectExecutionRoute({preference:"auto",task:"chat",online:true,cloudConnected:true});
  const value=await runWithFallback(decision,async()=>{cloudCalls++;throw new Error("Cloud/Studio: Cloud API request failed");},async()=>{localCalls++;return "local answer";});
  assert.deepEqual(value,{result:"local answer",target:"local",fallback:true});
  assert.equal(cloudCalls,1);
  assert.equal(localCalls,1);
});

test("validation and response-persistence errors are never replayed", async()=>{
  for (const error of [new Error("Cloud/Studio: HTTP 422: invalid model"),new ResponsePersistenceError("SQLite locked")]) {
    let localCalls=0;
    await assert.rejects(runWithFallback({target:"cloud",offline:false,reason:"Connected"},async()=>{throw error;},async()=>{localCalls++;return "wrong";}));
    assert.equal(localCalls,0);
  }
});

test("offline web search cannot claim verified live sources",()=>{
  const result=offlineSourceNotice("Is this headline true?");
  assert.match(result,/internet is unavailable/);
  assert.match(result,/Do not claim that live web search/);
  assert.ok(result.endsWith("Is this headline true?"));
  assert.equal(isRecoverableCloudFailure("HTTP 401: session expired"),false);
  assert.equal(isRecoverableCloudFailure("HTTP 503: unavailable"),true);
});
