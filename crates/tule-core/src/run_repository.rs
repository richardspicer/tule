//! Semantic repository contracts for Harness Run persistence.

use std::error::Error;

use crate::{
    ApprovalRecord, CapabilityGrant, CapabilityGrantId, Checkpoint, DenialEvidence, EffectRecord,
    EffectRecordId, ExecutionPlanVersion, FinalWorkResult, HarnessRun, HarnessRunId,
    ReplacementContentInput, RootLease, RunEvent, RunGraphVersion, ValidationResult,
};

/// Reconstructed durable state for one Harness Run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconstructedRun {
    /// Run header.
    pub run: HarnessRun,
    /// Ordered append-only events.
    pub events: Vec<RunEvent>,
    /// Frozen plan versions in lineage order.
    pub plans: Vec<ExecutionPlanVersion>,
    /// Frozen graph versions in lineage order.
    pub graphs: Vec<RunGraphVersion>,
    /// Replacement content inputs.
    pub replacements: Vec<ReplacementContentInput>,
    /// Approval records.
    pub approvals: Vec<ApprovalRecord>,
    /// Capability grants.
    pub grants: Vec<CapabilityGrant>,
    /// Effect records.
    pub effects: Vec<EffectRecord>,
    /// Quiescent checkpoints.
    pub checkpoints: Vec<Checkpoint>,
    /// Validation results.
    pub validations: Vec<ValidationResult>,
    /// Denial evidence.
    pub denials: Vec<DenialEvidence>,
    /// Optional active root lease.
    pub lease: Option<RootLease>,
    /// Optional final work result.
    pub final_result: Option<FinalWorkResult>,
}

/// Atomic intent to claim one prepared effect for a single claimant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimEffectIntent {
    /// Run that owns the effect.
    pub run_id: HarnessRunId,
    /// Effect to claim.
    pub effect_id: EffectRecordId,
    /// Claimant identity.
    pub claimant: String,
    /// Claim time.
    pub now_unix_ms: i64,
    /// Claim event persisted in the same atomic unit as the claim.
    pub event: RunEvent,
}

/// Atomic intent to consume one dispatch budget from a grant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsumeDispatchBudgetIntent {
    /// Run that owns the grant.
    pub run_id: HarnessRunId,
    /// Grant to consume.
    pub grant_id: CapabilityGrantId,
    /// Evaluation time.
    pub now_unix_ms: i64,
}

/// Atomic intent to persist one quiescent checkpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistCheckpointIntent {
    /// Checkpoint projection.
    pub checkpoint: Checkpoint,
    /// Accompanying checkpoint event.
    pub event: RunEvent,
}

/// Atomic lease acquisition intent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcquireLeaseIntent {
    /// Lease to acquire.
    pub lease: RootLease,
    /// Accompanying event.
    pub event: RunEvent,
}

/// Atomic lease release intent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseLeaseIntent {
    /// Run whose lease is released.
    pub run_id: HarnessRunId,
    /// Lease identity.
    pub lease_id: crate::RootLeaseId,
    /// Accompanying event.
    pub event: RunEvent,
}

/// Atomic lease takeover intent after safety checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TakeoverLeaseIntent {
    /// Replacement lease.
    pub lease: RootLease,
    /// Accompanying event.
    pub event: RunEvent,
}

/// Semantic repository for Harness Runs.
///
/// Methods express atomic intents rather than low-level mutable CRUD.
pub trait RunRepository: Send + Sync {
    /// Implementation-specific storage failure.
    type Error: Error + Send + Sync + 'static;

    /// Persists a newly created run and its creation event.
    fn create_run(&self, run: &HarnessRun, event: &RunEvent) -> Result<(), Self::Error>;

    /// Appends one ordered run event. Sequence must be exactly next.
    fn append_event(&self, event: &RunEvent) -> Result<(), Self::Error>;

    /// Persists a frozen plan/graph pair, replacement input, and freeze event atomically.
    fn persist_frozen_pair(
        &self,
        plan: &ExecutionPlanVersion,
        graph: &RunGraphVersion,
        event: &RunEvent,
    ) -> Result<(), Self::Error>;

    /// Persists an approval and its event atomically.
    fn persist_approval(
        &self,
        approval: &ApprovalRecord,
        event: &RunEvent,
    ) -> Result<(), Self::Error>;

    /// Persists a newly issued grant and its event atomically.
    fn persist_grant(&self, grant: &CapabilityGrant, event: &RunEvent) -> Result<(), Self::Error>;

    /// Persists grant revocation and its event atomically.
    fn persist_grant_revocation(
        &self,
        grant: &CapabilityGrant,
        event: &RunEvent,
    ) -> Result<(), Self::Error>;

    /// Persists denial evidence and its event atomically.
    fn persist_denial(&self, denial: &DenialEvidence, event: &RunEvent) -> Result<(), Self::Error>;

    /// Persists a prepared effect and its event atomically.
    fn persist_prepared_effect(
        &self,
        effect: &EffectRecord,
        event: &RunEvent,
    ) -> Result<(), Self::Error>;

    /// Atomically claims one prepared effect for a single claimant.
    fn claim_effect(&self, intent: &ClaimEffectIntent) -> Result<EffectRecord, Self::Error>;

    /// Persists dispatch transition and event for the successful claimant.
    fn persist_effect_dispatched(
        &self,
        effect: &EffectRecord,
        event: &RunEvent,
    ) -> Result<(), Self::Error>;

    /// Persists settlement and event.
    fn persist_effect_settled(
        &self,
        effect: &EffectRecord,
        event: &RunEvent,
    ) -> Result<(), Self::Error>;

    /// Atomically consumes one dispatch budget when the grant is still valid.
    fn consume_dispatch_budget(
        &self,
        intent: &ConsumeDispatchBudgetIntent,
    ) -> Result<CapabilityGrant, Self::Error>;

    /// Persists one quiescent checkpoint and event atomically.
    fn persist_quiescent_checkpoint(
        &self,
        intent: &PersistCheckpointIntent,
    ) -> Result<(), Self::Error>;

    /// Persists a validation result and event.
    fn persist_validation(
        &self,
        validation: &ValidationResult,
        event: &RunEvent,
    ) -> Result<(), Self::Error>;

    /// Persists the final work result and completion event.
    fn persist_final_result(
        &self,
        result: &FinalWorkResult,
        event: &RunEvent,
    ) -> Result<(), Self::Error>;

    /// Atomically acquires an exclusive root lease.
    fn acquire_lease(&self, intent: &AcquireLeaseIntent) -> Result<(), Self::Error>;

    /// Atomically releases an exclusive root lease.
    fn release_lease(&self, intent: &ReleaseLeaseIntent) -> Result<(), Self::Error>;

    /// Atomically takes over a lease after safety checks succeed.
    fn takeover_lease(&self, intent: &TakeoverLeaseIntent) -> Result<(), Self::Error>;

    /// Reconstructs complete durable run state.
    fn reconstruct_run(
        &self,
        run_id: &HarnessRunId,
    ) -> Result<Option<ReconstructedRun>, Self::Error>;
}
