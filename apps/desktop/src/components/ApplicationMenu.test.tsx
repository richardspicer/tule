import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, it, vi } from "vitest";
import type { AppCommandId } from "../platform/commands";
import { ApplicationMenu } from "./ApplicationMenu";

function MenuHarness({ onCommand }: { onCommand: (command: AppCommandId) => void }) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <textarea aria-label="Draft" defaultValue="hello world" />
      <ApplicationMenu open={open} onOpenChange={setOpen} onCommand={onCommand} />
    </>
  );
}

describe("ApplicationMenu", () => {
  it("preserves the prior editable field for mouse and keyboard Edit actions", async () => {
    const user = userEvent.setup();
    const onCommand = vi.fn();
    vi.spyOn(document, "queryCommandEnabled").mockImplementation(
      (command: string) =>
        command === "selectAll" || command === "undo" || command === "paste" || command === "copy",
    );
    vi.spyOn(document, "execCommand").mockImplementation(
      (command: string) => command === "selectAll" || command === "undo",
    );

    render(<MenuHarness onCommand={onCommand} />);

    const draft = screen.getByRole<HTMLTextAreaElement>("textbox", { name: "Draft" });
    draft.focus();
    expect(draft).toHaveFocus();

    await user.click(screen.getByRole("button", { name: "Application menu" }));
    const selectAll = screen.getByRole("menuitem", { name: "Select all" });
    expect(selectAll).toBeEnabled();
    expect(screen.getByRole("menuitem", { name: "Undo" })).toBeEnabled();

    fireEvent.mouseDown(selectAll);
    await user.click(selectAll);
    expect(draft).toHaveFocus();
    expect(draft.selectionStart).toBe(0);
    expect(draft.selectionEnd).toBe("hello world".length);
    expect(onCommand).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: "Application menu" }));
    const undo = screen.getByRole("menuitem", { name: "Undo" });
    fireEvent.mouseEnter(undo);
    fireEvent.keyDown(window, { key: "Enter" });
    expect(draft).toHaveFocus();
    expect(onCommand).not.toHaveBeenCalledWith("edit-undo");
  });
});
