import { getCurrentWindow } from "@tauri-apps/api/window";
import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import "./App.css";
import { AgentWorkspace } from "./components/AgentWorkspace";
import { ProjectManager } from "./components/ProjectManager";
import { SettingsSheet } from "./components/SettingsSheet";
import { WorkspaceSidebar } from "./components/WorkspaceSidebar";
import type { ProjectListState } from "./components/ProjectList";
import {
  cancelAgentTurn,
  getAgentErrorCode,
  getAgentSession,
  getSafeAgentErrorMessage,
  listAgentSessions,
  sendAgentMessage,
  setAgentSessionProject,
  type AgentSession,
  type AgentTurn,
} from "./platform/agents";
import { getApplicationInfo, type ApplicationInfo } from "./platform/application";
import {
  createProject,
  getProjectErrorCode,
  listProjects,
  openProject,
  type Project,
  updateProjectInstructions,
} from "./platform/projects";
import {
  cancelChatgptConnect,
  connectChatgpt,
  disconnectChatgpt,
  getConnectionStatus,
  type ConnectionState,
} from "./platform/provider";
import { applyThemePreference, loadThemePreference, type ThemePreference } from "./theme";

type MainView = "agent" | "projects";
type ProjectOperation =
  | { kind: "idle" }
  | { kind: "creating" }
  | { kind: "opening"; projectId: string }
  | { kind: "saving-instructions"; projectId: string };

const genericProjectErrorMessage = "Project storage is unavailable. Try again.";
const startupProjectErrorMessage = "Project storage is unavailable. Restart TULE to try again.";
const closeWithUnsavedInstructionsMessage = "Discard unsaved project instructions and close TULE?";

function getSafeProjectErrorMessage(error: unknown): string {
  switch (getProjectErrorCode(error)) {
    case "invalid_project_name":
      return "Enter a valid project name.";
    case "invalid_project_id":
      return "The selected project could not be opened.";
    case "project_not_found":
      return "That project is no longer available.";
    case "project_storage_unavailable":
      return genericProjectErrorMessage;
  }
}

function mergeProject(projects: readonly Project[], project: Project): Project[] {
  const existingIndex = projects.findIndex((candidate) => candidate.id === project.id);

  if (existingIndex === -1) {
    return [...projects, project];
  }

  return projects.map((candidate, index) => (index === existingIndex ? project : candidate));
}

function App() {
  const [applicationInfo, setApplicationInfo] = useState<ApplicationInfo | null>(null);
  const [theme, setTheme] = useState<ThemePreference>(loadThemePreference);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [connectionState, setConnectionState] = useState<ConnectionState>("disconnected");
  const [connectionBusy, setConnectionBusy] = useState(false);
  const [connectCancelRequested, setConnectCancelRequested] = useState(false);
  const [connectionStatusMessage, setConnectionStatusMessage] = useState<string | null>(null);
  const [connectionErrorMessage, setConnectionErrorMessage] = useState<string | null>(null);
  const [mainView, setMainView] = useState<MainView>("agent");
  const [sessions, setSessions] = useState<AgentSession[]>([]);
  const [activeSessionId, setActiveSessionId] = useState<string | null>(null);
  const [pendingProjectId, setPendingProjectId] = useState<string | null>(null);
  const [turns, setTurns] = useState<AgentTurn[]>([]);
  const [draft, setDraft] = useState("");
  const [sending, setSending] = useState(false);
  const sendingRef = useRef(false);
  const [activeTurnId, setActiveTurnId] = useState<string | null>(null);
  const [cancelRequested, setCancelRequested] = useState(false);
  const nativeActiveTurnIdRef = useRef<string | null>(null);
  const cancelRequestedRef = useRef(false);
  const cancelDispatchedRef = useRef(false);
  const [sessionLoadPending, setSessionLoadPending] = useState(false);
  const sessionLoadPendingRef = useRef(false);
  const sessionRequestGenerationRef = useRef(0);
  const [sessionProjectChangePending, setSessionProjectChangePending] = useState(false);
  const sessionProjectChangePendingRef = useRef(false);
  const [agentError, setAgentError] = useState<string | null>(null);
  const [projects, setProjects] = useState<Project[]>([]);
  const [projectLoadState, setProjectLoadState] = useState<ProjectListState>("loading");
  const [selectedProject, setSelectedProject] = useState<Project | null>(null);
  const [projectOperation, setProjectOperation] = useState<ProjectOperation>({ kind: "idle" });
  const [dirtyProjectInstructionsId, setDirtyProjectInstructionsId] = useState<string | null>(null);
  const dirtyProjectInstructionsIdRef = useRef<string | null>(null);
  const [projectName, setProjectName] = useState("");
  const [projectNameError, setProjectNameError] = useState<string | null>(null);
  const [projectError, setProjectError] = useState<string | null>(null);
  const settingsButtonRef = useRef<HTMLButtonElement>(null);

  const activeSession = sessions.find((session) => session.id === activeSessionId) ?? null;
  const contextProjectId = activeSession?.projectId ?? pendingProjectId;
  const sessionTitle = activeSession?.title ?? "New session";
  const modelLabel = "GPT-5.5";
  const connected = connectionState === "connected";

  useEffect(() => {
    let active = true;

    getApplicationInfo()
      .then((info) => {
        if (active) {
          setApplicationInfo(info);
        }
      })
      .catch(() => undefined);

    return () => {
      active = false;
    };
  }, []);

  useEffect(() => {
    let active = true;

    getConnectionStatus()
      .then((status) => {
        if (active) {
          setConnectionState(status.state);
        }
      })
      .catch(() => {
        if (active) {
          setConnectionState("unavailable_in_this_build");
        }
      });

    return () => {
      active = false;
    };
  }, []);

  useEffect(() => {
    let active = true;
    const requestGeneration = ++sessionRequestGenerationRef.current;

    Promise.all([listProjects(), listAgentSessions()])
      .then(async ([availableProjects, availableSessions]) => {
        if (!active) {
          return;
        }

        setProjects(availableProjects);
        setSessions(availableSessions);
        setProjectLoadState("ready");

        if (
          availableSessions.length > 0 &&
          sessionRequestGenerationRef.current === requestGeneration
        ) {
          const newest = availableSessions[0];
          if (newest === undefined) {
            return;
          }
          setActiveSessionId(newest.id);
          setPendingProjectId(newest.projectId);
          sessionLoadPendingRef.current = true;
          setSessionLoadPending(true);
          try {
            const detail = await getAgentSession(newest.id);
            if (active && sessionRequestGenerationRef.current === requestGeneration) {
              setTurns(detail.turns);
            }
          } catch (error: unknown) {
            if (active && sessionRequestGenerationRef.current === requestGeneration) {
              setAgentError(getSafeAgentErrorMessage(error));
            }
          } finally {
            if (active && sessionRequestGenerationRef.current === requestGeneration) {
              sessionLoadPendingRef.current = false;
              setSessionLoadPending(false);
            }
          }
        }
      })
      .catch(() => {
        if (active) {
          setProjectLoadState("failed");
          setProjectError(startupProjectErrorMessage);
        }
      });

    return () => {
      active = false;
    };
  }, []);

  useEffect(() => {
    applyThemePreference(theme);
  }, [theme]);

  useLayoutEffect(() => {
    dirtyProjectInstructionsIdRef.current = dirtyProjectInstructionsId;
  }, [dirtyProjectInstructionsId]);

  const handleOpenSettings = useCallback(() => {
    setSettingsOpen(true);
  }, []);

  const handleCloseSettings = useCallback(() => {
    setSettingsOpen(false);
  }, []);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;

    void getCurrentWindow()
      .onCloseRequested((event) => {
        if (
          dirtyProjectInstructionsIdRef.current !== null &&
          !window.confirm(closeWithUnsavedInstructionsMessage)
        ) {
          event.preventDefault();
        }
      })
      .then((removeListener) => {
        if (disposed) {
          removeListener();
        } else {
          unlisten = removeListener;
        }
      })
      .catch(() => undefined);

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    if (dirtyProjectInstructionsId === null) {
      return;
    }

    function preventUnloadWithUnsavedProjectInstructions(event: BeforeUnloadEvent) {
      event.preventDefault();
      event.returnValue = "Unsaved project instructions";
    }

    window.addEventListener("beforeunload", preventUnloadWithUnsavedProjectInstructions);

    return () => {
      window.removeEventListener("beforeunload", preventUnloadWithUnsavedProjectInstructions);
    };
  }, [dirtyProjectInstructionsId]);

  function updateProjectName(displayName: string) {
    setProjectName(displayName);
    setProjectNameError(null);
  }

  const handleProjectInstructionsDirtyChange = useCallback(
    (projectId: string, hasUnsavedChanges: boolean) => {
      setDirtyProjectInstructionsId((currentProjectId) => {
        if (hasUnsavedChanges) {
          return projectId;
        }

        return currentProjectId === projectId ? null : currentProjectId;
      });
    },
    [],
  );

  function canDiscardUnsavedProjectInstructions(): boolean {
    if (dirtyProjectInstructionsId === null) {
      return true;
    }

    if (!window.confirm("Discard unsaved project instructions and continue?")) {
      return false;
    }

    dirtyProjectInstructionsIdRef.current = null;
    setDirtyProjectInstructionsId(null);
    return true;
  }

  function invalidateSessionDetailRequest() {
    sessionRequestGenerationRef.current += 1;
    sessionLoadPendingRef.current = false;
    setSessionLoadPending(false);
  }

  function canNavigateAwayFromProjectManager(): boolean {
    return (
      !sendingRef.current &&
      projectOperation.kind === "idle" &&
      !sessionProjectChangePendingRef.current &&
      canDiscardUnsavedProjectInstructions()
    );
  }

  function handleNewSession() {
    if (!canNavigateAwayFromProjectManager()) {
      return;
    }

    invalidateSessionDetailRequest();
    setMainView("agent");
    setActiveSessionId(null);
    setPendingProjectId(null);
    setTurns([]);
    setDraft("");
    setAgentError(null);
  }

  async function handleSelectSession(sessionId: string) {
    if (!canNavigateAwayFromProjectManager()) {
      return;
    }

    const requestGeneration = ++sessionRequestGenerationRef.current;
    sessionLoadPendingRef.current = true;
    setSessionLoadPending(true);
    const summary = sessions.find((session) => session.id === sessionId);
    setMainView("agent");
    setActiveSessionId(sessionId);
    setPendingProjectId(summary?.projectId ?? null);
    setTurns([]);
    setAgentError(null);
    try {
      const detail = await getAgentSession(sessionId);
      if (sessionRequestGenerationRef.current !== requestGeneration) {
        return;
      }
      setActiveSessionId(detail.session.id);
      setPendingProjectId(detail.session.projectId);
      setTurns(detail.turns);
      setSessions((current) => {
        const exists = current.some((session) => session.id === detail.session.id);
        return exists
          ? current.map((session) => (session.id === detail.session.id ? detail.session : session))
          : [detail.session, ...current];
      });
    } catch (error: unknown) {
      if (sessionRequestGenerationRef.current === requestGeneration) {
        setAgentError(getSafeAgentErrorMessage(error));
      }
    } finally {
      if (sessionRequestGenerationRef.current === requestGeneration) {
        sessionLoadPendingRef.current = false;
        setSessionLoadPending(false);
      }
    }
  }

  function handleSelectProject(projectId: string) {
    if (!canNavigateAwayFromProjectManager()) {
      return;
    }

    invalidateSessionDetailRequest();
    setMainView("agent");
    setActiveSessionId(null);
    setPendingProjectId(projectId);
    setTurns([]);
    setDraft("");
    setAgentError(null);
  }

  function handleBackToAgents() {
    if (!canNavigateAwayFromProjectManager()) {
      return;
    }

    setMainView("agent");
  }

  function clearAgentCancellation() {
    nativeActiveTurnIdRef.current = null;
    cancelRequestedRef.current = false;
    cancelDispatchedRef.current = false;
    setCancelRequested(false);
  }

  function dispatchAgentCancellation(turnId: string) {
    if (cancelDispatchedRef.current) {
      return;
    }

    cancelDispatchedRef.current = true;
    void cancelAgentTurn(turnId).catch((error: unknown) => {
      cancelDispatchedRef.current = false;
      cancelRequestedRef.current = false;
      setCancelRequested(false);
      setAgentError(getSafeAgentErrorMessage(error));
    });
  }

  async function handleSend() {
    if (
      sendingRef.current ||
      sessionLoadPendingRef.current ||
      sessionProjectChangePendingRef.current ||
      draft.trim().length === 0
    ) {
      return;
    }

    const userText = draft;
    sendingRef.current = true;
    setSending(true);
    clearAgentCancellation();
    setAgentError(null);
    setDraft("");

    const optimisticTurn: AgentTurn = {
      id: `local-${Date.now()}`,
      ordinal: turns.length + 1,
      userText,
      agentText: "",
      state: "pending",
      errorCode: null,
    };
    setTurns((current) => [...current, optimisticTurn]);
    setActiveTurnId(optimisticTurn.id);

    try {
      await sendAgentMessage({
        sessionId: activeSessionId,
        userText,
        projectId: contextProjectId,
        onEvent: (event) => {
          if (event.kind === "started") {
            nativeActiveTurnIdRef.current = event.turn_id;
            setActiveSessionId(event.session_id);
            setActiveTurnId(event.turn_id);
            setTurns((current) =>
              current.map((turn) =>
                turn.id === optimisticTurn.id
                  ? { ...turn, id: event.turn_id, state: "streaming" }
                  : turn,
              ),
            );
            void listAgentSessions()
              .then(setSessions)
              .catch(() => undefined);
            if (cancelRequestedRef.current) {
              dispatchAgentCancellation(event.turn_id);
            }
          } else if (event.kind === "delta") {
            setTurns((current) =>
              current.map((turn) =>
                turn.id === event.turn_id
                  ? { ...turn, agentText: `${turn.agentText}${event.text}`, state: "streaming" }
                  : turn,
              ),
            );
          } else if (event.kind === "terminal") {
            setTurns((current) =>
              current.map((turn) => (turn.id === event.turn.id ? event.turn : turn)),
            );
            if (event.turn.errorCode === "authentication_required") {
              setConnectionState("reconnect_required");
            }
            clearAgentCancellation();
            setActiveTurnId(null);
          }
        },
      });
      const sessionsNow = await listAgentSessions();
      setSessions(sessionsNow);
    } catch (error: unknown) {
      setAgentError(getSafeAgentErrorMessage(error));
      setTurns((current) => current.filter((turn) => turn.id !== optimisticTurn.id));
      setDraft(userText);
      clearAgentCancellation();
      setActiveTurnId(null);
    } finally {
      clearAgentCancellation();
      sendingRef.current = false;
      setSending(false);
    }
  }

  function handleCancel() {
    if (!sending || cancelRequestedRef.current) {
      return;
    }

    cancelRequestedRef.current = true;
    setCancelRequested(true);
    const nativeTurnId = nativeActiveTurnIdRef.current;
    if (nativeTurnId !== null) {
      dispatchAgentCancellation(nativeTurnId);
    }
  }

  async function handleConnect() {
    setConnectionBusy(true);
    setConnectCancelRequested(false);
    setConnectionStatusMessage(null);
    setConnectionErrorMessage(null);
    setConnectionState("connecting");
    try {
      const status = await connectChatgpt();
      setConnectionState(status.state);
      setConnectionStatusMessage(null);
    } catch (error: unknown) {
      const errorCode = getAgentErrorCode(error);
      if (errorCode === "cancelled") {
        setConnectionStatusMessage("Browser connection cancelled.");
      } else {
        const message = getSafeAgentErrorMessage(error);
        setConnectionErrorMessage(message);
        setAgentError(message);
      }
      const status = await getConnectionStatus().catch(() => null);
      if (status !== null) {
        setConnectionState(status.state);
      } else {
        setConnectionState("disconnected");
      }
    } finally {
      setConnectCancelRequested(false);
      setConnectionBusy(false);
    }
  }

  async function handleCancelConnect() {
    if (connectionState !== "connecting" || connectCancelRequested) {
      return;
    }

    setConnectCancelRequested(true);
    setConnectionStatusMessage("Cancelling browser connection…");
    setConnectionErrorMessage(null);
    try {
      await cancelChatgptConnect();
    } catch (error: unknown) {
      setConnectCancelRequested(false);
      setConnectionStatusMessage(null);
      setConnectionErrorMessage(getSafeAgentErrorMessage(error));
    }
  }

  async function handleDisconnect() {
    setConnectionBusy(true);
    setConnectionStatusMessage(null);
    setConnectionErrorMessage(null);
    try {
      const status = await disconnectChatgpt();
      setConnectionState(status.state);
      if (status.state === "disconnected") {
        setConnectionStatusMessage("Removed from this device");
      }
    } catch (error: unknown) {
      const message = getSafeAgentErrorMessage(error);
      setConnectionErrorMessage(message);
      setAgentError(message);
    } finally {
      setConnectionBusy(false);
    }
  }

  async function handleCreateProject() {
    if (projectOperation.kind !== "idle" || projectLoadState !== "ready") {
      return;
    }

    const displayName = projectName.trim();

    if (displayName.length === 0) {
      setProjectNameError("Enter a project name.");
      return;
    }

    if (!canDiscardUnsavedProjectInstructions()) {
      return;
    }

    setProjectError(null);
    setProjectNameError(null);
    setProjectOperation({ kind: "creating" });

    try {
      const project = await createProject(displayName);
      setProjects((currentProjects) => mergeProject(currentProjects, project));
      setSelectedProject(project);
      setDirtyProjectInstructionsId(null);
      setProjectLoadState("ready");
      setProjectName("");
    } catch (error: unknown) {
      if (getProjectErrorCode(error) === "invalid_project_name") {
        setProjectNameError("Enter a valid project name.");
      } else {
        setProjectError(getSafeProjectErrorMessage(error));
      }
    } finally {
      setProjectOperation({ kind: "idle" });
    }
  }

  async function handleOpenProject(projectId: string) {
    if (projectOperation.kind !== "idle") {
      return;
    }

    if (selectedProject?.id === projectId) {
      return;
    }

    if (!canDiscardUnsavedProjectInstructions()) {
      return;
    }

    setProjectError(null);
    setProjectOperation({ kind: "opening", projectId });

    try {
      const project = await openProject(projectId);
      setProjects((currentProjects) => mergeProject(currentProjects, project));
      setSelectedProject(project);
      setDirtyProjectInstructionsId(null);
    } catch (error: unknown) {
      setProjectError(getSafeProjectErrorMessage(error));
    } finally {
      setProjectOperation({ kind: "idle" });
    }
  }

  async function handleUpdateProjectInstructions(
    projectId: string,
    instructions: string,
  ): Promise<Project> {
    if (projectOperation.kind !== "idle") {
      throw new Error("A project operation is already in progress.");
    }

    setProjectOperation({ kind: "saving-instructions", projectId });

    try {
      const project = await updateProjectInstructions(projectId, instructions);

      setProjects((currentProjects) => mergeProject(currentProjects, project));
      setSelectedProject((currentProject) =>
        currentProject?.id === projectId ? project : currentProject,
      );
      setDirtyProjectInstructionsId((currentProjectId) =>
        currentProjectId === projectId ? null : currentProjectId,
      );

      return project;
    } finally {
      setProjectOperation({ kind: "idle" });
    }
  }

  function handleUseWithAgents(project: Project) {
    if (!canNavigateAwayFromProjectManager()) {
      return;
    }

    invalidateSessionDetailRequest();
    setMainView("agent");
    setActiveSessionId(null);
    setPendingProjectId(project.id);
    setTurns([]);
    setDraft("");
    setAgentError(null);
  }

  async function handleChangePersistedProject(projectId: string | null) {
    if (sessionLoadPendingRef.current || sessionProjectChangePendingRef.current) {
      return;
    }

    if (activeSessionId === null) {
      setPendingProjectId(projectId);
      return;
    }

    if (projectId === activeSession?.projectId) {
      return;
    }

    const project = projects.find((item) => item.id === projectId);
    const name = project?.displayName ?? "No project";
    if (!window.confirm(`Use ${name} for future messages in this session?`)) {
      return;
    }

    sessionProjectChangePendingRef.current = true;
    setSessionProjectChangePending(true);
    try {
      const session = await setAgentSessionProject(activeSessionId, projectId);
      setSessions((current) => current.map((item) => (item.id === session.id ? session : item)));
      setPendingProjectId(session.projectId);
    } catch (error: unknown) {
      setAgentError(getSafeAgentErrorMessage(error));
    } finally {
      sessionProjectChangePendingRef.current = false;
      setSessionProjectChangePending(false);
    }
  }

  const openingProjectId = projectOperation.kind === "opening" ? projectOperation.projectId : null;
  const projectActionsDisabled = projectOperation.kind !== "idle" || projectLoadState !== "ready";
  void applicationInfo;

  return (
    <div className={`app-shell${settingsOpen ? " settings-open" : ""}`}>
      <WorkspaceSidebar
        projects={projects}
        sessions={sessions}
        activeSessionId={activeSessionId}
        pendingProjectId={pendingProjectId}
        inert={settingsOpen}
        navigationDisabled={
          sending || projectOperation.kind !== "idle" || sessionProjectChangePending
        }
        onNewSession={handleNewSession}
        onSelectSession={(sessionId) => void handleSelectSession(sessionId)}
        onSelectProject={handleSelectProject}
        onManageProjects={() => {
          if (canNavigateAwayFromProjectManager()) {
            setMainView("projects");
          }
        }}
      />

      <main className="main-panel" inert={settingsOpen ? true : undefined}>
        {mainView === "projects" ? (
          <ProjectManager
            projects={projects}
            loadState={projectLoadState}
            selectedProject={selectedProject}
            projectName={projectName}
            projectNameError={projectNameError}
            projectError={projectError}
            openingProjectId={openingProjectId}
            actionsDisabled={projectActionsDisabled}
            isCreating={projectOperation.kind === "creating"}
            onProjectNameChange={updateProjectName}
            onCreate={() => void handleCreateProject()}
            onOpen={(projectId) => void handleOpenProject(projectId)}
            onDirtyChange={handleProjectInstructionsDirtyChange}
            onSaveInstructions={handleUpdateProjectInstructions}
            onUseWithAgents={handleUseWithAgents}
            onBackToAgents={handleBackToAgents}
          />
        ) : (
          <AgentWorkspace
            title={sessionTitle}
            projectId={contextProjectId}
            projects={projects}
            modelLabel={modelLabel}
            turns={turns}
            draft={draft}
            connected={connected}
            sending={sending}
            sendBlocked={sessionLoadPending || sessionProjectChangePending}
            cancelRequested={cancelRequested}
            activeTurnId={activeTurnId}
            errorMessage={agentError}
            onDraftChange={setDraft}
            onSend={() => void handleSend()}
            onCancel={handleCancel}
            onProjectChange={(projectId) => void handleChangePersistedProject(projectId)}
            onOpenSettings={handleOpenSettings}
            settingsButtonRef={settingsButtonRef}
          />
        )}
      </main>

      <SettingsSheet
        open={settingsOpen}
        connectionState={connectionState}
        model="gpt-5.5"
        theme={theme}
        busy={connectionBusy}
        turnBusy={sending}
        cancelRequested={connectCancelRequested}
        statusMessage={connectionStatusMessage}
        errorMessage={connectionErrorMessage}
        onClose={handleCloseSettings}
        onConnect={() => void handleConnect()}
        onCancelConnect={() => void handleCancelConnect()}
        onDisconnect={() => void handleDisconnect()}
        onThemeChange={setTheme}
        returnFocusRef={settingsButtonRef}
      />
    </div>
  );
}

export default App;
