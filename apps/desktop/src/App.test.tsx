import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import App from "./App";
import type { AgentSession, AgentSessionDetail, AgentStreamEvent } from "./platform/agents";
import { ProjectStorageError, type Project } from "./platform/projects";

interface NativeCloseRequestedEvent {
  preventDefault: () => void;
}

type NativeCloseRequestedHandler = (event: NativeCloseRequestedEvent) => void;

const {
  cancelAgentTurnMock,
  createProjectMock,
  exitApplicationMock,
  getApplicationInfoMock,
  getAgentSessionMock,
  getConnectionStatusMock,
  getProviderModelCatalogMock,
  getProviderModelSelectionMock,
  listAgentSessionsMock,
  listProjectsMock,
  listenAppearanceChangedMock,
  listenConnectionStatusChangedMock,
  listenProviderModelCatalogChangedMock,
  listenProviderModelSelectionChangedMock,
  loadThemePreferenceMock,
  onCloseRequestedMock,
  openProjectMock,
  openSettingsWindowMock,
  sendAgentMessageMock,
  setAgentSessionProjectMock,
  syncConnectionStatusMock,
  unlistenCloseRequestedMock,
  updateProjectInstructionsMock,
} = vi.hoisted(() => ({
  cancelAgentTurnMock: vi.fn(),
  createProjectMock: vi.fn(),
  exitApplicationMock: vi.fn(),
  getApplicationInfoMock: vi.fn(),
  getAgentSessionMock: vi.fn(),
  getConnectionStatusMock: vi.fn(),
  getProviderModelCatalogMock: vi.fn(),
  getProviderModelSelectionMock: vi.fn(),
  listAgentSessionsMock: vi.fn(),
  listProjectsMock: vi.fn(),
  listenAppearanceChangedMock: vi.fn(),
  listenConnectionStatusChangedMock: vi.fn(),
  listenProviderModelCatalogChangedMock: vi.fn(),
  listenProviderModelSelectionChangedMock: vi.fn(),
  loadThemePreferenceMock: vi.fn(),
  onCloseRequestedMock: vi.fn<(handler: NativeCloseRequestedHandler) => Promise<() => void>>(),
  openProjectMock: vi.fn(),
  openSettingsWindowMock: vi.fn(),
  sendAgentMessageMock: vi.fn(),
  setAgentSessionProjectMock: vi.fn(),
  syncConnectionStatusMock: vi.fn(),
  unlistenCloseRequestedMock: vi.fn(),
  updateProjectInstructionsMock: vi.fn(),
}));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    label: "main",
    onCloseRequested: onCloseRequestedMock,
  }),
}));

vi.mock("./platform/application", () => ({
  getApplicationInfo: getApplicationInfoMock,
}));

vi.mock("./platform/settings", () => ({
  exitApplication: exitApplicationMock,
  listenConnectionStatusChanged: listenConnectionStatusChangedMock,
  listenSettingsNavigate: vi.fn(() => Promise.resolve(vi.fn())),
  openSettingsWindow: openSettingsWindowMock,
  refreshConnectionStatus: getConnectionStatusMock,
  syncConnectionStatus: syncConnectionStatusMock,
}));

vi.mock("./theme", async (importOriginal) => {
  const actual = await importOriginal<typeof import("./theme")>();
  return {
    ...actual,
    loadThemePreference: loadThemePreferenceMock,
    listenAppearanceChanged: listenAppearanceChangedMock,
  };
});

vi.mock("./platform/provider", async (importOriginal) => {
  const actual = await importOriginal<typeof import("./platform/provider")>();
  return {
    ...actual,
    getConnectionStatus: getConnectionStatusMock,
    getProviderModelCatalog: getProviderModelCatalogMock,
    getProviderModelSelection: getProviderModelSelectionMock,
    listenProviderModelCatalogChanged: listenProviderModelCatalogChangedMock,
    listenProviderModelSelectionChanged: listenProviderModelSelectionChangedMock,
  };
});

vi.mock("./platform/agents", async (importOriginal) => {
  const actual = await importOriginal<typeof import("./platform/agents")>();
  return {
    ...actual,
    listAgentSessions: listAgentSessionsMock,
    getAgentSession: getAgentSessionMock,
    sendAgentMessage: sendAgentMessageMock,
    cancelAgentTurn: cancelAgentTurnMock,
    setAgentSessionProject: setAgentSessionProjectMock,
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
    cancelAgentTurnMock.mockReset();
    exitApplicationMock.mockReset();
    getApplicationInfoMock.mockReset();
    getAgentSessionMock.mockReset();
    listProjectsMock.mockReset();
    openProjectMock.mockReset();
    openSettingsWindowMock.mockReset();
    sendAgentMessageMock.mockReset();
    setAgentSessionProjectMock.mockReset();
    syncConnectionStatusMock.mockReset();
    updateProjectInstructionsMock.mockReset();
    listAgentSessionsMock.mockReset();
    getConnectionStatusMock.mockReset();
    getProviderModelCatalogMock.mockReset();
    getProviderModelSelectionMock.mockReset();
    loadThemePreferenceMock.mockReset();
    listenAppearanceChangedMock.mockReset();
    listenConnectionStatusChangedMock.mockReset();
    listenProviderModelCatalogChangedMock.mockReset();
    listenProviderModelSelectionChangedMock.mockReset();
    onCloseRequestedMock.mockReset();
    unlistenCloseRequestedMock.mockReset();

    getApplicationInfoMock.mockResolvedValue({ name: "TULE", version: "0.1.0" });
    getConnectionStatusMock.mockResolvedValue({
      state: "disconnected",
      providerId: "openai-chatgpt-compat",
      model: "gpt-5.5",
    });
    getProviderModelCatalogMock.mockResolvedValue({
      providerId: "openai-chatgpt-compat",
      models: [
        {
          id: "gpt-5.5",
          displayName: "GPT-5.5",
          description: null,
          isProviderDefault: true,
        },
      ],
      freshness: "current",
      retrievedAtUnixMs: 1,
      compatibilityRevision: "1.0.0",
    });
    getProviderModelSelectionMock.mockResolvedValue({
      providerId: "openai-chatgpt-compat",
      selectedModelId: "gpt-5.5",
      requiresSelection: false,
    });
    loadThemePreferenceMock.mockResolvedValue("system");
    listenAppearanceChangedMock.mockResolvedValue(vi.fn());
    listenConnectionStatusChangedMock.mockResolvedValue(vi.fn());
    listenProviderModelCatalogChangedMock.mockResolvedValue(vi.fn());
    listenProviderModelSelectionChangedMock.mockResolvedValue(vi.fn());
    openSettingsWindowMock.mockResolvedValue(undefined);
    exitApplicationMock.mockResolvedValue(undefined);
    syncConnectionStatusMock.mockResolvedValue({
      state: "reconnect_required",
      providerId: "openai-chatgpt-compat",
      model: "gpt-5.5",
    });
    listProjectsMock.mockResolvedValue([]);
    listAgentSessionsMock.mockResolvedValue([]);
    cancelAgentTurnMock.mockResolvedValue(undefined);
    onCloseRequestedMock.mockResolvedValue(unlistenCloseRequestedMock);
  });

  it("opens to the Agent shell with empty-session wordmark, sidebar hierarchy, and global chrome", async () => {
    render(<App />);

    expect(await screen.findByRole("img", { name: "TULE" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "New session" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Projects" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "No project" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Manage projects" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Application menu" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Settings" })).toBeInTheDocument();
    expect(
      screen.getByText("Connect ChatGPT in Settings to message the Agent."),
    ).toBeInTheDocument();
  });

  it("converges on terminal Connected status from the native connection event", async () => {
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

    render(<App />);
    expect(
      await screen.findByText("Connect ChatGPT in Settings to message the Agent."),
    ).toBeInTheDocument();

    act(() => {
      emitStatus?.({
        state: "connected",
        providerId: "openai-chatgpt-compat",
        model: "gpt-5.5",
      });
    });

    await waitFor(() => {
      expect(
        screen.queryByText("Connect ChatGPT in Settings to message the Agent."),
      ).not.toBeInTheDocument();
    });
    expect(screen.getByRole("textbox", { name: "Message the Agent" })).toBeInTheDocument();
  });

  it("exits the whole application when an accepted main-window close occurs", async () => {
    const user = userEvent.setup();
    let closeHandler: NativeCloseRequestedHandler | undefined;
    onCloseRequestedMock.mockImplementation((handler) => {
      closeHandler = handler;
      return Promise.resolve(unlistenCloseRequestedMock);
    });

    render(<App />);
    await user.click(await screen.findByRole("button", { name: "Settings" }));
    expect(openSettingsWindowMock).toHaveBeenCalled();

    const preventDefault = vi.fn();
    closeHandler?.({ preventDefault });
    expect(preventDefault).toHaveBeenCalled();
    expect(exitApplicationMock).toHaveBeenCalled();
  });

  it("keeps the main window open when unsaved instructions decline the close guard", async () => {
    const user = userEvent.setup();
    const project: Project = {
      id: "11111111-1111-7111-8111-111111111111",
      displayName: "Atlas",
      instructions: "Saved guidance",
    };
    listProjectsMock.mockResolvedValue([project]);
    openProjectMock.mockResolvedValue(project);
    vi.spyOn(window, "confirm").mockReturnValue(false);
    let closeHandler: NativeCloseRequestedHandler | undefined;
    onCloseRequestedMock.mockImplementation((handler) => {
      closeHandler = handler;
      return Promise.resolve(unlistenCloseRequestedMock);
    });

    render(<App />);
    await user.click(await screen.findByRole("button", { name: "Manage projects" }));
    const atlasButtons = screen.getAllByRole("button", { name: /Atlas/ });
    const atlasButton = atlasButtons[atlasButtons.length - 1];
    if (atlasButton === undefined) {
      throw new Error("expected an Atlas project button");
    }
    await user.click(atlasButton);
    const editor = await screen.findByRole("textbox", { name: "Project instructions" });
    await user.clear(editor);
    await user.type(editor, "Changed guidance");

    const preventDefault = vi.fn();
    closeHandler?.({ preventDefault });
    expect(preventDefault).toHaveBeenCalled();
    expect(exitApplicationMock).not.toHaveBeenCalled();
  });

  it("routes Settings gear and menu commands through the shared open path", async () => {
    const user = userEvent.setup();
    render(<App />);

    await user.click(await screen.findByRole("button", { name: "Settings" }));
    expect(openSettingsWindowMock).toHaveBeenCalledWith();

    await user.click(screen.getByRole("button", { name: "Application menu" }));
    await user.click(screen.getByRole("menuitem", { name: /Open Settings/i }));
    expect(openSettingsWindowMock).toHaveBeenCalledTimes(2);

    await user.click(screen.getByRole("button", { name: "Open Settings" }));
    expect(openSettingsWindowMock).toHaveBeenCalledWith("providers");
  });

  it("exposes only implemented application menu groups", async () => {
    const user = userEvent.setup();
    render(<App />);

    await user.click(await screen.findByRole("button", { name: "Application menu" }));
    expect(screen.getByRole("group", { name: "File" })).toBeInTheDocument();
    expect(screen.getByRole("group", { name: "Edit" })).toBeInTheDocument();
    expect(screen.getByRole("group", { name: "Settings" })).toBeInTheDocument();
    expect(screen.queryByRole("group", { name: "View" })).not.toBeInTheDocument();
    expect(screen.queryByRole("group", { name: "Help" })).not.toBeInTheDocument();
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

  it("opens New project creation in the detail region", async () => {
    const user = userEvent.setup();
    createProjectMock.mockRejectedValue(new ProjectStorageError("invalid_project_name"));

    render(<App />);
    await user.click(await screen.findByRole("button", { name: "Manage projects" }));
    await user.click(screen.getByRole("button", { name: "New project" }));
    await user.click(screen.getByRole("button", { name: "Create project" }));
    expect(screen.getByText("Enter a project name.")).toBeInTheDocument();
  });

  it("resumes the most recent persisted session without empty-session wordmark", async () => {
    const session: AgentSession = {
      id: "22222222-2222-7222-8222-222222222222",
      title: "Existing session",
      projectId: null,
      modelId: "gpt-5.5",
    };
    listAgentSessionsMock.mockResolvedValue([session]);
    getAgentSessionMock.mockResolvedValue({
      session,
      turns: [
        {
          id: "33333333-3333-7333-8333-333333333333",
          ordinal: 1,
          userText: "Earlier question",
          agentText: "Persisted answer",
          state: "completed",
          errorCode: null,
        },
      ],
    });

    render(<App />);

    expect(await screen.findByRole("heading", { name: "Existing session" })).toBeInTheDocument();
    expect(screen.getByText("Earlier question")).toBeInTheDocument();
    expect(screen.getByText("Persisted answer")).toBeInTheDocument();
    expect(screen.queryByRole("img", { name: "TULE" })).not.toBeInTheDocument();
  });

  it("does not let a stale startup detail overwrite New session", async () => {
    const user = userEvent.setup();
    const session: AgentSession = {
      id: "22222222-2222-7222-8222-222222222222",
      title: "Existing session",
      projectId: null,
      modelId: "gpt-5.5",
    };
    let resolveDetail: ((detail: AgentSessionDetail) => void) | undefined;
    listAgentSessionsMock.mockResolvedValue([session]);
    getAgentSessionMock.mockReturnValue(
      new Promise<AgentSessionDetail>((resolve) => {
        resolveDetail = resolve;
      }),
    );

    render(<App />);
    await waitFor(() => expect(getAgentSessionMock).toHaveBeenCalledWith(session.id));
    await user.click(screen.getByRole("button", { name: "New session" }));
    expect(screen.getByRole("heading", { name: "New session" })).toBeInTheDocument();

    resolveDetail?.({
      session,
      turns: [
        {
          id: "33333333-3333-7333-8333-333333333333",
          ordinal: 1,
          userText: "Stale question",
          agentText: "Stale answer",
          state: "completed",
          errorCode: null,
        },
      ],
    });
    await waitFor(() => {
      expect(screen.getByRole("heading", { name: "New session" })).toBeInTheDocument();
      expect(screen.queryByText("Stale answer")).not.toBeInTheDocument();
    });
  });

  it("does not let a slower session selection overwrite the latest selection", async () => {
    const user = userEvent.setup();
    const first: AgentSession = {
      id: "22222222-2222-7222-8222-222222222222",
      title: "First session",
      projectId: null,
      modelId: "gpt-5.5",
    };
    const second: AgentSession = {
      id: "33333333-3333-7333-8333-333333333333",
      title: "Second session",
      projectId: null,
      modelId: "gpt-5.5",
    };
    let resolveFirst: ((detail: AgentSessionDetail) => void) | undefined;
    let resolveSecond: ((detail: AgentSessionDetail) => void) | undefined;
    listAgentSessionsMock.mockResolvedValue([first, second]);
    getAgentSessionMock
      .mockResolvedValueOnce({ session: first, turns: [] })
      .mockReturnValueOnce(
        new Promise<AgentSessionDetail>((resolve) => {
          resolveFirst = resolve;
        }),
      )
      .mockReturnValueOnce(
        new Promise<AgentSessionDetail>((resolve) => {
          resolveSecond = resolve;
        }),
      );

    render(<App />);
    expect(await screen.findByRole("heading", { name: "First session" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "First session" }));
    await user.click(screen.getByRole("button", { name: "Second session" }));

    resolveSecond?.({
      session: second,
      turns: [
        {
          id: "44444444-4444-7444-8444-444444444444",
          ordinal: 1,
          userText: "Latest question",
          agentText: "Latest answer",
          state: "completed",
          errorCode: null,
        },
      ],
    });
    expect(await screen.findByText("Latest answer")).toBeInTheDocument();

    resolveFirst?.({
      session: first,
      turns: [
        {
          id: "55555555-5555-7555-8555-555555555555",
          ordinal: 1,
          userText: "Older question",
          agentText: "Older answer",
          state: "completed",
          errorCode: null,
        },
      ],
    });
    await waitFor(() => {
      expect(screen.getByRole("heading", { name: "Second session" })).toBeInTheDocument();
      expect(screen.queryByText("Older answer")).not.toBeInTheDocument();
    });
  });

  it("confirms prospective Project changes on a persisted session", async () => {
    const user = userEvent.setup();
    const project: Project = {
      id: "11111111-1111-7111-8111-111111111111",
      displayName: "Atlas",
      instructions: "Keep answers short",
    };
    const session = {
      id: "22222222-2222-7222-8222-222222222222",
      title: "Existing session",
      projectId: null,
      modelId: "gpt-5.5",
    };
    listProjectsMock.mockResolvedValue([project]);
    listAgentSessionsMock.mockResolvedValue([session]);
    getAgentSessionMock.mockResolvedValue({ session, turns: [] });
    setAgentSessionProjectMock
      .mockResolvedValueOnce({ ...session, projectId: project.id })
      .mockResolvedValueOnce(session);
    const confirm = vi.spyOn(window, "confirm").mockReturnValueOnce(false).mockReturnValue(true);

    render(<App />);
    const context = await screen.findByRole("combobox", { name: "Project context" });

    await user.selectOptions(context, project.id);
    expect(confirm).toHaveBeenLastCalledWith("Use Atlas for future messages in this session?");
    expect(setAgentSessionProjectMock).not.toHaveBeenCalled();
    expect(context).toHaveValue("");

    await user.selectOptions(context, project.id);
    await waitFor(() => expect(context).toHaveValue(project.id));
    expect(setAgentSessionProjectMock).toHaveBeenCalledWith(session.id, project.id);

    await user.selectOptions(context, "");
    await waitFor(() => expect(context).toHaveValue(""));
    expect(confirm).toHaveBeenLastCalledWith("Use No project for future messages in this session?");
    expect(setAgentSessionProjectMock).toHaveBeenLastCalledWith(session.id, null);
  });

  it("blocks Send until a prospective Project change is committed", async () => {
    const user = userEvent.setup();
    const project: Project = {
      id: "11111111-1111-7111-8111-111111111111",
      displayName: "Atlas",
      instructions: "Keep answers short",
    };
    const session: AgentSession = {
      id: "22222222-2222-7222-8222-222222222222",
      title: "Existing session",
      projectId: null,
      modelId: "gpt-5.5",
    };
    let resolveProjectChange: ((updated: AgentSession) => void) | undefined;
    setAgentSessionProjectMock.mockReturnValue(
      new Promise<AgentSession>((resolve) => {
        resolveProjectChange = resolve;
      }),
    );
    getConnectionStatusMock.mockResolvedValue({
      state: "connected",
      providerId: "openai-chatgpt-compat",
      model: "gpt-5.5",
    });
    listProjectsMock.mockResolvedValue([project]);
    listAgentSessionsMock.mockResolvedValue([session]);
    getAgentSessionMock.mockResolvedValue({ session, turns: [] });
    vi.spyOn(window, "confirm").mockReturnValue(true);

    render(<App />);
    const composer = await screen.findByRole("textbox", { name: "Message the Agent" });
    await user.type(composer, "Question");
    await user.selectOptions(screen.getByRole("combobox", { name: "Project context" }), project.id);

    expect(screen.getByRole("button", { name: "Send" })).toBeDisabled();
    fireEvent.keyDown(composer, { key: "Enter", shiftKey: false });
    expect(sendAgentMessageMock).not.toHaveBeenCalled();

    resolveProjectChange?.({ ...session, projectId: project.id });
    await waitFor(() => expect(screen.getByRole("button", { name: "Send" })).toBeEnabled());
  });

  it("keeps an unsaved Project draft until guarded navigation is confirmed", async () => {
    const user = userEvent.setup();
    const project: Project = {
      id: "11111111-1111-7111-8111-111111111111",
      displayName: "Atlas",
      instructions: "Saved guidance",
    };
    listProjectsMock.mockResolvedValue([project]);
    openProjectMock.mockResolvedValue(project);
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);

    render(<App />);
    await user.click(await screen.findByRole("button", { name: "Manage projects" }));
    const atlasButtons = screen.getAllByRole("button", { name: /Atlas/ });
    await user.click(atlasButtons[atlasButtons.length - 1]);
    const editor = await screen.findByRole("textbox", { name: "Project instructions" });
    await user.clear(editor);
    await user.type(editor, "Changed guidance");
    expect(await screen.findByText("Unsaved changes")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "New session" }));
    expect(confirm).toHaveBeenCalledWith("Discard unsaved project instructions and continue?");
    expect(screen.getByDisplayValue("Changed guidance")).toBeInTheDocument();

    confirm.mockReturnValue(true);
    await user.click(screen.getByRole("button", { name: "New session" }));
    expect(screen.getByRole("heading", { name: "New session" })).toBeInTheDocument();
    expect(screen.queryByRole("textbox", { name: "Project instructions" })).not.toBeInTheDocument();

    const confirmationCount = confirm.mock.calls.length;
    await user.click(screen.getByRole("button", { name: "New session" }));
    expect(confirm).toHaveBeenCalledTimes(confirmationCount);
  });

  it("disables persistent navigation while Project instructions are saving", async () => {
    const user = userEvent.setup();
    const project: Project = {
      id: "11111111-1111-7111-8111-111111111111",
      displayName: "Atlas",
      instructions: "Saved guidance",
    };
    let resolveSave: ((saved: Project) => void) | undefined;
    updateProjectInstructionsMock.mockReturnValue(
      new Promise<Project>((resolve) => {
        resolveSave = resolve;
      }),
    );
    listProjectsMock.mockResolvedValue([project]);
    openProjectMock.mockResolvedValue(project);

    render(<App />);
    await user.click(await screen.findByRole("button", { name: "Manage projects" }));
    const atlasButtons = screen.getAllByRole("button", { name: /Atlas/ });
    await user.click(atlasButtons[atlasButtons.length - 1]);
    const editor = await screen.findByRole("textbox", { name: "Project instructions" });
    await user.clear(editor);
    await user.type(editor, "Changed guidance");
    await user.click(screen.getByRole("button", { name: "Save instructions" }));

    await waitFor(() => {
      expect(screen.getByRole("button", { name: "New session" })).toBeDisabled();
      expect(screen.getByRole("button", { name: "Manage projects" })).toBeDisabled();
      expect(screen.getByRole("button", { name: "Use with Agents" })).toBeDisabled();
    });

    resolveSave?.({ ...project, instructions: "Changed guidance" });
    await waitFor(() => expect(screen.getByText("Saved")).toBeInTheDocument());
    expect(screen.getByRole("button", { name: "New session" })).toBeEnabled();
  });

  it("moves the interface to Reconnect required after an authentication terminal", async () => {
    const user = userEvent.setup();
    getConnectionStatusMock.mockResolvedValue({
      state: "connected",
      providerId: "openai-chatgpt-compat",
      model: "gpt-5.5",
    });
    sendAgentMessageMock.mockImplementation(
      (options: { onEvent: (event: AgentStreamEvent) => void }) => {
        options.onEvent({ kind: "started", session_id: "s1", turn_id: "t1" });
        options.onEvent({
          kind: "terminal",
          turn: {
            id: "t1",
            ordinal: 1,
            userText: "Hello",
            agentText: "",
            state: "failed",
            errorCode: "authentication_required",
          },
        });
        return Promise.resolve();
      },
    );

    render(<App />);
    const composer = await screen.findByRole("textbox", { name: "Message the Agent" });
    await user.type(composer, "Hello");
    await user.click(screen.getByRole("button", { name: "Send" }));

    await waitFor(() => expect(syncConnectionStatusMock).toHaveBeenCalled());
    await user.click(screen.getByRole("button", { name: "Open Settings" }));
    expect(openSettingsWindowMock).toHaveBeenCalledWith("providers");
  });

  it("queues an immediate cancel until the native Started event supplies the turn ID", async () => {
    const user = userEvent.setup();
    let streamOptions: { onEvent: (event: AgentStreamEvent) => void } | undefined;
    let resolveSend: (() => void) | undefined;
    getConnectionStatusMock.mockResolvedValue({
      state: "connected",
      providerId: "openai-chatgpt-compat",
      model: "gpt-5.5",
    });
    sendAgentMessageMock.mockImplementation(
      (options: { onEvent: (event: AgentStreamEvent) => void }) => {
        streamOptions = options;
        return new Promise<void>((resolve) => {
          resolveSend = resolve;
        });
      },
    );

    render(<App />);
    const composer = await screen.findByRole("textbox", { name: "Message the Agent" });
    await user.type(composer, "Question");
    await user.click(screen.getByRole("button", { name: "Send" }));
    expect(screen.getByRole("button", { name: "New session" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Manage projects" })).toBeDisabled();
    await user.click(await screen.findByRole("button", { name: "Cancel" }));
    expect(cancelAgentTurnMock).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "Cancelling…" })).toBeDisabled();

    act(() => {
      streamOptions?.onEvent({ kind: "started", session_id: "s1", turn_id: "native-t1" });
    });
    await waitFor(() => expect(cancelAgentTurnMock).toHaveBeenCalledWith("native-t1"));

    act(() => {
      streamOptions?.onEvent({
        kind: "terminal",
        turn: {
          id: "native-t1",
          ordinal: 1,
          userText: "Question",
          agentText: "",
          state: "cancelled",
          errorCode: "cancelled",
        },
      });
      resolveSend?.();
    });
    await waitFor(() => expect(screen.queryByRole("button", { name: "Cancelling…" })).toBeNull());
    expect(screen.getByRole("button", { name: "New session" })).toBeEnabled();
  });

  it("loads native appearance preference on startup", async () => {
    loadThemePreferenceMock.mockResolvedValue("dark");
    render(<App />);
    await waitFor(() => expect(document.documentElement.dataset.theme).toBe("dark"));
  });
});
