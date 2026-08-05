import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type ConnectionState =
  "disconnected" | "connecting" | "connected" | "reconnect_required" | "unavailable_in_this_build";

export interface ConnectionStatus {
  state: ConnectionState;
  providerId: string;
  model: string;
}

export interface DevicePairingInfo {
  verificationUri: string;
  userCode: string;
}

export interface ProviderModelEntry {
  id: string;
  displayName: string;
  description: string | null;
  isProviderDefault: boolean;
}

export type CatalogFreshness = "current" | "stale";

export interface ProviderModelCatalog {
  providerId: string;
  models: ProviderModelEntry[];
  freshness: CatalogFreshness;
  retrievedAtUnixMs: number | null;
  compatibilityRevision: string | null;
}

export interface ProviderModelSelection {
  providerId: string;
  selectedModelId: string | null;
  requiresSelection: boolean;
}

export type ProviderErrorCode =
  | "not_connected"
  | "invalid_input"
  | "context_limit"
  | "session_busy"
  | "authentication_required"
  | "entitlement_unavailable"
  | "rate_limited"
  | "provider_unavailable"
  | "unsupported_provider_output"
  | "output_limit"
  | "cancelled"
  | "interrupted"
  | "credential_store_unavailable"
  | "agent_storage_unavailable"
  | "model_unavailable";

const providerErrorCodes: readonly ProviderErrorCode[] = [
  "not_connected",
  "invalid_input",
  "context_limit",
  "session_busy",
  "authentication_required",
  "entitlement_unavailable",
  "rate_limited",
  "provider_unavailable",
  "unsupported_provider_output",
  "output_limit",
  "cancelled",
  "interrupted",
  "credential_store_unavailable",
  "agent_storage_unavailable",
  "model_unavailable",
];

const connectionStates: readonly ConnectionState[] = [
  "disconnected",
  "connecting",
  "connected",
  "reconnect_required",
  "unavailable_in_this_build",
];

const catalogFreshnessValues: readonly CatalogFreshness[] = ["current", "stale"];

export class ProviderError extends Error {
  readonly code: ProviderErrorCode;

  constructor(code: ProviderErrorCode) {
    super(code);
    this.name = "ProviderError";
    this.code = code;
  }
}

export function isProviderErrorCode(value: unknown): value is ProviderErrorCode {
  return typeof value === "string" && providerErrorCodes.includes(value as ProviderErrorCode);
}

function toProviderError(error: unknown): ProviderError {
  return new ProviderError(isProviderErrorCode(error) ? error : "provider_unavailable");
}

export function getProviderErrorCode(error: unknown): ProviderErrorCode {
  return error instanceof ProviderError ? error.code : "provider_unavailable";
}

function isConnectionStatus(value: unknown): value is ConnectionStatus {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return false;
  }

  return (
    "state" in value &&
    typeof value.state === "string" &&
    connectionStates.includes(value.state as ConnectionState) &&
    "providerId" in value &&
    typeof value.providerId === "string" &&
    "model" in value &&
    typeof value.model === "string"
  );
}

export function validateConnectionStatusExport(value: unknown): ConnectionStatus {
  if (!isConnectionStatus(value)) {
    throw new ProviderError("provider_unavailable");
  }

  return value;
}

function isDevicePairingInfo(value: unknown): value is DevicePairingInfo {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return false;
  }

  return (
    "verificationUri" in value &&
    typeof value.verificationUri === "string" &&
    "userCode" in value &&
    typeof value.userCode === "string"
  );
}

export function validateDevicePairingInfo(value: unknown): DevicePairingInfo | null {
  if (value === null || value === undefined) {
    return null;
  }
  if (!isDevicePairingInfo(value)) {
    throw new ProviderError("provider_unavailable");
  }
  if (value.verificationUri === "" && value.userCode === "") {
    return null;
  }
  return value;
}

function isProviderModelEntry(value: unknown): value is ProviderModelEntry {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return false;
  }

  return (
    "id" in value &&
    typeof value.id === "string" &&
    "displayName" in value &&
    typeof value.displayName === "string" &&
    "description" in value &&
    (value.description === null || typeof value.description === "string") &&
    "isProviderDefault" in value &&
    typeof value.isProviderDefault === "boolean"
  );
}

function isProviderModelCatalog(value: unknown): value is ProviderModelCatalog {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return false;
  }

  return (
    "providerId" in value &&
    typeof value.providerId === "string" &&
    "models" in value &&
    Array.isArray(value.models) &&
    value.models.every(isProviderModelEntry) &&
    "freshness" in value &&
    typeof value.freshness === "string" &&
    catalogFreshnessValues.includes(value.freshness as CatalogFreshness) &&
    "retrievedAtUnixMs" in value &&
    (value.retrievedAtUnixMs === null || typeof value.retrievedAtUnixMs === "number") &&
    "compatibilityRevision" in value &&
    (value.compatibilityRevision === null || typeof value.compatibilityRevision === "string")
  );
}

export function validateProviderModelCatalog(value: unknown): ProviderModelCatalog {
  if (!isProviderModelCatalog(value)) {
    throw new ProviderError("provider_unavailable");
  }

  return value;
}

function isProviderModelSelection(value: unknown): value is ProviderModelSelection {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return false;
  }

  return (
    "providerId" in value &&
    typeof value.providerId === "string" &&
    "selectedModelId" in value &&
    (value.selectedModelId === null || typeof value.selectedModelId === "string") &&
    "requiresSelection" in value &&
    typeof value.requiresSelection === "boolean"
  );
}

export function validateProviderModelSelection(value: unknown): ProviderModelSelection {
  if (!isProviderModelSelection(value)) {
    throw new ProviderError("provider_unavailable");
  }

  return value;
}

async function invokeProviderCommand(
  command: string,
  args?: Record<string, unknown>,
): Promise<unknown> {
  try {
    return args === undefined ? await invoke(command) : await invoke(command, args);
  } catch (error: unknown) {
    throw toProviderError(error);
  }
}

export async function getConnectionStatus(): Promise<ConnectionStatus> {
  return validateConnectionStatusExport(await invokeProviderCommand("connection_status"));
}

export async function connectXai(): Promise<ConnectionStatus> {
  return validateConnectionStatusExport(await invokeProviderCommand("connect_xai"));
}

export async function cancelXaiConnect(): Promise<void> {
  await invokeProviderCommand("cancel_xai_connect");
}

/**
 * A late Cancel after the native connect operation already finished surfaces as
 * `invalid_input`. Callers must reconcile to the authoritative connection status
 * and must not present Agent-composer validation copy.
 */
export function isStaleConnectCancellation(error: unknown): boolean {
  return getProviderErrorCode(error) === "invalid_input";
}

export async function disconnectXai(): Promise<ConnectionStatus> {
  return validateConnectionStatusExport(await invokeProviderCommand("disconnect_xai"));
}

export async function getXaiDevicePairing(): Promise<DevicePairingInfo | null> {
  return validateDevicePairingInfo(await invokeProviderCommand("get_xai_device_pairing"));
}

export async function listenXaiDevicePairingChanged(
  onPairing: (pairing: DevicePairingInfo | null) => void,
): Promise<UnlistenFn> {
  return listen("xai-device-pairing-changed", (event) => {
    onPairing(validateDevicePairingInfo(event.payload));
  });
}

export async function getProviderModelCatalog(): Promise<ProviderModelCatalog> {
  return validateProviderModelCatalog(await invokeProviderCommand("get_provider_model_catalog"));
}

/** Cache-only recovery after a failed automatic refresh; never performs network I/O. */
export async function getPersistedProviderModelCatalog(): Promise<ProviderModelCatalog> {
  return validateProviderModelCatalog(
    await invokeProviderCommand("get_persisted_provider_model_catalog"),
  );
}

export async function refreshProviderModelCatalog(): Promise<ProviderModelCatalog> {
  return validateProviderModelCatalog(
    await invokeProviderCommand("refresh_provider_model_catalog"),
  );
}

export async function getProviderModelSelection(): Promise<ProviderModelSelection> {
  return validateProviderModelSelection(
    await invokeProviderCommand("get_provider_model_selection"),
  );
}

export async function setProviderModelSelection(modelId: string): Promise<ProviderModelSelection> {
  return validateProviderModelSelection(
    await invokeProviderCommand("set_provider_model_selection", { modelId }),
  );
}

export async function listenProviderModelCatalogChanged(
  onCatalog: (catalog: ProviderModelCatalog) => void,
): Promise<UnlistenFn> {
  return listen("provider-model-catalog-changed", (event) => {
    onCatalog(validateProviderModelCatalog(event.payload));
  });
}

export async function listenProviderModelSelectionChanged(
  onSelection: (selection: ProviderModelSelection) => void,
): Promise<UnlistenFn> {
  return listen("provider-model-selection-changed", (event) => {
    onSelection(validateProviderModelSelection(event.payload));
  });
}

export function formatModelLabel(modelId: string, catalog: readonly ProviderModelEntry[]): string {
  const match = catalog.find((entry) => entry.id === modelId);
  return match?.displayName ?? modelId;
}
