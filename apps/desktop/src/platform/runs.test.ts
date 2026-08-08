import { describe, expect, it, vi } from "vitest";
import {
  BLOCKED_RECONCILIATION_LABEL,
  NATIVE_STRUCTURAL_VALIDATION_LABEL,
  HarnessError,
  parseHarnessRunDetail,
} from "./runs";

const validDetail = {
  summary: {
    id: "01900000-0000-7000-8000-000000000001",
    runRootDisplayName: "fixture",
    lifecycle: "awaiting_approval",
    lifecycleLabel: "awaiting approval",
    createdAtUnixMs: 1,
  },
  context: {
    runRootDisplayName: "fixture",
    relativeTarget: "index.html",
    byteCount: 120,
    contentHash: "a".repeat(64),
    selectedContent: "<h1>Ready</h1>",
    providerProfileId: "xai-subscription-oauth",
    modelId: "fixture-controlled",
    proposedDisclosure: "Exact selected index.html bytes",
    manifestContentHash: "b".repeat(64),
    requestSemanticHash: "c".repeat(64),
  },
  diff: {
    version: "tule-native-diff-v1",
    text: "- <h1>Ready</h1>\n+ <h1>Ready for review</h1>",
    hash: "d".repeat(64),
    preimageHash: "e".repeat(64),
    postimageHash: "f".repeat(64),
  },
  graph: {
    id: "01900000-0000-7000-8000-000000000002",
    nodes: [
      {
        kind: "replace-existing-file-v1",
        responsibility: "builder",
        protectedValidation: false,
      },
      {
        kind: "verify-approved-postimage-v1",
        responsibility: "reviewer",
        protectedValidation: true,
      },
    ],
    edgeFrom: "replace-existing-file-v1",
    edgeTo: "verify-approved-postimage-v1",
    retryRule: "no_automatic_retry",
    validationRule: "native_postimage_v1",
    validationLabel: NATIVE_STRUCTURAL_VALIDATION_LABEL,
  },
  approval: {
    planVersionId: "01900000-0000-7000-8000-000000000003",
    graphVersionId: "01900000-0000-7000-8000-000000000002",
    approvalHash: "1".repeat(64),
    approved: false,
    approvalId: null,
    approver: null,
  },
  grants: [
    {
      id: "01900000-0000-7000-8000-000000000010",
      capability: "local_read",
      resourceSummary: "relative:index.html",
      actionScope: "run",
      expiresAtUnixMs: 100,
      revoked: false,
      dispatchBudgetRemaining: 0,
      relatedApprovalId: null,
    },
  ],
  requestedGrants: ["local_read", "provider_disclose", "create_or_replace", "native_inspection"],
  events: [
    {
      id: "01900000-0000-7000-8000-000000000020",
      sequence: 1,
      kind: "run_created",
      createdAtUnixMs: 1,
    },
  ],
  effects: [],
  denials: [],
  checkpoint: null,
  validation: null,
  providerDisclosure: {
    providerProfileId: "xai-subscription-oauth",
    modelId: "fixture-controlled",
    allowedDisclosure: "Exact selected index.html bytes",
    responseId: "fixture-response",
  },
  finalResult: null,
  capabilityEnvelope: {
    summary: "Controlled fixture envelope",
    requested: ["local_read", "provider_disclose", "create_or_replace", "native_inspection"],
  },
  resumeDecision: "continue",
};

describe("parseHarnessRunDetail", () => {
  it("accepts a truthful context, diff, and linear graph", () => {
    const detail = parseHarnessRunDetail(validDetail);
    expect(detail.context?.relativeTarget).toBe("index.html");
    expect(detail.diff?.text).toContain("Ready for review");
    expect(detail.graph?.nodes).toHaveLength(2);
    expect(detail.graph?.validationLabel).toBe(NATIVE_STRUCTURAL_VALIDATION_LABEL);
    expect(detail.approval?.approved).toBe(false);
    expect(detail.requestedGrants).toContain("create_or_replace");
  });

  it("keeps approval distinct from grants", () => {
    const detail = parseHarnessRunDetail({
      ...validDetail,
      approval: {
        ...validDetail.approval,
        approved: true,
        approvalId: "01900000-0000-7000-8000-000000000030",
        approver: "owner",
      },
      grants: [
        ...validDetail.grants,
        {
          id: "01900000-0000-7000-8000-000000000011",
          capability: "create_or_replace",
          resourceSummary: "replacement:index.html",
          actionScope: "node:replace-existing-file-v1",
          expiresAtUnixMs: 200,
          revoked: false,
          dispatchBudgetRemaining: 1,
          relatedApprovalId: "01900000-0000-7000-8000-000000000030",
        },
      ],
    });
    expect(detail.approval?.approved).toBe(true);
    expect(detail.grants.some((grant) => grant.capability === "create_or_replace")).toBe(true);
    expect(
      detail.grants.find((grant) => grant.capability === "create_or_replace")?.relatedApprovalId,
    ).not.toBeNull();
  });

  it("requires blocked reconciliation label and structural validation wording", () => {
    expect(() =>
      parseHarnessRunDetail({
        ...validDetail,
        summary: {
          ...validDetail.summary,
          lifecycle: "blocked_reconciliation_required",
          lifecycleLabel: "blocked",
        },
      }),
    ).toThrow(HarnessError);

    const blocked = parseHarnessRunDetail({
      ...validDetail,
      summary: {
        ...validDetail.summary,
        lifecycle: "blocked_reconciliation_required",
        lifecycleLabel: BLOCKED_RECONCILIATION_LABEL,
      },
    });
    expect(blocked.summary.lifecycleLabel).toBe(BLOCKED_RECONCILIATION_LABEL);

    expect(() =>
      parseHarnessRunDetail({
        ...validDetail,
        graph: { ...validDetail.graph, validationLabel: "process validation" },
      }),
    ).toThrow(HarnessError);
  });

  it("rejects malformed host records and publication-continued finals", () => {
    expect(() => parseHarnessRunDetail({ summary: null })).toThrow(HarnessError);
    expect(() =>
      parseHarnessRunDetail({
        ...validDetail,
        finalResult: {
          validationLabel: NATIVE_STRUCTURAL_VALIDATION_LABEL,
          publicationStopped: false,
          planVersionId: "01900000-0000-7000-8000-000000000003",
          graphVersionId: "01900000-0000-7000-8000-000000000002",
          completedAtUnixMs: 9,
        },
      }),
    ).toThrow(HarnessError);

    const completed = parseHarnessRunDetail({
      ...validDetail,
      summary: {
        ...validDetail.summary,
        lifecycle: "completed",
        lifecycleLabel: "completed",
      },
      finalResult: {
        validationLabel: NATIVE_STRUCTURAL_VALIDATION_LABEL,
        publicationStopped: true,
        planVersionId: "01900000-0000-7000-8000-000000000003",
        graphVersionId: "01900000-0000-7000-8000-000000000002",
        completedAtUnixMs: 9,
      },
      validation: {
        id: "01900000-0000-7000-8000-000000000040",
        label: NATIVE_STRUCTURAL_VALIDATION_LABEL,
        approvedPostimageHash: "f".repeat(64),
        observedPostimageHash: "f".repeat(64),
        nativeDiffHash: "d".repeat(64),
        passed: true,
        validatedAtUnixMs: 8,
      },
      denials: [
        {
          id: "01900000-0000-7000-8000-000000000050",
          reason: "unsupported operation denied: publication",
          grantId: null,
          recordedAtUnixMs: 10,
        },
      ],
    });
    expect(completed.finalResult?.publicationStopped).toBe(true);
    expect(completed.validation?.label).toBe(NATIVE_STRUCTURAL_VALIDATION_LABEL);
    expect(completed.denials[0]?.reason).toContain("publication");
  });

  it("preserves reopen evidence identity", () => {
    const first = parseHarnessRunDetail(validDetail);
    const second = parseHarnessRunDetail(structuredClone(validDetail));
    expect(second.summary.id).toBe(first.summary.id);
    expect(second.diff?.hash).toBe(first.diff?.hash);
    expect(second.approval?.approvalHash).toBe(first.approval?.approvalHash);
  });
});

describe("harness invoke wrappers", () => {
  it("maps host errors without leaking internals", async () => {
    vi.resetModules();
    vi.doMock("@tauri-apps/api/core", () => ({
      invoke: vi.fn().mockRejectedValue({ message: "denied" }),
    }));
    const { denyUnsupportedHarnessOperation, getSafeHarnessErrorMessage } = await import("./runs");
    await expect(denyUnsupportedHarnessOperation("run", "publication")).rejects.toMatchObject({
      code: "denied",
    });
    expect(getSafeHarnessErrorMessage(new HarnessError("blocked"))).toBe(
      BLOCKED_RECONCILIATION_LABEL,
    );
  });
});
