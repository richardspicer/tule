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

  it("aligns corner tooltips so they stay inside the window edges", () => {
    const { rerender } = render(
      <Tooltip label="Application menu" align="start">
        <button type="button" aria-label="Application menu">
          menu
        </button>
      </Tooltip>,
    );

    fireEvent.mouseEnter(screen.getByRole("button", { name: "Application menu" }).parentElement!);
    expect(screen.getByRole("tooltip").parentElement).toHaveAttribute(
      "data-tooltip-align",
      "start",
    );

    rerender(
      <Tooltip label="Settings" align="end">
        <button type="button" aria-label="Settings">
          gear
        </button>
      </Tooltip>,
    );
    fireEvent.mouseEnter(screen.getByRole("button", { name: "Settings" }).parentElement!);
    expect(screen.getByRole("tooltip").parentElement).toHaveAttribute("data-tooltip-align", "end");
  });
});
