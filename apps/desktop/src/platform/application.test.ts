import { beforeEach, describe, expect, it, vi } from "vitest";
import { getApplicationInfo } from "./application";

const { invokeMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
}));

describe("getApplicationInfo", () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it("returns a valid response from the narrow Tauri command", async () => {
    const applicationInfo = { name: "TULE", version: "0.1.0" };
    invokeMock.mockResolvedValue(applicationInfo);

    await expect(getApplicationInfo()).resolves.toEqual(applicationInfo);
    expect(invokeMock).toHaveBeenCalledWith("get_application_info");
  });

  it("rejects a response that violates the frontend contract", async () => {
    invokeMock.mockResolvedValue({ name: "TULE", version: 1 });

    await expect(getApplicationInfo()).rejects.toThrow("invalid application-info response");
  });
});
