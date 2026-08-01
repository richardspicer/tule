export type ThemePreference = "system" | "light" | "dark";

const themeOrder: readonly ThemePreference[] = ["system", "light", "dark"];
const themeStorageKey = "tule-theme";

export function loadThemePreference(): ThemePreference {
  const savedTheme = window.localStorage.getItem(themeStorageKey);
  return savedTheme === "light" || savedTheme === "dark" ? savedTheme : "system";
}

export function getNextThemePreference(theme: ThemePreference): ThemePreference {
  const nextIndex = (themeOrder.indexOf(theme) + 1) % themeOrder.length;
  return themeOrder[nextIndex] ?? "system";
}

export function applyThemePreference(theme: ThemePreference): void {
  if (theme === "system") {
    delete document.documentElement.dataset.theme;
    window.localStorage.removeItem(themeStorageKey);
    return;
  }

  document.documentElement.dataset.theme = theme;
  window.localStorage.setItem(themeStorageKey, theme);
}
