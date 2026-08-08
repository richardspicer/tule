//! Tauri-independent domain and application behavior for TULE.
#![warn(missing_docs)]

mod agent;
mod agent_repository;
mod agent_source;
mod agent_use_cases;
mod artifact;
mod artifact_repository;
mod artifact_use_cases;
mod capability;
mod project;
mod provider_catalog;
mod repository;
mod run;
mod run_repository;
mod run_use_cases;
mod use_cases;

pub use agent::{
    AgentContextError, AgentEffort, AgentEvent, AgentEventId, AgentEventKind, AgentInputError,
    AgentOutputLimitError, AgentReconstructionError, AgentRequestContext, AgentSession,
    AgentSessionId, AgentTurn, AgentTurnFinishError, AgentTurnId, AgentTurnState,
    CHECKPOINT_BYTE_THRESHOLD, CHECKPOINT_INTERVAL_MS, CompletedTurnContext, FIXED_INSTRUCTION,
    IllegalAgentTurnTransition, InvalidAgentEffort, InvalidAgentEventKind, InvalidAgentId,
    InvalidAgentTurnState, MAX_AGENT_OUTPUT_UTF8, MAX_CONTEXT_UTF8, MAX_USER_TEXT_UTF8,
    PROMPT_VERSION, ProviderRequestId, TITLE_MAX_SCALARS, assemble_instructions,
    build_agent_request_context, derive_session_title, should_checkpoint, validate_user_text,
};
pub use agent_repository::{AgentRepository, ProviderProfile};
pub use agent_source::{
    ATTACHED_SOURCE_FRAME_VERSION, MAX_CANONICAL_URL_UTF8, MAX_FOLDER_MEMBERS, MAX_SOURCE_UTF8,
    SOURCE_ORIGIN_LOCAL_TEXT_FILE, SOURCE_ORIGIN_LOCAL_TEXT_FOLDER, SOURCE_ORIGIN_REMOTE_TEXT_URL,
    Source, SourceContext, SourceId, SourceReconstructionError, SourceValidationError, TurnSource,
    count_folder_members, derive_remote_source_display_name, format_turn_user_content,
    frame_folder_members, hash_source_bytes, validate_canonical_https_url, validate_source_content,
    validate_source_display_name,
};
pub use agent_use_cases::{
    ApplyAgentDeltaError, FinishAgentTurnError, InvalidProviderProfileId, PrepareAgentSendError,
    PreparedAgentSend, SetSessionProjectError, apply_agent_delta, cancel_agent_turn,
    checkpoint_agent_turn, complete_agent_turn, completed_history_from_turns, fail_agent_turn,
    interrupt_inflight_turns, prepare_agent_send, set_session_project,
};
pub use artifact::{
    Artifact, ArtifactDetail, ArtifactId, ArtifactKind, ArtifactReconstructionError,
    ArtifactSummary, ArtifactValidationError, ArtifactVersion, ArtifactVersionId,
    ArtifactVersionProvenance, InvalidArtifactKind, MAX_ARTIFACT_CONTENT_UTF8,
    derive_artifact_title, resolve_artifact_title, validate_artifact_content,
};
pub use artifact_repository::ArtifactRepository;
pub use artifact_use_cases::{
    CreateArtifactFromTurnError, GetArtifactError, ListArtifactsError, create_artifact_from_turn,
    get_artifact, list_artifacts_for_session_context,
};
pub use capability::{
    BOOTSTRAP_GRANT_TTL_MS, CapabilityGrant, CapabilityGrantError, CapabilityGrantId,
    CapabilityType, DEFAULT_DISPATCH_BUDGET, GrantActionScope, GrantDenialReason, GrantEvaluation,
    GrantEvaluationRequest, GrantResourceSelector, InvalidCapabilityType, OP_CREATE_OR_REPLACE_V1,
    OP_LOCAL_READ_V1, OP_NATIVE_INSPECT_V1, OP_PROVIDER_DISCLOSE_V1, POST_APPROVAL_GRANT_TTL_MS,
    PlanGraphPairBinding, REGISTERED_OPERATION_SCHEMA_V1, RegisteredOperationIdentity,
    evaluate_grant,
};
pub use project::{
    InvalidProjectId, InvalidProjectName, Project, ProjectId, ProjectName,
    ProjectReconstructionError, ProjectTimeError,
};
pub use provider_catalog::{
    CATALOG_DESCRIPTION_MAX_SCALARS, CATALOG_TTL_MS, CatalogCandidate, CatalogFreshness,
    InvalidModelId, MODEL_ID_MAX_UTF8, ModelCatalogEntry, SelectedDefaultResolution,
    catalog_freshness, is_usable_catalog_candidate, model_id_in_catalog, resolve_selected_default,
    select_usable_catalog_entries, validate_model_id,
};
pub use repository::ProjectRepository;
pub use run::{
    ApprovalError, ApprovalRecord, ApprovalRecordId, BOOTSTRAP_HEADING_AFTER,
    BOOTSTRAP_HEADING_BEFORE, BootstrapProposalError, CANONICAL_ENCODING_VERSION,
    CONTROLLED_RELATIVE_TARGET, CapabilityEnvelope, Checkpoint, CheckpointId, Clock,
    ComparisonInstrumentation, ContextManifest, ContextManifestId, DenialEvidence,
    DenialEvidenceId, DisclosurePolicy, EXECUTION_POLICY_REVISION_V1, EffectCertainty, EffectError,
    EffectJournalPhase, EffectOperationResult, EffectRecord, EffectRecordId, ExecutionPlanVersion,
    ExecutionPlanVersionId, FakeClock, FinalWorkResult, GRAPH_SHAPE_FINGERPRINT_VERSION, GraphEdge,
    GraphNode, GraphShapeFingerprint, HarnessRun, HarnessRunId, HarnessRunLifecycle, InvalidRunId,
    LeaseError, MAX_RUN_CONTENT_UTF8, NATIVE_STRUCTURAL_VALIDATION_LABEL,
    NODE_REPLACE_EXISTING_FILE_V1, NODE_VERIFY_APPROVED_POSTIMAGE_V1, NodeAttempt, NodeAttemptId,
    RETRY_RULE_NO_AUTOMATIC, ROOT_LEASE_RENEW_INTERVAL_MS, ROOT_LEASE_TTL_MS, ReconciliationProbe,
    ReplacementContentId, ReplacementContentInput, ResumeDecision, ResumeRevalidation, RootLease,
    RootLeaseId, RunContentError, RunEvent, RunEventId, RunEventKind, RunGraphVersion,
    RunGraphVersionId, SystemClock, TaskCohortAssignment, VALIDATION_RULE_NATIVE_POSTIMAGE_V1,
    ValidationResult, ValidationResultId, append_canonical_field, derive_lifecycle,
    evaluate_resume, hash_canonical_fields, hash_event_chain, hash_expected_diff,
    is_quiescent_for_checkpoint, reconcile_replacement_certainty, reject_unknown_proposal_fields,
    sha256_hex, validate_bootstrap_proposal, validate_run_content_bytes,
};
pub use run_repository::{
    AcquireLeaseIntent, ClaimEffectIntent, ConsumeDispatchBudgetIntent, PersistCheckpointIntent,
    ReconstructedRun, ReleaseLeaseIntent, RunRepository, TakeoverLeaseIntent,
};
pub use run_use_cases::{
    ApproveError, CheckpointError, CompileFreezeError, CompleteError, CreateRunError,
    EffectUseCaseError, GrantUseCaseError, LeaseUseCaseError, LifecycleUseCaseError,
    MemoryRunRepository, MemoryRunRepositoryError, ValidationUseCaseError, acquire_root_lease,
    approve_pair, bootstrap_local_read_ttl_ms, cancel_run, checkpoint_run, claim_effect,
    compile_and_freeze_pair, complete_run, create_run, disclose_operation_id, dispatch_effect,
    inspection_operation_id, issue_grant, pause_run, post_approval_grant_ttl_ms, prepare_effect,
    reconcile_effect, record_denial, release_root_lease, replace_node_kind,
    replacement_operation_id, require_grant, resume_run, revoke_grant, settle_effect,
    takeover_root_lease, validate_native_structural,
};
pub use use_cases::{
    CreateProjectError, OpenProjectError, UpdateProjectInstructionsError, create_project,
    list_projects, open_project, update_project_instructions,
};

/// Stable application identity exposed to TULE hosts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplicationInfo {
    /// Human-readable product name.
    pub name: String,
    /// Product version supplied by the core crate package metadata.
    pub version: String,
}

/// Returns TULE's application identity without depending on a host framework.
#[must_use]
pub fn get_application_info() -> ApplicationInfo {
    ApplicationInfo {
        name: "TULE".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn application_info_reports_the_core_identity() {
        let info = get_application_info();

        assert_eq!(
            info,
            ApplicationInfo {
                name: "TULE".to_owned(),
                version: env!("CARGO_PKG_VERSION").to_owned(),
            }
        );
    }
}
