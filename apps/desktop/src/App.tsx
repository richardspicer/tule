import { getCurrentWindow } from "@tauri-apps/api/window";
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import "./App.css";
import { AgentWorkspace } from "./components/AgentWorkspace";
import { ApplicationChrome } from "./components/ApplicationChrome";
import { ProjectManager } from "./components/ProjectManager";
import { WorkspaceSidebar } from "./components/WorkspaceSidebar";
import type { ProjectListState } from "./components/ProjectList";
import {
  cancelAgentTurn,
  clearAgentTextSourceDraft,
  getAgentErrorCode,
  getAgentSession,
  getSafeAgentErrorMessage,
  listAgentSessions,
  pickAgentTextSource,
  sendAgentMessage,
  setAgentSessionProject,
  setAgentSourceDraftScope,
  type AgentSession,
  type AgentTurn,
  type PendingSourceAttachment,
} from "./platform/agents";

let nextOptimisticTurnId = 0;
import { getApplicationInfo, type ApplicationInfo } from "./platform/application";
import { createCommandDispatcher, type AppCommandId } from "./platform/commands";
import {
  createProject,
  getProjectErrorCode,
  listProjects,
  openProject,
  type Project,
  updateProjectInstructions,
} from "./platform/projects";
import {
  formatModelLabel,
  getConnectionStatus,
  getPersistedProviderModelCatalog,
  getProviderModelCatalog,
  getProviderModelSelection,
  listenProviderModelCatalogChanged,
  listenProviderModelSelectionChanged,
  type ConnectionState,
  type ProviderModelCatalog,
  type ProviderModelSelection,
} from "./platform/provider";
import {
  exitApplication,
  listenConnectionStatusChanged,
  openSettingsWindow,
  syncConnectionStatus,
} from "./platform/settings";
import {
  applyThemePreference,
  listenAppearanceChanged,
  loadThemePreference,
  type ThemePreference,
} from "./theme";

type MainView = "agent" | "projects";
type ProjectOperation =
  | { kind: "idle" }
  | { kind: "creating" }
  | { kind: "opening"; projectId: string }
  | { kind: "saving-instructions"; projectId: string };

const genericProjectErrorMessage = "Project storage is unavailable. Try again.";
const startupProjectErrorMessage = "Project storage is unavailable. Restart TULE to try again.";
const closeWithUnsavedInstructionsMessage = "Discard unsaved project instructions and close TULE?";
const projectsCompactQuery = "(max-width: 820px)";

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
  const [theme, setTheme] = useState<ThemePreference>("system");
  const [connectionState, setConnectionState] = useState<ConnectionState>("disconnected");
  const [mainView, setMainView] = useState<MainView>("agent");
  const [sessions, setSessions] = useState<AgentSession[]>([]);
  const [activeSessionId, setActiveSessionId] = useState<string | null>(null);
  const [pendingProjectId, setPendingProjectId] = useState<string | null>(null);
  const [turns, setTurns] = useState<AgentTurn[]>([]);
  const [draft, setDraft] = useState("");
  const [pendingAttachment, setPendingAttachment] = useState<PendingSourceAttachment | null>(null);
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
  const [creatingProject, setCreatingProject] = useState(false);
  const [projectOperation, setProjectOperation] = useState<ProjectOperation>({ kind: "idle" });
  const [dirtyProjectInstructionsId, setDirtyProjectInstructionsId] = useState<string | null>(null);
  const dirtyProjectInstructionsIdRef = useRef<string | null>(null);
  const [projectName, setProjectName] = useState("");
  const [projectNameError, setProjectNameError] = useState<string | null>(null);
  const [projectError, setProjectError] = useState<string | null>(null);
  const [projectsCompact, setProjectsCompact] = useState(
    () => window.matchMedia(projectsCompactQuery).matches,
  );
  const [catalog, setCatalog] = useState<ProviderModelCatalog | null>(null);
  const [selection, setSelection] = useState<ProviderModelSelection | null>(null);
  const [pendingModelId, setPendingModelId] = useState<string | null>(null);

  const activeSession = sessions.find((session) => session.id === activeSessionId) ?? null;
  const contextProjectId = activeSession?.projectId ?? pendingProjectId;
  const sessionTitle = activeSession?.title ?? "New session";
  const modelLocked = activeSession !== null;
  const catalogModels = catalog?.models ?? [];
  const catalogHasModel = (modelId: string | null | undefined): modelId is string =>
    typeof modelId === "string" && catalogModels.some((model) => model.id === modelId);
  const sessionModelId = modelLocked
    ? (activeSession?.modelId ?? null)
    : catalogHasModel(pendingModelId)
      ? pendingModelId
      : catalogHasModel(selection?.selectedModelId)
        ? selection.selectedModelId
        : null;
  const modelLabel =
    sessionModelId === null ? "Choose a model" : formatModelLabel(sessionModelId, catalogModels);
  const connected = connectionState === "connected";
  const hasValidNewSessionModel = catalogHasModel(sessionModelId);
  const newSessionNeedsModel = !modelLocked && !hasValidNewSessionModel;

  useEffect(() => {
    let active = true;

    void loadThemePreference().then((preference) => {
      if (active) {
        setTheme(preference);
        applyThemePreference(preference);
      }
    });

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
    applyThemePreference(theme);
  }, [theme]);

  useEffect(() => {
    let active = true;
    const cleanups: (() => void)[] = [];

    void (async () => {
      // Register listeners before catalog load so a stale emit cannot race past
      // initial subscription, then recover via the cache-only path on failure.
      const [unlistenAppearance, unlistenCatalog, unlistenSelection, unlistenConnection] =
        await Promise.all([
          listenAppearanceChanged((preference) => {
            setTheme(preference);
            applyThemePreference(preference);
          }),
          listenProviderModelCatalogChanged((next) => {
            setCatalog(next);
            setPendingModelId((current) => {
              if (current !== null && next.models.some((model) => model.id === current)) {
                return current;
              }
              return null;
            });
          }),
          listenProviderModelSelectionChanged((next) => {
            setSelection(next);
            setPendingModelId((current) => {
              if (next.selectedModelId === null && next.requiresSelection) {
                return null;
              }
              return current ?? next.selectedModelId;
            });
          }),
          listenConnectionStatusChanged((status) => {
            setConnectionState(status.state);
          }),
        ]);
      if (!active) {
        unlistenAppearance();
        unlistenCatalog();
        unlistenSelection();
        unlistenConnection();
        return;
      }
      cleanups.push(unlistenAppearance, unlistenCatalog, unlistenSelection, unlistenConnection);

      try {
        const [status, nextSelection] = await Promise.all([
          getConnectionStatus(),
          getProviderModelSelection(),
        ]);
        if (!active) {
          return;
        }
        setConnectionState(status.state);
        setSelection(nextSelection);
        try {
          const nextCatalog = await getProviderModelCatalog();
          if (!active) {
            return;
          }
          setCatalog(nextCatalog);
          const selected =
            nextSelection.selectedModelId !== null &&
            nextCatalog.models.some((model) => model.id === nextSelection.selectedModelId)
              ? nextSelection.selectedModelId
              : null;
          setPendingModelId((current) => {
            if (current !== null && nextCatalog.models.some((model) => model.id === current)) {
              return current;
            }
            return selected;
          });
        } catch (error: unknown) {
          if (!active) {
            return;
          }
          setAgentError(getSafeAgentErrorMessage(error));
          const stale = await getPersistedProviderModelCatalog().catch(() => null);
          if (!active || stale === null) {
            return;
          }
          setCatalog(stale);
          const selected =
            nextSelection.selectedModelId !== null &&
            stale.models.some((model) => model.id === nextSelection.selectedModelId)
              ? nextSelection.selectedModelId
              : null;
          setPendingModelId((current) => {
            if (current !== null && stale.models.some((model) => model.id === current)) {
              return current;
            }
            return selected;
          });
        }
      } catch {
        if (active) {
          setConnectionState("unavailable_in_this_build");
        }
      }
    })();

    return () => {
      active = false;
      for (const cleanup of cleanups) {
        cleanup();
      }
    };
  }, []);

  useEffect(() => {
    const media = window.matchMedia(projectsCompactQuery);
    function onChange() {
      setProjectsCompact(media.matches);
    }
    onChange();
    media.addEventListener("change", onChange);
    return () => media.removeEventListener("change", onChange);
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
          return;
        }

        // Prevent destroying only the main window while a hidden Settings
        // singleton would keep the process alive.
        event.preventDefault();
        void exitApplication();
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
    setPendingModelId(selection?.selectedModelId ?? null);
    setTurns([]);
    setDraft("");
    setAgentError(null);
    setPendingAttachment(null);
    void setAgentSourceDraftScope("").catch(() => undefined);
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
    setPendingAttachment(null);
    void setAgentSourceDraftScope(sessionId).catch(() => undefined);
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
    setPendingAttachment(null);
    void setAgentSourceDraftScope("").catch(() => undefined);
  }

  function handleManageProjects() {
    if (!canNavigateAwayFromProjectManager()) {
      return;
    }

    setCreatingProject(false);
    setMainView("projects");

    if (contextProjectId !== null) {
      void handleOpenProject(contextProjectId);
    }
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
      draft.trim().length === 0 ||
      (!modelLocked && !hasValidNewSessionModel)
    ) {
      return;
    }

    const userText = draft;
    const attachment = pendingAttachment;
    sendingRef.current = true;
    setSending(true);
    clearAgentCancellation();
    setAgentError(null);
    setDraft("");

    nextOptimisticTurnId += 1;
    const optimisticTurn: AgentTurn = {
      id: `local-${nextOptimisticTurnId}`,
      ordinal: turns.length + 1,
      userText,
      agentText: "",
      state: "pending",
      errorCode: null,
      sources:
        attachment === null
          ? []
          : [
              {
                id: attachment.draftHandle,
                originKind: attachment.originKind,
                displayName: attachment.displayName,
                byteCount: attachment.byteCount,
                contentSha256: "0".repeat(64),
              },
            ],
    };
    setTurns((current) => [...current, optimisticTurn]);
    setActiveTurnId(optimisticTurn.id);

    try {
      await sendAgentMessage({
        sessionId: activeSessionId,
        userText,
        projectId: contextProjectId,
        modelId: activeSessionId === null ? sessionModelId : null,
        sourceDraftHandle: attachment?.draftHandle ?? null,
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
              void syncConnectionStatus().catch(() => undefined);
            }
            clearAgentCancellation();
            setActiveTurnId(null);
          }
        },
      });
      const sessionsNow = await listAgentSessions();
      setSessions(sessionsNow);
      setPendingAttachment(null);
    } catch (error: unknown) {
      setAgentError(getSafeAgentErrorMessage(error));
      setTurns((current) => current.filter((turn) => turn.id !== optimisticTurn.id));
      setDraft(userText);
      const code = getAgentErrorCode(error);
      const keepPendingAttachment =
        code === "invalid_input" ||
        code === "context_limit" ||
        code === "source_unreadable" ||
        code === "source_unsupported" ||
        code === "source_too_large" ||
        code === "model_unavailable" ||
        code === "not_connected" ||
        code === "authentication_required" ||
        code === "session_busy" ||
        code === "agent_storage_unavailable" ||
        code === "credential_store_unavailable" ||
        code === "entitlement_unavailable";
      if (!keepPendingAttachment) {
        setPendingAttachment(null);
      }
      clearAgentCancellation();
      setActiveTurnId(null);
    } finally {
      clearAgentCancellation();
      sendingRef.current = false;
      setSending(false);
    }
  }

  async function handleAttach() {
    if (sendingRef.current) {
      return;
    }

    setAgentError(null);
    try {
      const result = await pickAgentTextSource();
      if (result.status === "cancelled") {
        return;
      }

      setPendingAttachment(result.attachment);
    } catch (error: unknown) {
      setAgentError(getSafeAgentErrorMessage(error));
    }
  }

  async function handleRemoveAttachment() {
    if (sendingRef.current) {
      return;
    }

    const current = pendingAttachment;
    setPendingAttachment(null);
    if (current !== null) {
      await clearAgentTextSourceDraft(current.draftHandle).catch(() => undefined);
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
      setCreatingProject(false);
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

    if (selectedProject?.id === projectId && !creatingProject) {
      return;
    }

    if (!canDiscardUnsavedProjectInstructions()) {
      return;
    }

    setCreatingProject(false);
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

  const dispatchCommand = useMemo(
    () =>
      createCommandDispatcher((command: AppCommandId) => {
        switch (command) {
          case "new-session":
            handleNewSession();
            return;
          case "manage-projects":
            handleManageProjects();
            return;
          case "exit":
            if (
              dirtyProjectInstructionsIdRef.current !== null &&
              !window.confirm(closeWithUnsavedInstructionsMessage)
            ) {
              return;
            }
            void exitApplication();
            return;
          case "open-settings":
            void openSettingsWindow();
            return;
          case "open-settings-providers":
            void openSettingsWindow("providers");
            return;
          default:
            return;
        }
      }),
    // Handlers close over latest state intentionally for command routing.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [
      dirtyProjectInstructionsId,
      projectOperation.kind,
      contextProjectId,
      projects,
      sessions,
      activeSessionId,
    ],
  );

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      if ((event.ctrlKey || event.metaKey) && event.key === ",") {
        event.preventDefault();
        void openSettingsWindow();
      }
    }

    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);

  const openingProjectId = projectOperation.kind === "opening" ? projectOperation.projectId : null;
  const projectActionsDisabled = projectOperation.kind !== "idle" || projectLoadState !== "ready";
  void applicationInfo;

  return (
    <div className="app-shell">
      <ApplicationChrome
        onCommand={(command) => {
          void dispatchCommand(command);
        }}
      />
      <div className="app-body">
        <WorkspaceSidebar
          projects={projects}
          sessions={sessions}
          activeSessionId={activeSessionId}
          pendingProjectId={pendingProjectId}
          navigationDisabled={
            sending || projectOperation.kind !== "idle" || sessionProjectChangePending
          }
          onNewSession={handleNewSession}
          onSelectSession={(sessionId) => void handleSelectSession(sessionId)}
          onSelectProject={handleSelectProject}
          onManageProjects={handleManageProjects}
        />

        <main className="main-panel">
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
              compact={projectsCompact}
              creating={creatingProject}
              onProjectNameChange={updateProjectName}
              onCreate={() => void handleCreateProject()}
              onOpen={(projectId) => void handleOpenProject(projectId)}
              onDirtyChange={handleProjectInstructionsDirtyChange}
              onSaveInstructions={handleUpdateProjectInstructions}
              onUseWithAgents={handleUseWithAgents}
              onClearSelection={() => {
                if (!canDiscardUnsavedProjectInstructions()) {
                  return;
                }
                setSelectedProject(null);
                setCreatingProject(false);
                setProjectName("");
                setProjectNameError(null);
              }}
              onBeginCreate={() => {
                if (!canDiscardUnsavedProjectInstructions()) {
                  return;
                }
                setSelectedProject(null);
                setCreatingProject(true);
                setProjectNameError(null);
              }}
            />
          ) : (
            <AgentWorkspace
              title={sessionTitle}
              projectId={contextProjectId}
              projects={projects}
              modelLabel={modelLabel}
              modelOptions={(catalog?.models ?? []).map((model) => ({
                id: model.id,
                displayName: model.displayName,
              }))}
              selectedModelId={sessionModelId}
              modelLocked={modelLocked}
              turns={turns}
              draft={draft}
              pendingAttachment={pendingAttachment}
              connected={connected}
              sending={sending}
              sendBlocked={
                sessionLoadPending || sessionProjectChangePending || newSessionNeedsModel
              }
              cancelRequested={cancelRequested}
              activeTurnId={activeTurnId}
              errorMessage={
                agentError ??
                (newSessionNeedsModel && connected
                  ? "Choose a model before sending the first message."
                  : null)
              }
              onDraftChange={setDraft}
              onSend={() => void handleSend()}
              onCancel={handleCancel}
              onAttach={() => void handleAttach()}
              onRemoveAttachment={() => void handleRemoveAttachment()}
              onProjectChange={(projectId) => void handleChangePersistedProject(projectId)}
              onModelChange={setPendingModelId}
              onOpenProvidersSettings={() => void openSettingsWindow("providers")}
            />
          )}
        </main>
      </div>
    </div>
  );
}

export default App;
