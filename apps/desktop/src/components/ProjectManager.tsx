import { CreateProjectForm } from "./CreateProjectForm";
import { PlusIcon } from "./icons";
import { ProjectInstructionsEditor } from "./ProjectInstructionsEditor";
import { ProjectList, type ProjectListState } from "./ProjectList";
import { Tooltip } from "./Tooltip";
import type { Project } from "../platform/projects";

type DetailMode = "idle" | "create" | "edit";

interface ProjectManagerProps {
  projects: readonly Project[];
  loadState: ProjectListState;
  selectedProject: Project | null;
  projectName: string;
  projectNameError: string | null;
  projectError: string | null;
  openingProjectId: string | null;
  actionsDisabled: boolean;
  isCreating: boolean;
  compact: boolean;
  onProjectNameChange: (value: string) => void;
  onCreate: () => void;
  onOpen: (projectId: string) => void;
  onDirtyChange: (projectId: string, dirty: boolean) => void;
  onSaveInstructions: (projectId: string, instructions: string) => Promise<Project>;
  onUseWithAgents: (project: Project) => void;
  onClearSelection: () => void;
  onBeginCreate: () => void;
  creating: boolean;
}

export function ProjectManager({
  projects,
  loadState,
  selectedProject,
  projectName,
  projectNameError,
  projectError,
  openingProjectId,
  actionsDisabled,
  isCreating,
  compact,
  onProjectNameChange,
  onCreate,
  onOpen,
  onDirtyChange,
  onSaveInstructions,
  onUseWithAgents,
  onClearSelection,
  onBeginCreate,
  creating,
}: ProjectManagerProps) {
  const detailMode: DetailMode = creating ? "create" : selectedProject !== null ? "edit" : "idle";
  const showList = !compact || detailMode === "idle";
  const showDetail = !compact || detailMode !== "idle";

  return (
    <section className="project-manager" aria-labelledby="project-manager-title">
      <header className="manager-header">
        <div>
          <h1 id="project-manager-title">Projects</h1>
          <p className="manager-copy">Manage local Projects used as optional Agent context.</p>
        </div>
        <Tooltip label="New project">
          <button
            className="icon-button"
            type="button"
            aria-label="New project"
            disabled={actionsDisabled}
            onClick={onBeginCreate}
          >
            <PlusIcon />
          </button>
        </Tooltip>
      </header>

      {projectError === null ? null : (
        <div className="project-error" role="alert">
          <strong>Project action unavailable</strong>
          <span>{projectError}</span>
        </div>
      )}

      <div className={`project-manager-grid${compact ? " is-compact" : ""}`}>
        {showList ? (
          <ProjectList
            disabled={actionsDisabled}
            loadState={loadState}
            openingProjectId={openingProjectId}
            projects={projects}
            selectedProjectId={selectedProject?.id ?? null}
            onOpen={onOpen}
          />
        ) : null}

        {showDetail ? (
          <div className="project-manager-detail">
            {compact && detailMode !== "idle" ? (
              <button
                className="secondary-action back-to-projects"
                type="button"
                onClick={onClearSelection}
              >
                Back to projects
              </button>
            ) : null}

            {detailMode === "create" ? (
              <CreateProjectForm
                displayName={projectName}
                disabled={actionsDisabled}
                isCreating={isCreating}
                validationMessage={projectNameError}
                onDisplayNameChange={onProjectNameChange}
                onSubmit={onCreate}
              />
            ) : (
              <section className="selected-project" aria-labelledby="selected-project-title">
                <h2 id="selected-project-title">
                  {selectedProject?.displayName ?? "Nothing open yet"}
                </h2>
                <p className="panel-copy">
                  {selectedProject === null
                    ? "Choose a project to edit its saved instructions."
                    : "Only saved instructions are sent with Agent messages."}
                </p>
                {selectedProject === null ? null : (
                  <>
                    <ProjectInstructionsEditor
                      project={selectedProject}
                      onDirtyChange={onDirtyChange}
                      onSave={onSaveInstructions}
                    />
                    <button
                      className="primary-action"
                      type="button"
                      disabled={actionsDisabled}
                      onClick={() => onUseWithAgents(selectedProject)}
                    >
                      Use with Agents
                    </button>
                  </>
                )}
              </section>
            )}
          </div>
        ) : null}
      </div>
    </section>
  );
}
