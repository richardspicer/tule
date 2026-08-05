import { fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Tooltip } from "./Tooltip";

function domRect(values: Partial<DOMRect>): DOMRect {
  return {
    x: 0,
    y: 0,
    top: 0,
    left: 0,
    right: 0,
    bottom: 0,
    width: 0,
    height: 0,
    toJSON: () => ({}),
    ...values,
  };
}

function mockTooltipRects(anchorRect: DOMRect, tooltipRect: DOMRect) {
  vi.spyOn(Element.prototype, "getBoundingClientRect").mockImplementation(function (this: Element) {
    if (this.classList.contains("tooltip-anchor")) {
      return anchorRect;
    }
    if (this.classList.contains("tooltip")) {
      return tooltipRect;
    }
    return domRect({});
  });
}

describe("Tooltip", () => {
  beforeEach(() => {
    Object.defineProperty(window, "innerWidth", {
      configurable: true,
      value: 800,
    });
    Object.defineProperty(window, "innerHeight", {
      configurable: true,
      value: 600,
    });
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

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
    fireEvent.blur(button);
    expect(screen.queryByRole("tooltip")).not.toBeInTheDocument();
  });

  it("dismisses when the window loses focus or pointer events are cancelled", () => {
    render(
      <Tooltip label="Settings">
        <button type="button" aria-label="Settings">
          gear
        </button>
      </Tooltip>,
    );

    const anchor = screen.getByRole("button", { name: "Settings" }).parentElement!;
    fireEvent.mouseEnter(anchor);
    expect(screen.getByRole("tooltip")).toHaveTextContent("Settings");

    fireEvent.blur(window);
    expect(screen.queryByRole("tooltip")).not.toBeInTheDocument();

    fireEvent.mouseEnter(anchor);
    expect(screen.getByRole("tooltip")).toHaveTextContent("Settings");

    fireEvent(window, new Event("pointercancel"));
    expect(screen.queryByRole("tooltip")).not.toBeInTheDocument();
  });

  it("dismisses when the document becomes hidden", () => {
    render(
      <Tooltip label="Settings">
        <button type="button" aria-label="Settings">
          gear
        </button>
      </Tooltip>,
    );

    const anchor = screen.getByRole("button", { name: "Settings" }).parentElement!;
    fireEvent.mouseEnter(anchor);
    expect(screen.getByRole("tooltip")).toHaveTextContent("Settings");

    Object.defineProperty(document, "visibilityState", {
      configurable: true,
      value: "hidden",
    });
    fireEvent(document, new Event("visibilitychange"));
    expect(screen.queryByRole("tooltip")).not.toBeInTheDocument();
  });

  it("flips above the anchor when there is insufficient space below", () => {
    mockTooltipRects(
      domRect({ top: 560, bottom: 580, left: 700, right: 740, width: 40, height: 20 }),
      domRect({ top: 586, bottom: 610, left: 680, right: 760, width: 80, height: 24 }),
    );

    render(
      <Tooltip label="Send">
        <button type="button" aria-label="Send">
          send
        </button>
      </Tooltip>,
    );

    const anchor = screen.getByRole("button", { name: "Send" }).parentElement!;
    fireEvent.mouseEnter(anchor);

    expect(anchor).toHaveAttribute("data-tooltip-placement", "above");
  });

  it("shifts horizontally so the label stays inside the window edges", () => {
    mockTooltipRects(
      domRect({ top: 40, bottom: 60, left: 4, right: 36, width: 32, height: 20 }),
      domRect({ top: 66, bottom: 90, left: -30, right: 70, width: 100, height: 24 }),
    );

    render(
      <Tooltip label="New session">
        <button type="button" aria-label="New session">
          plus
        </button>
      </Tooltip>,
    );

    const anchor = screen.getByRole("button", { name: "New session" }).parentElement!;
    fireEvent.mouseEnter(anchor);

    const tooltip = screen.getByRole("tooltip");
    expect(tooltip).toHaveStyle({ left: "4px" });
    expect(anchor).toHaveAttribute("data-tooltip-placement", "below");
  });

  it("respects end alignment while clamping inside the right edge", () => {
    mockTooltipRects(
      domRect({ top: 20, bottom: 40, left: 760, right: 792, width: 32, height: 20 }),
      domRect({ top: 46, bottom: 70, left: 720, right: 820, width: 100, height: 24 }),
    );

    render(
      <Tooltip label="Settings" align="end">
        <button type="button" aria-label="Settings">
          gear
        </button>
      </Tooltip>,
    );

    const anchor = screen.getByRole("button", { name: "Settings" }).parentElement!;
    fireEvent.mouseEnter(anchor);

    expect(screen.getByRole("tooltip")).toHaveStyle({ left: "-68px" });
  });
});
