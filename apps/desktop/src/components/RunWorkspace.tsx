import { useState } from "react";
import {
  BLOCKED_RECONCILIATION_LABEL,
  NATIVE_STRUCTURAL_VALIDATION_LABEL,
  approveHarnessPair,
  bootstrapHarnessPlan,
  cancelHarnessRun,
  denyUnsupportedHarnessOperation,
  executeHarnessRun,
  getHarnessRunDetail,
  getSafeHarnessErrorMessage,
  issueHarnessExecutionGrants,
  pauseHarnessRun,
  pickHarnessRunRoot,
  type HarnessRunDetail,
} from "../platform/runs";

interface RunWorkspaceProps {
  modelId: string | null;
}

export function RunWorkspace({ modelId }: RunWorkspaceProps) {
  const [open, setOpen] = useState(false);
  const [instructions, setInstructions] = useState(
    "Change the single heading <h1>Ready</h1> to <h1>Ready for review</h1>.",
  );
  const [busy, setBusy] = useState(false);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);
  const [statusMessage, setStatusMessage] = useState<string | null>(null);
  const [detail, setDetail] = useState<HarnessRunDetail | null>(null);
  const [providerMode, setProviderMode] = useState<"fixture" | "live">("fixture");

  async function runAction(
    action: () => Promise<HarnessRunDetail>,
    successMessage: string,
  ): Promise<void> {
    setBusy(true);
    setErrorMessage(null);
    setStatusMessage(null);
    try {
      const previousDenialCount = detail?.denials.length ?? 0;
      const previousEventCount = detail?.events.length ?? 0;
      const next = await action();
      setDetail(next);
      const denialDelta = next.denials.length - previousDenialCount;
      const eventDelta = next.events.length - previousEventCount;
      const extras: string[] = [];
      if (denialDelta > 0) {
        extras.push(
          `${denialDelta} new denial${denialDelta === 1 ? "" : "s"} (see Denials below)`,
        );
      }
      if (eventDelta > 0) {
        extras.push(`${eventDelta} new timeline event${eventDelta === 1 ? "" : "s"}`);
      }
      setStatusMessage(extras.length > 0 ? `${successMessage} — ${extras.join("; ")}` : successMessage);
    } catch (error) {
      setErrorMessage(getSafeHarnessErrorMessage(error));
    } finally {
      setBusy(false);
    }
  }

  const approved = detail?.approval?.approved === true;
  const hasExecutionGrants =
    detail?.grants.some(
      (grant) =>
        (grant.capability === "create_or_replace" || grant.capability === "native_inspection") &&
        !grant.revoked &&
        grant.dispatchBudgetRemaining > 0,
    ) ?? false;
  const blocked =
    detail?.summary.lifecycle === "blocked_reconciliation_required" ||
    detail?.summary.lifecycleLabel === BLOCKED_RECONCILIATION_LABEL;
  // Fixture acceptance must not require a connected provider/model.
  const effectiveModelId =
    modelId ?? (providerMode === "fixture" ? "fixture-controlled" : null);
  const canPreview = detail !== null && effectiveModelId !== null;

  return (
    <section className="run-workspace" aria-label="Consequential work">
      <div className="run-workspace-header">
        <h2>Consequential work</h2>
        <button
          className="secondary-action"
          type="button"
          aria-expanded={open}
          onClick={() => setOpen((value) => !value)}
        >
          {open ? "Hide harness journey" : "Start harness journey"}
        </button>
      </div>
      {open ? (
        <div className="run-workspace-body">
          <p className="run-workspace-copy">
            Exact-file replacement for a prepared static fixture. Validation is labelled{" "}
            <strong>{NATIVE_STRUCTURAL_VALIDATION_LABEL}</strong> and stops before publication.
          </p>
          <label className="run-field">
            Heading-change request
            <textarea
              value={instructions}
              disabled={busy}
              onChange={(event) => setInstructions(event.currentTarget.value)}
              rows={3}
            />
          </label>
          <label className="run-field">
            Provider mode
            <select
              aria-label="Provider mode"
              value={providerMode}
              disabled={busy}
              onChange={(event) =>
                setProviderMode(event.currentTarget.value === "live" ? "live" : "fixture")
              }
            >
              <option value="fixture">Fixture adapter (automated)</option>
              <option value="live">Live provider (owner smoke)</option>
            </select>
          </label>
          <div className="run-actions">
            <button
              className="secondary-action"
              type="button"
              disabled={busy}
              onClick={() =>
                void runAction(async () => {
                  const summary = await pickHarnessRunRoot();
                  return getHarnessRunDetail(summary.id);
                }, "Run root selected.")
              }
            >
              Select run root
            </button>
            <button
              className="secondary-action"
              type="button"
              disabled={busy || !canPreview}
              onClick={() =>
                void runAction(
                  () =>
                    bootstrapHarnessPlan({
                      runId: detail!.summary.id,
                      instructions,
                      modelId: effectiveModelId!,
                      providerMode,
                    }),
                  "Plan preview ready — review context, diff, and graph below.",
                )
              }
            >
              Preview plan
            </button>
            {!canPreview && detail !== null && providerMode === "live" ? (
              <p className="run-meta" role="status">
                Live preview needs a model selected in the session bar.
              </p>
            ) : null}
            <button
              className="secondary-action"
              type="button"
              disabled={busy || detail === null || approved || detail.approval === null}
              onClick={() =>
                void runAction(
                  () => approveHarnessPair(detail!.summary.id, "owner"),
                  "Pair approved. Next: Grant.",
                )
              }
            >
              Approve
            </button>
            <button
              className="secondary-action"
              type="button"
              disabled={busy || detail === null || !approved || hasExecutionGrants}
              onClick={() =>
                void runAction(
                  () => issueHarnessExecutionGrants(detail!.summary.id),
                  "Execution grants issued. Next: Execute.",
                )
              }
            >
              Grant
            </button>
            <button
              className="primary-action"
              type="button"
              disabled={busy || detail === null || !approved || !hasExecutionGrants || blocked}
              onClick={() =>
                void runAction(
                  () => executeHarnessRun(detail!.summary.id),
                  "Execute finished — check Final Work Result and Validation below.",
                )
              }
            >
              Execute
            </button>
            <button
              className="secondary-action"
              type="button"
              disabled={busy || detail === null || blocked}
              onClick={() =>
                void runAction(() => pauseHarnessRun(detail!.summary.id), "Run paused.")
              }
            >
              Pause
            </button>
            <button
              className="secondary-action"
              type="button"
              disabled={busy || detail === null || blocked}
              onClick={() =>
                void runAction(() => cancelHarnessRun(detail!.summary.id), "Run cancelled.")
              }
            >
              Cancel
            </button>
            <button
              className="secondary-action"
              type="button"
              disabled={busy || detail === null}
              onClick={() =>
                void runAction(
                  () => denyUnsupportedHarnessOperation(detail!.summary.id, "publication"),
                  "Publication denied and recorded. Scroll to Denials — file was not changed again.",
                )
              }
            >
              Deny publication
            </button>
            <button
              className="secondary-action"
              type="button"
              disabled={busy || detail === null}
              onClick={() =>
                void runAction(
                  () => getHarnessRunDetail(detail!.summary.id),
                  "Evidence refreshed from storage. Same run identity; confirmed replacement is not redispatched.",
                )
              }
            >
              Refresh / reopen evidence
            </button>
          </div>
          {busy ? (
            <p className="run-status run-status-busy" role="status" aria-live="polite">
              Working…
            </p>
          ) : null}
          {statusMessage ? (
            <p className="run-status" role="status" aria-live="polite">
              {statusMessage}
            </p>
          ) : null}
          {errorMessage ? <p className="workspace-error">{errorMessage}</p> : null}
          {detail ? (
            <div className="run-evidence">
              <div className="run-panel">
                <h3>State</h3>
                <p>
                  {detail.summary.lifecycleLabel}
                  {blocked ? ` (${BLOCKED_RECONCILIATION_LABEL})` : null}
                </p>
                {detail.resumeDecision ? (
                  <p className="run-meta">Resume: {detail.resumeDecision}</p>
                ) : null}
              </div>
              {detail.context ? (
                <div className="run-panel">
                  <h3>Context preview</h3>
                  <p>
                    {detail.context.runRootDisplayName} / {detail.context.relativeTarget} (
                    {detail.context.byteCount} bytes)
                  </p>
                  <p className="run-meta">
                    {detail.context.providerProfileId} · {detail.context.modelId}
                  </p>
                  <p className="run-meta">{detail.context.proposedDisclosure}</p>
                  <pre className="run-pre">{detail.context.selectedContent}</pre>
                </div>
              ) : null}
              {detail.diff ? (
                <div className="run-panel">
                  <h3>Exact diff</h3>
                  <pre className="run-pre">{detail.diff.text}</pre>
                  <p className="run-meta">diff {detail.diff.hash.slice(0, 12)}…</p>
                </div>
              ) : null}
              {detail.graph ? (
                <div className="run-panel">
                  <h3>Linear graph</h3>
                  <ol>
                    {detail.graph.nodes.map((node) => (
                      <li key={node.kind}>
                        {node.kind} · {node.responsibility}
                        {node.protectedValidation ? ` · ${detail.graph!.validationLabel}` : null}
                      </li>
                    ))}
                  </ol>
                  <p className="run-meta">
                    {detail.graph.edgeFrom} → {detail.graph.edgeTo} · {detail.graph.retryRule}
                  </p>
                </div>
              ) : null}
              {detail.approval ? (
                <div className="run-panel">
                  <h3>Approval identity</h3>
                  <p className="run-meta">hash {detail.approval.approvalHash.slice(0, 12)}…</p>
                  <p>
                    {detail.approval.approved
                      ? `Approved by ${detail.approval.approver ?? "unknown"}`
                      : "Not approved"}
                  </p>
                </div>
              ) : null}
              <div className="run-panel">
                <h3>Capability grants</h3>
                <p className="run-meta">Requested: {detail.requestedGrants.join(", ") || "none"}</p>
                <ul>
                  {detail.grants.map((grant) => (
                    <li key={grant.id}>
                      {grant.capability} · budget {grant.dispatchBudgetRemaining}
                      {grant.revoked ? " · revoked" : ""}
                      {grant.relatedApprovalId ? " · bound to approval" : " · bootstrap"}
                    </li>
                  ))}
                </ul>
              </div>
              {detail.denials.length > 0 ? (
                <div className="run-panel run-panel-attention" id="harness-denials">
                  <h3>Denials ({detail.denials.length})</h3>
                  <ul>
                    {detail.denials.map((denial) => (
                      <li key={denial.id}>{denial.reason}</li>
                    ))}
                  </ul>
                </div>
              ) : null}
              <div className="run-panel">
                <h3>Timeline</h3>
                <ol>
                  {detail.events.map((event) => (
                    <li key={event.id}>
                      #{event.sequence} {event.kind}
                    </li>
                  ))}
                </ol>
              </div>
              <div className="run-panel">
                <h3>Effects</h3>
                <ul>
                  {detail.effects.map((effect) => (
                    <li key={effect.id}>
                      {effect.operationId} · {effect.phase}
                      {effect.certainty ? ` · ${effect.certainty}` : ""}
                    </li>
                  ))}
                </ul>
              </div>
              {detail.checkpoint ? (
                <div className="run-panel">
                  <h3>Checkpoint</h3>
                  <p className="run-meta">
                    seq {detail.checkpoint.lastEventSequence} · expected{" "}
                    {detail.checkpoint.expectedPostimageHash.slice(0, 12)}…
                  </p>
                </div>
              ) : null}
              {detail.validation ? (
                <div className="run-panel">
                  <h3>Validation</h3>
                  <p>{detail.validation.label}</p>
                  <p className="run-meta">
                    {detail.validation.passed ? "passed" : "failed"} · native structural only
                  </p>
                </div>
              ) : null}
              {detail.providerDisclosure ? (
                <div className="run-panel">
                  <h3>Provider disclosure</h3>
                  <p className="run-meta">
                    {detail.providerDisclosure.providerProfileId} ·{" "}
                    {detail.providerDisclosure.modelId}
                  </p>
                  <p>{detail.providerDisclosure.allowedDisclosure}</p>
                </div>
              ) : null}
              {detail.finalResult ? (
                <div className="run-panel">
                  <h3>Final Work Result</h3>
                  <p>{detail.finalResult.validationLabel}</p>
                  <p className="run-meta">
                    {detail.finalResult.publicationStopped
                      ? "Publication stopped — no publish capability"
                      : "Publication state missing"}
                  </p>
                </div>
              ) : null}
              {detail.capabilityEnvelope ? (
                <div className="run-panel">
                  <h3>Capability envelope</h3>
                  <p>{detail.capabilityEnvelope.summary}</p>
                </div>
              ) : null}
            </div>
          ) : null}
        </div>
      ) : null}
    </section>
  );
}
