import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import App from "./App";
import type { AgentSession, AgentSessionDetail, AgentStreamEvent } from "./platform/agents";
import { ProjectStorageError, type Project } from "./platform/projects";
import { ProviderError } from "./platform/provider";

interface NativeCloseRequestedEvent {
  preventDefault: () => void;
}

type NativeCloseRequestedHandler = (event: NativeCloseRequestedEvent) => void;

const {
  cancelAgentTurnMock,
  cancelChatgptConnectMock,
  connectChatgptMock,
  createProjectMock,
  disconnectChatgptMock,
  getApplicationInfoMock,
  getAgentSessionMock,
  getConnectionStatusMock,
  listAgentSessionsMock,
  listProjectsMock,
  onCloseRequestedMock,
  openProjectMock,
  sendAgentMessageMock,
  setAgentSessionProjectMock,
  unlistenCloseRequestedMock,
  updateProjectInstructionsMock,
} = vi.hoisted(() => ({
  cancelAgentTurnMock: vi.fn(),
  cancelChatgptConnectMock: vi.fn(),
  connectChatgptMock: vi.fn(),
  createProjectMock: vi.fn(),
  disconnectChatgptMock: vi.fn(),
  getApplicationInfoMock: vi.fn(),
  getAgentSessionMock: vi.fn(),
  getConnectionStatusMock: vi.fn(),
  listAgentSessionsMock: vi.fn(),
  listProjectsMock: vi.fn(),
  onCloseRequestedMock: vi.fn<(handler: NativeCloseRequestedHandler) => Promise<() => void>>(),
  openProjectMock: vi.fn(),
  sendAgentMessageMock: vi.fn(),
  setAgentSessionProjectMock: vi.fn(),
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
    cancelChatgptConnect: cancelChatgptConnectMock,
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
    cancelChatgptConnectMock.mockReset();
    getApplicationInfoMock.mockReset();
    getAgentSessionMock.mockReset();
    listProjectsMock.mockReset();
    openProjectMock.mockReset();
    sendAgentMessageMock.mockReset();
    setAgentSessionProjectMock.mockReset();
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
    cancelAgentTurnMock.mockResolvedValue(undefined);
    cancelChatgptConnectMock.mockResolvedValue(undefined);
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
    expect(screen.getByRole("complementary", { name: "Workspace" })).toHaveAttribute("inert");
    expect(document.querySelector("main")).toHaveAttribute("inert");
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

  it("resumes the most recent persisted session", async () => {
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

    await user.click(screen.getByRole("button", { name: "Back to Agents" }));
    expect(confirm).toHaveBeenCalledWith("Discard unsaved project instructions and continue?");
    expect(screen.getByDisplayValue("Changed guidance")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "New session" }));
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
      expect(screen.getByRole("button", { name: "Back to Agents" })).toBeDisabled();
      expect(screen.getByRole("button", { name: "Use with Agents" })).toBeDisabled();
    });

    resolveSave?.({ ...project, instructions: "Changed guidance" });
    await waitFor(() => expect(screen.getByText("Saved")).toBeInTheDocument());
    expect(screen.getByRole("button", { name: "New session" })).toBeEnabled();
  });

  it("cancels browser connection explicitly and reports the safe result", async () => {
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

    render(<App />);
    await user.click(await screen.findByRole("button", { name: "Settings" }));
    await user.click(screen.getByRole("button", { name: "Connect in browser" }));
    await user.click(await screen.findByRole("button", { name: "Cancel connection" }));

    expect(cancelChatgptConnectMock).toHaveBeenCalledOnce();
    expect(await screen.findByText("Browser connection cancelled.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Connect in browser" })).toBeInTheDocument();
  });

  it("shows confirmed local credential removal after Disconnect", async () => {
    const user = userEvent.setup();
    getConnectionStatusMock.mockResolvedValue({
      state: "connected",
      providerId: "openai-chatgpt-compat",
      model: "gpt-5.5",
    });
    disconnectChatgptMock.mockResolvedValue({
      state: "disconnected",
      providerId: "openai-chatgpt-compat",
      model: "gpt-5.5",
    });

    render(<App />);
    await user.click(await screen.findByRole("button", { name: "Settings" }));
    await user.click(await screen.findByRole("button", { name: "Disconnect" }));

    expect(disconnectChatgptMock).toHaveBeenCalledOnce();
    expect(await screen.findByText("Removed from this device")).toBeInTheDocument();
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
    await user.click(screen.getByRole("button", { name: "Settings" }));

    expect(await screen.findByText("Reconnect required")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Connect in browser" })).toBeInTheDocument();
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

  it("loads a saved appearance and clears it when System is selected", async () => {
    const user = userEvent.setup();
    window.localStorage.setItem("tule-theme", "dark");

    render(<App />);
    await user.click(await screen.findByRole("button", { name: "Settings" }));
    const appearance = screen.getByRole("combobox", { name: "Appearance" });
    expect(appearance).toHaveValue("dark");
    await user.selectOptions(appearance, "system");

    expect(window.localStorage.getItem("tule-theme")).toBeNull();
    expect(document.documentElement.dataset.theme).toBeUndefined();
  });
});
