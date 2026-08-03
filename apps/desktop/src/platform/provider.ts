import { invoke } from "@tauri-apps/api/core";

export type ConnectionState =
  "disconnected" | "connecting" | "connected" | "reconnect_required" | "unavailable_in_this_build";

export interface ConnectionStatus {
  state: ConnectionState;
  providerId: string;
  model: string;
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
  | "agent_storage_unavailable";

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
];

const connectionStates: readonly ConnectionState[] = [
  "disconnected",
  "connecting",
  "connected",
  "reconnect_required",
  "unavailable_in_this_build",
];

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

async function invokeProviderCommand(command: string): Promise<unknown> {
  try {
    return await invoke(command);
  } catch (error: unknown) {
    throw toProviderError(error);
  }
}

export async function getConnectionStatus(): Promise<ConnectionStatus> {
  return validateConnectionStatusExport(await invokeProviderCommand("connection_status"));
}

export async function connectChatgpt(): Promise<ConnectionStatus> {
  return validateConnectionStatusExport(await invokeProviderCommand("connect_chatgpt"));
}

export async function cancelChatgptConnect(): Promise<void> {
  await invokeProviderCommand("cancel_chatgpt_connect");
}

export async function disconnectChatgpt(): Promise<ConnectionStatus> {
  return validateConnectionStatusExport(await invokeProviderCommand("disconnect_chatgpt"));
}
