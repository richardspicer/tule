import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  cancelChatgptConnect,
  connectChatgpt,
  disconnectChatgpt,
  getConnectionStatus,
  getProviderErrorCode,
  getProviderModelCatalog,
  getProviderModelSelection,
  isStaleConnectCancellation,
  ProviderError,
  setProviderModelSelection,
  validateProviderModelCatalog,
} from "./provider";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
}));

describe("provider platform", () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it("validates connection status shape", async () => {
    invokeMock.mockResolvedValue({
      state: "disconnected",
      providerId: "openai-chatgpt-compat",
      model: "gpt-5.5",
    });

    await expect(getConnectionStatus()).resolves.toEqual({
      state: "disconnected",
      providerId: "openai-chatgpt-compat",
      model: "gpt-5.5",
    });
  });

  it("maps connect and disconnect commands", async () => {
    invokeMock.mockResolvedValue({
      state: "connected",
      providerId: "openai-chatgpt-compat",
      model: "gpt-5.5",
    });

    await expect(connectChatgpt()).resolves.toMatchObject({ state: "connected" });
    await expect(cancelChatgptConnect()).resolves.toBeUndefined();
    await expect(disconnectChatgpt()).resolves.toMatchObject({ state: "connected" });
    expect(invokeMock).toHaveBeenCalledWith("connect_chatgpt");
    expect(invokeMock).toHaveBeenCalledWith("cancel_chatgpt_connect");
    expect(invokeMock).toHaveBeenCalledWith("disconnect_chatgpt");
  });

  it("maps allowlisted failures safely", async () => {
    invokeMock.mockRejectedValue("session_busy");
    await expect(connectChatgpt()).rejects.toBeInstanceOf(ProviderError);
    await expect(connectChatgpt()).rejects.toMatchObject({ code: "session_busy" });
    expect(getProviderErrorCode(new ProviderError("not_connected"))).toBe("not_connected");
  });

  it("identifies late connect cancellation without treating other failures as stale", () => {
    expect(isStaleConnectCancellation(new ProviderError("invalid_input"))).toBe(true);
    expect(isStaleConnectCancellation(new ProviderError("cancelled"))).toBe(false);
    expect(isStaleConnectCancellation(new ProviderError("session_busy"))).toBe(false);
    expect(isStaleConnectCancellation("invalid_input")).toBe(false);
  });

  it("rejects malformed status payloads", async () => {
    invokeMock.mockResolvedValue({ state: "weird" });
    await expect(getConnectionStatus()).rejects.toMatchObject({
      code: "provider_unavailable",
    });
  });

  it("validates bounded catalog and selection contracts", async () => {
    invokeMock.mockResolvedValueOnce({
      providerId: "openai-chatgpt-compat",
      models: [
        {
          id: "gpt-5.5",
          displayName: "GPT-5.5",
          description: "safe",
          isProviderDefault: true,
        },
      ],
      freshness: "current",
      retrievedAtUnixMs: 10,
      compatibilityRevision: "1.0.0",
    });
    await expect(getProviderModelCatalog()).resolves.toMatchObject({
      freshness: "current",
      models: [{ id: "gpt-5.5" }],
    });

    invokeMock.mockResolvedValueOnce({
      providerId: "openai-chatgpt-compat",
      selectedModelId: "gpt-5.5",
      requiresSelection: false,
    });
    await expect(getProviderModelSelection()).resolves.toMatchObject({
      selectedModelId: "gpt-5.5",
    });

    invokeMock.mockResolvedValueOnce({
      providerId: "openai-chatgpt-compat",
      selectedModelId: "other",
      requiresSelection: false,
    });
    await expect(setProviderModelSelection("other")).resolves.toMatchObject({
      selectedModelId: "other",
    });
    expect(invokeMock).toHaveBeenCalledWith("set_provider_model_selection", {
      modelId: "other",
    });

    expect(() =>
      validateProviderModelCatalog({
        providerId: "openai-chatgpt-compat",
        models: [{ id: "x" }],
        freshness: "current",
      }),
    ).toThrow(ProviderError);
  });
});
