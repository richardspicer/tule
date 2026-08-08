//! Use cases for Harness Run compile, approve, grant, effect, checkpoint, and resume flows.

use std::{
    collections::HashMap,
    error::Error,
    fmt,
    sync::{Mutex, MutexGuard},
};

use crate::{
    AcquireLeaseIntent, ApprovalError, ApprovalRecord, BOOTSTRAP_GRANT_TTL_MS,
    BootstrapProposalError, CapabilityEnvelope, CapabilityGrant, CapabilityGrantError,
    CapabilityGrantId, CapabilityType, Checkpoint, ClaimEffectIntent, Clock,
    ComparisonInstrumentation, ConsumeDispatchBudgetIntent, ContextManifest,
    DEFAULT_DISPATCH_BUDGET, DenialEvidence, DisclosurePolicy, EXECUTION_POLICY_REVISION_V1,
    EffectCertainty, EffectError, EffectJournalPhase, EffectOperationResult, EffectRecord,
    EffectRecordId, ExecutionPlanVersion, FinalWorkResult, GrantActionScope, GrantDenialReason,
    GrantEvaluation, GrantEvaluationRequest, GrantResourceSelector, GraphShapeFingerprint,
    HarnessRun, HarnessRunId, HarnessRunLifecycle, LeaseError, NATIVE_STRUCTURAL_VALIDATION_LABEL,
    NODE_REPLACE_EXISTING_FILE_V1, OP_CREATE_OR_REPLACE_V1, OP_NATIVE_INSPECT_V1,
    OP_PROVIDER_DISCLOSE_V1, POST_APPROVAL_GRANT_TTL_MS, PersistCheckpointIntent,
    PlanGraphPairBinding, REGISTERED_OPERATION_SCHEMA_V1, ROOT_LEASE_TTL_MS, ReconciliationProbe,
    ReconstructedRun, ReleaseLeaseIntent, ReplacementContentInput, ResumeDecision,
    ResumeRevalidation, RootLease, RootLeaseId, RunEvent, RunEventKind, RunGraphVersion,
    RunRepository, TakeoverLeaseIntent, TaskCohortAssignment, ValidationResult, derive_lifecycle,
    evaluate_grant, evaluate_resume, hash_event_chain, is_quiescent_for_checkpoint,
    reconcile_replacement_certainty,
};

/// Next event sequence for a reconstructed run.
fn next_sequence(events: &[RunEvent]) -> u64 {
    events.last().map_or(1, |event| event.sequence() + 1)
}

/// Compiles and freezes the fixed first plan/graph pair.
#[allow(clippy::too_many_arguments)]
pub fn compile_and_freeze_pair<R: RunRepository>(
    repository: &R,
    run_id: HarnessRunId,
    instructions: impl Into<String>,
    provider_profile_id: impl Into<String>,
    model_id: impl Into<String>,
    disclosure_policy: DisclosurePolicy,
    capability_envelope: CapabilityEnvelope,
    context_manifest: ContextManifest,
    preimage: &str,
    postimage: impl Into<String>,
    relative_target: impl Into<String>,
    preimage_filesystem_identity: impl Into<String>,
    provider_request_id: Option<String>,
    provider_response_id: Option<String>,
    now_unix_ms: i64,
) -> Result<(ExecutionPlanVersion, RunGraphVersion), CompileFreezeError<R::Error>> {
    let reconstructed = repository
        .reconstruct_run(&run_id)
        .map_err(CompileFreezeError::Repository)?
        .ok_or(CompileFreezeError::RunNotFound)?;
    let replacement = ReplacementContentInput::new(
        relative_target,
        preimage,
        postimage,
        provider_request_id,
        provider_response_id,
        now_unix_ms,
    )?;
    let graph = RunGraphVersion::compile_fixed_first_graph();
    let plan = ExecutionPlanVersion::freeze(
        &graph,
        instructions,
        provider_profile_id,
        model_id,
        disclosure_policy,
        capability_envelope,
        context_manifest,
        replacement,
        preimage_filesystem_identity,
        EXECUTION_POLICY_REVISION_V1,
        now_unix_ms,
    );
    let event = RunEvent::new(
        run_id,
        next_sequence(&reconstructed.events),
        RunEventKind::PairFrozen {
            plan_version_id: plan.id(),
            graph_version_id: graph.id(),
            approval_hash: plan.approval_hash().to_owned(),
        },
        now_unix_ms,
    );
    repository
        .persist_frozen_pair(&plan, &graph, &event)
        .map_err(CompileFreezeError::Repository)?;
    Ok((plan, graph))
}

/// Records approval for a frozen pair. Does not issue grants.
pub fn approve_pair<R: RunRepository>(
    repository: &R,
    run_id: HarnessRunId,
    plan: &ExecutionPlanVersion,
    graph: &RunGraphVersion,
    approver: impl Into<String>,
    now_unix_ms: i64,
) -> Result<ApprovalRecord, ApproveError<R::Error>> {
    let reconstructed = repository
        .reconstruct_run(&run_id)
        .map_err(ApproveError::Repository)?
        .ok_or(ApproveError::RunNotFound)?;
    let approval = ApprovalRecord::new(run_id, plan, graph, approver, now_unix_ms)?;
    let event = RunEvent::new(
        run_id,
        next_sequence(&reconstructed.events),
        RunEventKind::Approved {
            approval_id: approval.id(),
        },
        now_unix_ms,
    );
    repository
        .persist_approval(&approval, &event)
        .map_err(ApproveError::Repository)?;
    Ok(approval)
}

/// Issues a capability grant as a separate record from approval.
#[allow(clippy::too_many_arguments)]
pub fn issue_grant<R: RunRepository>(
    repository: &R,
    run_id: HarnessRunId,
    capability: CapabilityType,
    resource: GrantResourceSelector,
    action_scope: GrantActionScope,
    pair: Option<PlanGraphPairBinding>,
    related_approval_id: Option<crate::ApprovalRecordId>,
    issuer: impl Into<String>,
    now_unix_ms: i64,
) -> Result<CapabilityGrant, GrantUseCaseError<R::Error>> {
    let reconstructed = repository
        .reconstruct_run(&run_id)
        .map_err(GrantUseCaseError::Repository)?
        .ok_or(GrantUseCaseError::RunNotFound)?;
    let ttl = capability.default_ttl_ms();
    let grant = CapabilityGrant::issue(
        run_id,
        capability,
        resource,
        action_scope,
        pair,
        related_approval_id,
        issuer,
        now_unix_ms,
        now_unix_ms + ttl,
        DEFAULT_DISPATCH_BUDGET,
    )?;
    let event = RunEvent::new(
        run_id,
        next_sequence(&reconstructed.events),
        RunEventKind::GrantIssued {
            grant_id: grant.id(),
        },
        now_unix_ms,
    );
    repository
        .persist_grant(&grant, &event)
        .map_err(GrantUseCaseError::Repository)?;
    Ok(grant)
}

/// Revokes a grant without extending or reviving it later.
pub fn revoke_grant<R: RunRepository>(
    repository: &R,
    run_id: HarnessRunId,
    grant_id: CapabilityGrantId,
    now_unix_ms: i64,
) -> Result<CapabilityGrant, GrantUseCaseError<R::Error>> {
    let reconstructed = repository
        .reconstruct_run(&run_id)
        .map_err(GrantUseCaseError::Repository)?
        .ok_or(GrantUseCaseError::RunNotFound)?;
    let mut grant = reconstructed
        .grants
        .into_iter()
        .find(|grant| grant.id() == grant_id)
        .ok_or(GrantUseCaseError::GrantNotFound)?;
    grant.revoke(now_unix_ms)?;
    let event = RunEvent::new(
        run_id,
        next_sequence(&reconstructed.events),
        RunEventKind::GrantRevoked { grant_id },
        now_unix_ms,
    );
    repository
        .persist_grant_revocation(&grant, &event)
        .map_err(GrantUseCaseError::Repository)?;
    Ok(grant)
}

/// Appends denial evidence and a matching run event.
pub fn record_denial<R: RunRepository>(
    repository: &R,
    run_id: HarnessRunId,
    reason: impl Into<String>,
    grant_id: Option<CapabilityGrantId>,
    resource: Option<GrantResourceSelector>,
    now_unix_ms: i64,
) -> Result<DenialEvidence, GrantUseCaseError<R::Error>> {
    let reconstructed = repository
        .reconstruct_run(&run_id)
        .map_err(GrantUseCaseError::Repository)?
        .ok_or(GrantUseCaseError::RunNotFound)?;
    let denial = DenialEvidence::new(run_id, reason, grant_id, resource, now_unix_ms);
    let event = RunEvent::new(
        run_id,
        next_sequence(&reconstructed.events),
        RunEventKind::Denied {
            denial_id: denial.id(),
        },
        now_unix_ms,
    );
    repository
        .persist_denial(&denial, &event)
        .map_err(GrantUseCaseError::Repository)?;
    Ok(denial)
}

/// Evaluates a grant and records denial evidence on failure.
pub fn require_grant<R: RunRepository>(
    repository: &R,
    grant: &CapabilityGrant,
    request: &GrantEvaluationRequest<'_>,
) -> Result<(), GrantUseCaseError<R::Error>> {
    match evaluate_grant(grant, request) {
        GrantEvaluation::Allow => Ok(()),
        GrantEvaluation::Deny(reason) => {
            record_denial(
                repository,
                request.run_id,
                reason.to_string(),
                Some(grant.id()),
                Some(request.resource.clone()),
                request.now_unix_ms,
            )?;
            Err(GrantUseCaseError::Denied(reason))
        }
    }
}

/// Prepares an effect record.
#[allow(clippy::too_many_arguments)]
pub fn prepare_effect<R: RunRepository>(
    repository: &R,
    run_id: HarnessRunId,
    attempt_id: Option<crate::NodeAttemptId>,
    plan_version_id: Option<crate::ExecutionPlanVersionId>,
    graph_version_id: Option<crate::RunGraphVersionId>,
    operation_id: impl Into<String>,
    target_hash: impl Into<String>,
    grant_id: CapabilityGrantId,
    now_unix_ms: i64,
    expected_preimage_hash: Option<String>,
    expected_postimage_hash: Option<String>,
) -> Result<EffectRecord, EffectUseCaseError<R::Error>> {
    let reconstructed = repository
        .reconstruct_run(&run_id)
        .map_err(EffectUseCaseError::Repository)?
        .ok_or(EffectUseCaseError::RunNotFound)?;
    if matches!(
        derive_lifecycle(&reconstructed.events, &reconstructed.effects),
        HarnessRunLifecycle::BlockedReconciliationRequired
    ) {
        return Err(EffectUseCaseError::RunBlocked);
    }
    let effect = EffectRecord::prepare(
        run_id,
        attempt_id,
        plan_version_id,
        graph_version_id,
        operation_id,
        REGISTERED_OPERATION_SCHEMA_V1,
        target_hash,
        grant_id,
        now_unix_ms,
        expected_preimage_hash,
        expected_postimage_hash,
    );
    let event = RunEvent::new(
        run_id,
        next_sequence(&reconstructed.events),
        RunEventKind::EffectPrepared {
            effect_id: effect.id(),
        },
        now_unix_ms,
    );
    repository
        .persist_prepared_effect(&effect, &event)
        .map_err(EffectUseCaseError::Repository)?;
    Ok(effect)
}

/// Claims one effect for a claimant.
pub fn claim_effect<R: RunRepository>(
    repository: &R,
    run_id: HarnessRunId,
    effect_id: EffectRecordId,
    claimant: impl Into<String>,
    now_unix_ms: i64,
) -> Result<EffectRecord, EffectUseCaseError<R::Error>> {
    let claimant = claimant.into();
    let reconstructed = repository
        .reconstruct_run(&run_id)
        .map_err(EffectUseCaseError::Repository)?
        .ok_or(EffectUseCaseError::RunNotFound)?;
    let event = RunEvent::new(
        run_id,
        next_sequence(&reconstructed.events),
        RunEventKind::EffectClaimed {
            effect_id,
            claimant: claimant.clone(),
        },
        now_unix_ms,
    );
    repository
        .claim_effect(&ClaimEffectIntent {
            run_id,
            effect_id,
            claimant,
            now_unix_ms,
            event,
        })
        .map_err(EffectUseCaseError::Repository)
}

/// Consumes one dispatch budget, marks dispatched, and returns the effect.
pub fn dispatch_effect<R: RunRepository>(
    repository: &R,
    run_id: HarnessRunId,
    effect_id: EffectRecordId,
    grant_id: CapabilityGrantId,
    claimant: &str,
    now_unix_ms: i64,
) -> Result<EffectRecord, EffectUseCaseError<R::Error>> {
    repository
        .consume_dispatch_budget(&ConsumeDispatchBudgetIntent {
            run_id,
            grant_id,
            now_unix_ms,
        })
        .map_err(EffectUseCaseError::Repository)?;
    let mut reconstructed = repository
        .reconstruct_run(&run_id)
        .map_err(EffectUseCaseError::Repository)?
        .ok_or(EffectUseCaseError::RunNotFound)?;
    let effect = reconstructed
        .effects
        .iter_mut()
        .find(|effect| effect.id() == effect_id)
        .ok_or(EffectUseCaseError::EffectNotFound)?;
    effect.mark_dispatched(claimant, now_unix_ms)?;
    let updated = effect.clone();
    let event = RunEvent::new(
        run_id,
        next_sequence(&reconstructed.events),
        RunEventKind::EffectDispatched { effect_id },
        now_unix_ms,
    );
    repository
        .persist_effect_dispatched(&updated, &event)
        .map_err(EffectUseCaseError::Repository)?;
    Ok(updated)
}

/// Settles an effect. Unknown certainty blocks the whole Run.
pub fn settle_effect<R: RunRepository>(
    repository: &R,
    run_id: HarnessRunId,
    effect_id: EffectRecordId,
    claimant: &str,
    operation_result: EffectOperationResult,
    certainty: EffectCertainty,
    now_unix_ms: i64,
) -> Result<EffectRecord, EffectUseCaseError<R::Error>> {
    let mut reconstructed = repository
        .reconstruct_run(&run_id)
        .map_err(EffectUseCaseError::Repository)?
        .ok_or(EffectUseCaseError::RunNotFound)?;
    let effect = reconstructed
        .effects
        .iter_mut()
        .find(|effect| effect.id() == effect_id)
        .ok_or(EffectUseCaseError::EffectNotFound)?;
    effect.settle(claimant, operation_result, certainty, now_unix_ms)?;
    let updated = effect.clone();
    let event = RunEvent::new(
        run_id,
        next_sequence(&reconstructed.events),
        RunEventKind::EffectSettled {
            effect_id,
            certainty,
        },
        now_unix_ms,
    );
    repository
        .persist_effect_settled(&updated, &event)
        .map_err(EffectUseCaseError::Repository)?;
    Ok(updated)
}

/// Reconciles an unknown effect with positive evidence.
pub fn reconcile_effect<R: RunRepository>(
    repository: &R,
    run_id: HarnessRunId,
    effect_id: EffectRecordId,
    probe: ReconciliationProbe,
    now_unix_ms: i64,
) -> Result<EffectRecord, EffectUseCaseError<R::Error>> {
    let mut reconstructed = repository
        .reconstruct_run(&run_id)
        .map_err(EffectUseCaseError::Repository)?
        .ok_or(EffectUseCaseError::RunNotFound)?;
    let effect = reconstructed
        .effects
        .iter_mut()
        .find(|effect| effect.id() == effect_id)
        .ok_or(EffectUseCaseError::EffectNotFound)?;
    effect.reconcile(probe, now_unix_ms)?;
    let updated = effect.clone();
    let certainty = reconcile_replacement_certainty(probe);
    let event = RunEvent::new(
        run_id,
        next_sequence(&reconstructed.events),
        RunEventKind::EffectSettled {
            effect_id,
            certainty,
        },
        now_unix_ms,
    );
    repository
        .persist_effect_settled(&updated, &event)
        .map_err(EffectUseCaseError::Repository)?;
    Ok(updated)
}

/// Persists a quiescent checkpoint projection.
pub fn checkpoint_run<R: RunRepository>(
    repository: &R,
    run_id: HarnessRunId,
    plan: &ExecutionPlanVersion,
    graph: &RunGraphVersion,
    now_unix_ms: i64,
) -> Result<Checkpoint, CheckpointError<R::Error>> {
    let reconstructed = repository
        .reconstruct_run(&run_id)
        .map_err(CheckpointError::Repository)?
        .ok_or(CheckpointError::RunNotFound)?;
    if !is_quiescent_for_checkpoint(&reconstructed.effects) {
        return Err(CheckpointError::NotQuiescent);
    }
    if matches!(
        derive_lifecycle(&reconstructed.events, &reconstructed.effects),
        HarnessRunLifecycle::BlockedReconciliationRequired
    ) {
        return Err(CheckpointError::RunBlocked);
    }
    let last_event_sequence = reconstructed
        .events
        .last()
        .map(RunEvent::sequence)
        .unwrap_or(0);
    let checkpoint = Checkpoint::new(
        run_id,
        last_event_sequence,
        hash_event_chain(&reconstructed.events),
        plan.id(),
        graph.id(),
        plan.execution_policy_revision(),
        plan.replacement().postimage_hash(),
        now_unix_ms,
    );
    let event = RunEvent::new(
        run_id,
        next_sequence(&reconstructed.events),
        RunEventKind::Checkpointed {
            checkpoint_id: checkpoint.id(),
        },
        now_unix_ms,
    );
    repository
        .persist_quiescent_checkpoint(&PersistCheckpointIntent {
            checkpoint: checkpoint.clone(),
            event,
        })
        .map_err(CheckpointError::Repository)?;
    Ok(checkpoint)
}

/// Records native structural validation.
pub fn validate_native_structural<R: RunRepository>(
    repository: &R,
    run_id: HarnessRunId,
    plan: &ExecutionPlanVersion,
    graph: &RunGraphVersion,
    observed_postimage_hash: impl Into<String>,
    native_diff_hash: impl Into<String>,
    now_unix_ms: i64,
) -> Result<ValidationResult, ValidationUseCaseError<R::Error>> {
    let reconstructed = repository
        .reconstruct_run(&run_id)
        .map_err(ValidationUseCaseError::Repository)?
        .ok_or(ValidationUseCaseError::RunNotFound)?;
    let validation = ValidationResult::native_structural(
        run_id,
        plan.id(),
        graph.id(),
        plan.replacement().postimage_hash(),
        observed_postimage_hash,
        native_diff_hash,
        now_unix_ms,
    );
    debug_assert_eq!(validation.label(), NATIVE_STRUCTURAL_VALIDATION_LABEL);
    let event = RunEvent::new(
        run_id,
        next_sequence(&reconstructed.events),
        RunEventKind::Validated {
            validation_id: validation.id(),
        },
        now_unix_ms,
    );
    repository
        .persist_validation(&validation, &event)
        .map_err(ValidationUseCaseError::Repository)?;
    Ok(validation)
}

/// Completes a run with a Final Work Result.
pub fn complete_run<R: RunRepository>(
    repository: &R,
    run_id: HarnessRunId,
    plan: &ExecutionPlanVersion,
    graph: &RunGraphVersion,
    instrumentation: ComparisonInstrumentation,
    cohort: Option<TaskCohortAssignment>,
    now_unix_ms: i64,
) -> Result<FinalWorkResult, CompleteError<R::Error>> {
    let reconstructed = repository
        .reconstruct_run(&run_id)
        .map_err(CompleteError::Repository)?
        .ok_or(CompleteError::RunNotFound)?;
    let fingerprint = GraphShapeFingerprint::derive(graph, plan.execution_policy_revision());
    let result = FinalWorkResult::new(
        run_id,
        plan.id(),
        graph.id(),
        instrumentation,
        fingerprint,
        cohort.or_else(|| reconstructed.run.cohort().cloned()),
        now_unix_ms,
    );
    let event = RunEvent::new(
        run_id,
        next_sequence(&reconstructed.events),
        RunEventKind::Completed,
        now_unix_ms,
    );
    repository
        .persist_final_result(&result, &event)
        .map_err(CompleteError::Repository)?;
    Ok(result)
}

/// Pauses a run at a quiescent lifecycle boundary.
pub fn pause_run<R: RunRepository>(
    repository: &R,
    run_id: HarnessRunId,
    now_unix_ms: i64,
) -> Result<(), LifecycleUseCaseError<R::Error>> {
    let reconstructed = repository
        .reconstruct_run(&run_id)
        .map_err(LifecycleUseCaseError::Repository)?
        .ok_or(LifecycleUseCaseError::RunNotFound)?;
    let lifecycle = derive_lifecycle(&reconstructed.events, &reconstructed.effects);
    if lifecycle.is_terminal()
        || matches!(
            lifecycle,
            HarnessRunLifecycle::BlockedReconciliationRequired
        )
        || !is_quiescent_for_checkpoint(&reconstructed.effects)
    {
        return Err(LifecycleUseCaseError::UnsafeBoundary);
    }
    let event = RunEvent::new(
        run_id,
        next_sequence(&reconstructed.events),
        RunEventKind::Paused,
        now_unix_ms,
    );
    repository
        .append_event(&event)
        .map_err(LifecycleUseCaseError::Repository)?;
    Ok(())
}

/// Cancels a run at a safe boundary. Terminal and blocked-unknown states reject cancel.
pub fn cancel_run<R: RunRepository>(
    repository: &R,
    run_id: HarnessRunId,
    now_unix_ms: i64,
) -> Result<(), LifecycleUseCaseError<R::Error>> {
    let reconstructed = repository
        .reconstruct_run(&run_id)
        .map_err(LifecycleUseCaseError::Repository)?
        .ok_or(LifecycleUseCaseError::RunNotFound)?;
    let lifecycle = derive_lifecycle(&reconstructed.events, &reconstructed.effects);
    if lifecycle.is_terminal()
        || matches!(
            lifecycle,
            HarnessRunLifecycle::BlockedReconciliationRequired
        )
        || !is_quiescent_for_checkpoint(&reconstructed.effects)
    {
        return Err(LifecycleUseCaseError::UnsafeBoundary);
    }
    let event = RunEvent::new(
        run_id,
        next_sequence(&reconstructed.events),
        RunEventKind::Cancelled,
        now_unix_ms,
    );
    repository
        .append_event(&event)
        .map_err(LifecycleUseCaseError::Repository)?;
    Ok(())
}

/// Evaluates resume without reviving expired grants or redispatched confirmed writes.
pub fn resume_run<C: Clock>(
    reconstructed: &ReconstructedRun,
    revalidation: &ResumeRevalidation,
    clock: &C,
) -> ResumeDecision {
    let now = clock.unix_ms();
    let expired: Vec<_> = reconstructed
        .grants
        .iter()
        .filter(|grant| grant.is_revoked() || now >= grant.expires_at_unix_ms())
        .map(CapabilityGrant::id)
        .collect();
    let confirmed_replacement = reconstructed.effects.iter().find_map(|effect| {
        if effect.operation_id() == OP_CREATE_OR_REPLACE_V1
            && matches!(
                effect.certainty(),
                Some(EffectCertainty::ConfirmedCommitted)
            )
        {
            Some(effect.id())
        } else {
            None
        }
    });
    evaluate_resume(revalidation, &expired, confirmed_replacement)
}

/// Creates a run.
pub fn create_run<R: RunRepository>(
    repository: &R,
    run_root_display_name: impl Into<String>,
    cohort: Option<TaskCohortAssignment>,
    now_unix_ms: i64,
) -> Result<HarnessRun, CreateRunError<R::Error>> {
    let run = HarnessRun::new(run_root_display_name, now_unix_ms, cohort);
    let event = RunEvent::new(run.id(), 1, RunEventKind::RunCreated, now_unix_ms);
    repository
        .create_run(&run, &event)
        .map_err(CreateRunError::Repository)?;
    Ok(run)
}

/// Acquires the exclusive root lease.
pub fn acquire_root_lease<R: RunRepository>(
    repository: &R,
    run_id: HarnessRunId,
    owner_process_instance: impl Into<String>,
    now_unix_ms: i64,
) -> Result<RootLease, LeaseUseCaseError<R::Error>> {
    let reconstructed = repository
        .reconstruct_run(&run_id)
        .map_err(LeaseUseCaseError::Repository)?
        .ok_or(LeaseUseCaseError::RunNotFound)?;
    if let Some(existing) = &reconstructed.lease
        && !existing.is_expired(now_unix_ms)
    {
        return Err(LeaseUseCaseError::LeaseHeld);
    }
    let lease = RootLease::acquire(run_id, owner_process_instance, now_unix_ms);
    debug_assert_eq!(lease.expires_at_unix_ms(), now_unix_ms + ROOT_LEASE_TTL_MS);
    let event = RunEvent::new(
        run_id,
        next_sequence(&reconstructed.events),
        RunEventKind::LeaseAcquired {
            lease_id: lease.id(),
        },
        now_unix_ms,
    );
    repository
        .acquire_lease(&AcquireLeaseIntent {
            lease: lease.clone(),
            event,
        })
        .map_err(LeaseUseCaseError::Repository)?;
    Ok(lease)
}

/// Releases the exclusive root lease.
pub fn release_root_lease<R: RunRepository>(
    repository: &R,
    run_id: HarnessRunId,
    lease_id: RootLeaseId,
    now_unix_ms: i64,
) -> Result<(), LeaseUseCaseError<R::Error>> {
    let reconstructed = repository
        .reconstruct_run(&run_id)
        .map_err(LeaseUseCaseError::Repository)?
        .ok_or(LeaseUseCaseError::RunNotFound)?;
    let event = RunEvent::new(
        run_id,
        next_sequence(&reconstructed.events),
        RunEventKind::LeaseReleased { lease_id },
        now_unix_ms,
    );
    repository
        .release_lease(&ReleaseLeaseIntent {
            run_id,
            lease_id,
            event,
        })
        .map_err(LeaseUseCaseError::Repository)?;
    Ok(())
}

/// Takes over a lease only when prior-owner absence and reconciliation are confirmed.
pub fn takeover_root_lease<R: RunRepository>(
    repository: &R,
    run_id: HarnessRunId,
    owner_process_instance: impl Into<String>,
    prior_owner_confirmed_gone: bool,
    now_unix_ms: i64,
) -> Result<RootLease, LeaseUseCaseError<R::Error>> {
    let reconstructed = repository
        .reconstruct_run(&run_id)
        .map_err(LeaseUseCaseError::Repository)?
        .ok_or(LeaseUseCaseError::RunNotFound)?;
    let unsettled = reconstructed.effects.iter().any(|effect| {
        matches!(
            effect.phase(),
            EffectJournalPhase::Claimed | EffectJournalPhase::Dispatched
        ) || matches!(effect.certainty(), Some(EffectCertainty::UnknownOrPartial))
    });
    if let Some(existing) = &reconstructed.lease {
        existing
            .authorize_takeover(prior_owner_confirmed_gone, !unsettled)
            .map_err(LeaseUseCaseError::Lease)?;
    } else if !prior_owner_confirmed_gone {
        return Err(LeaseUseCaseError::Lease(LeaseError::PriorOwnerAlive));
    }
    let lease = RootLease::acquire(run_id, owner_process_instance, now_unix_ms);
    let event = RunEvent::new(
        run_id,
        next_sequence(&reconstructed.events),
        RunEventKind::LeaseTakeover {
            lease_id: lease.id(),
        },
        now_unix_ms,
    );
    repository
        .takeover_lease(&TakeoverLeaseIntent {
            lease: lease.clone(),
            event,
        })
        .map_err(LeaseUseCaseError::Repository)?;
    Ok(lease)
}

/// Helper constants re-exported for callers composing bootstrap grants.
#[must_use]
pub const fn bootstrap_local_read_ttl_ms() -> i64 {
    BOOTSTRAP_GRANT_TTL_MS
}

/// Helper for post-approval grant TTL.
#[must_use]
pub const fn post_approval_grant_ttl_ms() -> i64 {
    POST_APPROVAL_GRANT_TTL_MS
}

/// Operation ids used by Work 0022 registered operations.
#[must_use]
pub const fn replacement_operation_id() -> &'static str {
    OP_CREATE_OR_REPLACE_V1
}

/// Native inspection operation id.
#[must_use]
pub const fn inspection_operation_id() -> &'static str {
    OP_NATIVE_INSPECT_V1
}

/// Provider disclose operation id.
#[must_use]
pub const fn disclose_operation_id() -> &'static str {
    OP_PROVIDER_DISCLOSE_V1
}

/// Task node kind for the fixed graph.
#[must_use]
pub const fn replace_node_kind() -> &'static str {
    NODE_REPLACE_EXISTING_FILE_V1
}

/// Compile/freeze failure.
#[derive(Debug)]
pub enum CompileFreezeError<E: Error> {
    /// Run was not found.
    RunNotFound,
    /// Bootstrap proposal rejected.
    Proposal(BootstrapProposalError),
    /// Repository failure.
    Repository(E),
}

impl<E: Error> From<BootstrapProposalError> for CompileFreezeError<E> {
    fn from(error: BootstrapProposalError) -> Self {
        Self::Proposal(error)
    }
}

impl<E: Error> fmt::Display for CompileFreezeError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RunNotFound => formatter.write_str("harness run was not found"),
            Self::Proposal(error) => error.fmt(formatter),
            Self::Repository(error) => write!(formatter, "{error}"),
        }
    }
}

impl<E: Error + 'static> Error for CompileFreezeError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Proposal(error) => Some(error),
            Self::Repository(error) => Some(error),
            Self::RunNotFound => None,
        }
    }
}

/// Approval use-case failure.
#[derive(Debug)]
pub enum ApproveError<E: Error> {
    /// Run was not found.
    RunNotFound,
    /// Approval construction failed.
    Approval(ApprovalError),
    /// Repository failure.
    Repository(E),
}

impl<E: Error> From<ApprovalError> for ApproveError<E> {
    fn from(error: ApprovalError) -> Self {
        Self::Approval(error)
    }
}

impl<E: Error> fmt::Display for ApproveError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RunNotFound => formatter.write_str("harness run was not found"),
            Self::Approval(error) => error.fmt(formatter),
            Self::Repository(error) => write!(formatter, "{error}"),
        }
    }
}

impl<E: Error + 'static> Error for ApproveError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Approval(error) => Some(error),
            Self::Repository(error) => Some(error),
            Self::RunNotFound => None,
        }
    }
}

/// Grant use-case failure.
#[derive(Debug)]
pub enum GrantUseCaseError<E: Error> {
    /// Run was not found.
    RunNotFound,
    /// Grant was not found.
    GrantNotFound,
    /// Grant construction/mutation failed.
    Grant(CapabilityGrantError),
    /// Default-deny evaluation denied.
    Denied(GrantDenialReason),
    /// Repository failure.
    Repository(E),
}

/// Pause/cancel lifecycle transition failure.
#[derive(Debug)]
pub enum LifecycleUseCaseError<E: Error> {
    /// Run was not found.
    RunNotFound,
    /// Transition is unsafe at the current boundary.
    UnsafeBoundary,
    /// Repository failure.
    Repository(E),
}

impl<E: Error> fmt::Display for LifecycleUseCaseError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RunNotFound => formatter.write_str("harness run was not found"),
            Self::UnsafeBoundary => {
                formatter.write_str("lifecycle transition is unsafe at this boundary")
            }
            Self::Repository(error) => write!(formatter, "{error}"),
        }
    }
}

impl<E: Error + 'static> Error for LifecycleUseCaseError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Repository(error) => Some(error),
            Self::RunNotFound | Self::UnsafeBoundary => None,
        }
    }
}

impl<E: Error> From<CapabilityGrantError> for GrantUseCaseError<E> {
    fn from(error: CapabilityGrantError) -> Self {
        Self::Grant(error)
    }
}

impl<E: Error> fmt::Display for GrantUseCaseError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RunNotFound => formatter.write_str("harness run was not found"),
            Self::GrantNotFound => formatter.write_str("capability grant was not found"),
            Self::Grant(error) => error.fmt(formatter),
            Self::Denied(reason) => reason.fmt(formatter),
            Self::Repository(error) => write!(formatter, "{error}"),
        }
    }
}

impl<E: Error + 'static> Error for GrantUseCaseError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Grant(error) => Some(error),
            Self::Repository(error) => Some(error),
            Self::RunNotFound | Self::GrantNotFound | Self::Denied(_) => None,
        }
    }
}

/// Effect use-case failure.
#[derive(Debug)]
pub enum EffectUseCaseError<E: Error> {
    /// Run was not found.
    RunNotFound,
    /// Effect was not found.
    EffectNotFound,
    /// Whole run is blocked on unknown certainty.
    RunBlocked,
    /// Effect transition failed.
    Effect(EffectError),
    /// Repository failure.
    Repository(E),
}

impl<E: Error> From<EffectError> for EffectUseCaseError<E> {
    fn from(error: EffectError) -> Self {
        Self::Effect(error)
    }
}

impl<E: Error> fmt::Display for EffectUseCaseError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RunNotFound => formatter.write_str("harness run was not found"),
            Self::EffectNotFound => formatter.write_str("effect was not found"),
            Self::RunBlocked => formatter.write_str("run is blocked pending reconciliation"),
            Self::Effect(error) => error.fmt(formatter),
            Self::Repository(error) => write!(formatter, "{error}"),
        }
    }
}

impl<E: Error + 'static> Error for EffectUseCaseError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Effect(error) => Some(error),
            Self::Repository(error) => Some(error),
            Self::RunNotFound | Self::EffectNotFound | Self::RunBlocked => None,
        }
    }
}

/// Checkpoint use-case failure.
#[derive(Debug)]
pub enum CheckpointError<E: Error> {
    /// Run was not found.
    RunNotFound,
    /// Effects are not quiescent.
    NotQuiescent,
    /// Run is blocked.
    RunBlocked,
    /// Repository failure.
    Repository(E),
}

impl<E: Error> fmt::Display for CheckpointError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RunNotFound => formatter.write_str("harness run was not found"),
            Self::NotQuiescent => {
                formatter.write_str("checkpoint requires a quiescent effect boundary")
            }
            Self::RunBlocked => formatter.write_str("run is blocked pending reconciliation"),
            Self::Repository(error) => write!(formatter, "{error}"),
        }
    }
}

impl<E: Error + 'static> Error for CheckpointError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Repository(error) => Some(error),
            Self::RunNotFound | Self::NotQuiescent | Self::RunBlocked => None,
        }
    }
}

/// Validation use-case failure.
#[derive(Debug)]
pub enum ValidationUseCaseError<E: Error> {
    /// Run was not found.
    RunNotFound,
    /// Repository failure.
    Repository(E),
}

impl<E: Error> fmt::Display for ValidationUseCaseError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RunNotFound => formatter.write_str("harness run was not found"),
            Self::Repository(error) => write!(formatter, "{error}"),
        }
    }
}

impl<E: Error + 'static> Error for ValidationUseCaseError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Repository(error) => Some(error),
            Self::RunNotFound => None,
        }
    }
}

/// Completion use-case failure.
#[derive(Debug)]
pub enum CompleteError<E: Error> {
    /// Run was not found.
    RunNotFound,
    /// Repository failure.
    Repository(E),
}

impl<E: Error> fmt::Display for CompleteError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RunNotFound => formatter.write_str("harness run was not found"),
            Self::Repository(error) => write!(formatter, "{error}"),
        }
    }
}

impl<E: Error + 'static> Error for CompleteError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Repository(error) => Some(error),
            Self::RunNotFound => None,
        }
    }
}

/// Create-run failure.
#[derive(Debug)]
pub enum CreateRunError<E: Error> {
    /// Repository failure.
    Repository(E),
}

impl<E: Error> fmt::Display for CreateRunError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Repository(error) => write!(formatter, "{error}"),
        }
    }
}

impl<E: Error + 'static> Error for CreateRunError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Repository(error) => Some(error),
        }
    }
}

/// Lease use-case failure.
#[derive(Debug)]
pub enum LeaseUseCaseError<E: Error> {
    /// Run was not found.
    RunNotFound,
    /// Lease is already held.
    LeaseHeld,
    /// Lease policy failure.
    Lease(LeaseError),
    /// Repository failure.
    Repository(E),
}

impl<E: Error> fmt::Display for LeaseUseCaseError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RunNotFound => formatter.write_str("harness run was not found"),
            Self::LeaseHeld => formatter.write_str("root lease is already held"),
            Self::Lease(error) => error.fmt(formatter),
            Self::Repository(error) => write!(formatter, "{error}"),
        }
    }
}

impl<E: Error + 'static> Error for LeaseUseCaseError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Lease(error) => Some(error),
            Self::Repository(error) => Some(error),
            Self::RunNotFound | Self::LeaseHeld => None,
        }
    }
}

/// In-memory fake repository for core proofs.
#[derive(Debug, Default)]
pub struct MemoryRunRepository {
    inner: Mutex<HashMap<HarnessRunId, ReconstructedRun>>,
}

/// In-memory repository failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryRunRepositoryError {
    /// Run was not found.
    NotFound,
    /// Event sequence was not the next value.
    SequenceGap {
        /// Expected sequence.
        expected: u64,
        /// Provided sequence.
        actual: u64,
    },
    /// Effect claim lost the race.
    ClaimLost,
    /// Effect was not prepared.
    NotPrepared,
    /// Grant budget or validity failed.
    GrantDenied,
    /// Checkpoint was not quiescent.
    NotQuiescent,
    /// Lease conflict.
    LeaseConflict,
    /// Lock poisoned.
    LockPoisoned,
}

impl fmt::Display for MemoryRunRepositoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => formatter.write_str("run not found"),
            Self::SequenceGap { expected, actual } => {
                write!(
                    formatter,
                    "event sequence gap: expected {expected}, got {actual}"
                )
            }
            Self::ClaimLost => formatter.write_str("effect claim lost"),
            Self::NotPrepared => formatter.write_str("effect is not prepared"),
            Self::GrantDenied => formatter.write_str("grant dispatch denied"),
            Self::NotQuiescent => formatter.write_str("effects are not quiescent"),
            Self::LeaseConflict => formatter.write_str("lease conflict"),
            Self::LockPoisoned => formatter.write_str("repository lock poisoned"),
        }
    }
}

impl Error for MemoryRunRepositoryError {}

impl MemoryRunRepository {
    /// Creates an empty repository.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(
        &self,
    ) -> Result<MutexGuard<'_, HashMap<HarnessRunId, ReconstructedRun>>, MemoryRunRepositoryError>
    {
        self.inner
            .lock()
            .map_err(|_| MemoryRunRepositoryError::LockPoisoned)
    }

    fn append_checked(
        state: &mut ReconstructedRun,
        event: &RunEvent,
    ) -> Result<(), MemoryRunRepositoryError> {
        let expected = next_sequence(&state.events);
        if event.sequence() != expected {
            return Err(MemoryRunRepositoryError::SequenceGap {
                expected,
                actual: event.sequence(),
            });
        }
        state.events.push(event.clone());
        Ok(())
    }
}

impl RunRepository for MemoryRunRepository {
    type Error = MemoryRunRepositoryError;

    fn create_run(&self, run: &HarnessRun, event: &RunEvent) -> Result<(), Self::Error> {
        let mut guard = self.lock()?;
        guard.insert(
            run.id(),
            ReconstructedRun {
                run: run.clone(),
                events: vec![event.clone()],
                plans: Vec::new(),
                graphs: Vec::new(),
                replacements: Vec::new(),
                approvals: Vec::new(),
                grants: Vec::new(),
                effects: Vec::new(),
                checkpoints: Vec::new(),
                validations: Vec::new(),
                denials: Vec::new(),
                lease: None,
                final_result: None,
            },
        );
        Ok(())
    }

    fn append_event(&self, event: &RunEvent) -> Result<(), Self::Error> {
        let mut guard = self.lock()?;
        let state = guard
            .get_mut(&event.run_id())
            .ok_or(MemoryRunRepositoryError::NotFound)?;
        Self::append_checked(state, event)
    }

    fn persist_frozen_pair(
        &self,
        plan: &ExecutionPlanVersion,
        graph: &RunGraphVersion,
        event: &RunEvent,
    ) -> Result<(), Self::Error> {
        let mut guard = self.lock()?;
        let state = guard
            .get_mut(&event.run_id())
            .ok_or(MemoryRunRepositoryError::NotFound)?;
        Self::append_checked(state, event)?;
        state.replacements.push(plan.replacement().clone());
        state.plans.push(plan.clone());
        state.graphs.push(graph.clone());
        Ok(())
    }

    fn persist_approval(
        &self,
        approval: &ApprovalRecord,
        event: &RunEvent,
    ) -> Result<(), Self::Error> {
        let mut guard = self.lock()?;
        let state = guard
            .get_mut(&approval.run_id())
            .ok_or(MemoryRunRepositoryError::NotFound)?;
        Self::append_checked(state, event)?;
        state.approvals.push(approval.clone());
        Ok(())
    }

    fn persist_grant(&self, grant: &CapabilityGrant, event: &RunEvent) -> Result<(), Self::Error> {
        let mut guard = self.lock()?;
        let state = guard
            .get_mut(&grant.run_id())
            .ok_or(MemoryRunRepositoryError::NotFound)?;
        Self::append_checked(state, event)?;
        state.grants.push(grant.clone());
        Ok(())
    }

    fn persist_grant_revocation(
        &self,
        grant: &CapabilityGrant,
        event: &RunEvent,
    ) -> Result<(), Self::Error> {
        let mut guard = self.lock()?;
        let state = guard
            .get_mut(&grant.run_id())
            .ok_or(MemoryRunRepositoryError::NotFound)?;
        Self::append_checked(state, event)?;
        if let Some(existing) = state.grants.iter_mut().find(|item| item.id() == grant.id()) {
            *existing = grant.clone();
        }
        Ok(())
    }

    fn persist_denial(&self, denial: &DenialEvidence, event: &RunEvent) -> Result<(), Self::Error> {
        let mut guard = self.lock()?;
        let state = guard
            .get_mut(&denial.run_id())
            .ok_or(MemoryRunRepositoryError::NotFound)?;
        Self::append_checked(state, event)?;
        state.denials.push(denial.clone());
        Ok(())
    }

    fn persist_prepared_effect(
        &self,
        effect: &EffectRecord,
        event: &RunEvent,
    ) -> Result<(), Self::Error> {
        let mut guard = self.lock()?;
        let state = guard
            .get_mut(&effect.run_id())
            .ok_or(MemoryRunRepositoryError::NotFound)?;
        Self::append_checked(state, event)?;
        state.effects.push(effect.clone());
        Ok(())
    }

    fn claim_effect(&self, intent: &ClaimEffectIntent) -> Result<EffectRecord, Self::Error> {
        let mut guard = self.lock()?;
        let state = guard
            .get_mut(&intent.run_id)
            .ok_or(MemoryRunRepositoryError::NotFound)?;
        if intent.event.run_id() != intent.run_id {
            return Err(MemoryRunRepositoryError::NotFound);
        }
        let effect = state
            .effects
            .iter_mut()
            .find(|effect| effect.id() == intent.effect_id)
            .ok_or(MemoryRunRepositoryError::NotFound)?;
        if effect.phase() != EffectJournalPhase::Prepared {
            return Err(MemoryRunRepositoryError::NotPrepared);
        }
        effect
            .claim(intent.claimant.clone(), intent.now_unix_ms)
            .map_err(|_| MemoryRunRepositoryError::ClaimLost)?;
        let claimed = effect.clone();
        Self::append_checked(state, &intent.event)?;
        Ok(claimed)
    }

    fn persist_effect_dispatched(
        &self,
        effect: &EffectRecord,
        event: &RunEvent,
    ) -> Result<(), Self::Error> {
        let mut guard = self.lock()?;
        let state = guard
            .get_mut(&effect.run_id())
            .ok_or(MemoryRunRepositoryError::NotFound)?;
        Self::append_checked(state, event)?;
        if let Some(existing) = state
            .effects
            .iter_mut()
            .find(|item| item.id() == effect.id())
        {
            *existing = effect.clone();
        }
        Ok(())
    }

    fn persist_effect_settled(
        &self,
        effect: &EffectRecord,
        event: &RunEvent,
    ) -> Result<(), Self::Error> {
        let mut guard = self.lock()?;
        let state = guard
            .get_mut(&effect.run_id())
            .ok_or(MemoryRunRepositoryError::NotFound)?;
        Self::append_checked(state, event)?;
        if let Some(existing) = state
            .effects
            .iter_mut()
            .find(|item| item.id() == effect.id())
        {
            *existing = effect.clone();
        }
        Ok(())
    }

    fn consume_dispatch_budget(
        &self,
        intent: &ConsumeDispatchBudgetIntent,
    ) -> Result<CapabilityGrant, Self::Error> {
        let mut guard = self.lock()?;
        let state = guard
            .get_mut(&intent.run_id)
            .ok_or(MemoryRunRepositoryError::NotFound)?;
        let grant = state
            .grants
            .iter_mut()
            .find(|grant| grant.id() == intent.grant_id)
            .ok_or(MemoryRunRepositoryError::NotFound)?;
        if grant.is_revoked() || intent.now_unix_ms >= grant.expires_at_unix_ms() {
            return Err(MemoryRunRepositoryError::GrantDenied);
        }
        grant
            .consume_dispatch()
            .map_err(|_| MemoryRunRepositoryError::GrantDenied)?;
        Ok(grant.clone())
    }

    fn persist_quiescent_checkpoint(
        &self,
        intent: &PersistCheckpointIntent,
    ) -> Result<(), Self::Error> {
        let mut guard = self.lock()?;
        let state = guard
            .get_mut(&intent.checkpoint.run_id())
            .ok_or(MemoryRunRepositoryError::NotFound)?;
        if !is_quiescent_for_checkpoint(&state.effects) {
            return Err(MemoryRunRepositoryError::NotQuiescent);
        }
        Self::append_checked(state, &intent.event)?;
        state.checkpoints.push(intent.checkpoint.clone());
        Ok(())
    }

    fn persist_validation(
        &self,
        validation: &ValidationResult,
        event: &RunEvent,
    ) -> Result<(), Self::Error> {
        let mut guard = self.lock()?;
        let state = guard
            .get_mut(&validation.run_id())
            .ok_or(MemoryRunRepositoryError::NotFound)?;
        Self::append_checked(state, event)?;
        state.validations.push(validation.clone());
        Ok(())
    }

    fn persist_final_result(
        &self,
        result: &FinalWorkResult,
        event: &RunEvent,
    ) -> Result<(), Self::Error> {
        let mut guard = self.lock()?;
        let state = guard
            .get_mut(&result.run_id())
            .ok_or(MemoryRunRepositoryError::NotFound)?;
        Self::append_checked(state, event)?;
        state.final_result = Some(result.clone());
        Ok(())
    }

    fn acquire_lease(&self, intent: &AcquireLeaseIntent) -> Result<(), Self::Error> {
        let mut guard = self.lock()?;
        let state = guard
            .get_mut(&intent.lease.run_id())
            .ok_or(MemoryRunRepositoryError::NotFound)?;
        if let Some(existing) = &state.lease
            && !existing.is_expired(intent.lease.acquired_at_unix_ms())
        {
            return Err(MemoryRunRepositoryError::LeaseConflict);
        }
        Self::append_checked(state, &intent.event)?;
        state.lease = Some(intent.lease.clone());
        Ok(())
    }

    fn release_lease(&self, intent: &ReleaseLeaseIntent) -> Result<(), Self::Error> {
        let mut guard = self.lock()?;
        let state = guard
            .get_mut(&intent.run_id)
            .ok_or(MemoryRunRepositoryError::NotFound)?;
        Self::append_checked(state, &intent.event)?;
        state.lease = None;
        Ok(())
    }

    fn takeover_lease(&self, intent: &TakeoverLeaseIntent) -> Result<(), Self::Error> {
        let mut guard = self.lock()?;
        let state = guard
            .get_mut(&intent.lease.run_id())
            .ok_or(MemoryRunRepositoryError::NotFound)?;
        Self::append_checked(state, &intent.event)?;
        state.lease = Some(intent.lease.clone());
        Ok(())
    }

    fn reconstruct_run(
        &self,
        run_id: &HarnessRunId,
    ) -> Result<Option<ReconstructedRun>, Self::Error> {
        let guard = self.lock()?;
        Ok(guard.get(run_id).cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        CONTROLLED_RELATIVE_TARGET, FakeClock, LeaseError, OP_LOCAL_READ_V1, ROOT_LEASE_TTL_MS,
    };

    fn preimage() -> String {
        "<!doctype html><html><body><h1>Ready</h1><p>ok</p></body></html>".to_owned()
    }

    fn postimage() -> String {
        "<!doctype html><html><body><h1>Ready for review</h1><p>ok</p></body></html>".to_owned()
    }

    #[test]
    fn use_cases_cover_approve_grant_effect_checkpoint_resume_and_lease() {
        let repo = MemoryRunRepository::new();
        let clock = FakeClock::new(1_000);
        let cohort = TaskCohortAssignment::new(
            "tax-v1",
            "static-heading-fixture",
            "owner",
            "work 0022",
            clock.unix_ms(),
        );
        let run = create_run(&repo, "fixture-root", Some(cohort.clone()), clock.unix_ms()).unwrap();
        let manifest = ContextManifest::new(&preimage(), "heading", "preview").unwrap();
        let (plan, graph) = compile_and_freeze_pair(
            &repo,
            run.id(),
            "change heading",
            "profile",
            "model",
            DisclosurePolicy::new("d1", "index.html"),
            CapabilityEnvelope::new(
                vec![
                    CapabilityType::CreateOrReplace,
                    CapabilityType::NativeInspection,
                ],
                "replace+inspect",
            ),
            manifest,
            &preimage(),
            postimage(),
            CONTROLLED_RELATIVE_TARGET,
            "fs-1",
            Some("req".to_owned()),
            Some("resp".to_owned()),
            clock.unix_ms(),
        )
        .unwrap();
        let approval =
            approve_pair(&repo, run.id(), &plan, &graph, "owner", clock.unix_ms()).unwrap();
        let pair = PlanGraphPairBinding {
            plan_version_id: plan.id(),
            graph_version_id: graph.id(),
        };
        let grant = issue_grant(
            &repo,
            run.id(),
            CapabilityType::CreateOrReplace,
            GrantResourceSelector::ReplacementTarget {
                relative_target: CONTROLLED_RELATIVE_TARGET.to_owned(),
                expected_preimage_hash: plan.replacement().preimage_hash().to_owned(),
                expected_postimage_hash: plan.replacement().postimage_hash().to_owned(),
            },
            GrantActionScope::Node(NODE_REPLACE_EXISTING_FILE_V1.to_owned()),
            Some(pair),
            Some(approval.id()),
            "owner",
            clock.unix_ms(),
        )
        .unwrap();

        // Approval without evaluating this grant cannot authorise dispatch.
        let read_resource = GrantResourceSelector::RelativeTarget("index.html".to_owned());
        let read_scope = GrantActionScope::Run;
        let wrong = GrantEvaluationRequest {
            run_id: run.id(),
            capability: CapabilityType::LocalRead,
            operation_id: OP_LOCAL_READ_V1,
            resource: &read_resource,
            action_scope: &read_scope,
            pair: None,
            now_unix_ms: clock.unix_ms(),
        };
        assert!(matches!(
            require_grant(&repo, &grant, &wrong),
            Err(GrantUseCaseError::Denied(_))
        ));

        let lease = acquire_root_lease(&repo, run.id(), "proc-1", clock.unix_ms()).unwrap();
        let effect = prepare_effect(
            &repo,
            run.id(),
            None,
            Some(plan.id()),
            Some(graph.id()),
            OP_CREATE_OR_REPLACE_V1,
            plan.replacement().postimage_hash(),
            grant.id(),
            clock.unix_ms(),
            Some(plan.replacement().preimage_hash().to_owned()),
            Some(plan.replacement().postimage_hash().to_owned()),
        )
        .unwrap();
        claim_effect(&repo, run.id(), effect.id(), "broker", clock.unix_ms()).unwrap();
        assert!(matches!(
            checkpoint_run(&repo, run.id(), &plan, &graph, clock.unix_ms()),
            Err(CheckpointError::NotQuiescent)
        ));
        dispatch_effect(
            &repo,
            run.id(),
            effect.id(),
            grant.id(),
            "broker",
            clock.unix_ms(),
        )
        .unwrap();
        settle_effect(
            &repo,
            run.id(),
            effect.id(),
            "broker",
            EffectOperationResult::Success,
            EffectCertainty::ConfirmedCommitted,
            clock.unix_ms(),
        )
        .unwrap();
        let checkpoint = checkpoint_run(&repo, run.id(), &plan, &graph, clock.unix_ms()).unwrap();
        assert_eq!(checkpoint.plan_version_id(), plan.id());
        let validation = validate_native_structural(
            &repo,
            run.id(),
            &plan,
            &graph,
            plan.replacement().postimage_hash(),
            plan.replacement().expected_diff_hash(),
            clock.unix_ms(),
        )
        .unwrap();
        assert!(validation.passed());
        let result = complete_run(
            &repo,
            run.id(),
            &plan,
            &graph,
            ComparisonInstrumentation {
                retries: Some(0),
                task_success: Some(true),
                ..ComparisonInstrumentation::default()
            },
            None,
            clock.unix_ms(),
        )
        .unwrap();
        assert_eq!(result.cohort().unwrap().cohort_id(), cohort.cohort_id());
        assert!(result.publication_stopped());

        let reconstructed = repo.reconstruct_run(&run.id()).unwrap().unwrap();
        let decision = resume_run(
            &reconstructed,
            &ResumeRevalidation {
                event_chain_matches: true,
                filesystem_matches_expected: true,
                execution_policy_matches: true,
                operation_versions_match: true,
            },
            &clock,
        );
        assert!(matches!(
            decision,
            ResumeDecision::SkipConfirmedReplacement { .. }
        ));

        release_root_lease(&repo, run.id(), lease.id(), clock.unix_ms()).unwrap();
        clock.advance(POST_APPROVAL_GRANT_TTL_MS);
        let reconstructed = repo.reconstruct_run(&run.id()).unwrap().unwrap();
        assert!(matches!(
            resume_run(
                &reconstructed,
                &ResumeRevalidation {
                    event_chain_matches: true,
                    filesystem_matches_expected: true,
                    execution_policy_matches: true,
                    operation_versions_match: true,
                },
                &clock,
            ),
            ResumeDecision::SkipConfirmedReplacement { .. }
                | ResumeDecision::RequireFreshGrant { .. }
        ));
    }

    #[test]
    fn expired_root_lease_can_be_reacquired_without_takeover() {
        let repo = MemoryRunRepository::new();
        let clock = FakeClock::new(1_000);
        let run = create_run(&repo, "root", None, clock.unix_ms()).unwrap();
        let first = acquire_root_lease(&repo, run.id(), "proc-1", clock.unix_ms()).unwrap();
        assert!(matches!(
            acquire_root_lease(&repo, run.id(), "proc-2", clock.unix_ms()),
            Err(LeaseUseCaseError::LeaseHeld)
        ));

        clock.advance(ROOT_LEASE_TTL_MS);
        assert!(first.is_expired(clock.unix_ms()));
        let second = acquire_root_lease(&repo, run.id(), "proc-2", clock.unix_ms()).unwrap();
        assert_ne!(first.id(), second.id());
        assert_eq!(second.owner_process_instance(), "proc-2");

        // Takeover remains a distinct path that still needs positive evidence.
        assert!(matches!(
            takeover_root_lease(&repo, run.id(), "proc-3", false, clock.unix_ms()),
            Err(LeaseUseCaseError::Lease(LeaseError::PriorOwnerAlive))
        ));
        let third = takeover_root_lease(&repo, run.id(), "proc-3", true, clock.unix_ms()).unwrap();
        assert_eq!(third.owner_process_instance(), "proc-3");
        assert_ne!(second.id(), third.id());
    }

    #[test]
    fn unknown_effect_blocks_whole_run_until_reconciled() {
        let repo = MemoryRunRepository::new();
        let run = create_run(&repo, "root", None, 1).unwrap();
        let grant = issue_grant(
            &repo,
            run.id(),
            CapabilityType::LocalRead,
            GrantResourceSelector::RelativeTarget("index.html".to_owned()),
            GrantActionScope::Run,
            None,
            None,
            "owner",
            1,
        )
        .unwrap();
        let effect = prepare_effect(
            &repo,
            run.id(),
            None,
            None,
            None,
            OP_LOCAL_READ_V1,
            "t",
            grant.id(),
            2,
            None,
            None,
        )
        .unwrap();
        claim_effect(&repo, run.id(), effect.id(), "broker", 3).unwrap();
        dispatch_effect(&repo, run.id(), effect.id(), grant.id(), "broker", 4).unwrap();
        settle_effect(
            &repo,
            run.id(),
            effect.id(),
            "broker",
            EffectOperationResult::Error,
            EffectCertainty::UnknownOrPartial,
            5,
        )
        .unwrap();
        let reconstructed = repo.reconstruct_run(&run.id()).unwrap().unwrap();
        assert_eq!(
            derive_lifecycle(&reconstructed.events, &reconstructed.effects),
            HarnessRunLifecycle::BlockedReconciliationRequired
        );
        assert!(matches!(
            prepare_effect(
                &repo,
                run.id(),
                None,
                None,
                None,
                OP_LOCAL_READ_V1,
                "t2",
                grant.id(),
                6,
                None,
                None,
            ),
            Err(EffectUseCaseError::RunBlocked)
        ));
        reconcile_effect(
            &repo,
            run.id(),
            effect.id(),
            ReconciliationProbe::MatchesPreimage,
            7,
        )
        .unwrap();
        let reconstructed = repo.reconstruct_run(&run.id()).unwrap().unwrap();
        assert_ne!(
            derive_lifecycle(&reconstructed.events, &reconstructed.effects),
            HarnessRunLifecycle::BlockedReconciliationRequired
        );
    }
}
