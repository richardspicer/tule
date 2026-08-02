import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import App from "./App";
import { ProjectStorageError, type Project } from "./platform/projects";

interface NativeCloseRequestedEvent {
  preventDefault: () => void;
}

type NativeCloseRequestedHandler = (event: NativeCloseRequestedEvent) => void;

const {
  connectChatgptMock,
  createProjectMock,
  disconnectChatgptMock,
  getApplicationInfoMock,
  getConnectionStatusMock,
  listAgentSessionsMock,
  listProjectsMock,
  onCloseRequestedMock,
  openProjectMock,
  unlistenCloseRequestedMock,
  updateProjectInstructionsMock,
} = vi.hoisted(() => ({
  connectChatgptMock: vi.fn(),
  createProjectMock: vi.fn(),
  disconnectChatgptMock: vi.fn(),
  getApplicationInfoMock: vi.fn(),
  getConnectionStatusMock: vi.fn(),
  listAgentSessionsMock: vi.fn(),
  listProjectsMock: vi.fn(),
  onCloseRequestedMock: vi.fn<(handler: NativeCloseRequestedHandler) => Promise<() => void>>(),
  openProjectMock: vi.fn(),
  unlistenCloseRequestedMock: vi.fn(),
  updateProjectInstructionsMock: vi.fn(),
}));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    onCloseRequested: onCloseRequestedMock,
  }),
}));

vi.mock("./platform/application", () => ({
  getApplicationInfo: getApplicationInfoMock,
}));

vi.mock("./platform/provider", async (importOriginal) => {
  const actual = await importOriginal<typeof import("./platform/provider")>();
  return {
    ...actual,
    connectChatgpt: connectChatgptMock,
    disconnectChatgpt: disconnectChatgptMock,
    getConnectionStatus: getConnectionStatusMock,
  };
});

vi.mock("./platform/agents", async (importOriginal) => {
  const actual = await importOriginal<typeof import("./platform/agents")>();
  return {
    ...actual,
    listAgentSessions: listAgentSessionsMock,
    getAgentSession: vi.fn(),
    sendAgentMessage: vi.fn(),
    cancelAgentTurn: vi.fn(),
    setAgentSessionProject: vi.fn(),
  };
});

vi.mock("./platform/projects", async (importOriginal) => {
  const actual = await importOriginal<typeof import("./platform/projects")>();

  return {
    ...actual,
    createProject: createProjectMock,
    listProjects: listProjectsMock,
    openProject: openProjectMock,
    updateProjectInstructions: updateProjectInstructionsMock,
  };
});

describe("App", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  beforeEach(() => {
    createProjectMock.mockReset();
    getApplicationInfoMock.mockReset();
    listProjectsMock.mockReset();
    openProjectMock.mockReset();
    updateProjectInstructionsMock.mockReset();
    listAgentSessionsMock.mockReset();
    getConnectionStatusMock.mockReset();
    connectChatgptMock.mockReset();
    disconnectChatgptMock.mockReset();
    onCloseRequestedMock.mockReset();
    unlistenCloseRequestedMock.mockReset();
    window.localStorage.clear();

    getApplicationInfoMock.mockResolvedValue({ name: "TULE", version: "0.1.0" });
    getConnectionStatusMock.mockResolvedValue({
      state: "disconnected",
      providerId: "openai-chatgpt-compat",
      model: "gpt-5.5",
    });
    listProjectsMock.mockResolvedValue([]);
    listAgentSessionsMock.mockResolvedValue([]);
    onCloseRequestedMock.mockResolvedValue(unlistenCloseRequestedMock);
  });

  it("opens to the Agent shell with wordmark, sidebar hierarchy, and Settings gear", async () => {
    render(<App />);

    expect(await screen.findByRole("img", { name: "TULE" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "New session" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Projects" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Projectless recents" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Manage projects" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Settings" })).toBeInTheDocument();
    expect(
      screen.getByText("Connect ChatGPT in Settings to message the Agent."),
    ).toBeInTheDocument();
  });

  it("opens Settings from the gear with experimental disclosure", async () => {
    const user = userEvent.setup();
    render(<App />);

    await user.click(await screen.findByRole("button", { name: "Settings" }));
    expect(
      screen.getByText(
        "Uses a compatibility sign-in path that is not an official TULE integration and may stop working.",
      ),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Connect in browser" })).toBeInTheDocument();
  });

  it("switches to Project manager and returns with Use with Agents", async () => {
    const user = userEvent.setup();
    const project: Project = {
      id: "11111111-1111-7111-8111-111111111111",
      displayName: "Atlas",
      instructions: "Keep answers short",
    };
    listProjectsMock.mockResolvedValue([project]);
    openProjectMock.mockResolvedValue(project);

    render(<App />);
    await user.click(await screen.findByRole("button", { name: "Manage projects" }));
    expect(screen.getByRole("heading", { name: "Projects", level: 1 })).toBeInTheDocument();

    const atlasButtons = screen.getAllByRole("button", { name: /Atlas/ });
    const atlasOpen = atlasButtons[atlasButtons.length - 1];
    expect(atlasOpen).toBeDefined();
    await user.click(atlasOpen);
    await waitFor(() => expect(openProjectMock).toHaveBeenCalled());
    await user.click(await screen.findByRole("button", { name: "Use with Agents" }));
    expect(screen.getByRole("heading", { name: "New session" })).toBeInTheDocument();
    expect(screen.getAllByText("Atlas").length).toBeGreaterThan(0);
  });

  it("preserves project create validation", async () => {
    const user = userEvent.setup();
    createProjectMock.mockRejectedValue(new ProjectStorageError("invalid_project_name"));

    render(<App />);
    await user.click(await screen.findByRole("button", { name: "Manage projects" }));
    await user.click(screen.getByRole("button", { name: "Create project" }));
    expect(screen.getByText("Enter a project name.")).toBeInTheDocument();
  });
});
