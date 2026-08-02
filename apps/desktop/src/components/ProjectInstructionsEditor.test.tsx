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

    const editor = screen.getByLabelText("Project instructions");
    const status = screen.getByRole("status");

    expect(screen.getByRole("heading", { level: 3, name: "Instructions" })).toBeVisible();
    expect(editor).toHaveValue(firstProject.instructions);
    expect(editor).toHaveAttribute("aria-describedby", "project-instructions-status");
    expect(status).toHaveAttribute("id", "project-instructions-status");
    expect(status).toHaveTextContent("Saved");
    expect(screen.getByRole("button", { name: "Save instructions" })).toBeDisabled();
    expect(
      screen.queryByText("Keep the durable guidance for this project in plain text."),
    ).not.toBeInTheDocument();
    expect(screen.queryByText("PROJECT GUIDANCE")).not.toBeInTheDocument();
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

  it.each([
    {
      label: "uniform CRLF separators",
      persisted: "First line\r\nSecond line",
      expected: "First line\r\nSecond line!",
    },
    {
      label: "uniform lone-CR separators",
      persisted: "First line\rSecond line",
      expected: "First line\rSecond line!",
    },
    {
      label: "mixed existing separators",
      persisted: "First line\r\nSecond line\rThird line\nFourth line",
      expected: "First line\r\nSecond line\rThird line\nFourth line!",
    },
  ])("preserves $label through an ordinary text edit", async ({ persisted, expected }) => {
    const user = userEvent.setup();
    const project = { ...firstProject, instructions: persisted };
    const onSave = vi
      .fn<(projectId: string, instructions: string) => Promise<Project>>()
      .mockImplementation((_, instructions) => Promise.resolve({ ...project, instructions }));
    render(<ProjectInstructionsEditor project={project} onDirtyChange={vi.fn()} onSave={onSave} />);

    const editor = screen.getByLabelText<HTMLTextAreaElement>("Project instructions");
    expect(editor).toHaveValue(persisted.replace(/\r\n|\r/g, "\n"));

    await user.type(editor, "!");
    await user.click(screen.getByRole("button", { name: "Save instructions" }));

    expect(onSave).toHaveBeenCalledWith("project-1", expected);
    await waitFor(() => expect(screen.getByRole("status")).toHaveTextContent("Saved"));
  });

  it("uses the persisted line-ending style for a newly added line", async () => {
    const user = userEvent.setup();
    const project = { ...firstProject, instructions: "First line\r\nSecond line" };
    const onSave = vi
      .fn<(projectId: string, instructions: string) => Promise<Project>>()
      .mockImplementation((_, instructions) => Promise.resolve({ ...project, instructions }));
    render(<ProjectInstructionsEditor project={project} onDirtyChange={vi.fn()} onSave={onSave} />);

    const editor = screen.getByLabelText<HTMLTextAreaElement>("Project instructions");
    await user.type(editor, "\nThird line");
    await user.click(screen.getByRole("button", { name: "Save instructions" }));

    expect(onSave).toHaveBeenCalledWith("project-1", "First line\r\nSecond line\r\nThird line");
    await waitFor(() => expect(screen.getByRole("status")).toHaveTextContent("Saved"));
  });

  it("preserves the second raw separator when the first adjacent mixed break is deleted", async () => {
    const user = userEvent.setup();
    const project = { ...firstProject, instructions: "A\r\n\rB" };
    const onSave = vi
      .fn<(projectId: string, instructions: string) => Promise<Project>>()
      .mockImplementation((_, instructions) => Promise.resolve({ ...project, instructions }));
    render(<ProjectInstructionsEditor project={project} onDirtyChange={vi.fn()} onSave={onSave} />);

    const editor = screen.getByLabelText<HTMLTextAreaElement>("Project instructions");
    editor.focus();
    editor.setSelectionRange(1, 1);
    await user.keyboard("{Delete}");
    await user.click(screen.getByRole("button", { name: "Save instructions" }));

    expect(onSave).toHaveBeenCalledWith("project-1", "A\rB");
  });

  it("preserves the first raw separator when the second adjacent mixed break is deleted", async () => {
    const user = userEvent.setup();
    const project = { ...firstProject, instructions: "A\r\n\rB" };
    const onSave = vi
      .fn<(projectId: string, instructions: string) => Promise<Project>>()
      .mockImplementation((_, instructions) => Promise.resolve({ ...project, instructions }));
    render(<ProjectInstructionsEditor project={project} onDirtyChange={vi.fn()} onSave={onSave} />);

    const editor = screen.getByLabelText<HTMLTextAreaElement>("Project instructions");
    editor.focus();
    editor.setSelectionRange(3, 3);
    await user.keyboard("{Backspace}");
    await user.click(screen.getByRole("button", { name: "Save instructions" }));

    expect(onSave).toHaveBeenCalledWith("project-1", "A\r\nB");
  });

  it("uses the preferred style for an inserted break without changing the adjacent raw break", async () => {
    const user = userEvent.setup();
    const project = { ...firstProject, instructions: "A\rB\r\nC\rD" };
    const onSave = vi
      .fn<(projectId: string, instructions: string) => Promise<Project>>()
      .mockImplementation((_, instructions) => Promise.resolve({ ...project, instructions }));
    render(<ProjectInstructionsEditor project={project} onDirtyChange={vi.fn()} onSave={onSave} />);

    const editor = screen.getByLabelText<HTMLTextAreaElement>("Project instructions");
    editor.focus();
    editor.setSelectionRange(3, 3);
    await user.keyboard("{Enter}");
    await user.click(screen.getByRole("button", { name: "Save instructions" }));

    expect(onSave).toHaveBeenCalledWith("project-1", "A\rB\r\r\nC\rD");
  });

  it("replaces only the selected adjacent raw separator", async () => {
    const user = userEvent.setup();
    const project = { ...firstProject, instructions: "A\r\n\rB" };
    const onSave = vi
      .fn<(projectId: string, instructions: string) => Promise<Project>>()
      .mockImplementation((_, instructions) => Promise.resolve({ ...project, instructions }));
    render(<ProjectInstructionsEditor project={project} onDirtyChange={vi.fn()} onSave={onSave} />);

    const editor = screen.getByLabelText<HTMLTextAreaElement>("Project instructions");
    editor.focus();
    editor.setSelectionRange(1, 2);
    await user.keyboard("X");
    await user.click(screen.getByRole("button", { name: "Save instructions" }));

    expect(onSave).toHaveBeenCalledWith("project-1", "AX\rB");
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
