import { invoke } from "@tauri-apps/api/core";

export interface Project {
  id: string;
  displayName: string;
  instructions: string;
}

export type ProjectErrorCode =
  | "invalid_project_name"
  | "invalid_project_id"
  | "project_not_found"
  | "project_storage_unavailable";

const genericProjectErrorCode: ProjectErrorCode = "project_storage_unavailable";
const projectErrorCodes: readonly ProjectErrorCode[] = [
  "invalid_project_name",
  "invalid_project_id",
  "project_not_found",
  genericProjectErrorCode,
];

export class ProjectStorageError extends Error {
  readonly code: ProjectErrorCode;

  constructor(code: ProjectErrorCode) {
    super(code);
    this.name = "ProjectStorageError";
    this.code = code;
  }
}

function isProjectErrorCode(value: unknown): value is ProjectErrorCode {
  return (
    typeof value === "string" &&
    projectErrorCodes.some((projectErrorCode) => projectErrorCode === value)
  );
}

function toProjectStorageError(error: unknown): ProjectStorageError {
  return new ProjectStorageError(isProjectErrorCode(error) ? error : genericProjectErrorCode);
}

export function getProjectErrorCode(error: unknown): ProjectErrorCode {
  return error instanceof ProjectStorageError ? error.code : genericProjectErrorCode;
}

function isProject(value: unknown): value is Project {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return false;
  }

  const keys = Object.keys(value);

  return (
    keys.length === 3 &&
    keys.includes("id") &&
    keys.includes("displayName") &&
    keys.includes("instructions") &&
    "id" in value &&
    typeof value.id === "string" &&
    "displayName" in value &&
    typeof value.displayName === "string" &&
    "instructions" in value &&
    typeof value.instructions === "string"
  );
}

function validateProject(value: unknown): Project {
  if (!isProject(value)) {
    throw new ProjectStorageError(genericProjectErrorCode);
  }

  return value;
}

function validateProjectList(value: unknown): Project[] {
  if (!Array.isArray(value) || !value.every(isProject)) {
    throw new ProjectStorageError(genericProjectErrorCode);
  }

  const projectIds = new Set(value.map((project) => project.id));

  if (projectIds.size !== value.length) {
    throw new ProjectStorageError(genericProjectErrorCode);
  }

  return value;
}

async function invokeProjectCommand(
  command: string,
  args?: Record<string, unknown>,
): Promise<unknown> {
  try {
    return args === undefined ? await invoke(command) : await invoke(command, args);
  } catch (error: unknown) {
    throw toProjectStorageError(error);
  }
}

export async function listProjects(): Promise<Project[]> {
  return validateProjectList(await invokeProjectCommand("list_projects"));
}

export async function createProject(displayName: string): Promise<Project> {
  return validateProject(await invokeProjectCommand("create_project", { displayName }));
}

export async function openProject(projectId: string): Promise<Project> {
  const project = validateProject(await invokeProjectCommand("open_project", { projectId }));

  if (project.id !== projectId) {
    throw new ProjectStorageError(genericProjectErrorCode);
  }

  return project;
}

export async function updateProjectInstructions(
  projectId: string,
  instructions: string,
): Promise<Project> {
  const project = validateProject(
    await invokeProjectCommand("update_project_instructions", { projectId, instructions }),
  );

  if (project.id !== projectId) {
    throw new ProjectStorageError(genericProjectErrorCode);
  }

  return project;
}
