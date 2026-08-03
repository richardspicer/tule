import { describe, expect, it, vi } from "vitest";
import {
  createCommandDispatcher,
  isEditableTarget,
  queryEditCommandAvailability,
  runEditCommand,
} from "./commands";

describe("application command routing", () => {
  it("routes non-edit commands through one dispatcher", async () => {
    const handler = vi.fn();
    const dispatch = createCommandDispatcher(handler);
    await dispatch("open-settings-connections");
    expect(handler).toHaveBeenCalledWith("open-settings-connections");
  });

  it("keeps edit commands focus-aware and truthful", () => {
    const textarea = document.createElement("textarea");
    document.body.append(textarea);
    textarea.focus();
    textarea.value = "hello";
    textarea.setSelectionRange(0, 5);

    expect(isEditableTarget(textarea)).toBe(true);
    const availability = queryEditCommandAvailability();
    expect(availability.selectAll).toBe(true);
    expect(runEditCommand("edit-select-all")).toBe(true);

    textarea.remove();
    expect(queryEditCommandAvailability().selectAll).toBe(false);
  });

  it("can evaluate and run Edit commands against a preserved editable target", () => {
    const textarea = document.createElement("textarea");
    const distractor = document.createElement("button");
    distractor.type = "button";
    document.body.append(textarea, distractor);
    textarea.value = "preserved";
    textarea.focus();
    distractor.focus();

    expect(document.activeElement).toBe(distractor);
    expect(queryEditCommandAvailability(textarea).selectAll).toBe(true);
    expect(runEditCommand("edit-select-all", textarea)).toBe(true);
    expect(document.activeElement).toBe(textarea);
    expect(textarea.selectionStart).toBe(0);
    expect(textarea.selectionEnd).toBe("preserved".length);

    textarea.remove();
    distractor.remove();
  });
});
