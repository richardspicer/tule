import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { SettingsWindow } from "./SettingsWindow";
import { ProviderError } from "../platform/provider";

const {
  cancelXaiConnectMock,
  connectXaiMock,
  disconnectXaiMock,
  getConnectionStatusMock,
  getProviderModelCatalogMock,
  getProviderModelSelectionMock,
  listenAppearanceChangedMock,
  listenConnectionStatusChangedMock,
  listenProviderModelCatalogChangedMock,
  listenProviderModelSelectionChangedMock,
  listenSettingsNavigateMock,
  loadThemePreferenceMock,
  refreshProviderModelCatalogMock,
  saveThemePreferenceMock,
  setProviderModelSelectionMock,
  takeSettingsLaunchCategoryMock,
} = vi.hoisted(() => ({
  cancelXaiConnectMock: vi.fn(),
  connectXaiMock: vi.fn(),
  disconnectXaiMock: vi.fn(),
  getConnectionStatusMock: vi.fn(),
  getProviderModelCatalogMock: vi.fn(),
  getProviderModelSelectionMock: vi.fn(),
  listenAppearanceChangedMock: vi.fn(),
  listenConnectionStatusChangedMock: vi.fn(),
  listenProviderModelCatalogChangedMock: vi.fn(),
  listenProviderModelSelectionChangedMock: vi.fn(),
  listenSettingsNavigateMock: vi.fn(),
  loadThemePreferenceMock: vi.fn(),
  refreshProviderModelCatalogMock: vi.fn(),
  saveThemePreferenceMock: vi.fn(),
  setProviderModelSelectionMock: vi.fn(),
  takeSettingsLaunchCategoryMock: vi.fn(),
}));

vi.mock("../platform/provider", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../platform/provider")>();
  return {
    ...actual,
    cancelXaiConnect: cancelXaiConnectMock,
    connectXai: connectXaiMock,
    disconnectXai: disconnectXaiMock,
    getXaiDevicePairing: vi.fn().mockResolvedValue(null),
    listenXaiDevicePairingChanged: vi.fn().mockResolvedValue(() => undefined),
    getConnectionStatus: getConnectionStatusMock,
    getProviderModelCatalog: getProviderModelCatalogMock,
    getProviderModelSelection: getProviderModelSelectionMock,
    listenProviderModelCatalogChanged: listenProviderModelCatalogChangedMock,
    listenProviderModelSelectionChanged: listenProviderModelSelectionChangedMock,
    refreshProviderModelCatalog: refreshProviderModelCatalogMock,
    setProviderModelSelection: setProviderModelSelectionMock,
  };
});

vi.mock("../platform/settings", () => ({
  listenConnectionStatusChanged: listenConnectionStatusChangedMock,
  listenSettingsNavigate: listenSettingsNavigateMock,
  takeSettingsLaunchCategory: takeSettingsLaunchCategoryMock,
}));

vi.mock("../theme", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../theme")>();
  return {
    ...actual,
    loadThemePreference: loadThemePreferenceMock,
    listenAppearanceChanged: listenAppearanceChangedMock,
    saveThemePreference: saveThemePreferenceMock,
  };
});

describe("SettingsWindow", () => {
  beforeEach(() => {
    cancelXaiConnectMock.mockReset();
    connectXaiMock.mockReset();
    disconnectXaiMock.mockReset();
    getConnectionStatusMock.mockReset();
    getProviderModelCatalogMock.mockReset();
    getProviderModelSelectionMock.mockReset();
    listenAppearanceChangedMock.mockReset();
    listenConnectionStatusChangedMock.mockReset();
    listenProviderModelCatalogChangedMock.mockReset();
    listenProviderModelSelectionChangedMock.mockReset();
    listenSettingsNavigateMock.mockReset();
    loadThemePreferenceMock.mockReset();
    refreshProviderModelCatalogMock.mockReset();
    saveThemePreferenceMock.mockReset();
    setProviderModelSelectionMock.mockReset();
    takeSettingsLaunchCategoryMock.mockReset();

    getConnectionStatusMock.mockResolvedValue({
      state: "disconnected",
      providerId: "xai-subscription-oauth",
      model: "grok-3",
    });
    getProviderModelCatalogMock.mockResolvedValue({
      providerId: "xai-subscription-oauth",
      models: [],
      freshness: "stale",
      retrievedAtUnixMs: null,
      compatibilityRevision: null,
    });
    getProviderModelSelectionMock.mockResolvedValue({
      providerId: "xai-subscription-oauth",
      selectedModelId: "grok-3",
      requiresSelection: false,
    });
    loadThemePreferenceMock.mockResolvedValue("system");
    saveThemePreferenceMock.mockImplementation((theme: "system" | "light" | "dark") =>
      Promise.resolve(theme),
    );
    takeSettingsLaunchCategoryMock.mockResolvedValue(null);
    listenAppearanceChangedMock.mockResolvedValue(vi.fn());
    listenConnectionStatusChangedMock.mockResolvedValue(vi.fn());
    listenProviderModelCatalogChangedMock.mockResolvedValue(vi.fn());
    listenProviderModelSelectionChangedMock.mockResolvedValue(vi.fn());
    listenSettingsNavigateMock.mockResolvedValue(vi.fn());
    cancelXaiConnectMock.mockResolvedValue(undefined);
    refreshProviderModelCatalogMock.mockResolvedValue({
      providerId: "xai-subscription-oauth",
      models: [],
      freshness: "stale",
      retrievedAtUnixMs: null,
      compatibilityRevision: null,
    });
  });

  it("surfaces a cached catalog refresh failure without treating stale data as success", async () => {
    const user = userEvent.setup();
    getConnectionStatusMock.mockResolvedValue({
      state: "connected",
      providerId: "xai-subscription-oauth",
      model: "grok-3",
    });
    refreshProviderModelCatalogMock
      .mockResolvedValueOnce({
        providerId: "xai-subscription-oauth",
        models: [
          {
            id: "grok-3",
            displayName: "Grok 3",
            description: null,
            isProviderDefault: true,
          },
        ],
        freshness: "current",
        retrievedAtUnixMs: 10,
        compatibilityRevision: "1.0.0",
      })
      .mockRejectedValueOnce(new ProviderError("provider_unavailable"));
    getProviderModelSelectionMock.mockResolvedValue({
      providerId: "xai-subscription-oauth",
      selectedModelId: "grok-3",
      requiresSelection: false,
    });

    render(<SettingsWindow />);
    expect(await screen.findByRole("option", { name: "Grok 3" })).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Refresh models" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("The provider is unavailable.");
    expect(screen.getByRole("option", { name: "Grok 3" })).toBeInTheDocument();
    expect(getProviderModelCatalogMock).not.toHaveBeenCalled();
  });

  it("keeps Refresh models available when connected with an empty catalog", async () => {
    getConnectionStatusMock.mockResolvedValue({
      state: "connected",
      providerId: "xai-subscription-oauth",
      model: "grok-3",
    });
    refreshProviderModelCatalogMock.mockRejectedValue(new ProviderError("provider_unavailable"));
    getProviderModelCatalogMock.mockResolvedValue({
      providerId: "xai-subscription-oauth",
      models: [],
      freshness: "stale",
      retrievedAtUnixMs: null,
      compatibilityRevision: null,
    });

    render(<SettingsWindow />);
    expect(await screen.findByRole("button", { name: "Refresh models" })).toBeInTheDocument();
    expect(
      await screen.findByText(
        "No usable models are available yet. Refresh to recover the catalog.",
      ),
    ).toBeInTheDocument();
  });

  it("recovers the catalog after an initial refresh failure", async () => {
    const user = userEvent.setup();
    getConnectionStatusMock.mockResolvedValue({
      state: "connected",
      providerId: "xai-subscription-oauth",
      model: "grok-3",
    });
    refreshProviderModelCatalogMock
      .mockRejectedValueOnce(new ProviderError("provider_unavailable"))
      .mockResolvedValueOnce({
        providerId: "xai-subscription-oauth",
        models: [
          {
            id: "grok-3",
            displayName: "Grok 3",
            description: null,
            isProviderDefault: true,
          },
        ],
        freshness: "current",
        retrievedAtUnixMs: 10,
        compatibilityRevision: "1.0.0",
      });
    getProviderModelCatalogMock.mockResolvedValue({
      providerId: "xai-subscription-oauth",
      models: [],
      freshness: "stale",
      retrievedAtUnixMs: null,
      compatibilityRevision: null,
    });
    getProviderModelSelectionMock.mockResolvedValue({
      providerId: "xai-subscription-oauth",
      selectedModelId: null,
      requiresSelection: true,
    });

    render(<SettingsWindow />);
    await user.click(await screen.findByRole("button", { name: "Refresh models" }));
    expect(await screen.findByLabelText("Default model for new sessions")).toBeInTheDocument();
    expect(screen.getByRole("option", { name: "Grok 3" })).toBeInTheDocument();
  });

  it("renders Providers and Appearance categories only", async () => {
    render(<SettingsWindow />);
    const nav = await screen.findByRole("navigation", { name: "Settings categories" });
    expect(within(nav).getByRole("button", { name: "Providers" })).toBeInTheDocument();
    expect(within(nav).getByRole("button", { name: "Appearance" })).toBeInTheDocument();
    expect(within(nav).getByRole("button", { name: "Providers" })).toHaveAttribute(
      "aria-current",
      "page",
    );
    expect(screen.queryByRole("button", { name: "Close" })).not.toBeInTheDocument();
    expect(
      screen.getByText(
        "Connects through xAI subscription OAuth using a shared public Grok-CLI client. The browser consent screen may identify Grok rather than TULE.",
      ),
    ).toBeInTheDocument();
  });

  it("starts on Providers for a normal launch and follows reopen navigation", async () => {
    let navigate: ((category: "providers" | "appearance") => void) | undefined;
    listenSettingsNavigateMock.mockImplementation(
      (handler: (category: "providers" | "appearance") => void) => {
        navigate = handler;
        return Promise.resolve(vi.fn());
      },
    );

    const user = userEvent.setup();
    render(<SettingsWindow />);
    const nav = await screen.findByRole("navigation", { name: "Settings categories" });
    expect(within(nav).getByRole("button", { name: "Providers" })).toHaveAttribute(
      "aria-current",
      "page",
    );

    await user.click(within(nav).getByRole("button", { name: "Appearance" }));
    expect(screen.getByRole("combobox", { name: "Appearance" })).toBeInTheDocument();

    // Reopen-after-hidden semantics emit Providers.
    navigate?.("providers");
    expect(await screen.findByRole("button", { name: "Connect in browser" })).toBeInTheDocument();
    expect(within(nav).getByRole("button", { name: "Providers" })).toHaveAttribute(
      "aria-current",
      "page",
    );
  });

  it("selects Providers from a contextual settings navigate event", async () => {
    let navigate: ((category: "providers" | "appearance") => void) | undefined;
    listenSettingsNavigateMock.mockImplementation(
      (handler: (category: "providers" | "appearance") => void) => {
        navigate = handler;
        return Promise.resolve(vi.fn());
      },
    );

    const user = userEvent.setup();
    render(<SettingsWindow />);
    const nav = await screen.findByRole("navigation", { name: "Settings categories" });
    await user.click(within(nav).getByRole("button", { name: "Appearance" }));
    expect(screen.getByRole("combobox", { name: "Appearance" })).toBeInTheDocument();

    navigate?.("providers");
    expect(await screen.findByRole("button", { name: "Connect in browser" })).toBeInTheDocument();
  });

  it("honors an Appearance launch category without changing Connect labeling", async () => {
    takeSettingsLaunchCategoryMock.mockResolvedValue("appearance");
    render(<SettingsWindow />);
    expect(await screen.findByRole("combobox", { name: "Appearance" })).toBeInTheDocument();
    const nav = screen.getByRole("navigation", { name: "Settings categories" });
    expect(within(nav).getByRole("button", { name: "Providers" })).toBeInTheDocument();
  });

  it("shows Connected after successful browser connection without restart", async () => {
    const user = userEvent.setup();
    let resolveConnect:
      ((status: { state: "connected"; providerId: string; model: string }) => void) | undefined;
    connectXaiMock.mockReturnValue(
      new Promise((resolve) => {
        resolveConnect = resolve;
      }),
    );

    render(<SettingsWindow />);
    await user.click(await screen.findByRole("button", { name: "Connect in browser" }));
    expect(screen.getByText("Connecting")).toBeInTheDocument();

    resolveConnect?.({
      state: "connected",
      providerId: "xai-subscription-oauth",
      model: "grok-3",
    });

    expect(await screen.findByText("Connected")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Disconnect" })).toBeInTheDocument();
    expect(screen.queryByText("Connecting")).not.toBeInTheDocument();
    expect(screen.queryByText("Enter a valid message.")).not.toBeInTheDocument();
  });

  it("cancels browser connection and reports the safe result", async () => {
    const user = userEvent.setup();
    let rejectConnect: ((error: ProviderError) => void) | undefined;
    connectXaiMock.mockReturnValue(
      new Promise((_resolve, reject) => {
        rejectConnect = reject;
      }),
    );
    cancelXaiConnectMock.mockImplementation(() => {
      rejectConnect?.(new ProviderError("cancelled"));
      getConnectionStatusMock.mockResolvedValue({
        state: "disconnected",
        providerId: "xai-subscription-oauth",
        model: "grok-3",
      });
      return Promise.resolve();
    });

    render(<SettingsWindow />);
    await user.click(await screen.findByRole("button", { name: "Connect in browser" }));
    await user.click(await screen.findByRole("button", { name: "Cancel connection" }));

    expect(cancelXaiConnectMock).toHaveBeenCalledOnce();
    expect(await screen.findByText("Device sign-in cancelled.")).toBeInTheDocument();
    expect(await screen.findByText("Disconnected")).toBeInTheDocument();
    expect(screen.queryByText("Enter a valid message.")).not.toBeInTheDocument();
  });

  it("reconciles a late Cancel to Connected without Agent validation copy", async () => {
    const user = userEvent.setup();
    let resolveConnect:
      ((status: { state: "connected"; providerId: string; model: string }) => void) | undefined;
    connectXaiMock.mockReturnValue(
      new Promise((resolve) => {
        resolveConnect = resolve;
      }),
    );
    cancelXaiConnectMock.mockImplementation(() => {
      resolveConnect?.({
        state: "connected",
        providerId: "xai-subscription-oauth",
        model: "grok-3",
      });
      getConnectionStatusMock.mockResolvedValue({
        state: "connected",
        providerId: "xai-subscription-oauth",
        model: "grok-3",
      });
      return Promise.reject(new ProviderError("invalid_input"));
    });

    render(<SettingsWindow />);
    await user.click(await screen.findByRole("button", { name: "Connect in browser" }));
    await user.click(await screen.findByRole("button", { name: "Cancel connection" }));

    expect(await screen.findByText("Connected")).toBeInTheDocument();
    expect(screen.queryByText("Device sign-in cancelled.")).not.toBeInTheDocument();
    expect(screen.queryByText("Enter a valid message.")).not.toBeInTheDocument();
    expect(screen.queryByText("Cancelling device sign-in…")).not.toBeInTheDocument();
  });

  it("recovers from a safe provider failure without remaining Connecting", async () => {
    const user = userEvent.setup();
    connectXaiMock.mockRejectedValue(new ProviderError("provider_unavailable"));
    getConnectionStatusMock
      .mockResolvedValueOnce({
        state: "disconnected",
        providerId: "xai-subscription-oauth",
        model: "grok-3",
      })
      .mockResolvedValue({
        state: "disconnected",
        providerId: "xai-subscription-oauth",
        model: "grok-3",
      });

    render(<SettingsWindow />);
    await user.click(await screen.findByRole("button", { name: "Connect in browser" }));

    expect(await screen.findByText("Disconnected")).toBeInTheDocument();
    expect(screen.queryByText("Connecting")).not.toBeInTheDocument();
    expect(screen.getByRole("alert")).toHaveTextContent("The provider is unavailable. Try again.");
  });

  it("applies terminal connection status events from the native process", async () => {
    let emitStatus:
      | ((status: {
          state: "connected" | "disconnected" | "connecting";
          providerId: string;
          model: string;
        }) => void)
      | undefined;
    listenConnectionStatusChangedMock.mockImplementation(
      (
        handler: (status: {
          state: "connected" | "disconnected" | "connecting";
          providerId: string;
          model: string;
        }) => void,
      ) => {
        emitStatus = handler;
        return Promise.resolve(vi.fn());
      },
    );

    render(<SettingsWindow />);
    await screen.findByRole("button", { name: "Connect in browser" });

    emitStatus?.({
      state: "connected",
      providerId: "xai-subscription-oauth",
      model: "grok-3",
    });

    expect(await screen.findByText("Connected")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Disconnect" })).toBeInTheDocument();
  });

  it("persists appearance through the typed preference path", async () => {
    const user = userEvent.setup();
    loadThemePreferenceMock.mockResolvedValue("dark");
    render(<SettingsWindow />);

    const nav = await screen.findByRole("navigation", { name: "Settings categories" });
    await user.click(within(nav).getByRole("button", { name: "Appearance" }));
    const appearance = await screen.findByRole("combobox", { name: "Appearance" });
    await waitFor(() => expect(appearance).toHaveValue("dark"));
    await user.selectOptions(appearance, "system");
    expect(saveThemePreferenceMock).toHaveBeenCalledWith("system");
  });
});
