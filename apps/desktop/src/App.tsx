import { useEffect, useState } from "react";
import "./App.css";
import { CreateProjectForm } from "./components/CreateProjectForm";
import { ProjectList, type ProjectListState } from "./components/ProjectList";
import { getApplicationInfo, type ApplicationInfo } from "./platform/application";
import {
  createProject,
  getProjectErrorCode,
  listProjects,
  openProject,
  type Project,
} from "./platform/projects";
import {
  applyThemePreference,
  getNextThemePreference,
  loadThemePreference,
  type ThemePreference,
} from "./theme";

type ConnectionState = "checking" | "connected" | "unavailable";
type ProjectOperation =
  { kind: "idle" } | { kind: "creating" } | { kind: "opening"; projectId: string };

const genericProjectErrorMessage = "Project storage is unavailable. Try again.";
const startupProjectErrorMessage = "Project storage is unavailable. Restart Tule to try again.";

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
  const [connection, setConnection] = useState<ConnectionState>("checking");
  const [theme, setTheme] = useState<ThemePreference>(loadThemePreference);
  const [projects, setProjects] = useState<Project[]>([]);
  const [projectLoadState, setProjectLoadState] = useState<ProjectListState>("loading");
  const [selectedProject, setSelectedProject] = useState<Project | null>(null);
  const [projectOperation, setProjectOperation] = useState<ProjectOperation>({ kind: "idle" });
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

  function cycleTheme() {
    setTheme(getNextThemePreference(theme));
  }

  function updateProjectName(displayName: string) {
    setProjectName(displayName);
    setProjectNameError(null);
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

    setProjectError(null);
    setProjectNameError(null);
    setProjectOperation({ kind: "creating" });

    try {
      const project = await createProject(displayName);
      setProjects((currentProjects) => mergeProject(currentProjects, project));
      setSelectedProject(project);
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

    setProjectError(null);
    setProjectOperation({ kind: "opening", projectId });

    try {
      const project = await openProject(projectId);
      setProjects((currentProjects) => mergeProject(currentProjects, project));
      setSelectedProject(project);
    } catch (error: unknown) {
      setProjectError(getSafeProjectErrorMessage(error));
    } finally {
      setProjectOperation({ kind: "idle" });
    }
  }

  const openingProjectId = projectOperation.kind === "opening" ? projectOperation.projectId : null;
  const projectActionsDisabled = projectOperation.kind !== "idle" || projectLoadState !== "ready";

  return (
    <main className="app-shell">
      <nav className="topbar" aria-label="Application">
        <div className="wordmark">
          <span className="wordmark-mark" aria-hidden="true">
            t
          </span>
          <span>Tule</span>
        </div>
        <button className="theme-control" type="button" onClick={cycleTheme}>
          <span aria-hidden="true">
            {theme === "dark" ? "Moon" : theme === "light" ? "Sun" : "Auto"}
          </span>
          <span className="sr-only">Appearance: {theme}. Change appearance.</span>
        </button>
      </nav>

      <section className="hero" aria-labelledby="page-title">
        <p className="eyebrow">PROJECTS</p>
        <h1 id="page-title">Make room for the work that matters now.</h1>
        <p className="lede">
          Create a local project or open one already in motion. Tule keeps the boundary quiet and
          the next decision close.
        </p>
      </section>

      <section className="workspace-card" aria-labelledby="workspace-title">
        <div className="card-heading">
          <div>
            <p className="section-label">DESKTOP WORKSPACE</p>
            <h2 id="workspace-title">Your local workspace</h2>
          </div>
          <span className={`status-pill status-${connection}`} aria-live="polite">
            <span className="status-dot" aria-hidden="true" />
            {connection === "connected"
              ? "Core connected"
              : connection === "checking"
                ? "Checking core"
                : "Desktop required"}
          </span>
        </div>

        <p className="card-copy">
          Projects are owned by Tule and opened through a narrow native boundary.
        </p>

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
              <p className="section-label">SELECTED PROJECT</p>
              <h2 id="selected-project-title">
                {selectedProject?.displayName ?? "Nothing open yet"}
              </h2>
              <p className="panel-copy">
                {selectedProject === null
                  ? "Choose a project from the list when you are ready to continue."
                  : "This project is selected and ready for the next workflow."}
              </p>
              <span className="selection-state">
                {selectedProject === null ? "No project selected" : "Selected"}
              </span>
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

        <dl className="foundation-details">
          <div>
            <dt>Application</dt>
            <dd>{applicationInfo?.name ?? "Tule"}</dd>
          </div>
          <div>
            <dt>Build</dt>
            <dd>{applicationInfo?.version ?? "0.1.0"}</dd>
          </div>
          <div>
            <dt>Appearance</dt>
            <dd>{theme[0].toUpperCase() + theme.slice(1)}</dd>
          </div>
        </dl>
      </section>

      <footer>
        <span>Local first.</span>
        <span aria-hidden="true">&#183;</span>
        <span>Projects stay yours.</span>
      </footer>
    </main>
  );
}

export default App;
