import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { Project } from "../platform/projects";
import { ProjectInstructionsEditor } from "./ProjectInstructionsEditor";

const firstProject: Project = {
  id: "project-1",
  displayName: "First project",
  instructions: "  Start here.\n保留 this line.  ",
};

describe("ProjectInstructionsEditor", () => {
  it("shows the selected project's exact persisted instructions as saved", () => {
    render(
      <ProjectInstructionsEditor project={firstProject} onDirtyChange={vi.fn()} onSave={vi.fn()} />,
    );

    expect(screen.getByLabelText("Project instructions")).toHaveValue(firstProject.instructions);
    expect(screen.getByRole("status")).toHaveTextContent("Saved");
    expect(screen.getByRole("button", { name: "Save instructions" })).toBeDisabled();
  });

  it("marks edited content as unsaved until the explicit save action", async () => {
    const user = userEvent.setup();
    const onDirtyChange = vi.fn();
    const onSave = vi.fn();
    render(
      <ProjectInstructionsEditor
        project={firstProject}
        onDirtyChange={onDirtyChange}
        onSave={onSave}
      />,
    );

    await user.type(screen.getByLabelText("Project instructions"), "\nNext step");

    expect(screen.getByRole("status")).toHaveTextContent("Unsaved changes");
    expect(screen.getByRole("button", { name: "Save instructions" })).toBeEnabled();
    expect(onDirtyChange).toHaveBeenLastCalledWith("project-1", true);
    expect(onSave).not.toHaveBeenCalled();
  });

  it("shows saving progress and adopts the returned persisted value after success", async () => {
    const user = userEvent.setup();
    let resolveSave!: (project: Project) => void;
    const pendingSave = new Promise<Project>((resolve) => {
      resolveSave = resolve;
    });
    const onSave = vi.fn().mockReturnValue(pendingSave);
    render(
      <ProjectInstructionsEditor project={firstProject} onDirtyChange={vi.fn()} onSave={onSave} />,
    );

    const editor = screen.getByLabelText("Project instructions");
    await user.clear(editor);
    await user.type(editor, "Exact replacement\n第二行  ");
    await user.click(screen.getByRole("button", { name: "Save instructions" }));

    expect(onSave).toHaveBeenCalledWith("project-1", "Exact replacement\n第二行  ");
    expect(screen.getByRole("status")).toHaveTextContent("Saving…");
    expect(editor).toBeDisabled();

    resolveSave({
      ...firstProject,
      instructions: "Exact replacement\n第二行  ",
    });

    await waitFor(() => expect(screen.getByRole("status")).toHaveTextContent("Saved"));
    expect(screen.getByLabelText("Project instructions")).toHaveValue(
      "Exact replacement\n第二行  ",
    );
    expect(screen.getByRole("button", { name: "Save instructions" })).toBeDisabled();
  });

  it("keeps the draft visible and reports a safe failure", async () => {
    const user = userEvent.setup();
    const onSave = vi.fn().mockRejectedValue(new Error("C:\\private\\projects.db leaked"));
    render(
      <ProjectInstructionsEditor project={firstProject} onDirtyChange={vi.fn()} onSave={onSave} />,
    );

    const editor = screen.getByLabelText("Project instructions");
    await user.clear(editor);
    await user.type(editor, "Do not lose this draft");
    await user.click(screen.getByRole("button", { name: "Save instructions" }));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Save failed. Your changes are still here.",
    );
    expect(editor).toHaveValue("Do not lose this draft");
    expect(editor).toBeEnabled();
    expect(screen.getByRole("button", { name: "Save instructions" })).toBeEnabled();
    expect(screen.queryByText(/projects\.db|leaked/i)).not.toBeInTheDocument();
  });

  it("loads the newly selected project's persisted instructions after selection changes", async () => {
    const user = userEvent.setup();
    const onDirtyChange = vi.fn();
    const onSave = vi.fn();
    const { rerender } = render(
      <ProjectInstructionsEditor
        project={firstProject}
        onDirtyChange={onDirtyChange}
        onSave={onSave}
      />,
    );

    await user.clear(screen.getByLabelText("Project instructions"));
    await user.type(screen.getByLabelText("Project instructions"), "Unsaved first draft");

    const secondProject: Project = {
      id: "project-2",
      displayName: "Second project",
      instructions: "Persisted second guidance",
    };
    rerender(
      <ProjectInstructionsEditor
        project={secondProject}
        onDirtyChange={onDirtyChange}
        onSave={onSave}
      />,
    );

    expect(screen.getByLabelText("Project instructions")).toHaveValue("Persisted second guidance");
    expect(screen.getByRole("status")).toHaveTextContent("Saved");
    expect(onSave).not.toHaveBeenCalled();
  });
});
