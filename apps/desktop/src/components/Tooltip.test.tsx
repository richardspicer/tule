import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { Tooltip } from "./Tooltip";

describe("Tooltip", () => {
  it("shows on pointer hover and keyboard focus while keeping an accessible name on the control", () => {
    render(
      <Tooltip label="Settings">
        <button type="button" aria-label="Settings">
          gear
        </button>
      </Tooltip>,
    );

    const button = screen.getByRole("button", { name: "Settings" });
    fireEvent.mouseEnter(button.parentElement!);
    expect(screen.getByRole("tooltip")).toHaveTextContent("Settings");
    fireEvent.mouseLeave(button.parentElement!);

    fireEvent.focusIn(button.parentElement!);
    expect(screen.getByRole("tooltip")).toHaveTextContent("Settings");
  });
});
