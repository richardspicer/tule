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

export type SourceOriginKind = "local_text_file" | "local_text_folder" | "remote_text_url";

export interface AgentSourceMetadata {
  id: string;
  originKind: SourceOriginKind;
  displayName: string;
  byteCount: number;
  contentSha256: string;
  memberCount: number;
  canonicalUrl: string | null;
}

export type AgentEffort = "low" | "medium" | "high";

const agentEfforts: readonly AgentEffort[] = ["low", "medium", "high"];

export interface AgentTurn {
  id: string;
  ordinal: number;
  userText: string;
  agentText: string;
  state: AgentTurnState;
  errorCode: AgentErrorCode | null;
  effort: AgentEffort | null;
  startedAtUnixMs: number;
  finishedAtUnixMs: number | null;
  usageInputTokens: number | null;
  usageOutputTokens: number | null;
  sources: AgentSourceMetadata[];
}

/** Durable per-turn metrics snapshot returned by typed export IPC. */
export interface AgentTurnMetricsExport {
  turn_id: string;
  session_id: string;
  ordinal: number;
  state: AgentTurnState;
  provider_profile_id: string;
  model_id: string;
  effort: AgentEffort | null;
  started_at_unix_ms: number;
  finished_at_unix_ms: number | null;
  duration_ms: number | null;
  usage_input_tokens: number | null;
  usage_output_tokens: number | null;
}

export interface ModelRequestControls {
  modelId: string;
  effortAvailable: boolean;
  effortValues: AgentEffort[];
  effortDefault: AgentEffort | null;
  speedAvailable: boolean;
}

export type AgentEventKind =
  | "session_created"
  | "project_association_changed"
  | "turn_pending"
  | "turn_streaming"
  | "turn_completed"
  | "turn_cancelled"
  | "turn_failed"
  | "turn_interrupted";

const agentEventKinds: readonly AgentEventKind[] = [
  "session_created",
  "project_association_changed",
  "turn_pending",
  "turn_streaming",
  "turn_completed",
  "turn_cancelled",
  "turn_failed",
  "turn_interrupted",
];

export interface AgentEvent {
  id: string;
  sessionId: string;
  turnId: string | null;
  sequence: number;
  kind: AgentEventKind;
  createdAtUnixMs: number;
}

export interface AgentSessionDetail {
  session: AgentSession;
  turns: AgentTurn[];
  events: AgentEvent[];
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
  memberCount: number;
  canonicalUrl: string | null;
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

const MAX_SOURCE_UTF8 = 64 * 1024;
const MAX_FOLDER_MEMBERS = 32;
const MAX_CANONICAL_URL_UTF8 = 2048;
const SOURCE_METADATA_KEYS = [
  "byteCount",
  "canonicalUrl",
  "contentSha256",
  "displayName",
  "id",
  "memberCount",
  "originKind",
] as const;
const PICK_RESULT_KEYS = [
  "byteCount",
  "canonicalUrl",
  "displayName",
  "draftHandle",
  "memberCount",
  "originKind",
  "status",
] as const;
const EVENT_METADATA_KEYS = [
  "createdAtUnixMs",
  "id",
  "kind",
  "sequence",
  "sessionId",
  "turnId",
] as const;
const TURN_METRICS_EXPORT_KEYS = [
  "duration_ms",
  "effort",
  "finished_at_unix_ms",
  "model_id",
  "ordinal",
  "provider_profile_id",
  "session_id",
  "started_at_unix_ms",
  "state",
  "turn_id",
  "usage_input_tokens",
  "usage_output_tokens",
] as const;
/** ECMAScript Date time-value maximum (ms since Unix epoch). */
const MAX_DATE_UNIX_MS = 8_640_000_000_000_000;

function isUuidV7(value: string): boolean {
  return /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(value);
}

function isCanonicalSha256(value: string): boolean {
  return /^[0-9a-f]{64}$/.test(value);
}

function isDraftHandle(value: string): boolean {
  return /^[0-9a-f]{32}$/.test(value);
}

function isSafeSourceDisplayName(value: string): boolean {
  if (value.length === 0) {
    return false;
  }

  for (const char of value) {
    const code = char.codePointAt(0);
    if (code === undefined) {
      return false;
    }
    if (
      code <= 0x1f ||
      code === 0x7f ||
      (code >= 0x80 && code <= 0x9f) ||
      code === 0x061c ||
      (code >= 0x200e && code <= 0x200f) ||
      (code >= 0x202a && code <= 0x202e) ||
      (code >= 0x2028 && code <= 0x2029) ||
      (code >= 0x2066 && code <= 0x2069)
    ) {
      return false;
    }
  }

  return true;
}

function isSourceByteCount(value: unknown): value is number {
  return (
    typeof value === "number" && Number.isInteger(value) && value >= 0 && value <= MAX_SOURCE_UTF8
  );
}

function hasExactKeys(record: Record<string, unknown>, allowed: readonly string[]): boolean {
  const keys = Object.keys(record).sort();
  return keys.length === allowed.length && keys.every((key, index) => key === allowed[index]);
}

function isCanonicalHttpsUrl(value: string): boolean {
  if (value.length === 0) {
    return false;
  }
  const utf8Length = new TextEncoder().encode(value).length;
  if (utf8Length > MAX_CANONICAL_URL_UTF8) {
    return false;
  }
  if (!value.startsWith("https://")) {
    return false;
  }
  const afterScheme = value.slice("https://".length);
  const hostEnd = (() => {
    const slash = afterScheme.indexOf("/");
    const query = afterScheme.indexOf("?");
    const fragment = afterScheme.indexOf("#");
    const candidates = [slash, query, fragment].filter((index) => index !== -1);
    return candidates.length === 0 ? afterScheme.length : Math.min(...candidates);
  })();
  if (afterScheme.slice(0, hostEnd).length === 0) {
    return false;
  }
  if (value.includes("@")) {
    return false;
  }
  for (const char of value) {
    const code = char.codePointAt(0);
    if (code === undefined) {
      return false;
    }
    if (code <= 0x1f || code === 0x7f || (code >= 0x80 && code <= 0x9f)) {
      return false;
    }
  }
  return true;
}

function isSourceOriginKind(value: unknown): value is SourceOriginKind {
  return (
    value === "local_text_file" || value === "local_text_folder" || value === "remote_text_url"
  );
}

function isMemberCount(value: unknown, originKind: SourceOriginKind): value is number {
  if (typeof value !== "number" || !Number.isInteger(value)) {
    return false;
  }
  if (originKind === "local_text_file" || originKind === "remote_text_url") {
    return value === 1;
  }
  return value >= 1 && value <= MAX_FOLDER_MEMBERS;
}

function isCanonicalUrlField(value: unknown, originKind: SourceOriginKind): value is string | null {
  if (originKind === "remote_text_url") {
    return typeof value === "string" && isCanonicalHttpsUrl(value);
  }
  return value === null;
}

function isSourceMetadata(value: unknown): value is AgentSourceMetadata {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return false;
  }

  const record = value as Record<string, unknown>;
  return (
    hasExactKeys(record, SOURCE_METADATA_KEYS) &&
    typeof record.id === "string" &&
    isUuidV7(record.id) &&
    isSourceOriginKind(record.originKind) &&
    typeof record.displayName === "string" &&
    isSafeSourceDisplayName(record.displayName) &&
    isSourceByteCount(record.byteCount) &&
    typeof record.contentSha256 === "string" &&
    isCanonicalSha256(record.contentSha256) &&
    isMemberCount(record.memberCount, record.originKind) &&
    isCanonicalUrlField(record.canonicalUrl, record.originKind)
  );
}

function isAgentEventKind(value: unknown): value is AgentEventKind {
  return typeof value === "string" && agentEventKinds.includes(value as AgentEventKind);
}

function isEventSequence(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
}

function isEventTimestamp(value: unknown): value is number {
  return (
    typeof value === "number" &&
    Number.isSafeInteger(value) &&
    value >= 0 &&
    value <= MAX_DATE_UNIX_MS
  );
}

function isAgentEvent(value: unknown): value is AgentEvent {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return false;
  }

  const record = value as Record<string, unknown>;
  return (
    hasExactKeys(record, EVENT_METADATA_KEYS) &&
    typeof record.id === "string" &&
    isUuidV7(record.id) &&
    typeof record.sessionId === "string" &&
    isUuidV7(record.sessionId) &&
    (record.turnId === null || (typeof record.turnId === "string" && isUuidV7(record.turnId))) &&
    isEventSequence(record.sequence) &&
    isAgentEventKind(record.kind) &&
    isEventTimestamp(record.createdAtUnixMs)
  );
}

function isAgentEffort(value: unknown): value is AgentEffort {
  return typeof value === "string" && agentEfforts.includes(value as AgentEffort);
}

function isTokenCount(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
}

function isNullableTokenCount(value: unknown): value is number | null {
  return value === null || isTokenCount(value);
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
    "effort" in value &&
    (value.effort === null || isAgentEffort(value.effort)) &&
    "startedAtUnixMs" in value &&
    isEventTimestamp(value.startedAtUnixMs) &&
    "finishedAtUnixMs" in value &&
    (value.finishedAtUnixMs === null || isEventTimestamp(value.finishedAtUnixMs)) &&
    "usageInputTokens" in value &&
    isNullableTokenCount(value.usageInputTokens) &&
    "usageOutputTokens" in value &&
    isNullableTokenCount(value.usageOutputTokens) &&
    "sources" in value &&
    Array.isArray(value.sources) &&
    value.sources.every(isSourceMetadata)
  );
}

function isAgentTurnMetricsExport(value: unknown): value is AgentTurnMetricsExport {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return false;
  }
  const record = value as Record<string, unknown>;
  return (
    hasExactKeys(record, TURN_METRICS_EXPORT_KEYS) &&
    typeof record.turn_id === "string" &&
    isUuidV7(record.turn_id) &&
    typeof record.session_id === "string" &&
    isUuidV7(record.session_id) &&
    typeof record.ordinal === "number" &&
    Number.isSafeInteger(record.ordinal) &&
    record.ordinal >= 0 &&
    typeof record.state === "string" &&
    agentTurnStates.includes(record.state as AgentTurnState) &&
    typeof record.provider_profile_id === "string" &&
    record.provider_profile_id.length > 0 &&
    typeof record.model_id === "string" &&
    record.model_id.length > 0 &&
    (record.effort === null || isAgentEffort(record.effort)) &&
    isEventTimestamp(record.started_at_unix_ms) &&
    (record.finished_at_unix_ms === null || isEventTimestamp(record.finished_at_unix_ms)) &&
    (record.duration_ms === null ||
      (typeof record.duration_ms === "number" &&
        Number.isSafeInteger(record.duration_ms) &&
        record.duration_ms >= 0)) &&
    isNullableTokenCount(record.usage_input_tokens) &&
    isNullableTokenCount(record.usage_output_tokens)
  );
}

function validateTurnMetricsExport(value: unknown): AgentTurnMetricsExport {
  if (!isAgentTurnMetricsExport(value)) {
    throw new AgentError("agent_storage_unavailable");
  }
  return value;
}

function isModelRequestControls(value: unknown): value is ModelRequestControls {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return false;
  }
  const record = value as Record<string, unknown>;
  return (
    typeof record.modelId === "string" &&
    record.modelId.length > 0 &&
    typeof record.effortAvailable === "boolean" &&
    Array.isArray(record.effortValues) &&
    record.effortValues.every(isAgentEffort) &&
    (record.effortDefault === null || isAgentEffort(record.effortDefault)) &&
    typeof record.speedAvailable === "boolean" &&
    record.speedAvailable === false &&
    (record.effortAvailable
      ? record.effortValues.length === 3 &&
        record.effortDefault !== null &&
        record.effortValues.includes(record.effortDefault)
      : record.effortValues.length === 0 && record.effortDefault === null)
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

function validateEvent(value: unknown): AgentEvent {
  if (!isAgentEvent(value)) {
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

  if (!("events" in value) || !Array.isArray(value.events)) {
    throw new AgentError("agent_storage_unavailable");
  }

  return {
    session: validateSession(value.session),
    turns: value.turns.map(validateTurn),
    events: value.events.map(validateEvent),
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
  if (!hasExactKeys(record, PICK_RESULT_KEYS)) {
    throw new AgentError("agent_storage_unavailable");
  }

  if (record.status === "cancelled") {
    if (
      record.draftHandle !== null ||
      record.displayName !== null ||
      record.byteCount !== null ||
      record.originKind !== null ||
      record.memberCount !== null ||
      record.canonicalUrl !== null
    ) {
      throw new AgentError("agent_storage_unavailable");
    }
    return { status: "cancelled" };
  }

  if (
    record.status === "selected" &&
    typeof record.draftHandle === "string" &&
    isDraftHandle(record.draftHandle) &&
    typeof record.displayName === "string" &&
    isSafeSourceDisplayName(record.displayName) &&
    isSourceByteCount(record.byteCount) &&
    isSourceOriginKind(record.originKind) &&
    isMemberCount(record.memberCount, record.originKind) &&
    isCanonicalUrlField(record.canonicalUrl, record.originKind)
  ) {
    return {
      status: "selected",
      attachment: {
        draftHandle: record.draftHandle,
        displayName: record.displayName,
        byteCount: record.byteCount,
        originKind: record.originKind,
        memberCount: record.memberCount,
        canonicalUrl: record.canonicalUrl,
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

export async function pickAgentTextFolderSource(): Promise<PickAgentTextSourceResult> {
  return validatePickResult(await invokeAgentCommand("pick_agent_text_folder_source"));
}

export async function attachAgentTextLinkSource(url: string): Promise<PickAgentTextSourceResult> {
  return validatePickResult(await invokeAgentCommand("attach_agent_text_link_source", { url }));
}

export async function clearAgentTextSourceDraft(draftHandle: string | null): Promise<void> {
  await invokeAgentCommand("clear_agent_text_source_draft", { draftHandle });
}

export async function setAgentSourceDraftScope(sessionId: string | null): Promise<void> {
  await invokeAgentCommand("set_agent_source_draft_scope", { sessionId });
}

export type ArtifactKind =
  | "conclusion"
  | "recommendation"
  | "decision_record"
  | "requirements"
  | "implementation_plan"
  | "research_brief"
  | "critique";

const artifactKinds: readonly ArtifactKind[] = [
  "conclusion",
  "recommendation",
  "decision_record",
  "requirements",
  "implementation_plan",
  "research_brief",
  "critique",
];

export interface ArtifactVersionProvenance {
  sourceSessionId: string;
  sourceTurnId: string;
  providerProfileId: string;
  modelId: string;
  promptVersion: string;
  projectId: string | null;
  providerRequestId: string;
}

export interface ArtifactVersion {
  id: string;
  artifactId: string;
  versionOrdinal: number;
  content: string;
  contentSha256: string;
  provenance: ArtifactVersionProvenance;
  createdAtUnixMs: number;
}

export interface Artifact {
  id: string;
  title: string;
  kind: ArtifactKind;
  projectId: string | null;
  createdAtUnixMs: number;
}

export interface ArtifactSummary {
  id: string;
  title: string;
  kind: ArtifactKind;
  projectId: string | null;
  createdAtUnixMs: number;
  latestVersionId: string;
  latestVersionOrdinal: number;
}

export interface ArtifactDetail {
  artifact: Artifact;
  versions: ArtifactVersion[];
}

export interface CreateArtifactResult {
  artifact: Artifact;
  version: ArtifactVersion;
}

const ARTIFACT_SUMMARY_KEYS = [
  "createdAtUnixMs",
  "id",
  "kind",
  "latestVersionId",
  "latestVersionOrdinal",
  "projectId",
  "title",
] as const;

const ARTIFACT_KEYS = ["createdAtUnixMs", "id", "kind", "projectId", "title"] as const;

const ARTIFACT_PROVENANCE_KEYS = [
  "modelId",
  "projectId",
  "promptVersion",
  "providerProfileId",
  "providerRequestId",
  "sourceSessionId",
  "sourceTurnId",
] as const;

const ARTIFACT_VERSION_KEYS = [
  "artifactId",
  "content",
  "contentSha256",
  "createdAtUnixMs",
  "id",
  "provenance",
  "versionOrdinal",
] as const;

function isArtifactKind(value: unknown): value is ArtifactKind {
  return typeof value === "string" && artifactKinds.includes(value as ArtifactKind);
}

function isArtifactTimestamp(value: unknown): value is number {
  return isEventTimestamp(value);
}

function isVersionOrdinal(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 1;
}

function isArtifactProvenance(value: unknown): value is ArtifactVersionProvenance {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return false;
  }
  const record = value as Record<string, unknown>;
  return (
    hasExactKeys(record, ARTIFACT_PROVENANCE_KEYS) &&
    typeof record.sourceSessionId === "string" &&
    isUuidV7(record.sourceSessionId) &&
    typeof record.sourceTurnId === "string" &&
    isUuidV7(record.sourceTurnId) &&
    typeof record.providerProfileId === "string" &&
    record.providerProfileId.length > 0 &&
    typeof record.modelId === "string" &&
    record.modelId.length > 0 &&
    typeof record.promptVersion === "string" &&
    record.promptVersion.length > 0 &&
    (record.projectId === null ||
      (typeof record.projectId === "string" && isUuidV7(record.projectId))) &&
    typeof record.providerRequestId === "string" &&
    isUuidV7(record.providerRequestId)
  );
}

function isArtifact(value: unknown): value is Artifact {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return false;
  }
  const record = value as Record<string, unknown>;
  return (
    hasExactKeys(record, ARTIFACT_KEYS) &&
    typeof record.id === "string" &&
    isUuidV7(record.id) &&
    typeof record.title === "string" &&
    record.title.trim().length > 0 &&
    isArtifactKind(record.kind) &&
    (record.projectId === null ||
      (typeof record.projectId === "string" && isUuidV7(record.projectId))) &&
    isArtifactTimestamp(record.createdAtUnixMs)
  );
}

function isArtifactVersion(value: unknown): value is ArtifactVersion {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return false;
  }
  const record = value as Record<string, unknown>;
  return (
    hasExactKeys(record, ARTIFACT_VERSION_KEYS) &&
    typeof record.id === "string" &&
    isUuidV7(record.id) &&
    typeof record.artifactId === "string" &&
    isUuidV7(record.artifactId) &&
    isVersionOrdinal(record.versionOrdinal) &&
    typeof record.content === "string" &&
    record.content.length > 0 &&
    typeof record.contentSha256 === "string" &&
    isCanonicalSha256(record.contentSha256) &&
    isArtifactProvenance(record.provenance) &&
    isArtifactTimestamp(record.createdAtUnixMs)
  );
}

function isArtifactSummary(value: unknown): value is ArtifactSummary {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return false;
  }
  const record = value as Record<string, unknown>;
  return (
    hasExactKeys(record, ARTIFACT_SUMMARY_KEYS) &&
    typeof record.id === "string" &&
    isUuidV7(record.id) &&
    typeof record.title === "string" &&
    record.title.trim().length > 0 &&
    isArtifactKind(record.kind) &&
    (record.projectId === null ||
      (typeof record.projectId === "string" && isUuidV7(record.projectId))) &&
    isArtifactTimestamp(record.createdAtUnixMs) &&
    typeof record.latestVersionId === "string" &&
    isUuidV7(record.latestVersionId) &&
    isVersionOrdinal(record.latestVersionOrdinal)
  );
}

function validateArtifactSummaryList(value: unknown): ArtifactSummary[] {
  if (!Array.isArray(value) || !value.every(isArtifactSummary)) {
    throw new AgentError("agent_storage_unavailable");
  }
  return value;
}

function validateArtifactDetail(value: unknown): ArtifactDetail {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new AgentError("agent_storage_unavailable");
  }
  if (!("artifact" in value) || !("versions" in value) || !Array.isArray(value.versions)) {
    throw new AgentError("agent_storage_unavailable");
  }
  if (!isArtifact(value.artifact) || !value.versions.every(isArtifactVersion)) {
    throw new AgentError("agent_storage_unavailable");
  }
  if (value.versions.length === 0) {
    throw new AgentError("agent_storage_unavailable");
  }
  return {
    artifact: value.artifact,
    versions: value.versions,
  };
}

function validateCreateArtifactResult(value: unknown): CreateArtifactResult {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new AgentError("agent_storage_unavailable");
  }
  if (!("artifact" in value) || !("version" in value)) {
    throw new AgentError("agent_storage_unavailable");
  }
  if (!isArtifact(value.artifact) || !isArtifactVersion(value.version)) {
    throw new AgentError("agent_storage_unavailable");
  }
  return {
    artifact: value.artifact,
    version: value.version,
  };
}

export async function createArtifactFromTurn(options: {
  turnId: string;
  title?: string | null;
  kind?: ArtifactKind | null;
}): Promise<CreateArtifactResult> {
  return validateCreateArtifactResult(
    await invokeAgentCommand("create_artifact_from_turn", {
      turnId: options.turnId,
      title: options.title ?? null,
      kind: options.kind ?? null,
    }),
  );
}

export async function listArtifacts(
  sessionId: string,
  projectId: string | null,
): Promise<ArtifactSummary[]> {
  return validateArtifactSummaryList(
    await invokeAgentCommand("list_artifacts", { sessionId, projectId }),
  );
}

export async function getArtifact(artifactId: string): Promise<ArtifactDetail> {
  return validateArtifactDetail(await invokeAgentCommand("get_artifact", { artifactId }));
}

export async function exportAgentTurnMetrics(turnId: string): Promise<AgentTurnMetricsExport> {
  return validateTurnMetricsExport(
    await invokeAgentCommand("export_agent_turn_metrics", { turnId }),
  );
}

export async function getModelRequestControls(modelId: string): Promise<ModelRequestControls> {
  const value = await invokeAgentCommand("get_model_request_controls", { modelId });
  if (!isModelRequestControls(value)) {
    throw new AgentError("agent_storage_unavailable");
  }
  return value;
}

export async function sendAgentMessage(options: {
  sessionId: string | null;
  userText: string;
  projectId: string | null;
  modelId: string | null;
  effort: AgentEffort | null;
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
      effort: options.effort,
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

export const AGENT_COMPOSER_UNAVAILABLE_MESSAGE = "Add a Provider to get started.";

export function getSafeAgentErrorMessageForCode(code: AgentErrorCode): string {
  switch (code) {
    case "not_connected":
      return "Connect your xAI subscription in Settings to message the Agent.";
    case "invalid_input":
      return "Enter a valid message.";
    case "context_limit":
      return "This conversation is too large to send.";
    case "session_busy":
      return "Another Agent turn is already in progress.";
    case "authentication_required":
      return "Reconnect your xAI subscription in Settings to continue.";
    case "entitlement_unavailable":
      return "xAI subscription access is unavailable for this account.";
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
      return "That attachment could not be read.";
    case "source_unsupported":
      return "That attachment is not supported.";
    case "source_too_large":
      return "That attachment is too large.";
    case "source_draft_expired":
      return "The attachment is no longer available. Choose it again.";
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

export function formatSourceAttachmentSummary(
  attachment: Pick<
    PendingSourceAttachment,
    "originKind" | "displayName" | "byteCount" | "memberCount" | "canonicalUrl"
  >,
): string {
  const size = formatSourceByteCount(attachment.byteCount);
  if (attachment.originKind === "local_text_folder") {
    const files = attachment.memberCount === 1 ? "1 file" : `${attachment.memberCount} files`;
    return `${attachment.displayName} (${size}, ${files})`;
  }
  if (attachment.originKind === "remote_text_url" && attachment.canonicalUrl !== null) {
    return `${attachment.displayName} (${size}, ${attachment.canonicalUrl})`;
  }
  return `${attachment.displayName} (${size})`;
}

export function sourceAttachmentKindLabel(originKind: SourceOriginKind): string {
  if (originKind === "local_text_folder") {
    return "folder";
  }
  if (originKind === "remote_text_url") {
    return "link";
  }
  return "file";
}
