const tuleWordmark = [
  "▀▀▀▀█▀▀▀ ██    ██ ██      ██▀▀▀▀▀▀",
  "   ██    ██    ██ ██      ██▄▄▄▄▄ ",
  "   ██    ██    ██ ██      ██      ",
  "   ██    ██▄▄▄▄▄█ ██▄▄▄▄▄ ██▄▄▄▄▄▄",
].join("\n");

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
  onNewSession,
  onSelectSession,
  onSelectProject,
  onManageProjects,
}: WorkspaceSidebarProps) {
  const projectless = sessions.filter((session) => session.projectId === null);

  return (
    <aside className="workspace-sidebar" aria-label="Workspace">
      <div className="wordmark" role="img" aria-label="TULE">
        <pre className="wordmark-art" aria-hidden="true">
          {tuleWordmark}
        </pre>
      </div>

      <button className="sidebar-action" type="button" onClick={onNewSession}>
        New session
      </button>

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
        <h2 className="sidebar-heading">Projectless recents</h2>
        <ul className="sidebar-list">
          {projectless.length === 0 ? (
            <li className="sidebar-empty">No projectless sessions yet</li>
          ) : (
            projectless.map((session) => (
              <li key={session.id}>
                <button
                  className={`sidebar-row${activeSessionId === session.id ? " is-selected" : ""}`}
                  type="button"
                  onClick={() => onSelectSession(session.id)}
                >
                  <span className="sidebar-row-label">{session.title}</span>
                </button>
              </li>
            ))
          )}
        </ul>
      </div>

      <button className="sidebar-action secondary" type="button" onClick={onManageProjects}>
        Manage projects
      </button>
    </aside>
  );
}
