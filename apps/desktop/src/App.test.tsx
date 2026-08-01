import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import App from "./App";

const { getApplicationInfoMock } = vi.hoisted(() => ({
  getApplicationInfoMock: vi.fn(),
}));

vi.mock("./platform/application", () => ({
  getApplicationInfo: getApplicationInfoMock,
}));

describe("App", () => {
  beforeEach(() => {
    getApplicationInfoMock.mockReset();
  });

  it("shows application information after the Rust boundary connects", async () => {
    getApplicationInfoMock.mockResolvedValue({ name: "Tule Test", version: "9.8.7" });

    render(<App />);

    expect(await screen.findByText("Core connected")).toBeVisible();
    expect(screen.getByText("Tule Test")).toBeVisible();
    expect(screen.getByText("9.8.7")).toBeVisible();
  });

  it("reports when the desktop boundary is unavailable", async () => {
    getApplicationInfoMock.mockRejectedValue(new Error("desktop unavailable"));

    render(<App />);

    expect(await screen.findByText("Desktop required")).toBeVisible();
  });

  it("loads a saved theme and cycles back to the system preference", async () => {
    const user = userEvent.setup();
    window.localStorage.setItem("tule-theme", "dark");
    getApplicationInfoMock.mockResolvedValue({ name: "Tule", version: "0.1.0" });

    render(<App />);

    expect(document.documentElement).toHaveAttribute("data-theme", "dark");
    expect(screen.getByText("Dark", { selector: "dd" })).toBeVisible();

    await user.click(
      screen.getByRole("button", { name: /appearance: dark\. change appearance\./i }),
    );

    expect(document.documentElement).not.toHaveAttribute("data-theme");
    expect(window.localStorage.getItem("tule-theme")).toBeNull();
    expect(screen.getByText("System", { selector: "dd" })).toBeVisible();
  });
});
