import { describe, expect, it } from "vitest";
import { initialInstalledDatasets, recordCompletedDataset } from "./datasetInstalledState";

describe("dataset installed badge evidence", () => {
  it("does not restore stale IDs from legacy browser storage", () => {
    // Even a previously completed ID cannot prove files still exist after restart.
    expect(initialInstalledDatasets().size).toBe(0);
  });

  it("does not mark queued, failed or interrupted downloads installed", () => {
    const empty = initialInstalledDatasets();
    for (const state of ["queued", "downloading", "failed", "cancelled", "paused"]) {
      expect(recordCompletedDataset(empty, state, "dataset/a")).toBe(empty);
    }
    expect(recordCompletedDataset(empty, "completed", null)).toBe(empty);
    expect(recordCompletedDataset(empty, "completed", " ")).toBe(empty);
  });

  it("records only an observed completion without mutating prior state", () => {
    const initial = initialInstalledDatasets();
    const installed = recordCompletedDataset(initial, "completed", "dataset/a");
    expect(initial.has("dataset/a")).toBe(false);
    expect(installed.has("dataset/a")).toBe(true);
    expect(recordCompletedDataset(installed, "completed", "dataset/a")).toBe(installed);
  });
});
