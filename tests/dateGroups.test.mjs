import test from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";

// Separate processes are necessary because the local timezone is fixed at startup.
function runGrouping(tz, now, dates) {
  const script = `
    import { groupConversationsByDate } from "./src/lib/dateGroups.ts";
    const NativeDate = Date;
    const now = process.env.FIXED_NOW;
    globalThis.Date = class extends NativeDate {
      constructor(...args) { super(...(args.length ? args : [now])); }
      static now() { return new NativeDate(now).getTime(); }
    };
    const dates = JSON.parse(process.env.CHAT_DATES);
    const groups = groupConversationsByDate(dates.map((updatedAt, id) => ({
      id: String(id), updatedAt, pinned: false
    })));
    console.log(JSON.stringify(groups.map(g => [g.label, g.conversations.map(c => c.id)])));
  `;
  const result = spawnSync(process.execPath, ["--experimental-strip-types", "--input-type=module", "-e", script], {
    cwd: process.cwd(), encoding: "utf8",
    env: { ...process.env, TZ: tz, FIXED_NOW: now, CHAT_DATES: JSON.stringify(dates) },
  });
  assert.equal(result.status, 0, result.stderr);
  return Object.fromEntries(JSON.parse(result.stdout));
}

test("spring-forward boundary uses calendar dates, not elapsed hours", () => {
  const groups = runGrouping("America/New_York", "2026-03-09T12:00:00-04:00", [
    "2026-03-09T01:00:00-04:00",
    "2026-03-08T01:00:00-05:00",
    "2026-03-02T12:00:00-05:00",
    "2026-02-07T12:00:00-05:00",
  ]);
  assert.deepEqual(groups.Today, ["0"]);
  assert.deepEqual(groups.Yesterday, ["1"]);
  assert.deepEqual(groups["Previous 7 Days"], ["2"]);
  assert.deepEqual(groups["Previous 30 Days"], ["3"]);
});

test("fall-back and invalid dates do not mislabel malformed records as Today", () => {
  const groups = runGrouping("America/New_York", "2026-11-02T12:00:00-05:00", [
    "2026-11-01T01:30:00-04:00",
    "not-a-timestamp",
    "2026-10-26T10:00:00-04:00",
    "2026-09-01T10:00:00-04:00",
  ]);
  assert.deepEqual(groups.Yesterday, ["0"]);
  assert.deepEqual(groups["Previous 7 Days"], ["2"]);
  assert.deepEqual(groups.Older, ["1", "3"]);
});
