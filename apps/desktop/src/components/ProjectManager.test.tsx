import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { ProjectManager } from "./ProjectManager";

const project = {
  id: "11111111-1111-7111-8111-111111111111",
  displayName: "Atlas",
  instructions: "Keep answers short",
};

describe("ProjectManager", () => {
  it("uses master/detail at default width and single-pane create/edit at compact width", async () => {
    const user = userEvent.setup();
    const onBeginCreate = vi.fn();
    const onOpen = vi.fn();
    const onClearSelection = vi.fn();

    const { rerender } = render(
      <ProjectManager
        projects={[project]}
        loadState="ready"
        selectedProject={null}
        projectName=""
        projectNameError={null}
        projectError={null}
        openingProjectId={null}
        actionsDisabled={false}
        isCreating={false}
        compact={false}
        creating={false}
        onProjectNameChange={vi.fn()}
        onCreate={vi.fn()}
        onOpen={onOpen}
        onDirtyChange={vi.fn()}
        onSaveInstructions={vi.fn()}
        onUseWithAgents={vi.fn()}
        onClearSelection={onClearSelection}
        onBeginCreate={onBeginCreate}
      />,
    );

    expect(screen.getByRole("heading", { name: "Projects", level: 2 })).toBeInTheDocument();
    expect(screen.getByText("Nothing open yet")).toBeInTheDocument();

    rerender(
      <ProjectManager
        projects={[project]}
        loadState="ready"
        selectedProject={null}
        projectName=""
        projectNameError={null}
        projectError={null}
        openingProjectId={null}
        actionsDisabled={false}
        isCreating={false}
        compact
        creating={false}
        onProjectNameChange={vi.fn()}
        onCreate={vi.fn()}
        onOpen={onOpen}
        onDirtyChange={vi.fn()}
        onSaveInstructions={vi.fn()}
        onUseWithAgents={vi.fn()}
        onClearSelection={onClearSelection}
        onBeginCreate={onBeginCreate}
      />,
    );

    expect(screen.queryByText("Nothing open yet")).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "New project" }));
    expect(onBeginCreate).toHaveBeenCalled();

    rerender(
      <ProjectManager
        projects={[project]}
        loadState="ready"
        selectedProject={project}
        projectName=""
        projectNameError={null}
        projectError={null}
        openingProjectId={null}
        actionsDisabled={false}
        isCreating={false}
        compact
        creating={false}
        onProjectNameChange={vi.fn()}
        onCreate={vi.fn()}
        onOpen={onOpen}
        onDirtyChange={vi.fn()}
        onSaveInstructions={vi.fn(() => Promise.resolve(project))}
        onUseWithAgents={vi.fn()}
        onClearSelection={onClearSelection}
        onBeginCreate={onBeginCreate}
      />,
    );

    expect(screen.getByRole("button", { name: "Back to projects" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Use with Agents" })).toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: "Projects", level: 2 })).not.toBeInTheDocument();
  });
});
