import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  applyThemePreference,
  loadThemePreference,
  parseThemePreference,
  saveThemePreference,
  ThemePersistenceError,
} from "./theme";

const { invokeMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(),
}));

describe("theme preference facade", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    delete document.documentElement.dataset.theme;
  });

  it("parses invalid values as system", () => {
    expect(parseThemePreference("dark")).toBe("dark");
    expect(parseThemePreference("nope")).toBe("system");
    expect(parseThemePreference(null)).toBe("system");
  });

  it("loads and applies native appearance without localStorage", async () => {
    invokeMock.mockResolvedValueOnce("light");
    await expect(loadThemePreference()).resolves.toBe("light");
    applyThemePreference("light");
    expect(document.documentElement.dataset.theme).toBe("light");
    expect(window.localStorage.getItem("tule-theme")).toBeNull();
  });

  it("falls back to system when native load fails", async () => {
    invokeMock.mockRejectedValueOnce(new Error("unavailable"));
    await expect(loadThemePreference()).resolves.toBe("system");
  });

  it("persists appearance through the typed native command", async () => {
    invokeMock.mockResolvedValueOnce("dark");
    await expect(saveThemePreference("dark")).resolves.toBe("dark");
    expect(invokeMock).toHaveBeenCalledWith("set_appearance_preference", { value: "dark" });
    expect(document.documentElement.dataset.theme).toBe("dark");
  });

  it("applies appearance immediately but reports persistence failure safely", async () => {
    invokeMock.mockRejectedValueOnce(new Error("preference_storage_unavailable"));
    await expect(saveThemePreference("light")).rejects.toBeInstanceOf(ThemePersistenceError);
    expect(document.documentElement.dataset.theme).toBe("light");
  });
});
