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
        turnBusy={false}
        cancelRequested={false}
        statusMessage={null}
        errorMessage={null}
        onClose={vi.fn()}
        onConnect={vi.fn()}
        onCancelConnect={vi.fn()}
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

    const props = {
      connectionState: "unavailable_in_this_build" as const,
      model: "gpt-5.5",
      theme: "dark" as const,
      busy: false,
      turnBusy: false,
      cancelRequested: false,
      statusMessage: null,
      errorMessage: null,
      onClose,
      onConnect: vi.fn(),
      onCancelConnect: vi.fn(),
      onDisconnect: vi.fn(),
      onThemeChange: vi.fn(),
      returnFocusRef,
    };
    const { rerender } = render(<SettingsSheet open {...props} />);

    expect(screen.getByRole("button", { name: "Close" })).toHaveFocus();
    fireEvent.keyDown(window, { key: "Escape" });
    expect(onClose).toHaveBeenCalled();
    rerender(<SettingsSheet open={false} {...props} />);
    expect(gear).toHaveFocus();
    expect(
      screen.queryByText("ChatGPT connection is unavailable in this build."),
    ).not.toBeInTheDocument();
    gear.remove();
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
        turnBusy={false}
        cancelRequested={false}
        statusMessage={null}
        errorMessage={null}
        onClose={vi.fn()}
        onConnect={vi.fn()}
        onCancelConnect={vi.fn()}
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

  it("contains keyboard focus inside the modal sheet", () => {
    const returnFocusRef = createRef<HTMLButtonElement>();
    render(
      <SettingsSheet
        open
        connectionState="disconnected"
        model="gpt-5.5"
        theme="system"
        busy={false}
        turnBusy={false}
        cancelRequested={false}
        statusMessage={null}
        errorMessage={null}
        onClose={vi.fn()}
        onConnect={vi.fn()}
        onCancelConnect={vi.fn()}
        onDisconnect={vi.fn()}
        onThemeChange={vi.fn()}
        returnFocusRef={returnFocusRef}
      />,
    );

    const close = screen.getByRole("button", { name: "Close" });
    const appearance = screen.getByRole("combobox", { name: "Appearance" });
    appearance.focus();
    fireEvent.keyDown(window, { key: "Tab" });
    expect(close).toHaveFocus();
    fireEvent.keyDown(window, { key: "Tab", shiftKey: true });
    expect(appearance).toHaveFocus();
  });

  it("offers connection cancellation and blocks Disconnect during an Agent turn", () => {
    const onCancelConnect = vi.fn();
    const returnFocusRef = createRef<HTMLButtonElement>();
    const { rerender } = render(
      <SettingsSheet
        open
        connectionState="connecting"
        model="gpt-5.5"
        theme="system"
        busy
        turnBusy={false}
        cancelRequested={false}
        statusMessage="Cancelling browser connection…"
        errorMessage={null}
        onClose={vi.fn()}
        onConnect={vi.fn()}
        onCancelConnect={onCancelConnect}
        onDisconnect={vi.fn()}
        onThemeChange={vi.fn()}
        returnFocusRef={returnFocusRef}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Cancel connection" }));
    expect(onCancelConnect).toHaveBeenCalled();

    rerender(
      <SettingsSheet
        open
        connectionState="connected"
        model="gpt-5.5"
        theme="system"
        busy={false}
        turnBusy
        cancelRequested={false}
        statusMessage="Removed from this device"
        errorMessage={null}
        onClose={vi.fn()}
        onConnect={vi.fn()}
        onCancelConnect={onCancelConnect}
        onDisconnect={vi.fn()}
        onThemeChange={vi.fn()}
        returnFocusRef={returnFocusRef}
      />,
    );

    expect(screen.getByRole("button", { name: "Disconnect" })).toBeDisabled();
    expect(screen.getByText("Removed from this device")).toBeInTheDocument();
  });
});
