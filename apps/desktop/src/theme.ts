import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type ThemePreference = "system" | "light" | "dark";

const appearanceChangedEvent = "appearance-changed";
const legacyThemeStorageKey = "tule-theme";

function isThemePreference(value: unknown): value is ThemePreference {
  return value === "system" || value === "light" || value === "dark";
}

export function parseThemePreference(value: unknown): ThemePreference {
  return isThemePreference(value) ? value : "system";
}

function retireLegacyThemePreference(): void {
  try {
    window.localStorage.removeItem(legacyThemeStorageKey);
  } catch {
    // Best-effort retirement only; native storage remains authoritative.
  }
}

export function applyThemePreference(theme: ThemePreference): void {
  if (theme === "system") {
    delete document.documentElement.dataset.theme;
    return;
  }

  document.documentElement.dataset.theme = theme;
}

async function persistThemePreference(theme: ThemePreference): Promise<ThemePreference> {
  const response: unknown = await invoke("set_appearance_preference", { value: theme });
  return parseThemePreference(response);
}

export async function loadThemePreference(): Promise<ThemePreference> {
  let native: ThemePreference = "system";
  try {
    const response: unknown = await invoke("get_appearance_preference");
    native = parseThemePreference(response);
  } catch {
    native = "system";
  }

  let legacy: ThemePreference | null = null;
  try {
    const rawLegacy = window.localStorage.getItem(legacyThemeStorageKey);
    if (rawLegacy === null) {
      return native;
    }
    legacy = isThemePreference(rawLegacy) ? rawLegacy : null;
    if (legacy === null) {
      // Retire unrecognized leftover keys without writing native storage.
      retireLegacyThemePreference();
      return native;
    }
  } catch {
    return native;
  }

  // Upgrade path: import a valid legacy value once when native still looks unset,
  // then retire localStorage so the webview never re-owns durable preference storage.
  if (native === "system") {
    if (legacy !== "system") {
      try {
        const imported = await persistThemePreference(legacy);
        retireLegacyThemePreference();
        return imported;
      } catch {
        return legacy;
      }
    }
    retireLegacyThemePreference();
    return native;
  }

  retireLegacyThemePreference();
  return native;
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
    return await persistThemePreference(theme);
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
