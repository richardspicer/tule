import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import App from "./App";
import type { AgentSession, AgentSessionDetail, AgentStreamEvent } from "./platform/agents";
import { ProjectStorageError, type Project } from "./platform/projects";
import { ProviderError, type ProviderModelCatalog } from "./platform/provider";

interface NativeCloseRequestedEvent {
  preventDefault: () => void;
}

type NativeCloseRequestedHandler = (event: NativeCloseRequestedEvent) => void;

const {
  cancelAgentTurnMock,
  clearAgentTextSourceDraftMock,
  createProjectMock,
  exitApplicationMock,
  getApplicationInfoMock,
  getAgentSessionMock,
  getConnectionStatusMock,
  getPersistedProviderModelCatalogMock,
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
  pickAgentTextSourceMock,
  pickAgentTextFolderSourceMock,
  attachAgentTextLinkSourceMock,
  sendAgentMessageMock,
  setAgentSessionProjectMock,
  setAgentSourceDraftScopeMock,
  syncConnectionStatusMock,
  unlistenCloseRequestedMock,
  updateProjectInstructionsMock,
} = vi.hoisted(() => ({
  cancelAgentTurnMock: vi.fn(),
  clearAgentTextSourceDraftMock: vi.fn(),
  createProjectMock: vi.fn(),
  exitApplicationMock: vi.fn(),
  getApplicationInfoMock: vi.fn(),
  getAgentSessionMock: vi.fn(),
  getConnectionStatusMock: vi.fn(),
  getPersistedProviderModelCatalogMock: vi.fn(),
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
  pickAgentTextSourceMock: vi.fn(),
  pickAgentTextFolderSourceMock: vi.fn(),
  attachAgentTextLinkSourceMock: vi.fn(),
  sendAgentMessageMock: vi.fn(),
  setAgentSessionProjectMock: vi.fn(),
  setAgentSourceDraftScopeMock: vi.fn(),
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
    getPersistedProviderModelCatalog: getPersistedProviderModelCatalogMock,
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
    pickAgentTextSource: pickAgentTextSourceMock,
    pickAgentTextFolderSource: pickAgentTextFolderSourceMock,
    attachAgentTextLinkSource: attachAgentTextLinkSourceMock,
    clearAgentTextSourceDraft: clearAgentTextSourceDraftMock,
    setAgentSourceDraftScope: setAgentSourceDraftScopeMock,
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
    clearAgentTextSourceDraftMock.mockReset();
    exitApplicationMock.mockReset();
    getApplicationInfoMock.mockReset();
    getAgentSessionMock.mockReset();
    listProjectsMock.mockReset();
    openProjectMock.mockReset();
    openSettingsWindowMock.mockReset();
    pickAgentTextSourceMock.mockReset();
    pickAgentTextFolderSourceMock.mockReset();
    attachAgentTextLinkSourceMock.mockReset();
    sendAgentMessageMock.mockReset();
    setAgentSessionProjectMock.mockReset();
    setAgentSourceDraftScopeMock.mockReset();
    syncConnectionStatusMock.mockReset();
    clearAgentTextSourceDraftMock.mockResolvedValue(undefined);
    pickAgentTextSourceMock.mockResolvedValue({ status: "cancelled" });
    pickAgentTextFolderSourceMock.mockResolvedValue({ status: "cancelled" });
    attachAgentTextLinkSourceMock.mockResolvedValue({ status: "cancelled" });
    setAgentSourceDraftScopeMock.mockResolvedValue(undefined);
    updateProjectInstructionsMock.mockReset();
    listAgentSessionsMock.mockReset();
    getConnectionStatusMock.mockReset();
    getPersistedProviderModelCatalogMock.mockReset();
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
      providerId: "xai-subscription-oauth",
      model: "grok-3",
    });
    getProviderModelCatalogMock.mockResolvedValue({
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
      retrievedAtUnixMs: 1,
      compatibilityRevision: "1.0.0",
    });
    getPersistedProviderModelCatalogMock.mockResolvedValue({
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
    listenAppearanceChangedMock.mockResolvedValue(vi.fn());
    listenConnectionStatusChangedMock.mockResolvedValue(vi.fn());
    listenProviderModelCatalogChangedMock.mockResolvedValue(vi.fn());
    listenProviderModelSelectionChangedMock.mockResolvedValue(vi.fn());
    openSettingsWindowMock.mockResolvedValue(undefined);
    exitApplicationMock.mockResolvedValue(undefined);
    syncConnectionStatusMock.mockResolvedValue({
      state: "reconnect_required",
      providerId: "xai-subscription-oauth",
      model: "grok-3",
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
    expect(screen.getByText("Add a Provider to get started.")).toBeInTheDocument();
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
    expect(await screen.findByText("Add a Provider to get started.")).toBeInTheDocument();

    act(() => {
      emitStatus?.({
        state: "connected",
        providerId: "xai-subscription-oauth",
        model: "grok-3",
      });
    });

    await waitFor(() => {
      expect(screen.queryByText("Add a Provider to get started.")).not.toBeInTheDocument();
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

  it("clears the prior session attachment when Use with Agents starts a project session", async () => {
    const user = userEvent.setup();
    const session: AgentSession = {
      id: "22222222-2222-7222-8222-222222222222",
      title: "Existing session",
      projectId: null,
      modelId: "grok-3",
    };
    const project: Project = {
      id: "11111111-1111-7111-8111-111111111111",
      displayName: "Atlas",
      instructions: "Keep answers short",
    };
    listAgentSessionsMock.mockResolvedValue([session]);
    getAgentSessionMock.mockResolvedValue({ session, turns: [] });
    getConnectionStatusMock.mockResolvedValue({
      state: "connected",
      providerId: "xai-subscription-oauth",
      model: "grok-3",
    });
    listProjectsMock.mockResolvedValue([project]);
    openProjectMock.mockResolvedValue(project);
    pickAgentTextSourceMock.mockResolvedValue({
      status: "selected",
      attachment: {
        draftHandle: "deadbeef".repeat(4),
        displayName: "notes.txt",
        byteCount: 5,
        originKind: "local_text_file",
        memberCount: 1,
        canonicalUrl: null,
      },
    });

    render(<App />);
    expect(await screen.findByRole("heading", { name: "Existing session" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Attach file" }));
    expect(await screen.findByText(/Captured file snapshot: notes\.txt/)).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Manage projects" }));
    const atlasButtons = screen.getAllByRole("button", { name: /Atlas/ });
    const atlasOpen = atlasButtons[atlasButtons.length - 1];
    expect(atlasOpen).toBeDefined();
    await user.click(atlasOpen);
    await waitFor(() => expect(openProjectMock).toHaveBeenCalled());
    setAgentSourceDraftScopeMock.mockClear();

    await user.click(await screen.findByRole("button", { name: "Use with Agents" }));

    expect(screen.getByRole("heading", { name: "New session" })).toBeInTheDocument();
    expect(screen.queryByText(/Captured file snapshot: notes\.txt/)).not.toBeInTheDocument();
    expect(setAgentSourceDraftScopeMock).toHaveBeenCalledWith(null);
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
      modelId: "grok-3",
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
          sources: [],
        },
      ],
    });

    render(<App />);

    expect(await screen.findByRole("heading", { name: "Existing session" })).toBeInTheDocument();
    expect(screen.getByText("Earlier question")).toBeInTheDocument();
    expect(screen.getByText("Persisted answer")).toBeInTheDocument();
    expect(screen.queryByRole("img", { name: "TULE" })).not.toBeInTheDocument();
    await waitFor(() => expect(setAgentSourceDraftScopeMock).toHaveBeenCalledWith(session.id));
  });

  it("binds attachment scope on startup so a resumed session can attach without navigating", async () => {
    const user = userEvent.setup();
    const session: AgentSession = {
      id: "22222222-2222-7222-8222-222222222222",
      title: "Existing session",
      projectId: null,
      modelId: "grok-3",
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
          sources: [],
        },
      ],
    });
    getConnectionStatusMock.mockResolvedValue({
      state: "connected",
      providerId: "xai-subscription-oauth",
      model: "grok-3",
    });
    pickAgentTextSourceMock.mockResolvedValue({
      status: "selected",
      attachment: {
        draftHandle: "deadbeef".repeat(4),
        displayName: "notes.txt",
        byteCount: 5,
        originKind: "local_text_file",
        memberCount: 1,
        canonicalUrl: null,
      },
    });
    sendAgentMessageMock.mockImplementation(
      (options: {
        sessionId: string | null;
        sourceDraftHandle: string | null;
        onEvent: (event: AgentStreamEvent) => void;
      }) => {
        options.onEvent({
          kind: "started",
          session_id: session.id,
          turn_id: "44444444-4444-7444-8444-444444444444",
        });
        options.onEvent({
          kind: "terminal",
          turn: {
            id: "44444444-4444-7444-8444-444444444444",
            ordinal: 2,
            userText: "Use the file",
            agentText: "Used it",
            state: "completed",
            errorCode: null,
            sources: [
              {
                id: "55555555-5555-7555-8555-555555555555",
                originKind: "local_text_file",
                displayName: "notes.txt",
                byteCount: 5,
                contentSha256: "a".repeat(64),
                memberCount: 1,
                canonicalUrl: null,
              },
            ],
          },
        });
        expect(options.sessionId).toBe(session.id);
        expect(options.sourceDraftHandle).toBe("deadbeef".repeat(4));
        return Promise.resolve();
      },
    );

    render(<App />);
    expect(await screen.findByRole("heading", { name: "Existing session" })).toBeInTheDocument();
    await waitFor(() => expect(setAgentSourceDraftScopeMock).toHaveBeenCalledWith(session.id));

    await user.click(screen.getByRole("button", { name: "Attach file" }));
    expect(await screen.findByText(/Captured file snapshot: notes\.txt/)).toBeInTheDocument();
    const composer = screen.getByRole("textbox", { name: "Message the Agent" });
    await user.type(composer, "Use the file");
    await user.click(screen.getByRole("button", { name: "Send" }));

    await waitFor(() => expect(sendAgentMessageMock).toHaveBeenCalled());
    expect(screen.queryByText("source_draft_expired")).not.toBeInTheDocument();
  });

  it("binds attachment scope after the first turn so a follow-up can attach without navigating", async () => {
    const user = userEvent.setup();
    const sessionId = "22222222-2222-7222-8222-222222222222";
    getConnectionStatusMock.mockResolvedValue({
      state: "connected",
      providerId: "xai-subscription-oauth",
      model: "grok-3",
    });
    listAgentSessionsMock.mockResolvedValue([]);
    sendAgentMessageMock
      .mockImplementationOnce(
        (options: {
          sessionId: string | null;
          sourceDraftHandle: string | null;
          onEvent: (event: AgentStreamEvent) => void;
        }) => {
          expect(options.sessionId).toBeNull();
          options.onEvent({ kind: "started", session_id: sessionId, turn_id: "t1" });
          options.onEvent({
            kind: "terminal",
            turn: {
              id: "t1",
              ordinal: 1,
              userText: "Hello",
              agentText: "Hi",
              state: "completed",
              errorCode: null,
              sources: [],
            },
          });
          return Promise.resolve();
        },
      )
      .mockImplementationOnce(
        (options: {
          sessionId: string | null;
          sourceDraftHandle: string | null;
          onEvent: (event: AgentStreamEvent) => void;
        }) => {
          expect(options.sessionId).toBe(sessionId);
          expect(options.sourceDraftHandle).toBe("abcdabcd".repeat(4));
          options.onEvent({ kind: "started", session_id: sessionId, turn_id: "t2" });
          options.onEvent({
            kind: "terminal",
            turn: {
              id: "t2",
              ordinal: 2,
              userText: "With file",
              agentText: "Got it",
              state: "completed",
              errorCode: null,
              sources: [
                {
                  id: "55555555-5555-7555-8555-555555555555",
                  originKind: "local_text_file",
                  displayName: "follow.txt",
                  byteCount: 4,
                  contentSha256: "b".repeat(64),
                  memberCount: 1,
                  canonicalUrl: null,
                },
              ],
            },
          });
          return Promise.resolve();
        },
      );
    pickAgentTextSourceMock.mockResolvedValue({
      status: "selected",
      attachment: {
        draftHandle: "abcdabcd".repeat(4),
        displayName: "follow.txt",
        byteCount: 4,
        originKind: "local_text_file",
        memberCount: 1,
        canonicalUrl: null,
      },
    });

    render(<App />);
    const composer = await screen.findByRole("textbox", { name: "Message the Agent" });
    await user.type(composer, "Hello");
    await user.click(screen.getByRole("button", { name: "Send" }));

    await waitFor(() => expect(setAgentSourceDraftScopeMock).toHaveBeenCalledWith(sessionId));

    await user.click(screen.getByRole("button", { name: "Attach file" }));
    expect(await screen.findByText(/Captured file snapshot: follow\.txt/)).toBeInTheDocument();
    await user.type(screen.getByRole("textbox", { name: "Message the Agent" }), "With file");
    await user.click(screen.getByRole("button", { name: "Send" }));

    await waitFor(() => expect(sendAgentMessageMock).toHaveBeenCalledTimes(2));
    expect(sendAgentMessageMock.mock.calls[1]?.[0]).toEqual(
      expect.objectContaining({
        sessionId,
        sourceDraftHandle: "abcdabcd".repeat(4),
      }),
    );
  });

  it("does not let a stale startup detail overwrite New session", async () => {
    const user = userEvent.setup();
    const session: AgentSession = {
      id: "22222222-2222-7222-8222-222222222222",
      title: "Existing session",
      projectId: null,
      modelId: "grok-3",
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
          sources: [],
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
      modelId: "grok-3",
    };
    const second: AgentSession = {
      id: "33333333-3333-7333-8333-333333333333",
      title: "Second session",
      projectId: null,
      modelId: "grok-3",
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
          sources: [],
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
          sources: [],
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
      modelId: "grok-3",
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

  it("shows stale catalog models and the provider error after startup refresh failure", async () => {
    getConnectionStatusMock.mockResolvedValue({
      state: "connected",
      providerId: "xai-subscription-oauth",
      model: "grok-3",
    });
    getProviderModelCatalogMock.mockRejectedValue(new ProviderError("provider_unavailable"));
    getPersistedProviderModelCatalogMock.mockResolvedValue({
      providerId: "xai-subscription-oauth",
      models: [
        {
          id: "grok-3",
          displayName: "Grok 3",
          description: null,
          isProviderDefault: true,
        },
      ],
      freshness: "stale",
      retrievedAtUnixMs: 1,
      compatibilityRevision: "1.0.0",
    });
    getProviderModelSelectionMock.mockResolvedValue({
      providerId: "xai-subscription-oauth",
      selectedModelId: "grok-3",
      requiresSelection: false,
    });

    render(<App />);

    expect(await screen.findByRole("option", { name: "Grok 3" })).toBeInTheDocument();
    expect(screen.getByText("The provider is unavailable. Try again.")).toBeInTheDocument();
    expect(getPersistedProviderModelCatalogMock).toHaveBeenCalled();
  });

  it("blocks Send on a new session without a valid catalog model", async () => {
    const user = userEvent.setup();
    getConnectionStatusMock.mockResolvedValue({
      state: "connected",
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
      selectedModelId: null,
      requiresSelection: true,
    });

    render(<App />);
    const composer = await screen.findByRole("textbox", { name: "Message the Agent" });
    await user.type(composer, "Question");
    expect(screen.getByRole("button", { name: "Send" })).toBeDisabled();
    expect(
      screen.getByText("Choose a model before sending the first message."),
    ).toBeInTheDocument();
    fireEvent.keyDown(composer, { key: "Enter", shiftKey: false });
    expect(sendAgentMessageMock).not.toHaveBeenCalled();
  });

  it("reconciles a pending new-session model when the catalog changes", async () => {
    let emitCatalog: ((catalog: ProviderModelCatalog) => void) | undefined;
    listenProviderModelCatalogChangedMock.mockImplementation(
      (handler: (catalog: ProviderModelCatalog) => void) => {
        emitCatalog = handler;
        return Promise.resolve(vi.fn());
      },
    );
    getConnectionStatusMock.mockResolvedValue({
      state: "connected",
      providerId: "xai-subscription-oauth",
      model: "grok-3",
    });
    getProviderModelCatalogMock.mockResolvedValue({
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
      retrievedAtUnixMs: 1,
      compatibilityRevision: "1.0.0",
    });

    render(<App />);
    expect(await screen.findByRole("option", { name: "Grok 3" })).toBeInTheDocument();

    emitCatalog?.({
      providerId: "xai-subscription-oauth",
      models: [
        {
          id: "other-model",
          displayName: "Other",
          description: null,
          isProviderDefault: false,
        },
      ],
      freshness: "current",
      retrievedAtUnixMs: 2,
      compatibilityRevision: "1.0.0",
    });

    await waitFor(() => {
      expect(screen.queryByRole("option", { name: "Grok 3" })).not.toBeInTheDocument();
      expect(
        screen.getByText("Choose a model before sending the first message."),
      ).toBeInTheDocument();
      expect(screen.getByRole("button", { name: "Send" })).toBeDisabled();
    });
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
      modelId: "grok-3",
    };
    let resolveProjectChange: ((updated: AgentSession) => void) | undefined;
    setAgentSessionProjectMock.mockReturnValue(
      new Promise<AgentSession>((resolve) => {
        resolveProjectChange = resolve;
      }),
    );
    getConnectionStatusMock.mockResolvedValue({
      state: "connected",
      providerId: "xai-subscription-oauth",
      model: "grok-3",
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
      providerId: "xai-subscription-oauth",
      model: "grok-3",
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
            sources: [],
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
    expect(await screen.findByText("Add a Provider to get started.")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Open Settings" })).not.toBeInTheDocument();
  });

  it("queues an immediate cancel until the native Started event supplies the turn ID", async () => {
    const user = userEvent.setup();
    let streamOptions: { onEvent: (event: AgentStreamEvent) => void } | undefined;
    let resolveSend: (() => void) | undefined;
    getConnectionStatusMock.mockResolvedValue({
      state: "connected",
      providerId: "xai-subscription-oauth",
      model: "grok-3",
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
          sources: [],
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
