import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { SettingsCategory } from "./commands";
import {
  getConnectionStatus,
  type ConnectionStatus,
  validateConnectionStatusExport,
} from "./provider";

const settingsNavigateEvent = "settings-navigate";
const connectionStatusChangedEvent = "connection-status-changed";

export type { SettingsCategory };

export async function openSettingsWindow(category?: SettingsCategory): Promise<void> {
  await invoke("open_settings_window", { category: category ?? null });
}

export async function takeSettingsLaunchCategory(): Promise<SettingsCategory | null> {
  const response: unknown = await invoke("take_settings_launch_category");
  if (response === "providers" || response === "appearance") {
    return response;
  }
  return null;
}

export async function exitApplication(): Promise<void> {
  await invoke("exit_application");
}

export async function syncConnectionStatus(): Promise<ConnectionStatus> {
  const response: unknown = await invoke("sync_connection_status");
  return validateConnectionStatusExport(response);
}

export async function refreshConnectionStatus(): Promise<ConnectionStatus> {
  return getConnectionStatus();
}

export async function listenSettingsNavigate(
  onNavigate: (category: SettingsCategory) => void,
): Promise<UnlistenFn> {
  return listen<unknown>(settingsNavigateEvent, (event) => {
    if (event.payload === "providers" || event.payload === "appearance") {
      onNavigate(event.payload);
    }
  });
}

export async function listenConnectionStatusChanged(
  onStatus: (status: ConnectionStatus) => void,
): Promise<UnlistenFn> {
  return listen<unknown>(connectionStatusChangedEvent, (event) => {
    try {
      onStatus(validateConnectionStatusExport(event.payload));
    } catch {
      // Ignore malformed non-secret status payloads.
    }
  });
}
