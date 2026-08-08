import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { RunWorkspace } from "./RunWorkspace";
import {
  BLOCKED_RECONCILIATION_LABEL,
  NATIVE_STRUCTURAL_VALIDATION_LABEL,
  type HarnessRunDetail,
} from "../platform/runs";

const baseDetail: HarnessRunDetail = {
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
    byteCount: 42,
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

vi.mock("../platform/runs", async () => {
  const actual = await vi.importActual<typeof import("../platform/runs")>("../platform/runs");
  return {
    ...actual,
    pickHarnessRunRoot: vi.fn(),
    getHarnessRunDetail: vi.fn(),
    bootstrapHarnessPlan: vi.fn(),
    approveHarnessPair: vi.fn(),
    issueHarnessExecutionGrants: vi.fn(),
    executeHarnessRun: vi.fn(),
    pauseHarnessRun: vi.fn(),
    cancelHarnessRun: vi.fn(),
    denyUnsupportedHarnessOperation: vi.fn(),
  };
});

import * as runs from "../platform/runs";

describe("RunWorkspace", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("embeds the consequential journey with distinct approve/grant/execute controls", async () => {
    vi.mocked(runs.pickHarnessRunRoot).mockResolvedValue(baseDetail.summary);
    vi.mocked(runs.getHarnessRunDetail).mockResolvedValue(baseDetail);
    vi.mocked(runs.bootstrapHarnessPlan).mockResolvedValue(baseDetail);
    const approved = {
      ...baseDetail,
      approval: {
        ...baseDetail.approval!,
        approved: true,
        approvalId: "01900000-0000-7000-8000-000000000030",
        approver: "owner",
      },
    };
    vi.mocked(runs.approveHarnessPair).mockResolvedValue(approved);
    const granted: HarnessRunDetail = {
      ...approved,
      grants: [
        ...approved.grants,
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
        {
          id: "01900000-0000-7000-8000-000000000012",
          capability: "native_inspection",
          resourceSummary: "relative:index.html",
          actionScope: "node:verify-approved-postimage-v1",
          expiresAtUnixMs: 200,
          revoked: false,
          dispatchBudgetRemaining: 1,
          relatedApprovalId: "01900000-0000-7000-8000-000000000030",
        },
      ],
    };
    vi.mocked(runs.issueHarnessExecutionGrants).mockResolvedValue(granted);
    const completed: HarnessRunDetail = {
      ...granted,
      summary: {
        ...granted.summary,
        lifecycle: "completed",
        lifecycleLabel: "completed",
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
      finalResult: {
        validationLabel: NATIVE_STRUCTURAL_VALIDATION_LABEL,
        publicationStopped: true,
        planVersionId: granted.approval!.planVersionId,
        graphVersionId: granted.approval!.graphVersionId,
        completedAtUnixMs: 9,
      },
      denials: [
        {
          id: "01900000-0000-7000-8000-000000000050",
          reason: "unsupported operation denied: publication",
          grantId: null,
          recordedAtUnixMs: 10,
        },
      ],
      resumeDecision: "skip_confirmed_replacement:effect",
    };
    vi.mocked(runs.executeHarnessRun).mockResolvedValue(completed);
    vi.mocked(runs.denyUnsupportedHarnessOperation).mockResolvedValue(completed);

    render(<RunWorkspace modelId="fixture-controlled" />);
    fireEvent.click(screen.getByRole("button", { name: "Start harness journey" }));
    expect(screen.getByText(NATIVE_STRUCTURAL_VALIDATION_LABEL)).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Select run root" }));
    await waitFor(() => expect(screen.getByText("Context preview")).toBeInTheDocument());
    expect(screen.getAllByText(/index\.html/).length).toBeGreaterThan(0);
    expect(screen.getAllByText(/Ready for review/).length).toBeGreaterThan(0);
    expect(screen.getAllByText(/replace-existing-file-v1/).length).toBeGreaterThan(0);

    fireEvent.click(screen.getByRole("button", { name: "Approve" }));
    await waitFor(() => expect(screen.getByText(/Approved by owner/)).toBeInTheDocument());
    await waitFor(() => expect(screen.getByRole("button", { name: "Grant" })).not.toBeDisabled());
    fireEvent.click(screen.getByRole("button", { name: "Grant" }));
    await waitFor(() => expect(runs.issueHarnessExecutionGrants).toHaveBeenCalled());
    await waitFor(() => expect(screen.getAllByText(/bound to approval/).length).toBeGreaterThan(0));
    fireEvent.click(screen.getByRole("button", { name: "Execute" }));
    await waitFor(() => expect(screen.getByText("Final Work Result")).toBeInTheDocument());
    expect(screen.getByText(/Publication stopped/)).toBeInTheDocument();
    expect(screen.getAllByText(NATIVE_STRUCTURAL_VALIDATION_LABEL).length).toBeGreaterThan(0);

    fireEvent.click(screen.getByRole("button", { name: "Deny publication" }));
    await waitFor(() =>
      expect(screen.getByText(/unsupported operation denied: publication/)).toBeInTheDocument(),
    );
    await waitFor(() =>
      expect(screen.getByText(/Publication denied and recorded/)).toBeInTheDocument(),
    );

    fireEvent.click(screen.getByRole("button", { name: "Refresh / reopen evidence" }));
    await waitFor(() => expect(runs.getHarnessRunDetail).toHaveBeenCalled());
    await waitFor(() =>
      expect(screen.getByText(/Evidence refreshed from storage/)).toBeInTheDocument(),
    );
  });

  it("surfaces blocked reconciliation label", async () => {
    vi.mocked(runs.pickHarnessRunRoot).mockResolvedValue(baseDetail.summary);
    vi.mocked(runs.getHarnessRunDetail).mockResolvedValue({
      ...baseDetail,
      summary: {
        ...baseDetail.summary,
        lifecycle: "blocked_reconciliation_required",
        lifecycleLabel: BLOCKED_RECONCILIATION_LABEL,
      },
    });
    render(<RunWorkspace modelId="fixture-controlled" />);
    fireEvent.click(screen.getByRole("button", { name: "Start harness journey" }));
    fireEvent.click(screen.getByRole("button", { name: "Select run root" }));
    await waitFor(() =>
      expect(screen.getByText(BLOCKED_RECONCILIATION_LABEL, { exact: false })).toBeInTheDocument(),
    );
  });
});
