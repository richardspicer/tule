import { Channel, invoke } from "@tauri-apps/api/core";
import { getProviderErrorCode, type ProviderErrorCode, ProviderError } from "./provider";

export interface AgentSession {
  id: string;
  title: string;
  projectId: string | null;
  modelId: string;
}

export interface AgentTurn {
  id: string;
  ordinal: number;
  userText: string;
  agentText: string;
  state: string;
  errorCode: string | null;
}

export interface AgentSessionDetail {
  session: AgentSession;
  turns: AgentTurn[];
}

export type AgentStreamEvent =
  | { kind: "started"; session_id: string; turn_id: string }
  | { kind: "delta"; turn_id: string; text: string }
  | { kind: "terminal"; turn: AgentTurn };

export type AgentErrorCode = ProviderErrorCode;

export class AgentError extends Error {
  readonly code: AgentErrorCode;

  constructor(code: AgentErrorCode) {
    super(code);
    this.name = "AgentError";
    this.code = code;
  }
}

export function getAgentErrorCode(error: unknown): AgentErrorCode {
  if (error instanceof AgentError) {
    return error.code;
  }

  return getProviderErrorCode(error);
}

function toAgentError(error: unknown): AgentError {
  return new AgentError(getAgentErrorCode(error));
}

function isAgentSession(value: unknown): value is AgentSession {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return false;
  }

  return (
    "id" in value &&
    typeof value.id === "string" &&
    "title" in value &&
    typeof value.title === "string" &&
    "projectId" in value &&
    (value.projectId === null || typeof value.projectId === "string") &&
    "modelId" in value &&
    typeof value.modelId === "string"
  );
}

function isAgentTurn(value: unknown): value is AgentTurn {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return false;
  }

  return (
    "id" in value &&
    typeof value.id === "string" &&
    "ordinal" in value &&
    typeof value.ordinal === "number" &&
    "userText" in value &&
    typeof value.userText === "string" &&
    "agentText" in value &&
    typeof value.agentText === "string" &&
    "state" in value &&
    typeof value.state === "string" &&
    "errorCode" in value &&
    (value.errorCode === null || typeof value.errorCode === "string")
  );
}

function validateSession(value: unknown): AgentSession {
  if (!isAgentSession(value)) {
    throw new AgentError("agent_storage_unavailable");
  }

  return value;
}

function validateTurn(value: unknown): AgentTurn {
  if (!isAgentTurn(value)) {
    throw new AgentError("agent_storage_unavailable");
  }

  return value;
}

function validateSessionList(value: unknown): AgentSession[] {
  if (!Array.isArray(value) || !value.every(isAgentSession)) {
    throw new AgentError("agent_storage_unavailable");
  }

  return value;
}

function validateSessionDetail(value: unknown): AgentSessionDetail {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new AgentError("agent_storage_unavailable");
  }

  if (!("session" in value) || !("turns" in value) || !Array.isArray(value.turns)) {
    throw new AgentError("agent_storage_unavailable");
  }

  return {
    session: validateSession(value.session),
    turns: value.turns.map(validateTurn),
  };
}

function isAgentStreamEvent(value: unknown): value is AgentStreamEvent {
  if (typeof value !== "object" || value === null || !("kind" in value)) {
    return false;
  }

  if (value.kind === "started") {
    return (
      "session_id" in value &&
      typeof value.session_id === "string" &&
      "turn_id" in value &&
      typeof value.turn_id === "string"
    );
  }

  if (value.kind === "delta") {
    return (
      "turn_id" in value &&
      typeof value.turn_id === "string" &&
      "text" in value &&
      typeof value.text === "string"
    );
  }

  if (value.kind === "terminal") {
    return "turn" in value && isAgentTurn(value.turn);
  }

  return false;
}

async function invokeAgentCommand(
  command: string,
  args?: Record<string, unknown>,
): Promise<unknown> {
  try {
    return args === undefined ? await invoke(command) : await invoke(command, args);
  } catch (error: unknown) {
    throw toAgentError(error);
  }
}

export async function listAgentSessions(): Promise<AgentSession[]> {
  return validateSessionList(await invokeAgentCommand("list_agent_sessions"));
}

export async function getAgentSession(sessionId: string): Promise<AgentSessionDetail> {
  return validateSessionDetail(await invokeAgentCommand("get_agent_session", { sessionId }));
}

export async function setAgentSessionProject(
  sessionId: string,
  projectId: string | null,
): Promise<AgentSession> {
  return validateSession(
    await invokeAgentCommand("set_agent_session_project", { sessionId, projectId }),
  );
}

export async function cancelAgentTurn(turnId: string): Promise<void> {
  await invokeAgentCommand("cancel_agent_turn", { turnId });
}

export async function sendAgentMessage(options: {
  sessionId: string | null;
  userText: string;
  projectId: string | null;
  onEvent: (event: AgentStreamEvent) => void;
}): Promise<void> {
  const channel = new Channel<unknown>((payload) => {
    if (!isAgentStreamEvent(payload)) {
      throw new AgentError("provider_unavailable");
    }

    options.onEvent(payload);
  });

  try {
    await invoke("send_agent_message", {
      sessionId: options.sessionId,
      userText: options.userText,
      projectId: options.projectId,
      channel,
    });
  } catch (error: unknown) {
    if (error instanceof ProviderError || error instanceof AgentError) {
      throw toAgentError(error);
    }

    throw toAgentError(error);
  }
}

export function getSafeAgentErrorMessage(error: unknown): string {
  switch (getAgentErrorCode(error)) {
    case "not_connected":
      return "Connect ChatGPT in Settings to message the Agent.";
    case "invalid_input":
      return "Enter a valid message.";
    case "context_limit":
      return "This conversation is too large to send.";
    case "session_busy":
      return "Another Agent turn is already in progress.";
    case "authentication_required":
      return "Reconnect ChatGPT in Settings to continue.";
    case "entitlement_unavailable":
      return "ChatGPT access is unavailable for this account.";
    case "rate_limited":
      return "The provider rate limit was reached. Try again later.";
    case "unsupported_provider_output":
      return "The provider returned unsupported output.";
    case "output_limit":
      return "The Agent response reached the local size limit.";
    case "cancelled":
      return "TULE stopped receiving the response.";
    case "interrupted":
      return "The previous turn was interrupted.";
    case "credential_store_unavailable":
      return "Credential storage is unavailable on this device.";
    case "agent_storage_unavailable":
      return "Agent storage is unavailable. Try again.";
    case "provider_unavailable":
      return "The provider is unavailable. Try again.";
  }
}
