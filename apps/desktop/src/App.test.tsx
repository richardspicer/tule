import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import App from "./App";
import { ProjectStorageError, type Project } from "./platform/projects";

interface NativeCloseRequestedEvent {
  preventDefault: () => void;
}

type NativeCloseRequestedHandler = (event: NativeCloseRequestedEvent) => void;

const tuleWordmarkLines = [
  "▀▀▀▀█▀▀▀ ██    ██ ██      ██▀▀▀▀▀▀",
  "   ██    ██    ██ ██      ██▄▄▄▄▄ ",
  "   ██    ██    ██ ██      ██      ",
  "   ██    ██▄▄▄▄▄█ ██▄▄▄▄▄ ██▄▄▄▄▄▄",
] as const;

const {
  createProjectMock,
  getApplicationInfoMock,
  listProjectsMock,
  onCloseRequestedMock,
  openProjectMock,
  unlistenCloseRequestedMock,
  updateProjectInstructionsMock,
} = vi.hoisted(() => ({
  createProjectMock: vi.fn(),
  getApplicationInfoMock: vi.fn(),
  listProjectsMock: vi.fn(),
  onCloseRequestedMock: vi.fn<(handler: NativeCloseRequestedHandler) => Promise<() => void>>(),
  openProjectMock: vi.fn(),
  unlistenCloseRequestedMock: vi.fn(),
  updateProjectInstructionsMock: vi.fn(),
}));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    onCloseRequested: onCloseRequestedMock,
  }),
}));

vi.mock("./platform/application", () => ({
  getApplicationInfo: getApplicationInfoMock,
}));

vi.mock("./platform/projects", async (importOriginal) => {
  const actual = await importOriginal<typeof import("./platform/projects")>();

  return {
    ...actual,
    createProject: createProjectMock,
    listProjects: listProjectsMock,
    openProject: openProjectMock,
    updateProjectInstructions: updateProjectInstructionsMock,
  };
});

function createDeferred<T>() {
  let resolve!: (value: T | PromiseLike<T>) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });

  return { promise, reject, resolve };
}

function getCloseRequestedHandler(): NativeCloseRequestedHandler {
  const handler = onCloseRequestedMock.mock.calls[0]?.[0];

  if (handler === undefined) {
    throw new Error("Expected the native close-request listener to be registered");
  }

  return handler;
}

describe("App", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  beforeEach(() => {
    createProjectMock.mockReset();
    getApplicationInfoMock.mockReset();
    listProjectsMock.mockReset();
    onCloseRequestedMock.mockReset();
    openProjectMock.mockReset();
    unlistenCloseRequestedMock.mockReset();
    updateProjectInstructionsMock.mockReset();
    window.localStorage.clear();

    getApplicationInfoMock.mockResolvedValue({ name: "TULE", version: "0.1.0" });
    listProjectsMock.mockResolvedValue([]);
    onCloseRequestedMock.mockResolvedValue(unlistenCloseRequestedMock);
  });

  it("presents the TULE wordmark once as an accessible graphic", async () => {
    render(<App />);

    await screen.findByText("Desktop ready");

    const wordmark = screen.getByRole("img", { name: "TULE" });
    expect(wordmark).toBeVisible();
    expect(screen.getAllByRole("img", { name: "TULE" })).toHaveLength(1);

    const artwork = within(wordmark).getByText((_, element) => {
      return element?.classList.contains("wordmark-art") ?? false;
    });
    expect(artwork).toHaveAttribute("aria-hidden", "true");
    expect(artwork.textContent?.split("\n")).toEqual([...tuleWordmarkLines]);
  });

  it("uses the settled Projects headings and connection states", async () => {
    render(<App />);

    expect(screen.getByRole("heading", { level: 1, name: "Projects" })).toBeVisible();
    expect(screen.getByRole("heading", { level: 2, name: "Project list" })).toBeVisible();
    expect(screen.getByRole("heading", { level: 2, name: "No project selected" })).toBeVisible();
    expect(screen.getByRole("heading", { level: 2, name: "New project" })).toBeVisible();
    expect(screen.getByText("Current project")).toBeVisible();
    expect(screen.getByText("Select a project to view its instructions.")).toBeVisible();
    expect(screen.getByText("Desktop starting")).toBeVisible();
    expect(await screen.findByText("Desktop ready")).toBeVisible();
    expect(await screen.findByText("Version 0.1.0")).toBeVisible();
  });

  it("does not render removed promotional or implementation-oriented copy", async () => {
    render(<App />);

    await screen.findByText("Desktop ready");

    expect(screen.queryByText("Make room for the work that matters now.")).not.toBeInTheDocument();
    expect(screen.queryByText(/keeps the boundary quiet/i)).not.toBeInTheDocument();
    expect(screen.queryByText(/narrow native boundary/i)).not.toBeInTheDocument();
    expect(screen.queryByText("Local first.")).not.toBeInTheDocument();
    expect(screen.queryByText("Projects stay yours.")).not.toBeInTheDocument();
    expect(screen.queryByText("DESKTOP WORKSPACE")).not.toBeInTheDocument();
    expect(screen.queryByText("Your local workspace")).not.toBeInTheDocument();
    expect(screen.queryByText("LOCAL WORK")).not.toBeInTheDocument();
    expect(screen.queryByText("Local project")).not.toBeInTheDocument();
    expect(screen.queryByText("Selected project")).not.toBeInTheDocument();
    expect(
      screen.queryByText("Keep the durable guidance for this project in plain text."),
    ).not.toBeInTheDocument();
  });

  it("shows the version footer after the Rust boundary connects", async () => {
    getApplicationInfoMock.mockResolvedValue({ name: "TULE", version: "9.8.7" });

    render(<App />);

    expect(await screen.findByText("Desktop ready")).toBeVisible();
    expect(screen.getByText("Version 9.8.7")).toBeVisible();
    expect(screen.queryByText("TULE", { selector: "dd" })).not.toBeInTheDocument();
  });

  it("reports when the desktop boundary is unavailable", async () => {
    getApplicationInfoMock.mockRejectedValue(new Error("desktop unavailable"));

    render(<App />);

    expect(await screen.findByText("Desktop unavailable")).toBeVisible();
    expect(screen.queryByText(/^Version /)).not.toBeInTheDocument();
  });

  it("loads a saved theme and supports System, Light, and Dark selection", async () => {
    const user = userEvent.setup();
    window.localStorage.setItem("tule-theme", "dark");

    render(<App />);

    const appearance = screen.getByLabelText("Appearance");
    expect(document.documentElement).toHaveAttribute("data-theme", "dark");
    expect(appearance).toHaveValue("dark");

    await user.selectOptions(appearance, "system");

    expect(document.documentElement).not.toHaveAttribute("data-theme");
    expect(window.localStorage.getItem("tule-theme")).toBeNull();
    expect(appearance).toHaveValue("system");

    await user.selectOptions(appearance, "light");

    expect(document.documentElement).toHaveAttribute("data-theme", "light");
    expect(window.localStorage.getItem("tule-theme")).toBe("light");
    expect(appearance).toHaveValue("light");

    await user.selectOptions(appearance, "dark");

    expect(document.documentElement).toHaveAttribute("data-theme", "dark");
    expect(window.localStorage.getItem("tule-theme")).toBe("dark");
    expect(appearance).toHaveValue("dark");
  });

  it("shows loading before an empty project list resolves", async () => {
    const pendingProjects = createDeferred<never[]>();
    listProjectsMock.mockReturnValue(pendingProjects.promise);

    render(<App />);

    expect(screen.getByRole("status")).toHaveTextContent("Loading projects…");
    expect(screen.queryByText("No projects yet")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Create project" })).toBeDisabled();

    pendingProjects.resolve([]);

    expect(await screen.findByText("No projects yet")).toBeVisible();
    expect(screen.getByRole("button", { name: "Create project" })).toBeEnabled();
  });

  it("renders a populated project list without selecting a project", async () => {
    listProjectsMock.mockResolvedValue([
      { id: "project-1", displayName: "First project", instructions: "" },
      { id: "project-2", displayName: "Second project", instructions: "" },
    ]);

    render(<App />);

    const firstProject = await screen.findByRole("button", { name: /first project/i });
    const secondProject = screen.getByRole("button", { name: /second project/i });

    expect(firstProject).toHaveAttribute("aria-pressed", "false");
    expect(secondProject).toHaveAttribute("aria-pressed", "false");
    expect(screen.getByText("2 projects")).toBeVisible();
    expect(screen.getByRole("heading", { level: 2, name: "No project selected" })).toBeVisible();
  });

  it("submits a trimmed name, shows creation progress, and selects the returned project", async () => {
    const user = userEvent.setup();
    const pendingProject = createDeferred<Project>();
    createProjectMock.mockReturnValue(pendingProject.promise);

    render(<App />);

    await screen.findByText("No projects yet");
    await user.type(screen.getByLabelText("Project name"), "  New plan  ");
    await user.click(screen.getByRole("button", { name: "Create project" }));

    expect(createProjectMock).toHaveBeenCalledWith("New plan");
    expect(screen.getByRole("button", { name: "Creating…" })).toBeDisabled();
    expect(screen.getByLabelText("Project name")).toBeDisabled();

    pendingProject.resolve({ id: "project-new", displayName: "New plan", instructions: "" });

    const projectButton = await screen.findByRole("button", { name: /new plan/i });
    await waitFor(() => expect(projectButton).toHaveAttribute("aria-pressed", "true"));
    expect(screen.getByRole("heading", { level: 2, name: "New plan" })).toBeVisible();
    expect(projectButton).toHaveTextContent("Selected");
    expect(screen.getByLabelText("Project name")).toHaveValue("");
  });

  it("validates a blank project name before invoking the native boundary", async () => {
    const user = userEvent.setup();

    render(<App />);

    await screen.findByText("No projects yet");
    await user.type(screen.getByLabelText("Project name"), "   ");
    await user.click(screen.getByRole("button", { name: "Create project" }));

    expect(screen.getByText("Enter a project name.")).toBeVisible();
    expect(screen.getByLabelText("Project name")).toHaveAttribute("aria-invalid", "true");
    expect(createProjectMock).not.toHaveBeenCalled();
  });

  it("shows opening progress and selects only the returned project", async () => {
    const user = userEvent.setup();
    const pendingProject = createDeferred<Project>();
    listProjectsMock.mockResolvedValue([
      { id: "project-1", displayName: "First project", instructions: "" },
    ]);
    openProjectMock.mockReturnValue(pendingProject.promise);

    render(<App />);

    const projectButton = await screen.findByRole("button", { name: /first project/i });
    await user.click(projectButton);

    expect(openProjectMock).toHaveBeenCalledWith("project-1");
    expect(projectButton).toBeDisabled();
    expect(projectButton).toHaveTextContent("Opening…");
    expect(projectButton).toHaveAttribute("aria-pressed", "false");
    expect(projectButton).toHaveAttribute("aria-busy", "true");
    expect(screen.getByRole("button", { name: /first project.*opening/i })).toBe(projectButton);
    expect(screen.getByRole("heading", { level: 2, name: "No project selected" })).toBeVisible();

    pendingProject.resolve({
      id: "project-1",
      displayName: "First project",
      instructions: "Existing guidance",
    });

    await waitFor(() => expect(projectButton).toHaveAttribute("aria-pressed", "true"));
    expect(projectButton).not.toHaveAttribute("aria-busy");
    expect(screen.getByRole("heading", { level: 2, name: "First project" })).toBeVisible();
    expect(screen.getByLabelText("Project instructions")).toHaveValue("Existing guidance");
    expect(screen.getByRole("heading", { level: 3, name: "Instructions" })).toBeVisible();
  });

  it("saves exact instructions and replaces the selected and listed project", async () => {
    const user = userEvent.setup();
    const project: Project = {
      id: "project-1",
      displayName: "First project",
      instructions: "Before",
    };
    const savedProject: Project = {
      id: "project-1",
      displayName: "Persisted project",
      instructions: "  Exact replacement\n第二行  ",
    };
    listProjectsMock.mockResolvedValue([project]);
    openProjectMock.mockResolvedValue(project);
    updateProjectInstructionsMock.mockResolvedValue(savedProject);

    render(<App />);

    await user.click(await screen.findByRole("button", { name: /first project/i }));
    const editor = await screen.findByLabelText("Project instructions");
    await user.clear(editor);
    await user.type(editor, savedProject.instructions);
    await user.click(screen.getByRole("button", { name: "Save instructions" }));

    expect(updateProjectInstructionsMock).toHaveBeenCalledWith(
      "project-1",
      savedProject.instructions,
    );
    expect(
      await screen.findByRole("heading", { level: 2, name: "Persisted project" }),
    ).toBeVisible();
    expect(screen.getByRole("button", { name: /persisted project.*selected/i })).toBeVisible();
    expect(screen.getByLabelText("Project instructions")).toHaveValue(savedProject.instructions);
    expect(screen.getByRole("status")).toHaveTextContent("Saved");
  });

  it("prevents reload and a cancelled native close while instructions are dirty", async () => {
    const user = userEvent.setup();
    const confirmClose = vi.spyOn(window, "confirm").mockReturnValue(false);
    const project: Project = {
      id: "project-1",
      displayName: "First project",
      instructions: "Before",
    };
    listProjectsMock.mockResolvedValue([project]);
    openProjectMock.mockResolvedValue(project);

    const { unmount } = render(<App />);

    await user.click(await screen.findByRole("button", { name: /first project/i }));
    await user.type(screen.getByLabelText("Project instructions"), " changed");
    expect(screen.getByRole("status")).toHaveTextContent("Unsaved changes");

    await waitFor(() => {
      const dirtyUnload = new Event("beforeunload", { cancelable: true });
      expect(window.dispatchEvent(dirtyUnload)).toBe(false);
      expect(dirtyUnload.defaultPrevented).toBe(true);
    });

    expect(onCloseRequestedMock).toHaveBeenCalledOnce();
    const preventNativeClose = vi.fn();
    const closeRequestedHandler = getCloseRequestedHandler();
    closeRequestedHandler({ preventDefault: preventNativeClose });

    expect(confirmClose).toHaveBeenCalledWith(
      "Discard unsaved project instructions and close TULE?",
    );
    expect(preventNativeClose).toHaveBeenCalledOnce();

    unmount();

    await waitFor(() => expect(unlistenCloseRequestedMock).toHaveBeenCalledOnce());
    const unmountedUnload = new Event("beforeunload", { cancelable: true });
    expect(window.dispatchEvent(unmountedUnload)).toBe(true);
    expect(unmountedUnload.defaultPrevented).toBe(false);
  });

  it("allows reload and native close when instructions are clean or have been saved", async () => {
    const user = userEvent.setup();
    const confirmClose = vi.spyOn(window, "confirm");
    const project: Project = {
      id: "project-1",
      displayName: "First project",
      instructions: "Before",
    };
    const savedProject: Project = {
      ...project,
      instructions: "After",
    };
    listProjectsMock.mockResolvedValue([project]);
    openProjectMock.mockResolvedValue(project);
    updateProjectInstructionsMock.mockResolvedValue(savedProject);

    render(<App />);

    await user.click(await screen.findByRole("button", { name: /first project/i }));
    expect(onCloseRequestedMock).toHaveBeenCalledOnce();
    const closeRequestedHandler = getCloseRequestedHandler();
    const preventCleanClose = vi.fn();
    closeRequestedHandler({ preventDefault: preventCleanClose });

    const cleanUnload = new Event("beforeunload", { cancelable: true });
    expect(window.dispatchEvent(cleanUnload)).toBe(true);
    expect(cleanUnload.defaultPrevented).toBe(false);
    expect(confirmClose).not.toHaveBeenCalled();
    expect(preventCleanClose).not.toHaveBeenCalled();

    const editor = screen.getByLabelText("Project instructions");
    await user.clear(editor);
    await user.type(editor, savedProject.instructions);
    await user.click(screen.getByRole("button", { name: "Save instructions" }));
    await waitFor(() => expect(screen.getByRole("status")).toHaveTextContent("Saved"));

    await waitFor(() => {
      const savedUnload = new Event("beforeunload", { cancelable: true });
      expect(window.dispatchEvent(savedUnload)).toBe(true);
      expect(savedUnload.defaultPrevented).toBe(false);
    });

    const preventSavedClose = vi.fn();
    closeRequestedHandler({ preventDefault: preventSavedClose });
    expect(confirmClose).not.toHaveBeenCalled();
    expect(preventSavedClose).not.toHaveBeenCalled();
  });

  it("allows an explicitly confirmed native close with dirty instructions", async () => {
    const user = userEvent.setup();
    const confirmClose = vi.spyOn(window, "confirm").mockReturnValue(true);
    const project: Project = {
      id: "project-1",
      displayName: "First project",
      instructions: "Before",
    };
    listProjectsMock.mockResolvedValue([project]);
    openProjectMock.mockResolvedValue(project);

    render(<App />);

    await user.click(await screen.findByRole("button", { name: /first project/i }));
    await user.type(screen.getByLabelText("Project instructions"), " changed");
    expect(screen.getByRole("status")).toHaveTextContent("Unsaved changes");

    const preventNativeClose = vi.fn();
    const closeRequestedHandler = getCloseRequestedHandler();
    closeRequestedHandler({ preventDefault: preventNativeClose });

    expect(confirmClose).toHaveBeenCalledWith(
      "Discard unsaved project instructions and close TULE?",
    );
    expect(preventNativeClose).not.toHaveBeenCalled();
  });

  it("cleans up a close listener that resolves after unmount", async () => {
    const deferredListener = createDeferred<() => void>();
    onCloseRequestedMock.mockReturnValue(deferredListener.promise);

    const { unmount } = render(<App />);
    expect(onCloseRequestedMock).toHaveBeenCalledOnce();

    unmount();
    deferredListener.resolve(unlistenCloseRequestedMock);

    await waitFor(() => expect(unlistenCloseRequestedMock).toHaveBeenCalledOnce());
  });

  it("keeps an unsaved draft visible when saving fails safely", async () => {
    const user = userEvent.setup();
    const project: Project = {
      id: "project-1",
      displayName: "First project",
      instructions: "Before",
    };
    listProjectsMock.mockResolvedValue([project]);
    openProjectMock.mockResolvedValue(project);
    updateProjectInstructionsMock.mockRejectedValue(
      new Error("SQLITE_BUSY C:\\private\\projects.db"),
    );

    render(<App />);

    await user.click(await screen.findByRole("button", { name: /first project/i }));
    const editor = await screen.findByLabelText("Project instructions");
    await user.clear(editor);
    await user.type(editor, "Keep this draft");
    await user.click(screen.getByRole("button", { name: "Save instructions" }));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Save failed. Your changes are still here.",
    );
    expect(editor).toHaveValue("Keep this draft");
    expect(screen.queryByText(/SQLITE_BUSY|projects\.db/i)).not.toBeInTheDocument();
  });

  it("blocks selection-changing actions until a pending instruction save settles", async () => {
    const user = userEvent.setup();
    const firstProject: Project = {
      id: "project-1",
      displayName: "First project",
      instructions: "Before",
    };
    const secondProject: Project = {
      id: "project-2",
      displayName: "Second project",
      instructions: "Second guidance",
    };
    const pendingSave = createDeferred<Project>();
    listProjectsMock.mockResolvedValue([firstProject, secondProject]);
    openProjectMock.mockImplementation((projectId: string) =>
      Promise.resolve(projectId === firstProject.id ? firstProject : secondProject),
    );
    updateProjectInstructionsMock.mockReturnValue(pendingSave.promise);

    render(<App />);

    await user.click(await screen.findByRole("button", { name: /first project/i }));
    const editor = await screen.findByLabelText("Project instructions");
    await user.clear(editor);
    await user.type(editor, "Keep this pending draft");
    await user.click(screen.getByRole("button", { name: "Save instructions" }));

    const secondProjectButton = screen.getByRole("button", { name: /second project/i });
    expect(secondProjectButton).toBeDisabled();
    expect(screen.getByLabelText("Project name")).toBeDisabled();
    fireEvent.click(secondProjectButton);
    expect(openProjectMock).toHaveBeenCalledTimes(1);

    pendingSave.reject(new Error("C:\\private\\projects.db failed"));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Save failed. Your changes are still here.",
    );
    expect(editor).toHaveValue("Keep this pending draft");
    expect(secondProjectButton).toBeEnabled();
    expect(screen.getByLabelText("Project name")).toBeEnabled();
  });

  it("keeps an unsaved draft when the user cancels a project change", async () => {
    const user = userEvent.setup();
    const firstProject: Project = {
      id: "project-1",
      displayName: "First project",
      instructions: "First persisted guidance",
    };
    const secondProject: Project = {
      id: "project-2",
      displayName: "Second project",
      instructions: "Second persisted guidance",
    };
    const confirmDiscard = vi.spyOn(window, "confirm").mockReturnValue(false);
    listProjectsMock.mockResolvedValue([firstProject, secondProject]);
    openProjectMock.mockImplementation((projectId: string) =>
      Promise.resolve(projectId === firstProject.id ? firstProject : secondProject),
    );

    render(<App />);

    const firstProjectButton = await screen.findByRole("button", { name: /first project/i });
    const secondProjectButton = screen.getByRole("button", { name: /second project/i });
    await user.click(firstProjectButton);
    const editor = await screen.findByLabelText("Project instructions");
    await user.clear(editor);
    await user.type(editor, "Unsaved first draft");

    await user.click(firstProjectButton);

    expect(confirmDiscard).not.toHaveBeenCalled();
    expect(openProjectMock).toHaveBeenCalledTimes(1);
    expect(editor).toHaveValue("Unsaved first draft");

    await user.click(secondProjectButton);

    expect(confirmDiscard).toHaveBeenCalledWith(
      "Discard unsaved project instructions and continue?",
    );
    expect(openProjectMock).toHaveBeenCalledTimes(1);
    expect(firstProjectButton).toHaveAttribute("aria-pressed", "true");
    expect(editor).toHaveValue("Unsaved first draft");
    expect(screen.getByRole("status")).toHaveTextContent("Unsaved changes");
  });

  it("changes projects only after the user confirms discarding an unsaved draft", async () => {
    const user = userEvent.setup();
    const firstProject: Project = {
      id: "project-1",
      displayName: "First project",
      instructions: "First persisted guidance",
    };
    const secondProject: Project = {
      id: "project-2",
      displayName: "Second project",
      instructions: "Second persisted guidance",
    };
    const confirmDiscard = vi.spyOn(window, "confirm").mockReturnValue(true);
    listProjectsMock.mockResolvedValue([firstProject, secondProject]);
    openProjectMock.mockImplementation((projectId: string) =>
      Promise.resolve(projectId === firstProject.id ? firstProject : secondProject),
    );

    render(<App />);

    await user.click(await screen.findByRole("button", { name: /first project/i }));
    const editor = await screen.findByLabelText("Project instructions");
    await user.clear(editor);
    await user.type(editor, "Unsaved first draft");
    expect(screen.getByRole("status")).toHaveTextContent("Unsaved changes");

    await user.click(screen.getByRole("button", { name: /second project/i }));

    expect(confirmDiscard).toHaveBeenCalledWith(
      "Discard unsaved project instructions and continue?",
    );
    expect(await screen.findByLabelText("Project instructions")).toHaveValue(
      "Second persisted guidance",
    );
    expect(screen.getByRole("status")).toHaveTextContent("Saved");
    expect(updateProjectInstructionsMock).not.toHaveBeenCalled();
  });

  it("does not create a project when the user keeps an unsaved draft", async () => {
    const user = userEvent.setup();
    const project: Project = {
      id: "project-1",
      displayName: "First project",
      instructions: "Before",
    };
    const confirmDiscard = vi.spyOn(window, "confirm").mockReturnValue(false);
    listProjectsMock.mockResolvedValue([project]);
    openProjectMock.mockResolvedValue(project);

    render(<App />);

    await user.click(await screen.findByRole("button", { name: /first project/i }));
    const editor = await screen.findByLabelText("Project instructions");
    await user.clear(editor);
    await user.type(editor, "Keep this draft");
    await user.type(screen.getByLabelText("Project name"), "Another project");
    await user.click(screen.getByRole("button", { name: "Create project" }));

    expect(confirmDiscard).toHaveBeenCalledWith(
      "Discard unsaved project instructions and continue?",
    );
    expect(createProjectMock).not.toHaveBeenCalled();
    expect(editor).toHaveValue("Keep this draft");
    expect(screen.getByLabelText("Project name")).toHaveValue("Another project");
  });

  it("keeps the prior selection when a later open fails safely", async () => {
    const user = userEvent.setup();
    const firstProject = { id: "project-1", displayName: "First project", instructions: "" };
    const secondProject = { id: "project-2", displayName: "Second project", instructions: "" };
    listProjectsMock.mockResolvedValue([firstProject, secondProject]);
    openProjectMock
      .mockResolvedValueOnce(firstProject)
      .mockRejectedValueOnce(new Error("C:\\private\\projects.db contains secret-data"));

    render(<App />);

    const firstProjectButton = await screen.findByRole("button", { name: /first project/i });
    const secondProjectButton = screen.getByRole("button", { name: /second project/i });
    await user.click(firstProjectButton);
    await waitFor(() => expect(firstProjectButton).toHaveAttribute("aria-pressed", "true"));

    await user.click(secondProjectButton);

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("Project storage is unavailable. Try again.");
    expect(firstProjectButton).toHaveAttribute("aria-pressed", "true");
    expect(secondProjectButton).toHaveAttribute("aria-pressed", "false");
    expect(screen.queryByText(/projects\.db|secret-data/i)).not.toBeInTheDocument();
  });

  it("shows a generic safe list failure without showing an empty state or caught details", async () => {
    listProjectsMock.mockRejectedValue(
      new Error("SQLITE_CANTOPEN C:\\private\\projects.db account=owner"),
    );

    render(<App />);

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("Project storage is unavailable. Restart TULE to try again.");
    expect(screen.getByText("Projects are unavailable.")).toBeVisible();
    expect(screen.queryByText("No projects yet")).not.toBeInTheDocument();
    expect(
      screen.queryByText(/SQLITE_CANTOPEN|projects\.db|account=owner/i),
    ).not.toBeInTheDocument();
  });

  it("disables creation after a failed list load and keeps that failure authoritative", async () => {
    const user = userEvent.setup();
    listProjectsMock.mockRejectedValue(new Error("storage failed"));

    render(<App />);

    await screen.findByText("Projects are unavailable.");
    const createButton = screen.getByRole("button", { name: "Create project" });

    expect(createButton).toBeDisabled();
    expect(screen.getByLabelText("Project name")).toBeDisabled();
    await user.click(createButton);

    const createForm = createButton.closest("form");
    expect(createForm).not.toBeNull();
    if (createForm === null) {
      throw new Error("Expected the create button to belong to a form");
    }
    fireEvent.submit(createForm);

    expect(createProjectMock).not.toHaveBeenCalled();
    expect(screen.getByText("Projects are unavailable.")).toBeVisible();
    expect(screen.queryByText("No projects yet")).not.toBeInTheDocument();
  });

  it("shows a generic safe create failure without rendering caught details", async () => {
    const user = userEvent.setup();
    createProjectMock.mockRejectedValue(new Error("secret storage detail"));

    render(<App />);

    await screen.findByText("No projects yet");
    await user.type(screen.getByLabelText("Project name"), "New plan");
    await user.click(screen.getByRole("button", { name: "Create project" }));

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("Project storage is unavailable. Try again.");
    expect(screen.queryByText(/secret storage detail/i)).not.toBeInTheDocument();
    expect(screen.getByLabelText("Project name")).toHaveValue("New plan");
  });

  it("maps a native invalid-name rejection back to the project-name field", async () => {
    const user = userEvent.setup();
    createProjectMock.mockRejectedValue(new ProjectStorageError("invalid_project_name"));

    render(<App />);

    await screen.findByText("No projects yet");
    await user.type(screen.getByLabelText("Project name"), "Rejected name");
    await user.click(screen.getByRole("button", { name: "Create project" }));

    expect(await screen.findByText("Enter a valid project name.")).toBeVisible();
    expect(screen.getByLabelText("Project name")).toHaveAttribute("aria-invalid", "true");
    expect(screen.queryByText("Project action unavailable")).not.toBeInTheDocument();
    expect(
      screen.queryByText("Project storage is unavailable. Try again."),
    ).not.toBeInTheDocument();
  });
});
