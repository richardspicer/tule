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
  const [mainView, setMainView] = useState<MainView>("agent");
  const [sessions, setSessions] = useState<AgentSession[]>([]);
  const [activeSessionId, setActiveSessionId] = useState<string | null>(null);
  const [pendingProjectId, setPendingProjectId] = useState<string | null>(null);
  const [turns, setTurns] = useState<AgentTurn[]>([]);
  const [draft, setDraft] = useState("");
  const [sending, setSending] = useState(false);
  const [activeTurnId, setActiveTurnId] = useState<string | null>(null);
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
  const contextProject = projects.find((project) => project.id === contextProjectId) ?? null;
  const projectLabel = contextProject?.displayName ?? "No project";
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

    Promise.all([listProjects(), listAgentSessions()])
      .then(async ([availableProjects, availableSessions]) => {
        if (!active) {
          return;
        }

        setProjects(availableProjects);
        setSessions(availableSessions);
        setProjectLoadState("ready");

        if (availableSessions.length > 0) {
          const newest = availableSessions[0];
          if (newest === undefined) {
            return;
          }
          setActiveSessionId(newest.id);
          setPendingProjectId(newest.projectId);
          const detail = await getAgentSession(newest.id);
          if (active) {
            setTurns(detail.turns);
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
    return (
      dirtyProjectInstructionsId === null ||
      window.confirm("Discard unsaved project instructions and continue?")
    );
  }

  function handleNewSession() {
    setMainView("agent");
    setActiveSessionId(null);
    setPendingProjectId(null);
    setTurns([]);
    setDraft("");
    setAgentError(null);
  }

  async function handleSelectSession(sessionId: string) {
    setMainView("agent");
    setAgentError(null);
    try {
      const detail = await getAgentSession(sessionId);
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
      setAgentError(getSafeAgentErrorMessage(error));
    }
  }

  function handleSelectProject(projectId: string) {
    setMainView("agent");
    setActiveSessionId(null);
    setPendingProjectId(projectId);
    setTurns([]);
    setDraft("");
    setAgentError(null);
  }

  async function handleSend() {
    if (sending || draft.trim().length === 0) {
      return;
    }

    const userText = draft;
    setSending(true);
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
      setActiveTurnId(null);
    } finally {
      setSending(false);
    }
  }

  async function handleCancel() {
    if (activeTurnId === null) {
      return;
    }

    try {
      await cancelAgentTurn(activeTurnId);
    } catch (error: unknown) {
      setAgentError(getSafeAgentErrorMessage(error));
    }
  }

  async function handleConnect() {
    setConnectionBusy(true);
    setConnectionState("connecting");
    try {
      const status = await connectChatgpt();
      setConnectionState(status.state);
    } catch (error: unknown) {
      setAgentError(getSafeAgentErrorMessage(error));
      const status = await getConnectionStatus().catch(() => null);
      if (status !== null) {
        setConnectionState(status.state);
      } else {
        setConnectionState("disconnected");
      }
    } finally {
      setConnectionBusy(false);
    }
  }

  async function handleDisconnect() {
    setConnectionBusy(true);
    try {
      const status = await disconnectChatgpt();
      setConnectionState(status.state);
    } catch (error: unknown) {
      setAgentError(getSafeAgentErrorMessage(error));
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
    if (!canDiscardUnsavedProjectInstructions()) {
      return;
    }

    setMainView("agent");
    setActiveSessionId(null);
    setPendingProjectId(project.id);
    setTurns([]);
    setDraft("");
    setAgentError(null);
  }

  async function handleChangePersistedProject(projectId: string | null) {
    if (activeSessionId === null) {
      setPendingProjectId(projectId);
      return;
    }

    const project = projects.find((item) => item.id === projectId);
    const name = project?.displayName ?? "No project";
    if (projectId !== null && !window.confirm(`Use ${name} for future messages in this session?`)) {
      return;
    }

    try {
      const session = await setAgentSessionProject(activeSessionId, projectId);
      setSessions((current) => current.map((item) => (item.id === session.id ? session : item)));
      setPendingProjectId(session.projectId);
    } catch (error: unknown) {
      setAgentError(getSafeAgentErrorMessage(error));
    }
  }

  const openingProjectId = projectOperation.kind === "opening" ? projectOperation.projectId : null;
  const projectActionsDisabled = projectOperation.kind !== "idle" || projectLoadState !== "ready";
  void applicationInfo;
  void handleChangePersistedProject; // retained for persisted session Project association confirmations

  return (
    <div className={`app-shell${settingsOpen ? " settings-open" : ""}`}>
      <WorkspaceSidebar
        projects={projects}
        sessions={sessions}
        activeSessionId={activeSessionId}
        pendingProjectId={pendingProjectId}
        onNewSession={handleNewSession}
        onSelectSession={(sessionId) => void handleSelectSession(sessionId)}
        onSelectProject={handleSelectProject}
        onManageProjects={() => setMainView("projects")}
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
            onBackToAgents={() => setMainView("agent")}
          />
        ) : (
          <AgentWorkspace
            title={sessionTitle}
            projectLabel={projectLabel}
            modelLabel={modelLabel}
            turns={turns}
            draft={draft}
            connected={connected}
            sending={sending}
            activeTurnId={activeTurnId}
            errorMessage={agentError}
            onDraftChange={setDraft}
            onSend={() => void handleSend()}
            onCancel={() => void handleCancel()}
            onOpenSettings={() => setSettingsOpen(true)}
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
        onClose={() => setSettingsOpen(false)}
        onConnect={() => void handleConnect()}
        onDisconnect={() => void handleDisconnect()}
        onThemeChange={setTheme}
        returnFocusRef={settingsButtonRef}
      />
    </div>
  );
}

export default App;
