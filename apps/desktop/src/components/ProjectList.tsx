import type { Project } from "../platform/projects";

export type ProjectListState = "loading" | "ready" | "failed";

interface ProjectListProps {
  disabled: boolean;
  loadState: ProjectListState;
  openingProjectId: string | null;
  projects: readonly Project[];
  selectedProjectId: string | null;
  onOpen: (projectId: string) => void;
}

export function ProjectList({
  disabled,
  loadState,
  openingProjectId,
  projects,
  selectedProjectId,
  onOpen,
}: ProjectListProps) {
  let content;

  if (loadState === "loading") {
    content = (
      <div className="project-list-message" role="status">
        <span className="loading-mark" aria-hidden="true" />
        <span>Loading projects…</span>
      </div>
    );
  } else if (loadState === "failed") {
    content = <p className="project-list-message">Projects are unavailable.</p>;
  } else if (projects.length === 0) {
    content = (
      <div className="project-list-message empty-projects">
        <strong>No projects yet</strong>
        <span>Create one to begin.</span>
      </div>
    );
  } else {
    content = (
      <ul className="project-list">
        {projects.map((project) => {
          const isOpening = project.id === openingProjectId;
          const isSelected = project.id === selectedProjectId;

          return (
            <li key={project.id}>
              <button
                className="project-list-item"
                type="button"
                disabled={disabled}
                aria-pressed={isSelected}
                aria-busy={isOpening ? true : undefined}
                onClick={() => onOpen(project.id)}
              >
                <span className="project-list-copy">
                  <strong>{project.displayName}</strong>
                  <span>{isSelected ? "Selected project" : "Local project"}</span>
                </span>
                <span className="project-list-action" aria-live="polite">
                  {isOpening ? "Opening…" : isSelected ? "Selected" : "Open"}
                </span>
              </button>
            </li>
          );
        })}
      </ul>
    );
  }

  return (
    <section className="project-list-panel" aria-labelledby="project-list-title">
      <div className="panel-heading">
        <div>
          <p className="section-label">LOCAL WORK</p>
          <h2 id="project-list-title">Projects</h2>
        </div>
        {loadState === "ready" ? (
          <span className="project-count">
            {projects.length} {projects.length === 1 ? "project" : "projects"}
          </span>
        ) : null}
      </div>
      {content}
    </section>
  );
}
