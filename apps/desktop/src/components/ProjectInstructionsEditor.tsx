import { useEffect, useState, type FormEvent } from "react";
import type { Project } from "../platform/projects";

type SavePhase = "idle" | "saving" | "failed";

interface ProjectInstructionsEditorProps {
  project: Project;
  onDirtyChange: (projectId: string, hasUnsavedChanges: boolean) => void;
  onSave: (projectId: string, instructions: string) => Promise<Project>;
}

interface ProjectInstructionsEditorSessionProps extends ProjectInstructionsEditorProps {
  initialInstructions: string;
}

function ProjectInstructionsEditorSession({
  initialInstructions,
  project,
  onDirtyChange,
  onSave,
}: ProjectInstructionsEditorSessionProps) {
  const [draft, setDraft] = useState(initialInstructions);
  const [savedInstructions, setSavedInstructions] = useState(initialInstructions);
  const [savePhase, setSavePhase] = useState<SavePhase>("idle");
  const hasUnsavedChanges = draft !== savedInstructions;

  useEffect(() => {
    onDirtyChange(project.id, hasUnsavedChanges);
  }, [hasUnsavedChanges, onDirtyChange, project.id]);

  let statusMessage = "Saved";
  let statusClassName = "instructions-save-state instructions-save-state-saved";

  if (savePhase === "saving") {
    statusMessage = "Saving…";
    statusClassName = "instructions-save-state instructions-save-state-saving";
  } else if (savePhase === "failed") {
    statusMessage = "Save failed. Your changes are still here.";
    statusClassName = "instructions-save-state instructions-save-state-failed";
  } else if (hasUnsavedChanges) {
    statusMessage = "Unsaved changes";
    statusClassName = "instructions-save-state instructions-save-state-unsaved";
  }

  function updateDraft(instructions: string) {
    setDraft(instructions);
    setSavePhase("idle");
  }

  async function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();

    if (!hasUnsavedChanges || savePhase === "saving") {
      return;
    }

    const instructions = draft;
    setSavePhase("saving");

    try {
      const savedProject = await onSave(project.id, instructions);
      setDraft(savedProject.instructions);
      setSavedInstructions(savedProject.instructions);
      setSavePhase("idle");
    } catch {
      setSavePhase("failed");
    }
  }

  return (
    <form className="project-instructions-editor" onSubmit={(event) => void handleSubmit(event)}>
      <div className="instructions-heading">
        <div>
          <p className="section-label">PROJECT GUIDANCE</p>
          <h3 id="project-instructions-title">Instructions</h3>
        </div>
        <span
          className={statusClassName}
          id="project-instructions-status"
          role={savePhase === "failed" ? "alert" : "status"}
        >
          {statusMessage}
        </span>
      </div>

      <p className="instructions-copy" id="project-instructions-description">
        Keep the durable guidance for this project in plain text.
      </p>
      <label className="sr-only" htmlFor="project-instructions">
        Project instructions
      </label>
      <textarea
        id="project-instructions"
        name="instructions"
        value={draft}
        disabled={savePhase === "saving"}
        aria-describedby="project-instructions-description project-instructions-status"
        onChange={(event) => updateDraft(event.currentTarget.value)}
      />
      <div className="instructions-actions">
        <button
          className="primary-action"
          type="submit"
          disabled={!hasUnsavedChanges || savePhase === "saving"}
        >
          {savePhase === "saving" ? "Saving…" : "Save instructions"}
        </button>
      </div>
    </form>
  );
}

export function ProjectInstructionsEditor({
  project,
  onDirtyChange,
  onSave,
}: ProjectInstructionsEditorProps) {
  return (
    <ProjectInstructionsEditorSession
      key={`${project.id}\u0000${project.instructions}`}
      initialInstructions={project.instructions}
      project={project}
      onDirtyChange={onDirtyChange}
      onSave={onSave}
    />
  );
}
