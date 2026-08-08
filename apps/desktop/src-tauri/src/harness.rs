//! Separate Harness orchestration surface, independent of ordinary Agent commands.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tauri_plugin_dialog::{DialogExt, FilePath};
use tule_core::{
    BOOTSTRAP_HEADING_AFTER, BOOTSTRAP_HEADING_BEFORE, CONTROLLED_RELATIVE_TARGET,
    CapabilityEnvelope, CapabilityGrantId, CapabilityType, Clock, ComparisonInstrumentation,
    ContextManifest, DisclosurePolicy, EffectCertainty, EffectJournalPhase, ExecutionPlanVersion,
    GrantActionScope, GrantResourceSelector, HarnessRunId, HarnessRunLifecycle,
    NATIVE_STRUCTURAL_VALIDATION_LABEL, NODE_REPLACE_EXISTING_FILE_V1,
    NODE_VERIFY_APPROVED_POSTIMAGE_V1, PlanGraphPairBinding, ReconstructedRun, ResumeDecision,
    ResumeRevalidation, RunGraphVersion, RunRepository, SystemClock, acquire_root_lease,
    approve_pair, cancel_run, checkpoint_run, compile_and_freeze_pair, complete_run, create_run,
    derive_lifecycle, hash_event_chain, hash_source_bytes, issue_grant, pause_run, resume_run,
    revoke_grant, validate_native_structural,
};

use crate::operation_broker::{BrokerError, OperationBroker};
use crate::provider::{
    PROVIDER_PROFILE_ID, ProviderAdapter, ProviderEvent, ProviderRequest, PublicError,
};
use crate::sqlite::SqliteStore;
use crate::windows_fs::{FilesystemIdentity, native_diff as windows_native_diff};

/// In-memory native session binding for a run root (never sent to the renderer).
#[derive(Debug, Clone)]
struct RunBinding {
    root: PathBuf,
    relative_target: String,
    preimage: Option<String>,
    identity: Option<FilesystemIdentity>,
}

/// Managed Harness state kept separate from AgentState.
pub(crate) struct HarnessState {
    pub(crate) store: Arc<SqliteStore>,
    pub(crate) broker: OperationBroker,
    provider: Arc<dyn ProviderAdapter>,
    bindings: Mutex<HashMap<String, RunBinding>>,
}

impl HarnessState {
    pub(crate) fn new(store: Arc<SqliteStore>, provider: Arc<dyn ProviderAdapter>) -> Self {
        Self {
            broker: OperationBroker::new(Arc::clone(&store)),
            store,
            provider,
            bindings: Mutex::new(HashMap::new()),
        }
    }

    fn bind_root(&self, run_id: &str, root: PathBuf, relative_target: String) {
        if let Ok(mut guard) = self.bindings.lock() {
            guard.insert(
                run_id.to_owned(),
                RunBinding {
                    root,
                    relative_target,
                    preimage: None,
                    identity: None,
                },
            );
        }
    }

    fn binding(&self, run_id: &str) -> Result<RunBinding, HarnessPublicError> {
        self.bindings
            .lock()
            .map_err(|_| HarnessPublicError::StorageUnavailable)?
            .get(run_id)
            .cloned()
            .ok_or(HarnessPublicError::InvalidInput)
    }

    fn update_binding_after_read(
        &self,
        run_id: &str,
        preimage: String,
        identity: FilesystemIdentity,
    ) -> Result<(), HarnessPublicError> {
        let mut guard = self
            .bindings
            .lock()
            .map_err(|_| HarnessPublicError::StorageUnavailable)?;
        let binding = guard
            .get_mut(run_id)
            .ok_or(HarnessPublicError::InvalidInput)?;
        binding.preimage = Some(preimage);
        binding.identity = Some(identity);
        Ok(())
    }
}

/// Controlled-fixture provider that returns one complete UTF-8 postimage.
struct FixtureBootstrapProvider {
    postimage: String,
}

impl ProviderAdapter for FixtureBootstrapProvider {
    fn connection_status(&self) -> crate::provider::ConnectionStatus {
        crate::provider::ConnectionStatus {
            state: crate::provider::ConnectionState::Connected,
            provider_id: PROVIDER_PROFILE_ID,
            model: "fixture-controlled",
        }
    }

    fn stream<'a>(
        &'a self,
        _: ProviderRequest,
        cancel: tokio_util::sync::CancellationToken,
        mut on_event: crate::provider::ProviderEventSink,
    ) -> crate::provider::ProviderFuture<'a> {
        let postimage = self.postimage.clone();
        Box::pin(async move {
            if cancel.is_cancelled() {
                return Err(PublicError::Cancelled);
            }
            on_event(ProviderEvent::Delta(postimage))?;
            on_event(ProviderEvent::Completed {
                response_id: Some("fixture-response".to_owned()),
                input_tokens: Some(0),
                output_tokens: Some(0),
            })?;
            Ok(Vec::new())
        })
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HarnessRunSummaryDto {
    pub(crate) id: String,
    pub(crate) run_root_display_name: String,
    pub(crate) lifecycle: String,
    pub(crate) lifecycle_label: String,
    pub(crate) created_at_unix_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ContextPreviewDto {
    pub(crate) run_root_display_name: String,
    pub(crate) relative_target: String,
    pub(crate) byte_count: u64,
    pub(crate) content_hash: String,
    pub(crate) selected_content: String,
    pub(crate) provider_profile_id: String,
    pub(crate) model_id: String,
    pub(crate) proposed_disclosure: String,
    pub(crate) manifest_content_hash: String,
    pub(crate) request_semantic_hash: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DiffPreviewDto {
    pub(crate) version: String,
    pub(crate) text: String,
    pub(crate) hash: String,
    pub(crate) preimage_hash: String,
    pub(crate) postimage_hash: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GraphNodeDto {
    pub(crate) kind: String,
    pub(crate) responsibility: String,
    pub(crate) protected_validation: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GraphSummaryDto {
    pub(crate) id: String,
    pub(crate) nodes: Vec<GraphNodeDto>,
    pub(crate) edge_from: String,
    pub(crate) edge_to: String,
    pub(crate) retry_rule: String,
    pub(crate) validation_rule: String,
    pub(crate) validation_label: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ApprovalIdentityDto {
    pub(crate) plan_version_id: String,
    pub(crate) graph_version_id: String,
    pub(crate) approval_hash: String,
    pub(crate) approved: bool,
    pub(crate) approval_id: Option<String>,
    pub(crate) approver: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GrantDto {
    pub(crate) id: String,
    pub(crate) capability: String,
    pub(crate) resource_summary: String,
    pub(crate) action_scope: String,
    pub(crate) expires_at_unix_ms: i64,
    pub(crate) revoked: bool,
    pub(crate) dispatch_budget_remaining: u32,
    pub(crate) related_approval_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EffectDto {
    pub(crate) id: String,
    pub(crate) operation_id: String,
    pub(crate) phase: String,
    pub(crate) certainty: Option<String>,
    pub(crate) grant_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DenialDto {
    pub(crate) id: String,
    pub(crate) reason: String,
    pub(crate) grant_id: Option<String>,
    pub(crate) recorded_at_unix_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EventDto {
    pub(crate) id: String,
    pub(crate) sequence: u64,
    pub(crate) kind: String,
    pub(crate) created_at_unix_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CheckpointDto {
    pub(crate) id: String,
    pub(crate) last_event_sequence: u64,
    pub(crate) expected_postimage_hash: String,
    pub(crate) created_at_unix_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ValidationDto {
    pub(crate) id: String,
    pub(crate) label: String,
    pub(crate) approved_postimage_hash: String,
    pub(crate) observed_postimage_hash: String,
    pub(crate) native_diff_hash: String,
    pub(crate) passed: bool,
    pub(crate) validated_at_unix_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FinalResultDto {
    pub(crate) validation_label: String,
    pub(crate) publication_stopped: bool,
    pub(crate) plan_version_id: String,
    pub(crate) graph_version_id: String,
    pub(crate) completed_at_unix_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProviderDisclosureDto {
    pub(crate) provider_profile_id: String,
    pub(crate) model_id: String,
    pub(crate) allowed_disclosure: String,
    pub(crate) response_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CapabilityEnvelopeDto {
    pub(crate) summary: String,
    pub(crate) requested: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HarnessRunDetailDto {
    pub(crate) summary: HarnessRunSummaryDto,
    pub(crate) context: Option<ContextPreviewDto>,
    pub(crate) diff: Option<DiffPreviewDto>,
    pub(crate) graph: Option<GraphSummaryDto>,
    pub(crate) approval: Option<ApprovalIdentityDto>,
    pub(crate) grants: Vec<GrantDto>,
    pub(crate) requested_grants: Vec<String>,
    pub(crate) events: Vec<EventDto>,
    pub(crate) effects: Vec<EffectDto>,
    pub(crate) denials: Vec<DenialDto>,
    pub(crate) checkpoint: Option<CheckpointDto>,
    pub(crate) validation: Option<ValidationDto>,
    pub(crate) provider_disclosure: Option<ProviderDisclosureDto>,
    pub(crate) final_result: Option<FinalResultDto>,
    pub(crate) capability_envelope: Option<CapabilityEnvelopeDto>,
    pub(crate) resume_decision: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BootstrapHarnessRequest {
    pub(crate) run_id: String,
    pub(crate) instructions: String,
    pub(crate) model_id: String,
    /// `"fixture"` uses the controlled fake adapter; `"live"` uses the host provider.
    pub(crate) provider_mode: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum HarnessPublicError {
    InvalidInput,
    StorageUnavailable,
    Denied,
    UnsupportedOperation,
    Blocked,
    ProviderUnavailable,
}

impl From<BrokerError> for HarnessPublicError {
    fn from(value: BrokerError) -> Self {
        match value {
            BrokerError::UnsupportedOperation(_) => Self::UnsupportedOperation,
            BrokerError::GrantDenied(_)
            | BrokerError::MissingGrant
            | BrokerError::AuthorityMismatch(_) => Self::Denied,
            BrokerError::MissingRun | BrokerError::Windows(_) | BrokerError::Storage(_) => {
                Self::StorageUnavailable
            }
            BrokerError::Provider(PublicError::ProviderUnavailable)
            | BrokerError::Provider(PublicError::NotConnected) => Self::ProviderUnavailable,
            BrokerError::Provider(_) | BrokerError::InjectedFault(_) => Self::Blocked,
        }
    }
}

impl From<PublicError> for HarnessPublicError {
    fn from(value: PublicError) -> Self {
        match value {
            PublicError::InvalidInput => Self::InvalidInput,
            PublicError::ProviderUnavailable | PublicError::NotConnected => {
                Self::ProviderUnavailable
            }
            _ => Self::StorageUnavailable,
        }
    }
}

fn lifecycle_label(lifecycle: HarnessRunLifecycle) -> String {
    match lifecycle {
        HarnessRunLifecycle::BlockedReconciliationRequired => {
            "Blocked — reconciliation required".to_owned()
        }
        other => other.as_str().replace('_', " "),
    }
}

fn to_summary(reconstructed: &ReconstructedRun) -> HarnessRunSummaryDto {
    let lifecycle = derive_lifecycle(&reconstructed.events, &reconstructed.effects);
    HarnessRunSummaryDto {
        id: reconstructed.run.id().to_string(),
        run_root_display_name: reconstructed.run.run_root_display_name().to_owned(),
        lifecycle: lifecycle.as_str().to_owned(),
        lifecycle_label: lifecycle_label(lifecycle),
        created_at_unix_ms: reconstructed.run.created_at_unix_ms(),
    }
}

fn event_kind_label(kind: &tule_core::RunEventKind) -> String {
    match kind {
        tule_core::RunEventKind::RunCreated => "run_created".to_owned(),
        tule_core::RunEventKind::PairFrozen { .. } => "pair_frozen".to_owned(),
        tule_core::RunEventKind::Approved { .. } => "approved".to_owned(),
        tule_core::RunEventKind::GrantIssued { .. } => "grant_issued".to_owned(),
        tule_core::RunEventKind::GrantRevoked { .. } => "grant_revoked".to_owned(),
        tule_core::RunEventKind::Denied { .. } => "denied".to_owned(),
        tule_core::RunEventKind::EffectPrepared { .. } => "effect_prepared".to_owned(),
        tule_core::RunEventKind::EffectClaimed { .. } => "effect_claimed".to_owned(),
        tule_core::RunEventKind::EffectDispatched { .. } => "effect_dispatched".to_owned(),
        tule_core::RunEventKind::EffectSettled { .. } => "effect_settled".to_owned(),
        tule_core::RunEventKind::Checkpointed { .. } => "checkpointed".to_owned(),
        tule_core::RunEventKind::Validated { .. } => "validated".to_owned(),
        tule_core::RunEventKind::Completed => "completed".to_owned(),
        tule_core::RunEventKind::Paused => "paused".to_owned(),
        tule_core::RunEventKind::Cancelled => "cancelled".to_owned(),
        tule_core::RunEventKind::Abandoned => "abandoned".to_owned(),
        tule_core::RunEventKind::LeaseAcquired { .. } => "lease_acquired".to_owned(),
        tule_core::RunEventKind::LeaseReleased { .. } => "lease_released".to_owned(),
        tule_core::RunEventKind::LeaseTakeover { .. } => "lease_takeover".to_owned(),
        tule_core::RunEventKind::Resumed => "resumed".to_owned(),
    }
}

fn grant_resource_summary(resource: &GrantResourceSelector) -> String {
    match resource {
        GrantResourceSelector::RunRoot => "run_root".to_owned(),
        GrantResourceSelector::RelativeTarget(path) => format!("relative:{path}"),
        GrantResourceSelector::ContextManifestHash(hash) => {
            format!("manifest:{}", &hash[..8.min(hash.len())])
        }
        GrantResourceSelector::ReplacementTarget {
            relative_target, ..
        } => format!("replacement:{relative_target}"),
    }
}

fn grant_scope_label(scope: &GrantActionScope) -> String {
    match scope {
        GrantActionScope::Run => "run".to_owned(),
        GrantActionScope::Node(kind) => format!("node:{kind}"),
        GrantActionScope::Effect(id) => format!("effect:{id}"),
        GrantActionScope::Attempt(id) => format!("attempt:{id}"),
    }
}

fn collect_provider_text(events: &[ProviderEvent]) -> String {
    let mut text = String::new();
    for event in events {
        if let ProviderEvent::Delta(delta) = event {
            text.push_str(delta);
        }
    }
    text
}

fn expected_fixture_postimage(preimage: &str) -> String {
    preimage.replacen(BOOTSTRAP_HEADING_BEFORE, BOOTSTRAP_HEADING_AFTER, 1)
}

fn latest_plan_graph(
    reconstructed: &ReconstructedRun,
) -> Option<(&ExecutionPlanVersion, &RunGraphVersion)> {
    let plan = reconstructed.plans.last()?;
    let graph = reconstructed
        .graphs
        .iter()
        .find(|graph| graph.id() == plan.graph_version_id())?;
    Some((plan, graph))
}

fn to_detail(
    reconstructed: &ReconstructedRun,
    binding: Option<&RunBinding>,
    resume_decision: Option<String>,
) -> HarnessRunDetailDto {
    let summary = to_summary(reconstructed);
    let pair = latest_plan_graph(reconstructed);
    let context = pair.map(|(plan, _)| {
        let manifest = plan.context_manifest();
        let content = binding
            .and_then(|value| value.preimage.clone())
            .unwrap_or_default();
        ContextPreviewDto {
            run_root_display_name: reconstructed.run.run_root_display_name().to_owned(),
            relative_target: plan.replacement().relative_target().to_owned(),
            byte_count: manifest.disclosed_byte_count(),
            content_hash: manifest.content_hash().to_owned(),
            selected_content: content,
            provider_profile_id: plan.provider_profile_id().to_owned(),
            model_id: plan.model_id().to_owned(),
            proposed_disclosure: plan.disclosure_policy().allowed_disclosure().to_owned(),
            manifest_content_hash: manifest.content_hash().to_owned(),
            request_semantic_hash: manifest.request_semantic_hash().to_owned(),
        }
    });
    let diff = pair.and_then(|(plan, _)| {
        let preimage = binding?.preimage.as_ref()?;
        let postimage = plan.replacement().postimage_utf8();
        let native = windows_native_diff(preimage, postimage);
        Some(DiffPreviewDto {
            version: native.version.to_owned(),
            text: native.text,
            hash: native.hash,
            preimage_hash: plan.replacement().preimage_hash().to_owned(),
            postimage_hash: plan.replacement().postimage_hash().to_owned(),
        })
    });
    let graph = pair.map(|(_, graph)| {
        let nodes = graph
            .nodes()
            .iter()
            .map(|node| GraphNodeDto {
                kind: node.kind().to_owned(),
                responsibility: node.responsibility().to_owned(),
                protected_validation: node.is_protected_validation(),
            })
            .collect();
        let edge = graph.edges().first();
        GraphSummaryDto {
            id: graph.id().to_string(),
            nodes,
            edge_from: edge
                .map(|value| value.from_kind().to_owned())
                .unwrap_or_else(|| NODE_REPLACE_EXISTING_FILE_V1.to_owned()),
            edge_to: edge
                .map(|value| value.to_kind().to_owned())
                .unwrap_or_else(|| NODE_VERIFY_APPROVED_POSTIMAGE_V1.to_owned()),
            retry_rule: graph.retry_rule().to_owned(),
            validation_rule: graph.validation_rule().to_owned(),
            validation_label: NATIVE_STRUCTURAL_VALIDATION_LABEL.to_owned(),
        }
    });
    let approval = pair.map(|(plan, graph)| {
        let existing = reconstructed
            .approvals
            .iter()
            .find(|approval| approval.approval_hash() == plan.approval_hash());
        ApprovalIdentityDto {
            plan_version_id: plan.id().to_string(),
            graph_version_id: graph.id().to_string(),
            approval_hash: plan.approval_hash().to_owned(),
            approved: existing.is_some(),
            approval_id: existing.map(|value| value.id().to_string()),
            approver: existing.map(|value| value.approver().to_owned()),
        }
    });
    let grants = reconstructed
        .grants
        .iter()
        .map(|grant| GrantDto {
            id: grant.id().to_string(),
            capability: grant.capability().as_str().to_owned(),
            resource_summary: grant_resource_summary(grant.resource()),
            action_scope: grant_scope_label(grant.action_scope()),
            expires_at_unix_ms: grant.expires_at_unix_ms(),
            revoked: grant.is_revoked(),
            dispatch_budget_remaining: grant.dispatch_budget_remaining(),
            related_approval_id: grant.related_approval_id().map(|id| id.to_string()),
        })
        .collect();
    let requested_grants = pair
        .map(|(plan, _)| {
            plan.capability_envelope()
                .requested()
                .iter()
                .map(|capability| capability.as_str().to_owned())
                .collect()
        })
        .unwrap_or_default();
    let events = reconstructed
        .events
        .iter()
        .map(|event| EventDto {
            id: event.id().to_string(),
            sequence: event.sequence(),
            kind: event_kind_label(event.kind()),
            created_at_unix_ms: event.recorded_at_unix_ms(),
        })
        .collect();
    let effects = reconstructed
        .effects
        .iter()
        .map(|effect| EffectDto {
            id: effect.id().to_string(),
            operation_id: effect.operation_id().to_owned(),
            phase: match effect.phase() {
                EffectJournalPhase::Prepared => "prepared",
                EffectJournalPhase::Claimed => "claimed",
                EffectJournalPhase::Dispatched => "dispatched",
                EffectJournalPhase::Settled => "settled",
            }
            .to_owned(),
            certainty: effect.certainty().map(|certainty| match certainty {
                EffectCertainty::ConfirmedCommitted => "confirmed_committed".to_owned(),
                EffectCertainty::ConfirmedNoEffect => "confirmed_no_effect".to_owned(),
                EffectCertainty::UnknownOrPartial => "unknown_or_partial".to_owned(),
            }),
            grant_id: effect.grant_id().to_string(),
        })
        .collect();
    let denials = reconstructed
        .denials
        .iter()
        .map(|denial| DenialDto {
            id: denial.id().to_string(),
            reason: denial.reason().to_owned(),
            grant_id: denial.grant_id().map(|id| id.to_string()),
            recorded_at_unix_ms: denial.recorded_at_unix_ms(),
        })
        .collect();
    let checkpoint = reconstructed
        .checkpoints
        .last()
        .map(|checkpoint| CheckpointDto {
            id: checkpoint.id().to_string(),
            last_event_sequence: checkpoint.last_event_sequence(),
            expected_postimage_hash: checkpoint.expected_postimage_hash().to_owned(),
            created_at_unix_ms: checkpoint.created_at_unix_ms(),
        });
    let validation = reconstructed
        .validations
        .last()
        .map(|validation| ValidationDto {
            id: validation.id().to_string(),
            label: validation.label().to_owned(),
            approved_postimage_hash: validation.approved_postimage_hash().to_owned(),
            observed_postimage_hash: validation.observed_postimage_hash().to_owned(),
            native_diff_hash: validation.native_diff_hash().to_owned(),
            passed: validation.passed(),
            validated_at_unix_ms: validation.validated_at_unix_ms(),
        });
    let provider_disclosure = pair.map(|(plan, _)| ProviderDisclosureDto {
        provider_profile_id: plan.provider_profile_id().to_owned(),
        model_id: plan.model_id().to_owned(),
        allowed_disclosure: plan.disclosure_policy().allowed_disclosure().to_owned(),
        response_id: plan.replacement().provider_response_id().map(str::to_owned),
    });
    let final_result = reconstructed
        .final_result
        .as_ref()
        .map(|result| FinalResultDto {
            validation_label: result.validation_label().to_owned(),
            publication_stopped: result.publication_stopped(),
            plan_version_id: result.plan_version_id().to_string(),
            graph_version_id: result.graph_version_id().to_string(),
            completed_at_unix_ms: result.completed_at_unix_ms(),
        });
    let capability_envelope = pair.map(|(plan, _)| CapabilityEnvelopeDto {
        summary: plan.capability_envelope().summary().to_owned(),
        requested: plan
            .capability_envelope()
            .requested()
            .iter()
            .map(|capability| capability.as_str().to_owned())
            .collect(),
    });
    HarnessRunDetailDto {
        summary,
        context,
        diff,
        graph,
        approval,
        grants,
        requested_grants,
        events,
        effects,
        denials,
        checkpoint,
        validation,
        provider_disclosure,
        final_result,
        capability_envelope,
        resume_decision,
    }
}

fn reconstruct(
    state: &HarnessState,
    run_id: &HarnessRunId,
) -> Result<ReconstructedRun, HarnessPublicError> {
    state
        .store
        .reconstruct_run(run_id)
        .map_err(|_| HarnessPublicError::StorageUnavailable)?
        .ok_or(HarnessPublicError::InvalidInput)
}

fn display_name_for_root(root: &Path) -> String {
    root.file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("run-root")
        .to_owned()
}

/// Creates a Harness run and binds a picked folder as the exclusive run root.
#[tauri::command]
pub(crate) fn pick_harness_run_root(
    app: tauri::AppHandle,
    state: tauri::State<'_, HarnessState>,
) -> Result<HarnessRunSummaryDto, HarnessPublicError> {
    let picked = app
        .dialog()
        .file()
        .blocking_pick_folder()
        .ok_or(HarnessPublicError::InvalidInput)?;
    let root = match picked {
        FilePath::Path(path) => path,
        _ => return Err(HarnessPublicError::InvalidInput),
    };
    if !root.join(CONTROLLED_RELATIVE_TARGET).is_file() {
        return Err(HarnessPublicError::InvalidInput);
    }
    let display_name = display_name_for_root(&root);
    let now = SystemClock.unix_ms();
    let run = create_run(state.store.as_ref(), display_name, None, now)
        .map_err(|_| HarnessPublicError::StorageUnavailable)?;
    state.bind_root(
        &run.id().to_string(),
        root,
        CONTROLLED_RELATIVE_TARGET.to_owned(),
    );
    let reconstructed = reconstruct(&state, &run.id())?;
    Ok(to_summary(&reconstructed))
}

/// Binds an existing run to a picked root after reopen (path stays native).
#[tauri::command]
pub(crate) fn rebind_harness_run_root(
    app: tauri::AppHandle,
    state: tauri::State<'_, HarnessState>,
    run_id: String,
) -> Result<HarnessRunDetailDto, HarnessPublicError> {
    let run_id_parsed =
        HarnessRunId::parse(&run_id).map_err(|_| HarnessPublicError::InvalidInput)?;
    let _ = reconstruct(&state, &run_id_parsed)?;
    let picked = app
        .dialog()
        .file()
        .blocking_pick_folder()
        .ok_or(HarnessPublicError::InvalidInput)?;
    let root = match picked {
        FilePath::Path(path) => path,
        _ => return Err(HarnessPublicError::InvalidInput),
    };
    if !root.join(CONTROLLED_RELATIVE_TARGET).is_file() {
        return Err(HarnessPublicError::InvalidInput);
    }
    state.bind_root(&run_id, root, CONTROLLED_RELATIVE_TARGET.to_owned());
    get_harness_run_detail(state, run_id)
}

/// Creates a Harness run header. Ordinary Agent sessions are unchanged.
#[tauri::command]
pub(crate) fn create_harness_run(
    state: tauri::State<'_, HarnessState>,
    run_root_display_name: String,
) -> Result<HarnessRunSummaryDto, HarnessPublicError> {
    if run_root_display_name.trim().is_empty() {
        return Err(HarnessPublicError::InvalidInput);
    }
    let now = SystemClock.unix_ms();
    let run = create_run(state.store.as_ref(), run_root_display_name, None, now)
        .map_err(|_| HarnessPublicError::StorageUnavailable)?;
    let reconstructed = reconstruct(&state, &run.id())?;
    Ok(to_summary(&reconstructed))
}

/// Returns a reconstructed Harness run summary.
#[tauri::command]
pub(crate) fn get_harness_run(
    state: tauri::State<'_, HarnessState>,
    run_id: String,
) -> Result<HarnessRunSummaryDto, HarnessPublicError> {
    let run_id = HarnessRunId::parse(&run_id).map_err(|_| HarnessPublicError::InvalidInput)?;
    let reconstructed = reconstruct(&state, &run_id)?;
    Ok(to_summary(&reconstructed))
}

/// Returns the full allowlisted run evidence DTO for the interface.
#[tauri::command]
pub(crate) fn get_harness_run_detail(
    state: tauri::State<'_, HarnessState>,
    run_id: String,
) -> Result<HarnessRunDetailDto, HarnessPublicError> {
    let run_id_parsed =
        HarnessRunId::parse(&run_id).map_err(|_| HarnessPublicError::InvalidInput)?;
    let reconstructed = reconstruct(&state, &run_id_parsed)?;
    let binding = state.binding(&run_id).ok();
    let resume_decision = compute_resume_decision(&reconstructed, binding.as_ref())?;
    Ok(to_detail(
        &reconstructed,
        binding.as_ref(),
        Some(resume_decision),
    ))
}

fn compute_resume_decision(
    reconstructed: &ReconstructedRun,
    binding: Option<&RunBinding>,
) -> Result<String, HarnessPublicError> {
    let Some(checkpoint) = reconstructed.checkpoints.last() else {
        return Ok("continue".to_owned());
    };
    let prefix: Vec<_> = reconstructed
        .events
        .iter()
        .filter(|event| event.sequence() <= checkpoint.last_event_sequence())
        .cloned()
        .collect();
    let event_chain_matches = hash_event_chain(&prefix) == checkpoint.event_chain_hash();
    let filesystem_matches_expected = if let Some(binding) = binding {
        let path = binding.root.join(&binding.relative_target);
        match crate::windows_fs::read_utf8_file(&path) {
            Ok(current) => {
                let hash = hash_source_bytes(current.as_bytes());
                let confirmed = reconstructed.effects.iter().any(|effect| {
                    effect.operation_id() == tule_core::OP_CREATE_OR_REPLACE_V1
                        && matches!(
                            effect.certainty(),
                            Some(EffectCertainty::ConfirmedCommitted)
                        )
                });
                if confirmed {
                    hash == checkpoint.expected_postimage_hash()
                } else if let Some((plan, _)) = latest_plan_graph(reconstructed) {
                    hash == plan.replacement().preimage_hash()
                        || hash == checkpoint.expected_postimage_hash()
                } else {
                    true
                }
            }
            Err(_) => false,
        }
    } else {
        // Evidence-only reopen without rebound root: treat as matching for view/resume label.
        true
    };
    let (execution_policy_matches, operation_versions_match) =
        match latest_plan_graph(reconstructed) {
            Some((plan, _)) => (
                plan.execution_policy_revision() == checkpoint.execution_policy_revision(),
                true,
            ),
            None => (true, true),
        };
    let decision = resume_run(
        reconstructed,
        &ResumeRevalidation {
            event_chain_matches,
            filesystem_matches_expected,
            execution_policy_matches,
            operation_versions_match,
        },
        &SystemClock,
    );
    Ok(match decision {
        ResumeDecision::Continue => "continue".to_owned(),
        ResumeDecision::RequireReapprovalOrAbandon => "require_reapproval_or_abandon".to_owned(),
        ResumeDecision::SkipConfirmedReplacement { effect_id } => {
            format!("skip_confirmed_replacement:{effect_id}")
        }
        ResumeDecision::RequireFreshGrant { expired_grant_id } => {
            format!("require_fresh_grant:{expired_grant_id}")
        }
    })
}

/// Bootstrap local read + provider disclose + compile/freeze pair preview.
#[tauri::command]
pub(crate) fn bootstrap_harness_plan(
    state: tauri::State<'_, HarnessState>,
    request: BootstrapHarnessRequest,
) -> Result<HarnessRunDetailDto, HarnessPublicError> {
    let run_id =
        HarnessRunId::parse(&request.run_id).map_err(|_| HarnessPublicError::InvalidInput)?;
    if request.instructions.trim().is_empty() || request.model_id.trim().is_empty() {
        return Err(HarnessPublicError::InvalidInput);
    }
    let binding = state.binding(&request.run_id)?;
    let now = SystemClock.unix_ms();

    // Bootstrap local-read grant (separate from provider disclose).
    let read_grant = issue_grant(
        state.store.as_ref(),
        run_id,
        CapabilityType::LocalRead,
        GrantResourceSelector::RelativeTarget(binding.relative_target.clone()),
        GrantActionScope::Run,
        None,
        None,
        "owner",
        now,
    )
    .map_err(|_| HarnessPublicError::Denied)?;
    let read = state
        .broker
        .local_read(
            run_id,
            &binding.root,
            &binding.relative_target,
            &read_grant,
            now,
            None,
        )
        .map_err(HarnessPublicError::from)?;
    state.update_binding_after_read(
        &request.run_id,
        read.content.clone(),
        read.identity.clone(),
    )?;

    let request_semantics = request.instructions.clone();
    let manifest = ContextManifest::new(
        &read.content,
        &request_semantics,
        "controlled fixture context",
    )
    .map_err(|_| HarnessPublicError::InvalidInput)?;
    let disclosure = DisclosurePolicy::new(
        "controlled-fixture-disclosure-v1",
        "Exact selected index.html bytes and the heading-change request only",
    );

    let disclose_grant = issue_grant(
        state.store.as_ref(),
        run_id,
        CapabilityType::ProviderDisclose,
        GrantResourceSelector::ContextManifestHash(manifest.content_hash().to_owned()),
        GrantActionScope::Run,
        None,
        None,
        "owner",
        now,
    )
    .map_err(|_| HarnessPublicError::Denied)?;

    let postimage_for_fixture = expected_fixture_postimage(&read.content);
    let fixture_provider = FixtureBootstrapProvider {
        postimage: postimage_for_fixture.clone(),
    };
    let provider: &dyn ProviderAdapter = match request.provider_mode.as_str() {
        "fixture" => &fixture_provider,
        "live" => state.provider.as_ref(),
        _ => return Err(HarnessPublicError::InvalidInput),
    };

    let request_json = serde_json::json!({
        "model": request.model_id,
        "purpose": "bootstrap-plan-proposal",
        "relativeTarget": binding.relative_target,
        "allowedFields": ["postimageUtf8"],
    })
    .to_string();

    let disclosed = state
        .broker
        .provider_disclose(
            run_id,
            &disclose_grant,
            manifest.content_hash(),
            manifest.request_semantic_hash(),
            request_json,
            provider,
            now,
        )
        .map_err(HarnessPublicError::from)?;
    let postimage = {
        let collected = collect_provider_text(&disclosed.events);
        if collected.is_empty() {
            // Live adapters may only surface completion; fixture mode always sends deltas.
            if request.provider_mode == "fixture" {
                postimage_for_fixture
            } else {
                return Err(HarnessPublicError::InvalidInput);
            }
        } else {
            collected
        }
    };

    let envelope = CapabilityEnvelope::new(
        vec![
            CapabilityType::LocalRead,
            CapabilityType::ProviderDisclose,
            CapabilityType::CreateOrReplace,
            CapabilityType::NativeInspection,
        ],
        "Controlled fixture: read, disclose, replace index.html, native inspect",
    );
    compile_and_freeze_pair(
        state.store.as_ref(),
        run_id,
        request.instructions,
        PROVIDER_PROFILE_ID,
        request.model_id,
        disclosure,
        envelope,
        manifest,
        &read.content,
        postimage,
        binding.relative_target,
        read.identity.fingerprint(),
        Some(format!("harness-request-{}", disclosed.effect_id)),
        disclosed.response_id,
        now,
    )
    .map_err(|_| HarnessPublicError::InvalidInput)?;

    get_harness_run_detail(state, request.run_id)
}

/// Approves the frozen plan/graph pair without issuing grants.
#[tauri::command]
pub(crate) fn approve_harness_pair(
    state: tauri::State<'_, HarnessState>,
    run_id: String,
    approver: String,
) -> Result<HarnessRunDetailDto, HarnessPublicError> {
    let run_id_parsed =
        HarnessRunId::parse(&run_id).map_err(|_| HarnessPublicError::InvalidInput)?;
    if approver.trim().is_empty() {
        return Err(HarnessPublicError::InvalidInput);
    }
    let reconstructed = reconstruct(&state, &run_id_parsed)?;
    let (plan, graph) =
        latest_plan_graph(&reconstructed).ok_or(HarnessPublicError::InvalidInput)?;
    let now = SystemClock.unix_ms();
    approve_pair(
        state.store.as_ref(),
        run_id_parsed,
        plan,
        graph,
        approver,
        now,
    )
    .map_err(|_| HarnessPublicError::Denied)?;
    get_harness_run_detail(state, run_id)
}

/// Issues post-approval replacement and native-inspection grants as distinct records.
#[tauri::command]
pub(crate) fn issue_harness_execution_grants(
    state: tauri::State<'_, HarnessState>,
    run_id: String,
) -> Result<HarnessRunDetailDto, HarnessPublicError> {
    let run_id_parsed =
        HarnessRunId::parse(&run_id).map_err(|_| HarnessPublicError::InvalidInput)?;
    let reconstructed = reconstruct(&state, &run_id_parsed)?;
    let (plan, graph) =
        latest_plan_graph(&reconstructed).ok_or(HarnessPublicError::InvalidInput)?;
    let approval = reconstructed
        .approvals
        .iter()
        .find(|approval| approval.approval_hash() == plan.approval_hash())
        .ok_or(HarnessPublicError::Denied)?;
    let pair = PlanGraphPairBinding {
        plan_version_id: plan.id(),
        graph_version_id: graph.id(),
    };
    let now = SystemClock.unix_ms();
    let replacement = plan.replacement();
    issue_grant(
        state.store.as_ref(),
        run_id_parsed,
        CapabilityType::CreateOrReplace,
        GrantResourceSelector::ReplacementTarget {
            relative_target: replacement.relative_target().to_owned(),
            expected_preimage_hash: replacement.preimage_hash().to_owned(),
            expected_postimage_hash: replacement.postimage_hash().to_owned(),
        },
        GrantActionScope::Node(NODE_REPLACE_EXISTING_FILE_V1.to_owned()),
        Some(pair),
        Some(approval.id()),
        "owner",
        now,
    )
    .map_err(|_| HarnessPublicError::Denied)?;
    issue_grant(
        state.store.as_ref(),
        run_id_parsed,
        CapabilityType::NativeInspection,
        GrantResourceSelector::RelativeTarget(replacement.relative_target().to_owned()),
        GrantActionScope::Node(NODE_VERIFY_APPROVED_POSTIMAGE_V1.to_owned()),
        Some(pair),
        Some(approval.id()),
        "owner",
        now,
    )
    .map_err(|_| HarnessPublicError::Denied)?;
    get_harness_run_detail(state, run_id)
}

/// Executes the approved replacement, checkpoint, native inspection, validation, and final result.
#[tauri::command]
pub(crate) fn execute_harness_run(
    state: tauri::State<'_, HarnessState>,
    run_id: String,
) -> Result<HarnessRunDetailDto, HarnessPublicError> {
    let run_id_parsed =
        HarnessRunId::parse(&run_id).map_err(|_| HarnessPublicError::InvalidInput)?;
    let binding = state.binding(&run_id)?;
    let reconstructed = reconstruct(&state, &run_id_parsed)?;
    let lifecycle = derive_lifecycle(&reconstructed.events, &reconstructed.effects);
    if matches!(
        lifecycle,
        HarnessRunLifecycle::BlockedReconciliationRequired
    ) {
        return Err(HarnessPublicError::Blocked);
    }
    let (plan, graph) =
        latest_plan_graph(&reconstructed).ok_or(HarnessPublicError::InvalidInput)?;
    if reconstructed
        .approvals
        .iter()
        .all(|approval| approval.approval_hash() != plan.approval_hash())
    {
        return Err(HarnessPublicError::Denied);
    }
    let pair = PlanGraphPairBinding {
        plan_version_id: plan.id(),
        graph_version_id: graph.id(),
    };
    let now = SystemClock.unix_ms();
    let owner = format!("pid:{}", std::process::id());
    acquire_root_lease(state.store.as_ref(), run_id_parsed, &owner, now)
        .map_err(|_| HarnessPublicError::Denied)?;

    let replace_grant = reconstructed
        .grants
        .iter()
        .find(|grant| {
            grant.capability() == CapabilityType::CreateOrReplace
                && !grant.is_revoked()
                && grant.dispatch_budget_remaining() > 0
        })
        .cloned()
        .ok_or(HarnessPublicError::Denied)?;
    // Re-load grant after lease for budget freshness.
    let replace_grant = state
        .broker
        .find_grant(run_id_parsed, replace_grant.id())
        .map_err(HarnessPublicError::from)?;

    let replacement = plan.replacement();
    let preimage = binding
        .preimage
        .clone()
        .ok_or(HarnessPublicError::InvalidInput)?;
    let _ = state
        .broker
        .create_or_replace(
            run_id_parsed,
            &binding.root,
            replacement.relative_target(),
            binding.identity.as_ref(),
            replacement.preimage_hash(),
            replacement.postimage_hash(),
            replacement.postimage_utf8(),
            &replace_grant,
            now,
            pair,
        )
        .map_err(HarnessPublicError::from)?;

    let plan = {
        let reconstructed = reconstruct(&state, &run_id_parsed)?;
        latest_plan_graph(&reconstructed)
            .map(|(plan, _)| plan.clone())
            .ok_or(HarnessPublicError::StorageUnavailable)?
    };
    let graph = {
        let reconstructed = reconstruct(&state, &run_id_parsed)?;
        latest_plan_graph(&reconstructed)
            .map(|(_, graph)| graph.clone())
            .ok_or(HarnessPublicError::StorageUnavailable)?
    };

    checkpoint_run(state.store.as_ref(), run_id_parsed, &plan, &graph, now)
        .map_err(|_| HarnessPublicError::Blocked)?;

    let inspect_grant = {
        let reconstructed = reconstruct(&state, &run_id_parsed)?;
        let grant = reconstructed
            .grants
            .iter()
            .find(|grant| {
                grant.capability() == CapabilityType::NativeInspection
                    && !grant.is_revoked()
                    && grant.dispatch_budget_remaining() > 0
            })
            .ok_or(HarnessPublicError::Denied)?;
        state
            .broker
            .find_grant(run_id_parsed, grant.id())
            .map_err(HarnessPublicError::from)?
    };
    let inspected = state
        .broker
        .native_inspect(
            run_id_parsed,
            &binding.root,
            replacement.relative_target(),
            &preimage,
            &inspect_grant,
            now,
            pair,
        )
        .map_err(HarnessPublicError::from)?;

    validate_native_structural(
        state.store.as_ref(),
        run_id_parsed,
        &plan,
        &graph,
        inspected.observed_hash,
        inspected.diff.hash,
        now,
    )
    .map_err(|_| HarnessPublicError::Blocked)?;

    let instrumentation = ComparisonInstrumentation {
        model_turns: Some(1),
        registered_operation_calls: Some(4),
        retries: Some(0),
        task_success: Some(true),
        ..ComparisonInstrumentation::default()
    };
    complete_run(
        state.store.as_ref(),
        run_id_parsed,
        &plan,
        &graph,
        instrumentation,
        None,
        now,
    )
    .map_err(|_| HarnessPublicError::StorageUnavailable)?;

    get_harness_run_detail(state, run_id)
}

/// Pauses the run at a safe quiescent boundary.
#[tauri::command]
pub(crate) fn pause_harness_run(
    state: tauri::State<'_, HarnessState>,
    run_id: String,
) -> Result<HarnessRunDetailDto, HarnessPublicError> {
    let run_id_parsed =
        HarnessRunId::parse(&run_id).map_err(|_| HarnessPublicError::InvalidInput)?;
    let now = SystemClock.unix_ms();
    pause_run(state.store.as_ref(), run_id_parsed, now).map_err(|_| HarnessPublicError::Blocked)?;
    get_harness_run_detail(state, run_id)
}

/// Cancels the run at a safe quiescent boundary.
#[tauri::command]
pub(crate) fn cancel_harness_run(
    state: tauri::State<'_, HarnessState>,
    run_id: String,
) -> Result<HarnessRunDetailDto, HarnessPublicError> {
    let run_id_parsed =
        HarnessRunId::parse(&run_id).map_err(|_| HarnessPublicError::InvalidInput)?;
    let now = SystemClock.unix_ms();
    cancel_run(state.store.as_ref(), run_id_parsed, now)
        .map_err(|_| HarnessPublicError::Blocked)?;
    get_harness_run_detail(state, run_id)
}

/// Revokes a grant and returns updated evidence (denial demo support).
#[tauri::command]
pub(crate) fn revoke_harness_grant(
    state: tauri::State<'_, HarnessState>,
    run_id: String,
    grant_id: String,
) -> Result<HarnessRunDetailDto, HarnessPublicError> {
    let run_id_parsed =
        HarnessRunId::parse(&run_id).map_err(|_| HarnessPublicError::InvalidInput)?;
    let grant_id =
        CapabilityGrantId::parse(&grant_id).map_err(|_| HarnessPublicError::InvalidInput)?;
    let now = SystemClock.unix_ms();
    revoke_grant(state.store.as_ref(), run_id_parsed, grant_id, now)
        .map_err(|_| HarnessPublicError::Denied)?;
    get_harness_run_detail(state, run_id)
}

/// Records a denied unsupported operation through the broker boundary.
#[tauri::command]
pub(crate) fn deny_unsupported_harness_operation(
    state: tauri::State<'_, HarnessState>,
    run_id: String,
    operation: String,
) -> Result<HarnessRunDetailDto, HarnessPublicError> {
    let run_id_parsed =
        HarnessRunId::parse(&run_id).map_err(|_| HarnessPublicError::InvalidInput)?;
    let static_name = match operation.as_str() {
        "process-exec" => "process-exec",
        "git-write" => "git-write",
        "publication" => "publication",
        "arbitrary-network" => "arbitrary-network",
        _ => return Err(HarnessPublicError::InvalidInput),
    };
    let now = SystemClock.unix_ms();
    let err = state
        .broker
        .deny_unsupported(run_id_parsed, static_name, now);
    // Denial is persisted; return detail with denial evidence.
    let _ = HarnessPublicError::from(err);
    get_harness_run_detail(state, run_id)
}

/// Takes over a harness root lease using Windows positive-evidence prior-owner probing.
#[tauri::command]
pub(crate) fn takeover_harness_root_lease(
    state: tauri::State<'_, HarnessState>,
    run_id: String,
    new_owner_process_instance: String,
) -> Result<String, HarnessPublicError> {
    let run_id = HarnessRunId::parse(&run_id).map_err(|_| HarnessPublicError::InvalidInput)?;
    if new_owner_process_instance.trim().is_empty() {
        return Err(HarnessPublicError::InvalidInput);
    }
    let now = SystemClock.unix_ms();
    let lease = state
        .broker
        .takeover_root_lease_with_windows_evidence(run_id, &new_owner_process_instance, now)
        .map_err(HarnessPublicError::from)?;
    Ok(lease.id().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{ConnectionState, ConnectionStatus, FakeProvider};
    use crate::sqlite::DATABASE_FILENAME;
    use std::fs;
    use tempfile::TempDir;

    fn open_state() -> (TempDir, HarnessState) {
        let directory = tempfile::tempdir().unwrap();
        let store = Arc::new(SqliteStore::open(directory.path().join(DATABASE_FILENAME)).unwrap());
        let provider: Arc<dyn ProviderAdapter> = Arc::new(FakeProvider::new(
            ConnectionStatus {
                state: ConnectionState::Connected,
                provider_id: PROVIDER_PROFILE_ID,
                model: "fixture",
            },
            Ok(vec![ProviderEvent::Completed {
                response_id: Some("live".to_owned()),
                input_tokens: None,
                output_tokens: None,
            }]),
        ));
        (directory, HarnessState::new(store, provider))
    }

    fn copy_fixture(root: &Path) {
        let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../..")
            .join("fixtures/harness-static-app/index.html");
        fs::create_dir_all(root).unwrap();
        fs::copy(source, root.join("index.html")).unwrap();
    }

    #[test]
    fn harness_state_is_separate_from_agent_and_creates_runs() {
        let (_dir, state) = open_state();
        let now = 42_i64;
        let run = create_run(state.store.as_ref(), "fixture-root", None, now).unwrap();
        let summary = state.store.reconstruct_run(&run.id()).unwrap().unwrap();
        assert_eq!(summary.run.run_root_display_name(), "fixture-root");
        assert_eq!(
            derive_lifecycle(&summary.events, &summary.effects),
            HarnessRunLifecycle::Created
        );
        let _ = CapabilityGrantId::generate();
    }

    #[test]
    fn unsupported_operations_persist_denial_evidence() {
        let (_dir, state) = open_state();
        let run = create_run(state.store.as_ref(), "fixture-root", None, 42).unwrap();
        let err = state.broker.deny_unsupported(run.id(), "publication", 42);
        assert!(matches!(
            HarnessPublicError::from(err),
            HarnessPublicError::UnsupportedOperation
        ));
        let reconstructed = state.store.reconstruct_run(&run.id()).unwrap().unwrap();
        assert_eq!(reconstructed.denials.len(), 1);
        assert!(reconstructed.denials[0].reason().contains("publication"));
    }

    #[test]
    fn complete_controlled_fixture_journey_with_fixture_provider() {
        let fixture = tempfile::tempdir().unwrap();
        copy_fixture(fixture.path());
        let (_dir, state) = open_state();
        let now = SystemClock.unix_ms();
        let run = create_run(
            state.store.as_ref(),
            display_name_for_root(fixture.path()),
            None,
            now,
        )
        .unwrap();
        state.bind_root(
            &run.id().to_string(),
            fixture.path().to_path_buf(),
            CONTROLLED_RELATIVE_TARGET.to_owned(),
        );
        let run_id = run.id().to_string();

        let request = BootstrapHarnessRequest {
            run_id: run_id.clone(),
            instructions: "Change the heading Ready to Ready for review".to_owned(),
            model_id: "fixture-controlled".to_owned(),
            provider_mode: "fixture".to_owned(),
        };
        let detail = bootstrap_plan_for_test(&state, request).unwrap();
        assert!(detail.diff.is_some());
        assert_eq!(
            detail.graph.as_ref().unwrap().validation_label,
            NATIVE_STRUCTURAL_VALIDATION_LABEL
        );
        assert!(!detail.approval.as_ref().unwrap().approved);
        assert!(
            detail
                .requested_grants
                .iter()
                .any(|value| value == "create_or_replace")
        );

        let detail = approve_pair_for_test(&state, &run_id, "owner").unwrap();
        assert!(detail.approval.as_ref().unwrap().approved);
        let detail = issue_grants_for_test(&state, &run_id).unwrap();
        assert!(
            detail
                .grants
                .iter()
                .any(|grant| grant.capability == "create_or_replace")
        );
        assert!(
            detail
                .grants
                .iter()
                .any(|grant| grant.capability == "native_inspection")
        );
        assert!(
            detail
                .grants
                .iter()
                .filter(|grant| {
                    grant.capability == "create_or_replace"
                        || grant.capability == "native_inspection"
                })
                .all(|grant| grant.related_approval_id.is_some())
        );

        let detail = execute_for_test(&state, &run_id).unwrap();
        assert_eq!(detail.summary.lifecycle, "completed");
        let final_result = detail.final_result.as_ref().unwrap();
        assert_eq!(
            final_result.validation_label,
            NATIVE_STRUCTURAL_VALIDATION_LABEL
        );
        assert!(final_result.publication_stopped);
        assert_eq!(
            detail.validation.as_ref().unwrap().label,
            NATIVE_STRUCTURAL_VALIDATION_LABEL
        );

        let post = fs::read_to_string(fixture.path().join("index.html")).unwrap();
        assert!(post.contains("<h1>Ready for review</h1>"));
        assert!(!post.contains("<h1>Ready</h1>"));

        let reopened = to_detail(
            &state
                .store
                .reconstruct_run(&HarnessRunId::parse(&run_id).unwrap())
                .unwrap()
                .unwrap(),
            state.binding(&run_id).ok().as_ref(),
            Some("skip_confirmed_replacement".to_owned()),
        );
        assert_eq!(reopened.summary.id, detail.summary.id);
        assert_eq!(
            reopened.final_result.as_ref().unwrap().validation_label,
            detail.final_result.as_ref().unwrap().validation_label
        );

        let run_id_parsed = HarnessRunId::parse(&run_id).unwrap();
        let _ = state
            .broker
            .deny_unsupported(run_id_parsed, "publication", SystemClock.unix_ms());
        let denied = to_detail(
            &state
                .store
                .reconstruct_run(&run_id_parsed)
                .unwrap()
                .unwrap(),
            state.binding(&run_id).ok().as_ref(),
            None,
        );
        assert!(
            denied
                .denials
                .iter()
                .any(|denial| denial.reason.contains("publication"))
        );
        let after = fs::read_to_string(fixture.path().join("index.html")).unwrap();
        assert_eq!(after, post);
    }

    fn bootstrap_plan_for_test(
        state: &HarnessState,
        request: BootstrapHarnessRequest,
    ) -> Result<HarnessRunDetailDto, HarnessPublicError> {
        let run_id =
            HarnessRunId::parse(&request.run_id).map_err(|_| HarnessPublicError::InvalidInput)?;
        let binding = state.binding(&request.run_id)?;
        let now = SystemClock.unix_ms();
        let read_grant = issue_grant(
            state.store.as_ref(),
            run_id,
            CapabilityType::LocalRead,
            GrantResourceSelector::RelativeTarget(binding.relative_target.clone()),
            GrantActionScope::Run,
            None,
            None,
            "owner",
            now,
        )
        .map_err(|_| HarnessPublicError::Denied)?;
        let read = state
            .broker
            .local_read(
                run_id,
                &binding.root,
                &binding.relative_target,
                &read_grant,
                now,
                None,
            )
            .map_err(HarnessPublicError::from)?;
        state.update_binding_after_read(
            &request.run_id,
            read.content.clone(),
            read.identity.clone(),
        )?;
        let manifest = ContextManifest::new(
            &read.content,
            &request.instructions,
            "controlled fixture context",
        )
        .map_err(|_| HarnessPublicError::InvalidInput)?;
        let disclosure = DisclosurePolicy::new(
            "controlled-fixture-disclosure-v1",
            "Exact selected index.html bytes and the heading-change request only",
        );
        let disclose_grant = issue_grant(
            state.store.as_ref(),
            run_id,
            CapabilityType::ProviderDisclose,
            GrantResourceSelector::ContextManifestHash(manifest.content_hash().to_owned()),
            GrantActionScope::Run,
            None,
            None,
            "owner",
            now,
        )
        .map_err(|_| HarnessPublicError::Denied)?;
        let postimage_for_fixture = expected_fixture_postimage(&read.content);
        let fixture_provider = FixtureBootstrapProvider {
            postimage: postimage_for_fixture.clone(),
        };
        let disclosed = state
            .broker
            .provider_disclose(
                run_id,
                &disclose_grant,
                manifest.content_hash(),
                manifest.request_semantic_hash(),
                "{\"model\":\"fixture\"}".to_owned(),
                &fixture_provider,
                now,
            )
            .map_err(HarnessPublicError::from)?;
        let postimage = collect_provider_text(&disclosed.events);
        let envelope = CapabilityEnvelope::new(
            vec![
                CapabilityType::LocalRead,
                CapabilityType::ProviderDisclose,
                CapabilityType::CreateOrReplace,
                CapabilityType::NativeInspection,
            ],
            "Controlled fixture envelope",
        );
        compile_and_freeze_pair(
            state.store.as_ref(),
            run_id,
            request.instructions,
            PROVIDER_PROFILE_ID,
            request.model_id,
            disclosure,
            envelope,
            manifest,
            &read.content,
            postimage,
            binding.relative_target,
            read.identity.fingerprint(),
            Some(format!("req-{}", disclosed.effect_id)),
            disclosed.response_id,
            now,
        )
        .map_err(|_| HarnessPublicError::InvalidInput)?;
        let reconstructed = reconstruct(state, &run_id)?;
        Ok(to_detail(
            &reconstructed,
            state.binding(&request.run_id).ok().as_ref(),
            None,
        ))
    }

    fn approve_pair_for_test(
        state: &HarnessState,
        run_id: &str,
        approver: &str,
    ) -> Result<HarnessRunDetailDto, HarnessPublicError> {
        let run_id_parsed =
            HarnessRunId::parse(run_id).map_err(|_| HarnessPublicError::InvalidInput)?;
        let reconstructed = reconstruct(state, &run_id_parsed)?;
        let (plan, graph) =
            latest_plan_graph(&reconstructed).ok_or(HarnessPublicError::InvalidInput)?;
        approve_pair(
            state.store.as_ref(),
            run_id_parsed,
            plan,
            graph,
            approver,
            SystemClock.unix_ms(),
        )
        .map_err(|_| HarnessPublicError::Denied)?;
        let reconstructed = reconstruct(state, &run_id_parsed)?;
        Ok(to_detail(
            &reconstructed,
            state.binding(run_id).ok().as_ref(),
            None,
        ))
    }

    fn issue_grants_for_test(
        state: &HarnessState,
        run_id: &str,
    ) -> Result<HarnessRunDetailDto, HarnessPublicError> {
        let run_id_parsed =
            HarnessRunId::parse(run_id).map_err(|_| HarnessPublicError::InvalidInput)?;
        let reconstructed = reconstruct(state, &run_id_parsed)?;
        let (plan, graph) =
            latest_plan_graph(&reconstructed).ok_or(HarnessPublicError::InvalidInput)?;
        let approval = reconstructed
            .approvals
            .iter()
            .find(|approval| approval.approval_hash() == plan.approval_hash())
            .ok_or(HarnessPublicError::Denied)?;
        let pair = PlanGraphPairBinding {
            plan_version_id: plan.id(),
            graph_version_id: graph.id(),
        };
        let now = SystemClock.unix_ms();
        let replacement = plan.replacement();
        issue_grant(
            state.store.as_ref(),
            run_id_parsed,
            CapabilityType::CreateOrReplace,
            GrantResourceSelector::ReplacementTarget {
                relative_target: replacement.relative_target().to_owned(),
                expected_preimage_hash: replacement.preimage_hash().to_owned(),
                expected_postimage_hash: replacement.postimage_hash().to_owned(),
            },
            GrantActionScope::Node(NODE_REPLACE_EXISTING_FILE_V1.to_owned()),
            Some(pair),
            Some(approval.id()),
            "owner",
            now,
        )
        .map_err(|_| HarnessPublicError::Denied)?;
        issue_grant(
            state.store.as_ref(),
            run_id_parsed,
            CapabilityType::NativeInspection,
            GrantResourceSelector::RelativeTarget(replacement.relative_target().to_owned()),
            GrantActionScope::Node(NODE_VERIFY_APPROVED_POSTIMAGE_V1.to_owned()),
            Some(pair),
            Some(approval.id()),
            "owner",
            now,
        )
        .map_err(|_| HarnessPublicError::Denied)?;
        let reconstructed = reconstruct(state, &run_id_parsed)?;
        Ok(to_detail(
            &reconstructed,
            state.binding(run_id).ok().as_ref(),
            None,
        ))
    }

    fn execute_for_test(
        state: &HarnessState,
        run_id: &str,
    ) -> Result<HarnessRunDetailDto, HarnessPublicError> {
        let run_id_parsed =
            HarnessRunId::parse(run_id).map_err(|_| HarnessPublicError::InvalidInput)?;
        let binding = state.binding(run_id)?;
        let reconstructed = reconstruct(state, &run_id_parsed)?;
        let (plan, graph) =
            latest_plan_graph(&reconstructed).ok_or(HarnessPublicError::InvalidInput)?;
        let pair = PlanGraphPairBinding {
            plan_version_id: plan.id(),
            graph_version_id: graph.id(),
        };
        let now = SystemClock.unix_ms();
        acquire_root_lease(
            state.store.as_ref(),
            run_id_parsed,
            format!("pid:{}", std::process::id()),
            now,
        )
        .map_err(|_| HarnessPublicError::Denied)?;
        let replace_grant = reconstructed
            .grants
            .iter()
            .find(|grant| grant.capability() == CapabilityType::CreateOrReplace)
            .ok_or(HarnessPublicError::Denied)?;
        let replace_grant = state
            .broker
            .find_grant(run_id_parsed, replace_grant.id())
            .map_err(HarnessPublicError::from)?;
        let replacement = plan.replacement().clone();
        let preimage = binding
            .preimage
            .clone()
            .ok_or(HarnessPublicError::InvalidInput)?;
        state
            .broker
            .create_or_replace(
                run_id_parsed,
                &binding.root,
                replacement.relative_target(),
                binding.identity.as_ref(),
                replacement.preimage_hash(),
                replacement.postimage_hash(),
                replacement.postimage_utf8(),
                &replace_grant,
                now,
                pair,
            )
            .map_err(HarnessPublicError::from)?;
        let reconstructed = reconstruct(state, &run_id_parsed)?;
        let (plan, graph) =
            latest_plan_graph(&reconstructed).ok_or(HarnessPublicError::InvalidInput)?;
        let plan = plan.clone();
        let graph = graph.clone();
        checkpoint_run(state.store.as_ref(), run_id_parsed, &plan, &graph, now)
            .map_err(|_| HarnessPublicError::Blocked)?;
        let inspect_grant = {
            let reconstructed = reconstruct(state, &run_id_parsed)?;
            let grant = reconstructed
                .grants
                .iter()
                .find(|grant| grant.capability() == CapabilityType::NativeInspection)
                .ok_or(HarnessPublicError::Denied)?;
            state
                .broker
                .find_grant(run_id_parsed, grant.id())
                .map_err(HarnessPublicError::from)?
        };
        let inspected = state
            .broker
            .native_inspect(
                run_id_parsed,
                &binding.root,
                replacement.relative_target(),
                &preimage,
                &inspect_grant,
                now,
                pair,
            )
            .map_err(HarnessPublicError::from)?;
        validate_native_structural(
            state.store.as_ref(),
            run_id_parsed,
            &plan,
            &graph,
            inspected.observed_hash,
            inspected.diff.hash,
            now,
        )
        .map_err(|_| HarnessPublicError::Blocked)?;
        complete_run(
            state.store.as_ref(),
            run_id_parsed,
            &plan,
            &graph,
            ComparisonInstrumentation {
                task_success: Some(true),
                retries: Some(0),
                ..ComparisonInstrumentation::default()
            },
            None,
            now,
        )
        .map_err(|_| HarnessPublicError::StorageUnavailable)?;
        let reconstructed = reconstruct(state, &run_id_parsed)?;
        Ok(to_detail(
            &reconstructed,
            state.binding(run_id).ok().as_ref(),
            None,
        ))
    }
}
