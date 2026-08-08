import { invoke } from "@tauri-apps/api/core";

export const NATIVE_STRUCTURAL_VALIDATION_LABEL = "native structural validation";
export const BLOCKED_RECONCILIATION_LABEL = "Blocked — reconciliation required";

export type HarnessErrorCode =
  | "invalid_input"
  | "storage_unavailable"
  | "denied"
  | "unsupported_operation"
  | "blocked"
  | "provider_unavailable"
  | "malformed_response";

const harnessErrorCodes: readonly HarnessErrorCode[] = [
  "invalid_input",
  "storage_unavailable",
  "denied",
  "unsupported_operation",
  "blocked",
  "provider_unavailable",
  "malformed_response",
];

export class HarnessError extends Error {
  readonly code: HarnessErrorCode;

  constructor(code: HarnessErrorCode, message?: string) {
    super(message ?? code);
    this.name = "HarnessError";
    this.code = code;
  }
}

export interface HarnessRunSummary {
  id: string;
  runRootDisplayName: string;
  lifecycle: string;
  lifecycleLabel: string;
  createdAtUnixMs: number;
}

export interface ContextPreview {
  runRootDisplayName: string;
  relativeTarget: string;
  byteCount: number;
  contentHash: string;
  selectedContent: string;
  providerProfileId: string;
  modelId: string;
  proposedDisclosure: string;
  manifestContentHash: string;
  requestSemanticHash: string;
}

export interface DiffPreview {
  version: string;
  text: string;
  hash: string;
  preimageHash: string;
  postimageHash: string;
}

export interface GraphNodePreview {
  kind: string;
  responsibility: string;
  protectedValidation: boolean;
}

export interface GraphSummary {
  id: string;
  nodes: GraphNodePreview[];
  edgeFrom: string;
  edgeTo: string;
  retryRule: string;
  validationRule: string;
  validationLabel: string;
}

export interface ApprovalIdentity {
  planVersionId: string;
  graphVersionId: string;
  approvalHash: string;
  approved: boolean;
  approvalId: string | null;
  approver: string | null;
}

export interface GrantRecord {
  id: string;
  capability: string;
  resourceSummary: string;
  actionScope: string;
  expiresAtUnixMs: number;
  revoked: boolean;
  dispatchBudgetRemaining: number;
  relatedApprovalId: string | null;
}

export interface EffectRecord {
  id: string;
  operationId: string;
  phase: string;
  certainty: string | null;
  grantId: string;
}

export interface DenialRecord {
  id: string;
  reason: string;
  grantId: string | null;
  recordedAtUnixMs: number;
}

export interface RunEventRecord {
  id: string;
  sequence: number;
  kind: string;
  createdAtUnixMs: number;
}

export interface CheckpointRecord {
  id: string;
  lastEventSequence: number;
  expectedPostimageHash: string;
  createdAtUnixMs: number;
}

export interface ValidationRecord {
  id: string;
  label: string;
  approvedPostimageHash: string;
  observedPostimageHash: string;
  nativeDiffHash: string;
  passed: boolean;
  validatedAtUnixMs: number;
}

export interface FinalWorkResultRecord {
  validationLabel: string;
  publicationStopped: boolean;
  planVersionId: string;
  graphVersionId: string;
  completedAtUnixMs: number;
}

export interface ProviderDisclosureRecord {
  providerProfileId: string;
  modelId: string;
  allowedDisclosure: string;
  responseId: string | null;
}

export interface CapabilityEnvelopeRecord {
  summary: string;
  requested: string[];
}

export interface HarnessRunDetail {
  summary: HarnessRunSummary;
  context: ContextPreview | null;
  diff: DiffPreview | null;
  graph: GraphSummary | null;
  approval: ApprovalIdentity | null;
  grants: GrantRecord[];
  requestedGrants: string[];
  events: RunEventRecord[];
  effects: EffectRecord[];
  denials: DenialRecord[];
  checkpoint: CheckpointRecord | null;
  validation: ValidationRecord | null;
  providerDisclosure: ProviderDisclosureRecord | null;
  finalResult: FinalWorkResultRecord | null;
  capabilityEnvelope: CapabilityEnvelopeRecord | null;
  resumeDecision: string | null;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isString(value: unknown): value is string {
  return typeof value === "string";
}

function isSafeInteger(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value);
}

function isBoolean(value: unknown): value is boolean {
  return typeof value === "boolean";
}

function isHarnessErrorCode(value: unknown): value is HarnessErrorCode {
  return typeof value === "string" && harnessErrorCodes.includes(value as HarnessErrorCode);
}

function mapHostError(error: unknown): HarnessError {
  if (error instanceof HarnessError) {
    return error;
  }
  if (isRecord(error) && isHarnessErrorCode(error.message)) {
    return new HarnessError(error.message);
  }
  if (typeof error === "string" && isHarnessErrorCode(error)) {
    return new HarnessError(error);
  }
  return new HarnessError("storage_unavailable");
}

function parseSummary(value: unknown): HarnessRunSummary {
  if (!isRecord(value)) {
    throw new HarnessError("malformed_response");
  }
  if (
    !isString(value.id) ||
    !isString(value.runRootDisplayName) ||
    !isString(value.lifecycle) ||
    !isString(value.lifecycleLabel) ||
    !isSafeInteger(value.createdAtUnixMs)
  ) {
    throw new HarnessError("malformed_response");
  }
  return {
    id: value.id,
    runRootDisplayName: value.runRootDisplayName,
    lifecycle: value.lifecycle,
    lifecycleLabel: value.lifecycleLabel,
    createdAtUnixMs: value.createdAtUnixMs,
  };
}

function parseContext(value: unknown): ContextPreview | null {
  if (value === null) {
    return null;
  }
  if (!isRecord(value)) {
    throw new HarnessError("malformed_response");
  }
  if (
    !isString(value.runRootDisplayName) ||
    !isString(value.relativeTarget) ||
    !isSafeInteger(value.byteCount) ||
    !isString(value.contentHash) ||
    !isString(value.selectedContent) ||
    !isString(value.providerProfileId) ||
    !isString(value.modelId) ||
    !isString(value.proposedDisclosure) ||
    !isString(value.manifestContentHash) ||
    !isString(value.requestSemanticHash)
  ) {
    throw new HarnessError("malformed_response");
  }
  return {
    runRootDisplayName: value.runRootDisplayName,
    relativeTarget: value.relativeTarget,
    byteCount: value.byteCount,
    contentHash: value.contentHash,
    selectedContent: value.selectedContent,
    providerProfileId: value.providerProfileId,
    modelId: value.modelId,
    proposedDisclosure: value.proposedDisclosure,
    manifestContentHash: value.manifestContentHash,
    requestSemanticHash: value.requestSemanticHash,
  };
}

function parseDiff(value: unknown): DiffPreview | null {
  if (value === null) {
    return null;
  }
  if (!isRecord(value)) {
    throw new HarnessError("malformed_response");
  }
  if (
    !isString(value.version) ||
    !isString(value.text) ||
    !isString(value.hash) ||
    !isString(value.preimageHash) ||
    !isString(value.postimageHash)
  ) {
    throw new HarnessError("malformed_response");
  }
  return {
    version: value.version,
    text: value.text,
    hash: value.hash,
    preimageHash: value.preimageHash,
    postimageHash: value.postimageHash,
  };
}

function parseGraph(value: unknown): GraphSummary | null {
  if (value === null) {
    return null;
  }
  if (!isRecord(value) || !Array.isArray(value.nodes)) {
    throw new HarnessError("malformed_response");
  }
  const nodes: GraphNodePreview[] = [];
  for (const node of value.nodes) {
    if (
      !isRecord(node) ||
      !isString(node.kind) ||
      !isString(node.responsibility) ||
      !isBoolean(node.protectedValidation)
    ) {
      throw new HarnessError("malformed_response");
    }
    nodes.push({
      kind: node.kind,
      responsibility: node.responsibility,
      protectedValidation: node.protectedValidation,
    });
  }
  if (
    !isString(value.id) ||
    !isString(value.edgeFrom) ||
    !isString(value.edgeTo) ||
    !isString(value.retryRule) ||
    !isString(value.validationRule) ||
    !isString(value.validationLabel)
  ) {
    throw new HarnessError("malformed_response");
  }
  if (value.validationLabel !== NATIVE_STRUCTURAL_VALIDATION_LABEL) {
    throw new HarnessError("malformed_response");
  }
  return {
    id: value.id,
    nodes,
    edgeFrom: value.edgeFrom,
    edgeTo: value.edgeTo,
    retryRule: value.retryRule,
    validationRule: value.validationRule,
    validationLabel: value.validationLabel,
  };
}

function parseApproval(value: unknown): ApprovalIdentity | null {
  if (value === null) {
    return null;
  }
  if (!isRecord(value)) {
    throw new HarnessError("malformed_response");
  }
  if (
    !isString(value.planVersionId) ||
    !isString(value.graphVersionId) ||
    !isString(value.approvalHash) ||
    !isBoolean(value.approved) ||
    !(value.approvalId === null || isString(value.approvalId)) ||
    !(value.approver === null || isString(value.approver))
  ) {
    throw new HarnessError("malformed_response");
  }
  return {
    planVersionId: value.planVersionId,
    graphVersionId: value.graphVersionId,
    approvalHash: value.approvalHash,
    approved: value.approved,
    approvalId: value.approvalId,
    approver: value.approver,
  };
}

function parseGrant(value: unknown): GrantRecord {
  if (!isRecord(value)) {
    throw new HarnessError("malformed_response");
  }
  if (
    !isString(value.id) ||
    !isString(value.capability) ||
    !isString(value.resourceSummary) ||
    !isString(value.actionScope) ||
    !isSafeInteger(value.expiresAtUnixMs) ||
    !isBoolean(value.revoked) ||
    !isSafeInteger(value.dispatchBudgetRemaining) ||
    !(value.relatedApprovalId === null || isString(value.relatedApprovalId))
  ) {
    throw new HarnessError("malformed_response");
  }
  return {
    id: value.id,
    capability: value.capability,
    resourceSummary: value.resourceSummary,
    actionScope: value.actionScope,
    expiresAtUnixMs: value.expiresAtUnixMs,
    revoked: value.revoked,
    dispatchBudgetRemaining: value.dispatchBudgetRemaining,
    relatedApprovalId: value.relatedApprovalId,
  };
}

function parseEffect(value: unknown): EffectRecord {
  if (!isRecord(value)) {
    throw new HarnessError("malformed_response");
  }
  if (
    !isString(value.id) ||
    !isString(value.operationId) ||
    !isString(value.phase) ||
    !(value.certainty === null || isString(value.certainty)) ||
    !isString(value.grantId)
  ) {
    throw new HarnessError("malformed_response");
  }
  return {
    id: value.id,
    operationId: value.operationId,
    phase: value.phase,
    certainty: value.certainty,
    grantId: value.grantId,
  };
}

function parseDenial(value: unknown): DenialRecord {
  if (!isRecord(value)) {
    throw new HarnessError("malformed_response");
  }
  if (
    !isString(value.id) ||
    !isString(value.reason) ||
    !(value.grantId === null || isString(value.grantId)) ||
    !isSafeInteger(value.recordedAtUnixMs)
  ) {
    throw new HarnessError("malformed_response");
  }
  return {
    id: value.id,
    reason: value.reason,
    grantId: value.grantId,
    recordedAtUnixMs: value.recordedAtUnixMs,
  };
}

function parseEvent(value: unknown): RunEventRecord {
  if (!isRecord(value)) {
    throw new HarnessError("malformed_response");
  }
  if (
    !isString(value.id) ||
    !isSafeInteger(value.sequence) ||
    !isString(value.kind) ||
    !isSafeInteger(value.createdAtUnixMs)
  ) {
    throw new HarnessError("malformed_response");
  }
  return {
    id: value.id,
    sequence: value.sequence,
    kind: value.kind,
    createdAtUnixMs: value.createdAtUnixMs,
  };
}

function parseCheckpoint(value: unknown): CheckpointRecord | null {
  if (value === null) {
    return null;
  }
  if (!isRecord(value)) {
    throw new HarnessError("malformed_response");
  }
  if (
    !isString(value.id) ||
    !isSafeInteger(value.lastEventSequence) ||
    !isString(value.expectedPostimageHash) ||
    !isSafeInteger(value.createdAtUnixMs)
  ) {
    throw new HarnessError("malformed_response");
  }
  return {
    id: value.id,
    lastEventSequence: value.lastEventSequence,
    expectedPostimageHash: value.expectedPostimageHash,
    createdAtUnixMs: value.createdAtUnixMs,
  };
}

function parseValidation(value: unknown): ValidationRecord | null {
  if (value === null) {
    return null;
  }
  if (!isRecord(value)) {
    throw new HarnessError("malformed_response");
  }
  if (
    !isString(value.id) ||
    !isString(value.label) ||
    !isString(value.approvedPostimageHash) ||
    !isString(value.observedPostimageHash) ||
    !isString(value.nativeDiffHash) ||
    !isBoolean(value.passed) ||
    !isSafeInteger(value.validatedAtUnixMs)
  ) {
    throw new HarnessError("malformed_response");
  }
  if (value.label !== NATIVE_STRUCTURAL_VALIDATION_LABEL) {
    throw new HarnessError("malformed_response");
  }
  return {
    id: value.id,
    label: value.label,
    approvedPostimageHash: value.approvedPostimageHash,
    observedPostimageHash: value.observedPostimageHash,
    nativeDiffHash: value.nativeDiffHash,
    passed: value.passed,
    validatedAtUnixMs: value.validatedAtUnixMs,
  };
}

function parseFinalResult(value: unknown): FinalWorkResultRecord | null {
  if (value === null) {
    return null;
  }
  if (!isRecord(value)) {
    throw new HarnessError("malformed_response");
  }
  if (
    !isString(value.validationLabel) ||
    !isBoolean(value.publicationStopped) ||
    !isString(value.planVersionId) ||
    !isString(value.graphVersionId) ||
    !isSafeInteger(value.completedAtUnixMs)
  ) {
    throw new HarnessError("malformed_response");
  }
  if (value.validationLabel !== NATIVE_STRUCTURAL_VALIDATION_LABEL) {
    throw new HarnessError("malformed_response");
  }
  if (value.publicationStopped !== true) {
    throw new HarnessError("malformed_response");
  }
  return {
    validationLabel: value.validationLabel,
    publicationStopped: value.publicationStopped,
    planVersionId: value.planVersionId,
    graphVersionId: value.graphVersionId,
    completedAtUnixMs: value.completedAtUnixMs,
  };
}

function parseProviderDisclosure(value: unknown): ProviderDisclosureRecord | null {
  if (value === null) {
    return null;
  }
  if (!isRecord(value)) {
    throw new HarnessError("malformed_response");
  }
  if (
    !isString(value.providerProfileId) ||
    !isString(value.modelId) ||
    !isString(value.allowedDisclosure) ||
    !(value.responseId === null || isString(value.responseId))
  ) {
    throw new HarnessError("malformed_response");
  }
  return {
    providerProfileId: value.providerProfileId,
    modelId: value.modelId,
    allowedDisclosure: value.allowedDisclosure,
    responseId: value.responseId,
  };
}

function parseEnvelope(value: unknown): CapabilityEnvelopeRecord | null {
  if (value === null) {
    return null;
  }
  if (!isRecord(value) || !isString(value.summary) || !Array.isArray(value.requested)) {
    throw new HarnessError("malformed_response");
  }
  const requested: string[] = [];
  for (const item of value.requested) {
    if (!isString(item)) {
      throw new HarnessError("malformed_response");
    }
    requested.push(item);
  }
  return { summary: value.summary, requested };
}

/** Fail-closed parse of a host Harness run detail DTO. */
export function parseHarnessRunDetail(value: unknown): HarnessRunDetail {
  if (!isRecord(value)) {
    throw new HarnessError("malformed_response");
  }
  if (
    !Array.isArray(value.grants) ||
    !Array.isArray(value.requestedGrants) ||
    !Array.isArray(value.events) ||
    !Array.isArray(value.effects) ||
    !Array.isArray(value.denials)
  ) {
    throw new HarnessError("malformed_response");
  }
  const requestedGrants: string[] = [];
  for (const item of value.requestedGrants) {
    if (!isString(item)) {
      throw new HarnessError("malformed_response");
    }
    requestedGrants.push(item);
  }
  const detail: HarnessRunDetail = {
    summary: parseSummary(value.summary),
    context: parseContext(value.context),
    diff: parseDiff(value.diff),
    graph: parseGraph(value.graph),
    approval: parseApproval(value.approval),
    grants: value.grants.map(parseGrant),
    requestedGrants,
    events: value.events.map(parseEvent),
    effects: value.effects.map(parseEffect),
    denials: value.denials.map(parseDenial),
    checkpoint: parseCheckpoint(value.checkpoint),
    validation: parseValidation(value.validation),
    providerDisclosure: parseProviderDisclosure(value.providerDisclosure),
    finalResult: parseFinalResult(value.finalResult),
    capabilityEnvelope: parseEnvelope(value.capabilityEnvelope),
    resumeDecision:
      value.resumeDecision === null || isString(value.resumeDecision)
        ? value.resumeDecision
        : (() => {
            throw new HarnessError("malformed_response");
          })(),
  };
  if (detail.summary.lifecycle === "blocked_reconciliation_required") {
    if (detail.summary.lifecycleLabel !== BLOCKED_RECONCILIATION_LABEL) {
      throw new HarnessError("malformed_response");
    }
  }
  return detail;
}

async function invokeHarness<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (error) {
    throw mapHostError(error);
  }
}

export async function pickHarnessRunRoot(): Promise<HarnessRunSummary> {
  const raw = await invokeHarness<unknown>("pick_harness_run_root");
  return parseSummary(raw);
}

export async function getHarnessRunDetail(runId: string): Promise<HarnessRunDetail> {
  const raw = await invokeHarness<unknown>("get_harness_run_detail", { runId });
  return parseHarnessRunDetail(raw);
}

export async function bootstrapHarnessPlan(input: {
  runId: string;
  instructions: string;
  modelId: string;
  providerMode: "fixture" | "live";
}): Promise<HarnessRunDetail> {
  const raw = await invokeHarness<unknown>("bootstrap_harness_plan", {
    request: {
      runId: input.runId,
      instructions: input.instructions,
      modelId: input.modelId,
      providerMode: input.providerMode,
    },
  });
  return parseHarnessRunDetail(raw);
}

export async function approveHarnessPair(
  runId: string,
  approver: string,
): Promise<HarnessRunDetail> {
  const raw = await invokeHarness<unknown>("approve_harness_pair", { runId, approver });
  return parseHarnessRunDetail(raw);
}

export async function issueHarnessExecutionGrants(runId: string): Promise<HarnessRunDetail> {
  const raw = await invokeHarness<unknown>("issue_harness_execution_grants", { runId });
  return parseHarnessRunDetail(raw);
}

export async function executeHarnessRun(runId: string): Promise<HarnessRunDetail> {
  const raw = await invokeHarness<unknown>("execute_harness_run", { runId });
  return parseHarnessRunDetail(raw);
}

export async function pauseHarnessRun(runId: string): Promise<HarnessRunDetail> {
  const raw = await invokeHarness<unknown>("pause_harness_run", { runId });
  return parseHarnessRunDetail(raw);
}

export async function cancelHarnessRun(runId: string): Promise<HarnessRunDetail> {
  const raw = await invokeHarness<unknown>("cancel_harness_run", { runId });
  return parseHarnessRunDetail(raw);
}

export async function revokeHarnessGrant(
  runId: string,
  grantId: string,
): Promise<HarnessRunDetail> {
  const raw = await invokeHarness<unknown>("revoke_harness_grant", { runId, grantId });
  return parseHarnessRunDetail(raw);
}

export async function denyUnsupportedHarnessOperation(
  runId: string,
  operation: "publication" | "process-exec" | "git-write" | "arbitrary-network",
): Promise<HarnessRunDetail> {
  const raw = await invokeHarness<unknown>("deny_unsupported_harness_operation", {
    runId,
    operation,
  });
  return parseHarnessRunDetail(raw);
}

export function getSafeHarnessErrorMessage(error: unknown): string {
  const code = error instanceof HarnessError ? error.code : mapHostError(error).code;
  switch (code) {
    case "invalid_input":
      return "The harness request was invalid.";
    case "denied":
      return "Authority was denied for that harness action.";
    case "unsupported_operation":
      return "That operation is unavailable and was recorded as denied.";
    case "blocked":
      return BLOCKED_RECONCILIATION_LABEL;
    case "provider_unavailable":
      return "The provider is unavailable for harness disclosure.";
    case "malformed_response":
      return "The host returned an unusable harness record.";
    case "storage_unavailable":
    default:
      return "Harness storage is unavailable. Try again.";
  }
}
