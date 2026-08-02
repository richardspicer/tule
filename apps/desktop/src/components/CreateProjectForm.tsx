import type { FormEvent } from "react";

interface CreateProjectFormProps {
  displayName: string;
  disabled: boolean;
  isCreating: boolean;
  validationMessage: string | null;
  onDisplayNameChange: (displayName: string) => void;
  onSubmit: () => void;
}

export function CreateProjectForm({
  displayName,
  disabled,
  isCreating,
  validationMessage,
  onDisplayNameChange,
  onSubmit,
}: CreateProjectFormProps) {
  function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    onSubmit();
  }

  return (
    <section className="create-project" aria-labelledby="create-project-title">
      <p className="section-label">NEW PROJECT</p>
      <h2 id="create-project-title">Create a project</h2>
      <p className="panel-copy">Give the work a clear name. TULE will keep it local.</p>

      <form onSubmit={handleSubmit} noValidate>
        <label htmlFor="project-display-name">Project name</label>
        <input
          id="project-display-name"
          name="displayName"
          type="text"
          value={displayName}
          disabled={disabled}
          aria-invalid={validationMessage === null ? undefined : true}
          aria-describedby={validationMessage === null ? undefined : "project-name-error"}
          onChange={(event) => onDisplayNameChange(event.currentTarget.value)}
        />
        {validationMessage === null ? null : (
          <p className="field-error" id="project-name-error" role="alert">
            {validationMessage}
          </p>
        )}
        <button className="primary-action" type="submit" disabled={disabled}>
          {isCreating ? "Creating…" : "Create project"}
        </button>
      </form>
    </section>
  );
}
