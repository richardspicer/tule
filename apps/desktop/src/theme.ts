import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type ThemePreference = "system" | "light" | "dark";

const appearanceChangedEvent = "appearance-changed";

function isThemePreference(value: unknown): value is ThemePreference {
  return value === "system" || value === "light" || value === "dark";
}

export function parseThemePreference(value: unknown): ThemePreference {
  return isThemePreference(value) ? value : "system";
}

export function applyThemePreference(theme: ThemePreference): void {
  if (theme === "system") {
    delete document.documentElement.dataset.theme;
    return;
  }

  document.documentElement.dataset.theme = theme;
}

export async function loadThemePreference(): Promise<ThemePreference> {
  try {
    const response: unknown = await invoke("get_appearance_preference");
    return parseThemePreference(response);
  } catch {
    return "system";
  }
}

export class ThemePersistenceError extends Error {
  constructor() {
    super("Appearance could not be saved.");
    this.name = "ThemePersistenceError";
  }
}

export async function saveThemePreference(theme: ThemePreference): Promise<ThemePreference> {
  applyThemePreference(theme);
  try {
    const response: unknown = await invoke("set_appearance_preference", { value: theme });
    return parseThemePreference(response);
  } catch {
    throw new ThemePersistenceError();
  }
}

export async function listenAppearanceChanged(
  onChange: (theme: ThemePreference) => void,
): Promise<UnlistenFn> {
  return listen<unknown>(appearanceChangedEvent, (event) => {
    onChange(parseThemePreference(event.payload));
  });
}
