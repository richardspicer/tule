import { Channel, invoke } from "@tauri-apps/api/core";
import {
  getProviderErrorCode,
  isProviderErrorCode,
  type ProviderErrorCode,
  ProviderError,
} from "./provider";

export interface AgentSession {
  id: string;
  title: string;
  projectId: string | null;
  modelId: string;
}

export type AgentTurnState =
  "pending" | "streaming" | "completed" | "cancelled" | "failed" | "interrupted";

const agentTurnStates: readonly AgentTurnState[] = [
  "pending",
  "streaming",
  "completed",
  "cancelled",
  "failed",
  "interrupted",
];

export type SourceOriginKind = "local_text_file";

export interface AgentSourceMetadata {
  id: string;
  originKind: SourceOriginKind;
  displayName: string;
  byteCount: number;
  contentSha256: string;
}

export interface AgentTurn {
  id: string;
  ordinal: number;
  userText: string;
  agentText: string;
  state: AgentTurnState;
  errorCode: AgentErrorCode | null;
  sources: AgentSourceMetadata[];
}

export interface AgentSessionDetail {
  session: AgentSession;
  turns: AgentTurn[];
}

export type AgentStreamEvent =
  | { kind: "started"; session_id: string; turn_id: string }
  | { kind: "delta"; turn_id: string; text: string }
  | { kind: "terminal"; turn: AgentTurn };

export type AgentSourceErrorCode =
  "source_unreadable" | "source_unsupported" | "source_too_large" | "source_draft_expired";

export type AgentErrorCode = ProviderErrorCode | AgentSourceErrorCode;

const agentSourceErrorCodes: readonly AgentSourceErrorCode[] = [
  "source_unreadable",
  "source_unsupported",
  "source_too_large",
  "source_draft_expired",
];

export interface PendingSourceAttachment {
  draftHandle: string;
  displayName: string;
  byteCount: number;
  originKind: SourceOriginKind;
}

export type PickAgentTextSourceResult =
  { status: "cancelled" } | { status: "selected"; attachment: PendingSourceAttachment };

export class AgentError extends Error {
  readonly code: AgentErrorCode;

  constructor(code: AgentErrorCode) {
    super(code);
    this.name = "AgentError";
    this.code = code;
  }
}

function isAgentSourceErrorCode(value: unknown): value is AgentSourceErrorCode {
  return typeof value === "string" && agentSourceErrorCodes.includes(value as AgentSourceErrorCode);
}

function isAgentErrorCode(value: unknown): value is AgentErrorCode {
  return isProviderErrorCode(value) || isAgentSourceErrorCode(value);
}

function extractErrorCode(error: unknown): unknown {
  if (typeof error === "string") {
    return error;
  }

  if (typeof error === "object" && error !== null) {
    if ("code" in error) {
      return error.code;
    }

    if ("message" in error && typeof error.message === "string") {
      return error.message;
    }
  }

  return undefined;
}

export function getAgentErrorCode(error: unknown): AgentErrorCode {
  if (error instanceof AgentError) {
    return error.code;
  }

  const code = extractErrorCode(error);
  if (isAgentErrorCode(code)) {
    return code;
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

function isSourceMetadata(value: unknown): value is AgentSourceMetadata {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return false;
  }

  return (
    "id" in value &&
    typeof value.id === "string" &&
    "originKind" in value &&
    value.originKind === "local_text_file" &&
    "displayName" in value &&
    typeof value.displayName === "string" &&
    "byteCount" in value &&
    typeof value.byteCount === "number" &&
    Number.isFinite(value.byteCount) &&
    value.byteCount >= 0 &&
    "contentSha256" in value &&
    typeof value.contentSha256 === "string" &&
    /^[0-9a-f]{64}$/.test(value.contentSha256)
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
    agentTurnStates.includes(value.state as AgentTurnState) &&
    "errorCode" in value &&
    (value.errorCode === null || isAgentErrorCode(value.errorCode)) &&
    "sources" in value &&
    Array.isArray(value.sources) &&
    value.sources.every(isSourceMetadata)
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

function validatePickResult(value: unknown): PickAgentTextSourceResult {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new AgentError("agent_storage_unavailable");
  }

  const record = value as Record<string, unknown>;
  const keys = Object.keys(record).sort();
  if (record.status === "cancelled") {
    const allowed = ["byteCount", "displayName", "draftHandle", "originKind", "status"];
    if (!keys.every((key) => allowed.includes(key))) {
      throw new AgentError("agent_storage_unavailable");
    }
    return { status: "cancelled" };
  }

  if (
    record.status === "selected" &&
    typeof record.draftHandle === "string" &&
    record.draftHandle.length > 0 &&
    typeof record.displayName === "string" &&
    typeof record.byteCount === "number" &&
    Number.isFinite(record.byteCount) &&
    record.byteCount >= 0 &&
    record.originKind === "local_text_file" &&
    keys.every((key) =>
      ["byteCount", "displayName", "draftHandle", "originKind", "status"].includes(key),
    )
  ) {
    return {
      status: "selected",
      attachment: {
        draftHandle: record.draftHandle,
        displayName: record.displayName,
        byteCount: record.byteCount,
        originKind: "local_text_file",
      },
    };
  }

  throw new AgentError("agent_storage_unavailable");
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

export async function pickAgentTextSource(): Promise<PickAgentTextSourceResult> {
  return validatePickResult(await invokeAgentCommand("pick_agent_text_source"));
}

export async function clearAgentTextSourceDraft(draftHandle: string | null): Promise<void> {
  await invokeAgentCommand("clear_agent_text_source_draft", { draftHandle });
}

export async function setAgentSourceDraftScope(scopeKey: string): Promise<void> {
  await invokeAgentCommand("set_agent_source_draft_scope", { scopeKey });
}

export async function sendAgentMessage(options: {
  sessionId: string | null;
  userText: string;
  projectId: string | null;
  modelId: string | null;
  sourceDraftHandle: string | null;
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
      modelId: options.modelId,
      sourceDraftHandle: options.sourceDraftHandle,
      channel,
    });
  } catch (error: unknown) {
    if (error instanceof ProviderError || error instanceof AgentError) {
      throw toAgentError(error);
    }

    throw toAgentError(error);
  }
}

export function getSafeAgentErrorMessageForCode(code: AgentErrorCode): string {
  switch (code) {
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
    case "model_unavailable":
      return "That model is unavailable. Choose another model for a new session.";
    case "provider_unavailable":
      return "The provider is unavailable. Try again.";
    case "source_unreadable":
      return "That file could not be read.";
    case "source_unsupported":
      return "That file is not supported as a text attachment.";
    case "source_too_large":
      return "That file is too large to attach.";
    case "source_draft_expired":
      return "The attachment is no longer available. Choose the file again.";
  }
}

export function getSafeAgentErrorMessage(error: unknown): string {
  return getSafeAgentErrorMessageForCode(getAgentErrorCode(error));
}

export function formatSourceByteCount(byteCount: number): string {
  if (byteCount < 1024) {
    return `${byteCount} B`;
  }

  return `${(byteCount / 1024).toFixed(byteCount < 10 * 1024 ? 1 : 0)} KB`;
}
