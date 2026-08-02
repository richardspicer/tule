import { fireEvent, render, screen } from "@testing-library/react";
import { createRef } from "react";
import { describe, expect, it, vi } from "vitest";
import { SettingsSheet } from "./SettingsSheet";

describe("SettingsSheet", () => {
  it("shows experimental disclosure and connection action", () => {
    const returnFocusRef = createRef<HTMLButtonElement>();
    render(
      <SettingsSheet
        open
        connectionState="disconnected"
        model="gpt-5.5"
        theme="system"
        busy={false}
        onClose={vi.fn()}
        onConnect={vi.fn()}
        onDisconnect={vi.fn()}
        onThemeChange={vi.fn()}
        returnFocusRef={returnFocusRef}
      />,
    );

    expect(screen.getByText("ChatGPT subscription")).toBeInTheDocument();
    expect(screen.getByText("Experimental")).toBeInTheDocument();
    expect(
      screen.getByText(
        "Uses a compatibility sign-in path that is not an official TULE integration and may stop working.",
      ),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Connect in browser" })).toBeInTheDocument();
    expect(screen.getByText("GPT-5.5")).toBeInTheDocument();
  });

  it("closes on Escape and returns focus", () => {
    const onClose = vi.fn();
    const returnFocusRef = createRef<HTMLButtonElement>();
    const gear = document.createElement("button");
    document.body.appendChild(gear);
    Object.defineProperty(returnFocusRef, "current", { value: gear, writable: true });

    render(
      <SettingsSheet
        open
        connectionState="unavailable_in_this_build"
        model="gpt-5.5"
        theme="dark"
        busy={false}
        onClose={onClose}
        onConnect={vi.fn()}
        onDisconnect={vi.fn()}
        onThemeChange={vi.fn()}
        returnFocusRef={returnFocusRef}
      />,
    );

    fireEvent.keyDown(window, { key: "Escape" });
    expect(onClose).toHaveBeenCalled();
    expect(
      screen.getByText("ChatGPT connection is unavailable in this build."),
    ).toBeInTheDocument();
  });

  it("exposes appearance options", () => {
    const onThemeChange = vi.fn();
    const returnFocusRef = createRef<HTMLButtonElement>();
    render(
      <SettingsSheet
        open
        connectionState="connected"
        model="gpt-5.5"
        theme="light"
        busy={false}
        onClose={vi.fn()}
        onConnect={vi.fn()}
        onDisconnect={vi.fn()}
        onThemeChange={onThemeChange}
        returnFocusRef={returnFocusRef}
      />,
    );

    fireEvent.change(screen.getByRole("combobox", { name: "Appearance" }), {
      target: { value: "system" },
    });
    expect(onThemeChange).toHaveBeenCalledWith("system");
  });
});
