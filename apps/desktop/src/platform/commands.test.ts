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
});
