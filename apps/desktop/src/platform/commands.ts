export type AppCommandId =
  | "new-session"
  | "manage-projects"
  | "exit"
  | "edit-undo"
  | "edit-redo"
  | "edit-cut"
  | "edit-copy"
  | "edit-paste"
  | "edit-select-all"
  | "open-settings"
  | "open-settings-connections";

export type SettingsCategory = "connections" | "appearance";

export type AppCommandHandler = (command: AppCommandId) => void | Promise<void>;

const editCommands = new Set<AppCommandId>([
  "edit-undo",
  "edit-redo",
  "edit-cut",
  "edit-copy",
  "edit-paste",
  "edit-select-all",
]);

function isTextEntry(element: Element | null): element is HTMLInputElement | HTMLTextAreaElement {
  return element instanceof HTMLInputElement || element instanceof HTMLTextAreaElement;
}

export function isEditableTarget(element: Element | null): boolean {
  if (element === null || !(element instanceof HTMLElement)) {
    return false;
  }

  if (element.isContentEditable) {
    return true;
  }

  if (isTextEntry(element)) {
    return !element.disabled && !element.readOnly;
  }

  return false;
}

function hasSelectableText(element: Element | null): boolean {
  if (isTextEntry(element)) {
    return element.value.length > 0;
  }

  const selection = window.getSelection();
  return selection !== null && selection.toString().length > 0;
}

export interface EditCommandAvailability {
  undo: boolean;
  redo: boolean;
  cut: boolean;
  copy: boolean;
  paste: boolean;
  selectAll: boolean;
}

export function queryEditCommandAvailability(): EditCommandAvailability {
  const active = document.activeElement;
  const editable = isEditableTarget(active);

  if (!editable) {
    return {
      undo: false,
      redo: false,
      cut: false,
      copy: hasSelectableText(active),
      paste: false,
      selectAll: false,
    };
  }

  return {
    undo: queryCommandEnabled("undo"),
    redo: queryCommandEnabled("redo"),
    cut: queryCommandEnabled("cut"),
    copy: queryCommandEnabled("copy") || hasSelectableText(active),
    paste: queryCommandEnabled("paste"),
    selectAll: true,
  };
}

function queryCommandEnabled(command: string): boolean {
  try {
    return typeof document.queryCommandEnabled === "function"
      ? document.queryCommandEnabled(command)
      : false;
  } catch {
    return false;
  }
}

export function runEditCommand(command: AppCommandId): boolean {
  if (!editCommands.has(command)) {
    return false;
  }

  const availability = queryEditCommandAvailability();
  const active = document.activeElement;

  switch (command) {
    case "edit-undo":
      return availability.undo && execCommand("undo");
    case "edit-redo":
      return availability.redo && execCommand("redo");
    case "edit-cut":
      return availability.cut && execCommand("cut");
    case "edit-copy":
      return availability.copy && execCommand("copy");
    case "edit-paste":
      return availability.paste && execCommand("paste");
    case "edit-select-all":
      if (!availability.selectAll) {
        return false;
      }
      if (isTextEntry(active)) {
        active.select();
        return true;
      }
      return execCommand("selectAll");
    default:
      return false;
  }
}

function execCommand(command: string): boolean {
  try {
    return typeof document.execCommand === "function" ? document.execCommand(command) : false;
  } catch {
    return false;
  }
}

export function createCommandDispatcher(handler: AppCommandHandler): AppCommandHandler {
  return (command) => {
    if (editCommands.has(command)) {
      runEditCommand(command);
      return;
    }

    return handler(command);
  };
}
