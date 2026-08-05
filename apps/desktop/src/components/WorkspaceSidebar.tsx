import { PlusIcon, ProjectsIcon } from "./icons";
import { Tooltip } from "./Tooltip";

export interface SidebarSession {
  id: string;
  title: string;
  projectId: string | null;
}

export interface SidebarProject {
  id: string;
  displayName: string;
}

interface WorkspaceSidebarProps {
  projects: readonly SidebarProject[];
  sessions: readonly SidebarSession[];
  activeSessionId: string | null;
  pendingProjectId: string | null;
  navigationDisabled: boolean;
  onNewSession: () => void;
  onSelectSession: (sessionId: string) => void;
  onSelectProject: (projectId: string) => void;
  onManageProjects: () => void;
}

export function WorkspaceSidebar({
  projects,
  sessions,
  activeSessionId,
  pendingProjectId,
  navigationDisabled,
  onNewSession,
  onSelectSession,
  onSelectProject,
  onManageProjects,
}: WorkspaceSidebarProps) {
  const projectless = sessions.filter((session) => session.projectId === null);

  return (
    <aside className="workspace-sidebar" aria-label="Workspace">
      <div className="sidebar-toolbar">
        <Tooltip label="New session" align="start">
          <button
            className="icon-button sidebar-icon"
            type="button"
            aria-label="New session"
            disabled={navigationDisabled}
            onClick={onNewSession}
          >
            <PlusIcon />
          </button>
        </Tooltip>
        <Tooltip label="Manage projects">
          <button
            className="icon-button sidebar-icon"
            type="button"
            aria-label="Manage projects"
            disabled={navigationDisabled}
            onClick={onManageProjects}
          >
            <ProjectsIcon />
          </button>
        </Tooltip>
      </div>

      <div className="sidebar-scroll">
        <div className="sidebar-section">
          <h2 className="sidebar-heading">Projects</h2>
          <ul className="sidebar-list">
            {projects.map((project) => {
              const related = sessions.filter((session) => session.projectId === project.id);
              const projectSelected = pendingProjectId === project.id && activeSessionId === null;

              return (
                <li key={project.id} className="sidebar-project-group">
                  <button
                    className={`sidebar-row${projectSelected ? " is-selected" : ""}`}
                    type="button"
                    disabled={navigationDisabled}
                    onClick={() => onSelectProject(project.id)}
                  >
                    <span className="sidebar-row-label">{project.displayName}</span>
                  </button>
                  {related.length === 0 ? null : (
                    <ul className="sidebar-sublist">
                      {related.map((session) => (
                        <li key={session.id}>
                          <button
                            className={`sidebar-row nested${activeSessionId === session.id ? " is-selected" : ""}`}
                            type="button"
                            disabled={navigationDisabled}
                            onClick={() => onSelectSession(session.id)}
                          >
                            <span className="sidebar-row-label">{session.title}</span>
                          </button>
                        </li>
                      ))}
                    </ul>
                  )}
                </li>
              );
            })}
          </ul>
        </div>

        <div className="sidebar-section">
          <h2 className="sidebar-heading">No project</h2>
          <ul className="sidebar-list">
            {projectless.length === 0 ? (
              <li className="sidebar-empty">No projectless sessions yet</li>
            ) : (
              projectless.map((session) => (
                <li key={session.id}>
                  <button
                    className={`sidebar-row${activeSessionId === session.id ? " is-selected" : ""}`}
                    type="button"
                    disabled={navigationDisabled}
                    onClick={() => onSelectSession(session.id)}
                  >
                    <span className="sidebar-row-label">{session.title}</span>
                  </button>
                </li>
              ))
            )}
          </ul>
        </div>
      </div>
    </aside>
  );
}
