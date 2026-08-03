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
    window.localStorage.clear();
  });

  it("parses invalid values as system", () => {
    expect(parseThemePreference("dark")).toBe("dark");
    expect(parseThemePreference("nope")).toBe("system");
    expect(parseThemePreference(null)).toBe("system");
  });

  it("loads and applies native appearance without localStorage ownership", async () => {
    invokeMock.mockResolvedValueOnce("light");
    await expect(loadThemePreference()).resolves.toBe("light");
    applyThemePreference("light");
    expect(document.documentElement.dataset.theme).toBe("light");
    expect(window.localStorage.getItem("tule-theme")).toBeNull();
    expect(invokeMock).toHaveBeenCalledWith("get_appearance_preference");
    expect(invokeMock).not.toHaveBeenCalledWith("set_appearance_preference", expect.anything());
  });

  it("falls back to system when native load fails", async () => {
    invokeMock.mockRejectedValueOnce(new Error("unavailable"));
    await expect(loadThemePreference()).resolves.toBe("system");
  });

  it("imports a valid legacy tule-theme value once then retires it", async () => {
    window.localStorage.setItem("tule-theme", "dark");
    invokeMock.mockResolvedValueOnce("system").mockResolvedValueOnce("dark");

    await expect(loadThemePreference()).resolves.toBe("dark");
    expect(invokeMock).toHaveBeenNthCalledWith(1, "get_appearance_preference");
    expect(invokeMock).toHaveBeenNthCalledWith(2, "set_appearance_preference", {
      value: "dark",
    });
    expect(window.localStorage.getItem("tule-theme")).toBeNull();
  });

  it("retires legacy tule-theme without overwriting an existing native preference", async () => {
    window.localStorage.setItem("tule-theme", "dark");
    invokeMock.mockResolvedValueOnce("light");

    await expect(loadThemePreference()).resolves.toBe("light");
    expect(invokeMock).toHaveBeenCalledTimes(1);
    expect(invokeMock).toHaveBeenCalledWith("get_appearance_preference");
    expect(window.localStorage.getItem("tule-theme")).toBeNull();
  });

  it("keeps legacy tule-theme for retry when native import fails", async () => {
    window.localStorage.setItem("tule-theme", "light");
    invokeMock
      .mockResolvedValueOnce("system")
      .mockRejectedValueOnce(new Error("preference_storage_unavailable"));

    await expect(loadThemePreference()).resolves.toBe("light");
    expect(window.localStorage.getItem("tule-theme")).toBe("light");
  });

  it("retires invalid legacy tule-theme values without writing native storage", async () => {
    window.localStorage.setItem("tule-theme", "nope");
    invokeMock.mockResolvedValueOnce("system");

    await expect(loadThemePreference()).resolves.toBe("system");
    expect(invokeMock).toHaveBeenCalledTimes(1);
    expect(window.localStorage.getItem("tule-theme")).toBeNull();
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
