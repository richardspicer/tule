import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  cancelXaiConnect,
  connectXai,
  disconnectXai,
  getConnectionStatus,
  getProviderErrorCode,
  getProviderModelCatalog,
  getProviderModelSelection,
  getXaiDevicePairing,
  isStaleConnectCancellation,
  ProviderError,
  setProviderModelSelection,
  validateDevicePairingInfo,
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
      providerId: "xai-subscription-oauth",
      model: "grok-3",
    });

    await expect(getConnectionStatus()).resolves.toEqual({
      state: "disconnected",
      providerId: "xai-subscription-oauth",
      model: "grok-3",
    });
  });

  it("maps connect and disconnect commands", async () => {
    invokeMock.mockResolvedValue({
      state: "connected",
      providerId: "xai-subscription-oauth",
      model: "grok-3",
    });

    await expect(connectXai()).resolves.toMatchObject({ state: "connected" });
    await expect(cancelXaiConnect()).resolves.toBeUndefined();
    await expect(disconnectXai()).resolves.toMatchObject({ state: "connected" });
    expect(invokeMock).toHaveBeenCalledWith("connect_xai");
    expect(invokeMock).toHaveBeenCalledWith("cancel_xai_connect");
    expect(invokeMock).toHaveBeenCalledWith("disconnect_xai");
  });

  it("maps allowlisted failures safely", async () => {
    invokeMock.mockRejectedValue("session_busy");
    await expect(connectXai()).rejects.toBeInstanceOf(ProviderError);
    await expect(connectXai()).rejects.toMatchObject({ code: "session_busy" });
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
      providerId: "xai-subscription-oauth",
      models: [
        {
          id: "grok-3",
          displayName: "Grok 3",
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
      models: [{ id: "grok-3" }],
    });

    invokeMock.mockResolvedValueOnce({
      providerId: "xai-subscription-oauth",
      selectedModelId: "grok-3",
      requiresSelection: false,
    });
    await expect(getProviderModelSelection()).resolves.toMatchObject({
      selectedModelId: "grok-3",
    });

    invokeMock.mockResolvedValueOnce({
      providerId: "xai-subscription-oauth",
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
        providerId: "xai-subscription-oauth",
        models: [{ id: "x" }],
        freshness: "current",
      }),
    ).toThrow(ProviderError);
  });

  it("allows only trimmed https auth.x.ai device pairing metadata", async () => {
    expect(
      validateDevicePairingInfo({
        verificationUri: "  https://auth.x.ai/device  ",
        userCode: "  ABCD-EFGH  ",
      }),
    ).toEqual({
      verificationUri: "https://auth.x.ai/device",
      userCode: "ABCD-EFGH",
    });
    expect(validateDevicePairingInfo({ verificationUri: "", userCode: "" })).toBeNull();
    expect(() =>
      validateDevicePairingInfo({
        verificationUri: "https://auth.x.ai/device",
        userCode: "",
      }),
    ).toThrow(ProviderError);
    expect(() =>
      validateDevicePairingInfo({
        verificationUri: "http://auth.x.ai/device",
        userCode: "ABCD",
      }),
    ).toThrow(ProviderError);
    expect(() =>
      validateDevicePairingInfo({
        verificationUri: "https://evil.example/device",
        userCode: "ABCD",
      }),
    ).toThrow(ProviderError);

    invokeMock.mockResolvedValueOnce({
      verificationUri: "https://auth.x.ai/device?user_code=ABCD",
      userCode: "ABCD",
    });
    await expect(getXaiDevicePairing()).resolves.toEqual({
      verificationUri: "https://auth.x.ai/device?user_code=ABCD",
      userCode: "ABCD",
    });
  });
});
