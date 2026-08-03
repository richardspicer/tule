import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { SettingsWindow } from "./SettingsWindow";
import { ProviderError } from "../platform/provider";

const {
  cancelChatgptConnectMock,
  connectChatgptMock,
  disconnectChatgptMock,
  getConnectionStatusMock,
  listenAppearanceChangedMock,
  listenConnectionStatusChangedMock,
  listenSettingsNavigateMock,
  loadThemePreferenceMock,
  saveThemePreferenceMock,
  takeSettingsLaunchCategoryMock,
} = vi.hoisted(() => ({
  cancelChatgptConnectMock: vi.fn(),
  connectChatgptMock: vi.fn(),
  disconnectChatgptMock: vi.fn(),
  getConnectionStatusMock: vi.fn(),
  listenAppearanceChangedMock: vi.fn(),
  listenConnectionStatusChangedMock: vi.fn(),
  listenSettingsNavigateMock: vi.fn(),
  loadThemePreferenceMock: vi.fn(),
  saveThemePreferenceMock: vi.fn(),
  takeSettingsLaunchCategoryMock: vi.fn(),
}));

vi.mock("../platform/provider", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../platform/provider")>();
  return {
    ...actual,
    cancelChatgptConnect: cancelChatgptConnectMock,
    connectChatgpt: connectChatgptMock,
    disconnectChatgpt: disconnectChatgptMock,
    getConnectionStatus: getConnectionStatusMock,
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
    cancelChatgptConnectMock.mockReset();
    connectChatgptMock.mockReset();
    disconnectChatgptMock.mockReset();
    getConnectionStatusMock.mockReset();
    listenAppearanceChangedMock.mockReset();
    listenConnectionStatusChangedMock.mockReset();
    listenSettingsNavigateMock.mockReset();
    loadThemePreferenceMock.mockReset();
    saveThemePreferenceMock.mockReset();
    takeSettingsLaunchCategoryMock.mockReset();

    getConnectionStatusMock.mockResolvedValue({
      state: "disconnected",
      providerId: "openai-chatgpt-compat",
      model: "gpt-5.5",
    });
    loadThemePreferenceMock.mockResolvedValue("system");
    saveThemePreferenceMock.mockImplementation((theme: "system" | "light" | "dark") =>
      Promise.resolve(theme),
    );
    takeSettingsLaunchCategoryMock.mockResolvedValue(null);
    listenAppearanceChangedMock.mockResolvedValue(vi.fn());
    listenConnectionStatusChangedMock.mockResolvedValue(vi.fn());
    listenSettingsNavigateMock.mockResolvedValue(vi.fn());
    cancelChatgptConnectMock.mockResolvedValue(undefined);
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
        "Uses a compatibility sign-in path that is not an official TULE integration and may stop working.",
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

  it("cancels browser connection and reports the safe result", async () => {
    const user = userEvent.setup();
    let rejectConnect: ((error: ProviderError) => void) | undefined;
    connectChatgptMock.mockReturnValue(
      new Promise((_resolve, reject) => {
        rejectConnect = reject;
      }),
    );
    cancelChatgptConnectMock.mockImplementation(() => {
      rejectConnect?.(new ProviderError("cancelled"));
      return Promise.resolve();
    });

    render(<SettingsWindow />);
    await user.click(await screen.findByRole("button", { name: "Connect in browser" }));
    await user.click(await screen.findByRole("button", { name: "Cancel connection" }));

    expect(cancelChatgptConnectMock).toHaveBeenCalledOnce();
    expect(await screen.findByText("Browser connection cancelled.")).toBeInTheDocument();
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
