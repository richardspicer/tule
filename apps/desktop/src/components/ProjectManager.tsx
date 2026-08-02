import { CreateProjectForm } from "./CreateProjectForm";
import { ProjectInstructionsEditor } from "./ProjectInstructionsEditor";
import { ProjectList, type ProjectListState } from "./ProjectList";
import type { Project } from "../platform/projects";

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
  onProjectNameChange: (value: string) => void;
  onCreate: () => void;
  onOpen: (projectId: string) => void;
  onDirtyChange: (projectId: string, dirty: boolean) => void;
  onSaveInstructions: (projectId: string, instructions: string) => Promise<Project>;
  onUseWithAgents: (project: Project) => void;
  onBackToAgents: () => void;
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
  onProjectNameChange,
  onCreate,
  onOpen,
  onDirtyChange,
  onSaveInstructions,
  onUseWithAgents,
  onBackToAgents,
}: ProjectManagerProps) {
  return (
    <section className="project-manager" aria-labelledby="project-manager-title">
      <header className="manager-header">
        <div>
          <h1 id="project-manager-title">Projects</h1>
          <p className="manager-copy">Manage local Projects used as optional Agent context.</p>
        </div>
        <button className="secondary-action" type="button" onClick={onBackToAgents}>
          Back to Agents
        </button>
      </header>

      {projectError === null ? null : (
        <div className="project-error" role="alert">
          <strong>Project action unavailable</strong>
          <span>{projectError}</span>
        </div>
      )}

      <div className="project-manager-grid">
        <ProjectList
          disabled={actionsDisabled && !isCreating}
          loadState={loadState}
          openingProjectId={openingProjectId}
          projects={projects}
          selectedProjectId={selectedProject?.id ?? null}
          onOpen={onOpen}
        />

        <div className="project-manager-side">
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
                  onClick={() => onUseWithAgents(selectedProject)}
                >
                  Use with Agents
                </button>
              </>
            )}
          </section>

          <CreateProjectForm
            displayName={projectName}
            disabled={actionsDisabled}
            isCreating={isCreating}
            validationMessage={projectNameError}
            onDisplayNameChange={onProjectNameChange}
            onSubmit={onCreate}
          />
        </div>
      </div>
    </section>
  );
}
