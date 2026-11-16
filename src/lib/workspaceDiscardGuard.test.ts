import { describe, expect, it, vi } from "vitest";
import { confirmWorkspaceDiscard } from "./workspaceDiscardGuard";

describe("workspace discard navigation guard", () => {
  it("allows clean navigation without prompting", () => {
    const confirm = vi.fn(() => false);
    expect(confirmWorkspaceDiscard(false, "switch attached folders", confirm)).toBe(true);
    expect(confirm).not.toHaveBeenCalled();
  });

  it("blocks dirty navigation when the user cancels", () => {
    const confirm = vi.fn(() => false);
    expect(confirmWorkspaceDiscard(true, "switch attached folders", confirm)).toBe(false);
    expect(confirm).toHaveBeenCalledExactlyOnceWith(
      "Discard unsaved editor changes and switch attached folders?",
    );
  });

  it("allows dirty navigation only after explicit confirmation", () => {
    const confirm = vi.fn(() => true);
    expect(confirmWorkspaceDiscard(true, "navigate to the parent folder", confirm)).toBe(true);
    expect(confirm).toHaveBeenCalledTimes(1);
  });

  it("does not prompt twice for a single root switch", () => {
    const confirm = vi.fn(() => true);
    expect(confirmWorkspaceDiscard(true, "switch attached folders", confirm)).toBe(true);
    expect(confirm).toHaveBeenCalledTimes(1);
  });
});
