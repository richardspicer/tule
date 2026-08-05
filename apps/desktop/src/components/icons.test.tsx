import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { ApplicationChrome } from "./ApplicationChrome";
import {
  AttachFileIcon,
  AttachFolderIcon,
  JumpToLatestIcon,
  RemoveIcon,
  SendIcon,
  SETTINGS_ICON_EDGE_PADDING,
  SettingsIcon,
  settingsIconStrokedBounds,
} from "./icons";
import { Tooltip } from "./Tooltip";

describe("chrome and transcript icons", () => {
  it("keeps the Settings gear decorative while the control stays named Settings", () => {
    const { container } = render(<ApplicationChrome onCommand={() => undefined} />);

    const settings = screen.getByRole("button", { name: "Settings" });
    expect(settings.querySelector("svg")).toHaveAttribute("aria-hidden", "true");
    expect(settings.querySelector("svg")).toHaveAttribute("viewBox", "0 0 16 16");
    expect(settings).not.toHaveTextContent(/gear|settings/i);

    fireEvent.mouseEnter(settings.parentElement!);
    expect(screen.getByRole("tooltip")).toHaveTextContent("Settings");
    expect(container.querySelectorAll("svg path").length).toBeGreaterThan(0);
  });

  it("exposes Jump to latest through accessible name and tooltip", () => {
    render(
      <Tooltip label="Jump to latest" align="end">
        <button type="button" aria-label="Jump to latest">
          <JumpToLatestIcon />
        </button>
      </Tooltip>,
    );

    const control = screen.getByRole("button", { name: "Jump to latest" });
    expect(control.querySelector("svg")).toHaveAttribute("aria-hidden", "true");
    expect(control.textContent?.trim()).toBe("");

    fireEvent.mouseEnter(control.parentElement!);
    expect(screen.getByRole("tooltip")).toHaveTextContent("Jump to latest");
    expect(control.parentElement).toHaveAttribute("data-tooltip-align", "end");
  });

  it("renders the Settings gear with the shared 16px icon contract", () => {
    const { container } = render(<SettingsIcon />);
    const svg = container.querySelector("svg");
    expect(svg).toHaveAttribute("width", "16");
    expect(svg).toHaveAttribute("height", "16");
    expect(svg).toHaveAttribute("aria-hidden", "true");
    expect(svg?.querySelector("circle")).toBeTruthy();
    expect(svg?.querySelector("path")).toHaveAttribute("stroke-width", "1.5");
  });

  it("keeps the complete Settings gear stroke inside the viewBox with padding", () => {
    const { container } = render(<SettingsIcon />);
    const svg = container.querySelector("svg");
    expect(svg).not.toBeNull();
    expect(svg).toHaveAttribute("viewBox", "0 0 16 16");

    const bounds = settingsIconStrokedBounds(svg!);
    expect(bounds.min).toBeGreaterThanOrEqual(SETTINGS_ICON_EDGE_PADDING);
    expect(bounds.max).toBeLessThanOrEqual(16 - SETTINGS_ICON_EDGE_PADDING);
  });
});

describe("composer icons", () => {
  it.each([
    ["Send", SendIcon],
    ["Attach file", AttachFileIcon],
    ["Attach folder", AttachFolderIcon],
    ["Remove", RemoveIcon],
  ] as const)("renders the %s icon at 16×16 with decorative SVG", (_label, Icon) => {
    const { container } = render(<Icon />);
    const svg = container.querySelector("svg");
    expect(svg).toHaveAttribute("width", "16");
    expect(svg).toHaveAttribute("height", "16");
    expect(svg).toHaveAttribute("viewBox", "0 0 16 16");
    expect(svg).toHaveAttribute("aria-hidden", "true");
    expect(svg?.querySelector("path")).toHaveAttribute("stroke-width", "1.5");
  });

  it("exposes Send through accessible name and tooltip without visible text", () => {
    render(
      <Tooltip label="Send">
        <button type="button" aria-label="Send">
          <SendIcon />
        </button>
      </Tooltip>,
    );

    const control = screen.getByRole("button", { name: "Send" });
    expect(control.textContent?.trim()).toBe("");
    fireEvent.mouseEnter(control.parentElement!);
    expect(screen.getByRole("tooltip")).toHaveTextContent("Send");
  });
});
