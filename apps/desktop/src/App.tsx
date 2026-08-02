import { getCurrentWindow } from "@tauri-apps/api/window";
import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import "./App.css";
import { CreateProjectForm } from "./components/CreateProjectForm";
import { ProjectInstructionsEditor } from "./components/ProjectInstructionsEditor";
import { ProjectList, type ProjectListState } from "./components/ProjectList";
import { getApplicationInfo, type ApplicationInfo } from "./platform/application";
import {
  createProject,
  getProjectErrorCode,
  listProjects,
  openProject,
  type Project,
  updateProjectInstructions,
} from "./platform/projects";
import { applyThemePreference, loadThemePreference, type ThemePreference } from "./theme";

type ConnectionState = "checking" | "connected" | "unavailable";
type ProjectOperation =
  | { kind: "idle" }
  | { kind: "creating" }
  | { kind: "opening"; projectId: string }
  | { kind: "saving-instructions"; projectId: string };

const tuleWordmark = [
  "▀▀▀▀█▀▀▀ ██    ██ ██      ██▀▀▀▀▀▀",
  "   ██    ██    ██ ██      ██▄▄▄▄▄ ",
  "   ██    ██    ██ ██      ██      ",
  "   ██    ██▄▄▄▄▄█ ██▄▄▄▄▄ ██▄▄▄▄▄▄",
].join("\n");

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

function connectionLabel(connection: ConnectionState): string {
  switch (connection) {
    case "checking":
      return "Desktop starting";
    case "connected":
      return "Desktop ready";
    case "unavailable":
      return "Desktop unavailable";
  }
}

function App() {
  const [applicationInfo, setApplicationInfo] = useState<ApplicationInfo | null>(null);
  const [connection, setConnection] = useState<ConnectionState>("checking");
  const [theme, setTheme] = useState<ThemePreference>(loadThemePreference);
  const [projects, setProjects] = useState<Project[]>([]);
  const [projectLoadState, setProjectLoadState] = useState<ProjectListState>("loading");
  const [selectedProject, setSelectedProject] = useState<Project | null>(null);
  const [projectOperation, setProjectOperation] = useState<ProjectOperation>({ kind: "idle" });
  const [dirtyProjectInstructionsId, setDirtyProjectInstructionsId] = useState<string | null>(null);
  const dirtyProjectInstructionsIdRef = useRef<string | null>(null);
  const [projectName, setProjectName] = useState("");
  const [projectNameError, setProjectNameError] = useState<string | null>(null);
  const [projectError, setProjectError] = useState<string | null>(null);

  useEffect(() => {
    let active = true;

    getApplicationInfo()
      .then((info) => {
        if (active) {
          setApplicationInfo(info);
          setConnection("connected");
        }
      })
      .catch(() => {
        if (active) {
          setConnection("unavailable");
        }
      });

    return () => {
      active = false;
    };
  }, []);

  useEffect(() => {
    let active = true;

    listProjects()
      .then((availableProjects) => {
        if (active) {
          setProjects(availableProjects);
          setProjectLoadState("ready");
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

  function updateThemePreference(nextTheme: ThemePreference) {
    setTheme(nextTheme);
  }

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

  const openingProjectId = projectOperation.kind === "opening" ? projectOperation.projectId : null;
  const projectActionsDisabled = projectOperation.kind !== "idle" || projectLoadState !== "ready";

  return (
    <main className="app-shell">
      <header className="topbar">
        <div className="wordmark" role="img" aria-label="TULE">
          <pre className="wordmark-art" aria-hidden="true">
            {tuleWordmark}
          </pre>
        </div>
        <div className="appearance-control">
          <label htmlFor="appearance-preference">Appearance</label>
          <select
            id="appearance-preference"
            value={theme}
            onChange={(event) =>
              updateThemePreference(event.currentTarget.value as ThemePreference)
            }
          >
            <option value="system">System</option>
            <option value="light">Light</option>
            <option value="dark">Dark</option>
          </select>
        </div>
      </header>

      <section className="workspace" aria-labelledby="page-title">
        <div className="workspace-heading">
          <h1 id="page-title">Projects</h1>
          <span className={`status-pill status-${connection}`} aria-live="polite">
            <span className="status-dot" aria-hidden="true" />
            {connectionLabel(connection)}
          </span>
        </div>

        {projectError === null ? null : (
          <div className="project-error" role="alert">
            <strong>Project action unavailable</strong>
            <span>{projectError}</span>
          </div>
        )}

        <div className="project-workspace">
          <ProjectList
            disabled={projectOperation.kind !== "idle"}
            loadState={projectLoadState}
            openingProjectId={openingProjectId}
            projects={projects}
            selectedProjectId={selectedProject?.id ?? null}
            onOpen={(projectId) => void handleOpenProject(projectId)}
          />

          <div className="project-side-panel">
            <section className="selected-project" aria-labelledby="selected-project-title">
              <p className="panel-label">Current project</p>
              <h2 id="selected-project-title">
                {selectedProject?.displayName ?? "No project selected"}
              </h2>
              {selectedProject === null ? (
                <p className="panel-copy">Select a project to view its instructions.</p>
              ) : (
                <ProjectInstructionsEditor
                  project={selectedProject}
                  onDirtyChange={handleProjectInstructionsDirtyChange}
                  onSave={handleUpdateProjectInstructions}
                />
              )}
            </section>

            <CreateProjectForm
              displayName={projectName}
              disabled={projectActionsDisabled}
              isCreating={projectOperation.kind === "creating"}
              validationMessage={projectNameError}
              onDisplayNameChange={updateProjectName}
              onSubmit={() => void handleCreateProject()}
            />
          </div>
        </div>
      </section>

      {applicationInfo === null ? null : (
        <footer>
          <span>Version {applicationInfo.version}</span>
        </footer>
      )}
    </main>
  );
}

export default App;
