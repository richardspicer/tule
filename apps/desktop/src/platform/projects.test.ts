import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  createProject,
  getProjectErrorCode,
  listProjects,
  openProject,
  ProjectStorageError,
  updateProjectInstructions,
} from "./projects";

const { invokeMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
}));

describe("project platform boundary", () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it("lists projects through the argument-free command", async () => {
    const projects = [
      { id: "project-1", displayName: "First project", instructions: "" },
      {
        id: "project-2",
        displayName: "Second project",
        instructions: "Preserve this exactly.\n第二行",
      },
    ];
    invokeMock.mockResolvedValue(projects);

    await expect(listProjects()).resolves.toEqual(projects);
    expect(invokeMock).toHaveBeenCalledWith("list_projects");
  });

  it("creates a project with the native display-name argument", async () => {
    const project = { id: "project-1", displayName: "First project", instructions: "" };
    invokeMock.mockResolvedValue(project);

    await expect(createProject("First project")).resolves.toEqual(project);
    expect(invokeMock).toHaveBeenCalledWith("create_project", {
      displayName: "First project",
    });
  });

  it("opens a project with the native project-id argument", async () => {
    const project = {
      id: "project-1",
      displayName: "First project",
      instructions: "Ask for evidence.",
    };
    invokeMock.mockResolvedValue(project);

    await expect(openProject("project-1")).resolves.toEqual(project);
    expect(invokeMock).toHaveBeenCalledWith("open_project", { projectId: "project-1" });
  });

  it("updates instructions with the native project fields and preserves exact content", async () => {
    const instructions = "  Keep leading space.\n保留 punctuation!  ";
    const project = { id: "project-1", displayName: "First project", instructions };
    invokeMock.mockResolvedValue(project);

    await expect(updateProjectInstructions("project-1", instructions)).resolves.toEqual(project);
    expect(invokeMock).toHaveBeenCalledWith("update_project_instructions", {
      projectId: "project-1",
      instructions,
    });
  });

  it.each([
    null,
    {},
    { id: "project-1", displayName: 1, instructions: "" },
    { id: "project-1", displayName: "First project" },
    { id: "project-1", displayName: "First project", instructions: 1 },
    { id: "project-1", displayName: "First project", instructions: "", path: "private" },
  ])("rejects an invalid exact project response: %o", async (response) => {
    invokeMock.mockResolvedValue(response);

    await expect(createProject("First project")).rejects.toMatchObject({
      name: "ProjectStorageError",
      code: "project_storage_unavailable",
    });
  });

  it("rejects a project list containing an invalid item", async () => {
    invokeMock.mockResolvedValue([
      { id: "project-1", displayName: "First project", instructions: "" },
      { id: "project-2", displayName: "Second project", instructions: "", extra: true },
    ]);

    await expect(listProjects()).rejects.toMatchObject({
      name: "ProjectStorageError",
      code: "project_storage_unavailable",
    });
  });

  it("rejects a project list containing duplicate project ids", async () => {
    invokeMock.mockResolvedValue([
      { id: "project-1", displayName: "First project", instructions: "" },
      { id: "project-1", displayName: "Duplicate project", instructions: "" },
    ]);

    await expect(listProjects()).rejects.toMatchObject({
      name: "ProjectStorageError",
      code: "project_storage_unavailable",
    });
  });

  it("rejects an open response whose id does not match the requested project", async () => {
    invokeMock.mockResolvedValue({
      id: "project-2",
      displayName: "Second project",
      instructions: "",
    });

    await expect(openProject("project-1")).rejects.toMatchObject({
      name: "ProjectStorageError",
      code: "project_storage_unavailable",
    });
  });

  it("rejects an update response whose id does not match the requested project", async () => {
    invokeMock.mockResolvedValue({
      id: "project-2",
      displayName: "Second project",
      instructions: "Updated",
    });

    await expect(updateProjectInstructions("project-1", "Updated")).rejects.toMatchObject({
      name: "ProjectStorageError",
      code: "project_storage_unavailable",
    });
  });

  it.each([
    "invalid_project_name",
    "invalid_project_id",
    "project_not_found",
    "project_storage_unavailable",
  ] as const)("preserves the allowlisted native error code %s", async (code) => {
    invokeMock.mockRejectedValue(code);

    await expect(openProject("project-1")).rejects.toMatchObject({
      name: "ProjectStorageError",
      code,
    });
  });

  it("collapses an unknown native failure without retaining its details", async () => {
    invokeMock.mockRejectedValue(new Error("C:\\private\\projects.db: access denied"));

    try {
      await listProjects();
      throw new Error("Expected listProjects to reject");
    } catch (error: unknown) {
      expect(error).toBeInstanceOf(ProjectStorageError);
      expect(getProjectErrorCode(error)).toBe("project_storage_unavailable");
      expect(String(error)).not.toContain("projects.db");
      expect(String(error)).not.toContain("access denied");
    }
  });
});
