//! Sole Harness operation boundary for local read, replace, inspect, and provider disclose.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use tule_core::{
    CapabilityGrant, CapabilityGrantId, CapabilityType, EffectCertainty, EffectOperationResult,
    EffectRecord, EffectRecordId, GrantActionScope, GrantEvaluationRequest, GrantResourceSelector,
    GrantUseCaseError, HarnessRunId, OP_CREATE_OR_REPLACE_V1, OP_LOCAL_READ_V1,
    OP_NATIVE_INSPECT_V1, OP_PROVIDER_DISCLOSE_V1, PlanGraphPairBinding,
    REGISTERED_OPERATION_SCHEMA_V1, RootLease, RunRepository, claim_effect, dispatch_effect,
    hash_source_bytes, prepare_effect, record_denial, require_grant, settle_effect,
    takeover_root_lease,
};

use crate::provider::{
    HarnessDisclosureAuthority, ProviderAdapter, ProviderEvent, ProviderRequest, PublicError,
    dispatch_harness_provider,
};
use crate::sqlite::SqliteStore;
use crate::windows_fs::{
    FilesystemIdentity, NativeDiff, WindowsFsError, content_hash, deny_unsupported,
    exact_create_or_replace, inspect_baseline_to_current, prior_owner_process_gone, read_identity,
    read_utf8_file, resolve_target_under_root,
};

pub(crate) const CLAIMANT: &str = "operation-broker";

/// Deterministic test-only fault injection points.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
#[allow(dead_code)]
pub(crate) enum BrokerFaultPoint {
    None = 0,
    AfterPrepared = 1,
    AfterClaimed = 2,
    AfterDispatchedBeforeWrite = 3,
    AfterWriteBeforeSettlement = 4,
}

#[derive(Debug, Default)]
pub(crate) struct BrokerFaultHook {
    point: AtomicU8,
}

impl BrokerFaultHook {
    #[allow(dead_code)]
    pub(crate) fn set(&self, point: BrokerFaultPoint) {
        self.point.store(point as u8, Ordering::SeqCst);
    }

    fn trip(&self, point: BrokerFaultPoint) -> Result<(), BrokerError> {
        if self.point.load(Ordering::SeqCst) == point as u8 {
            return Err(BrokerError::InjectedFault(point));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BrokerError {
    UnsupportedOperation(&'static str),
    Windows(WindowsFsError),
    GrantDenied(String),
    MissingGrant,
    MissingRun,
    Provider(PublicError),
    Storage(String),
    InjectedFault(BrokerFaultPoint),
    AuthorityMismatch(&'static str),
}

impl std::fmt::Display for BrokerError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedOperation(name) => write!(formatter, "denied unsupported op: {name}"),
            Self::Windows(error) => error.fmt(formatter),
            Self::GrantDenied(reason) => write!(formatter, "grant denied: {reason}"),
            Self::MissingGrant => formatter.write_str("required grant is missing"),
            Self::MissingRun => formatter.write_str("harness run is missing"),
            Self::Provider(error) => write!(formatter, "provider error: {error:?}"),
            Self::Storage(message) => write!(formatter, "storage error: {message}"),
            Self::InjectedFault(point) => write!(formatter, "injected fault at {point:?}"),
            Self::AuthorityMismatch(detail) => write!(formatter, "authority mismatch: {detail}"),
        }
    }
}

impl std::error::Error for BrokerError {}

impl From<WindowsFsError> for BrokerError {
    fn from(value: WindowsFsError) -> Self {
        match value {
            WindowsFsError::UnsupportedOperation(name) => Self::UnsupportedOperation(name),
            other => Self::Windows(other),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LocalReadResult {
    pub(crate) path: PathBuf,
    pub(crate) content: String,
    pub(crate) content_hash: String,
    pub(crate) identity: FilesystemIdentity,
    pub(crate) effect_id: EffectRecordId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReplaceResult {
    pub(crate) identity: FilesystemIdentity,
    pub(crate) effect_id: EffectRecordId,
    pub(crate) postimage_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InspectResult {
    pub(crate) observed_hash: String,
    pub(crate) diff: NativeDiff,
    pub(crate) effect_id: EffectRecordId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DiscloseResult {
    pub(crate) effect_id: EffectRecordId,
    pub(crate) events: Vec<ProviderEvent>,
    pub(crate) response_id: Option<String>,
}

pub(crate) struct OperationBroker {
    store: Arc<SqliteStore>,
    fault: BrokerFaultHook,
}

impl OperationBroker {
    pub(crate) fn new(store: Arc<SqliteStore>) -> Self {
        Self {
            store,
            fault: BrokerFaultHook::default(),
        }
    }

    #[allow(dead_code)]
    pub(crate) fn fault_hook(&self) -> &BrokerFaultHook {
        &self.fault
    }

    /// Denies an unsupported operation and appends durable denial evidence for the run.
    pub(crate) fn deny_unsupported(
        &self,
        run_id: HarnessRunId,
        operation: &'static str,
        now_unix_ms: i64,
    ) -> BrokerError {
        let _ = deny_unsupported(operation);
        let reason = format!("unsupported operation denied: {operation}");
        if let Err(error) =
            record_denial(self.store.as_ref(), run_id, reason, None, None, now_unix_ms)
        {
            return BrokerError::Storage(error.to_string());
        }
        BrokerError::UnsupportedOperation(operation)
    }

    /// Takes over a root lease only after Windows positive-evidence that the prior owner is gone.
    pub(crate) fn takeover_root_lease_with_windows_evidence(
        &self,
        run_id: HarnessRunId,
        new_owner_process_instance: &str,
        now_unix_ms: i64,
    ) -> Result<RootLease, BrokerError> {
        let reconstructed = self
            .store
            .reconstruct_run(&run_id)
            .map_err(|error| BrokerError::Storage(error.to_string()))?
            .ok_or(BrokerError::MissingRun)?;
        let prior_owner_confirmed_gone = match reconstructed.lease.as_ref() {
            Some(lease) => prior_owner_process_gone(lease.owner_process_instance())
                .map_err(BrokerError::from)?,
            None => true,
        };
        takeover_root_lease(
            self.store.as_ref(),
            run_id,
            new_owner_process_instance,
            prior_owner_confirmed_gone,
            now_unix_ms,
        )
        .map_err(|error| BrokerError::Storage(error.to_string()))
    }

    pub(crate) fn local_read(
        &self,
        run_id: HarnessRunId,
        run_root: &Path,
        relative_target: &str,
        grant: &CapabilityGrant,
        now_unix_ms: i64,
        pair: Option<PlanGraphPairBinding>,
    ) -> Result<LocalReadResult, BrokerError> {
        let resource = GrantResourceSelector::RelativeTarget(relative_target.to_owned());
        let scope = GrantActionScope::Run;
        self.ensure_grant(
            grant,
            run_id,
            CapabilityType::LocalRead,
            OP_LOCAL_READ_V1,
            &resource,
            &scope,
            pair,
            now_unix_ms,
        )?;
        let path = resolve_target_under_root(run_root, relative_target)?;
        let effect = prepare_effect(
            self.store.as_ref(),
            run_id,
            None,
            pair.map(|value| value.plan_version_id),
            pair.map(|value| value.graph_version_id),
            OP_LOCAL_READ_V1,
            hash_source_bytes(relative_target.as_bytes()),
            grant.id(),
            now_unix_ms,
            None,
            None,
        )
        .map_err(|error| BrokerError::Storage(error.to_string()))?;
        self.fault.trip(BrokerFaultPoint::AfterPrepared)?;
        claim_effect(
            self.store.as_ref(),
            run_id,
            effect.id(),
            CLAIMANT,
            now_unix_ms,
        )
        .map_err(|error| BrokerError::Storage(error.to_string()))?;
        self.fault.trip(BrokerFaultPoint::AfterClaimed)?;
        let dispatched = dispatch_effect(
            self.store.as_ref(),
            run_id,
            effect.id(),
            grant.id(),
            CLAIMANT,
            now_unix_ms,
        )
        .map_err(|error| BrokerError::Storage(error.to_string()))?;
        self.fault
            .trip(BrokerFaultPoint::AfterDispatchedBeforeWrite)?;
        let content = read_utf8_file(&path)?;
        let identity = read_identity(&path)?;
        let hash = content_hash(&content);
        settle_effect(
            self.store.as_ref(),
            run_id,
            dispatched.id(),
            CLAIMANT,
            EffectOperationResult::Success,
            EffectCertainty::ConfirmedCommitted,
            now_unix_ms,
        )
        .map_err(|error| BrokerError::Storage(error.to_string()))?;
        Ok(LocalReadResult {
            path,
            content,
            content_hash: hash,
            identity,
            effect_id: dispatched.id(),
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn create_or_replace(
        &self,
        run_id: HarnessRunId,
        run_root: &Path,
        relative_target: &str,
        expected_identity: Option<&FilesystemIdentity>,
        expected_preimage_hash: &str,
        expected_postimage_hash: &str,
        postimage_utf8: &str,
        grant: &CapabilityGrant,
        now_unix_ms: i64,
        pair: PlanGraphPairBinding,
    ) -> Result<ReplaceResult, BrokerError> {
        let resource = GrantResourceSelector::ReplacementTarget {
            relative_target: relative_target.to_owned(),
            expected_preimage_hash: expected_preimage_hash.to_owned(),
            expected_postimage_hash: expected_postimage_hash.to_owned(),
        };
        let scope = GrantActionScope::Node(tule_core::NODE_REPLACE_EXISTING_FILE_V1.to_owned());
        self.ensure_grant(
            grant,
            run_id,
            CapabilityType::CreateOrReplace,
            OP_CREATE_OR_REPLACE_V1,
            &resource,
            &scope,
            Some(pair),
            now_unix_ms,
        )?;
        let effect = prepare_effect(
            self.store.as_ref(),
            run_id,
            None,
            Some(pair.plan_version_id),
            Some(pair.graph_version_id),
            OP_CREATE_OR_REPLACE_V1,
            expected_postimage_hash,
            grant.id(),
            now_unix_ms,
            Some(expected_preimage_hash.to_owned()),
            Some(expected_postimage_hash.to_owned()),
        )
        .map_err(|error| BrokerError::Storage(error.to_string()))?;
        self.fault.trip(BrokerFaultPoint::AfterPrepared)?;
        claim_effect(
            self.store.as_ref(),
            run_id,
            effect.id(),
            CLAIMANT,
            now_unix_ms,
        )
        .map_err(|error| BrokerError::Storage(error.to_string()))?;
        self.fault.trip(BrokerFaultPoint::AfterClaimed)?;
        let dispatched = dispatch_effect(
            self.store.as_ref(),
            run_id,
            effect.id(),
            grant.id(),
            CLAIMANT,
            now_unix_ms,
        )
        .map_err(|error| BrokerError::Storage(error.to_string()))?;
        self.fault
            .trip(BrokerFaultPoint::AfterDispatchedBeforeWrite)?;
        let identity = match exact_create_or_replace(
            run_root,
            relative_target,
            expected_identity,
            expected_preimage_hash,
            postimage_utf8,
        ) {
            Ok(identity) => identity,
            Err(error) => {
                let _ = settle_effect(
                    self.store.as_ref(),
                    run_id,
                    dispatched.id(),
                    CLAIMANT,
                    EffectOperationResult::Error,
                    EffectCertainty::UnknownOrPartial,
                    now_unix_ms,
                );
                return Err(error.into());
            }
        };
        self.fault
            .trip(BrokerFaultPoint::AfterWriteBeforeSettlement)?;
        settle_effect(
            self.store.as_ref(),
            run_id,
            dispatched.id(),
            CLAIMANT,
            EffectOperationResult::Success,
            EffectCertainty::ConfirmedCommitted,
            now_unix_ms,
        )
        .map_err(|error| BrokerError::Storage(error.to_string()))?;
        Ok(ReplaceResult {
            identity,
            effect_id: dispatched.id(),
            postimage_hash: expected_postimage_hash.to_owned(),
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn native_inspect(
        &self,
        run_id: HarnessRunId,
        run_root: &Path,
        relative_target: &str,
        baseline_utf8: &str,
        grant: &CapabilityGrant,
        now_unix_ms: i64,
        pair: PlanGraphPairBinding,
    ) -> Result<InspectResult, BrokerError> {
        let resource = GrantResourceSelector::RelativeTarget(relative_target.to_owned());
        let scope = GrantActionScope::Node(tule_core::NODE_VERIFY_APPROVED_POSTIMAGE_V1.to_owned());
        self.ensure_grant(
            grant,
            run_id,
            CapabilityType::NativeInspection,
            OP_NATIVE_INSPECT_V1,
            &resource,
            &scope,
            Some(pair),
            now_unix_ms,
        )?;
        let effect = prepare_effect(
            self.store.as_ref(),
            run_id,
            None,
            Some(pair.plan_version_id),
            Some(pair.graph_version_id),
            OP_NATIVE_INSPECT_V1,
            hash_source_bytes(relative_target.as_bytes()),
            grant.id(),
            now_unix_ms,
            None,
            None,
        )
        .map_err(|error| BrokerError::Storage(error.to_string()))?;
        claim_effect(
            self.store.as_ref(),
            run_id,
            effect.id(),
            CLAIMANT,
            now_unix_ms,
        )
        .map_err(|error| BrokerError::Storage(error.to_string()))?;
        let dispatched = dispatch_effect(
            self.store.as_ref(),
            run_id,
            effect.id(),
            grant.id(),
            CLAIMANT,
            now_unix_ms,
        )
        .map_err(|error| BrokerError::Storage(error.to_string()))?;
        let (observed_hash, diff) =
            inspect_baseline_to_current(run_root, relative_target, baseline_utf8)?;
        settle_effect(
            self.store.as_ref(),
            run_id,
            dispatched.id(),
            CLAIMANT,
            EffectOperationResult::Success,
            EffectCertainty::ConfirmedCommitted,
            now_unix_ms,
        )
        .map_err(|error| BrokerError::Storage(error.to_string()))?;
        Ok(InspectResult {
            observed_hash,
            diff,
            effect_id: dispatched.id(),
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn provider_disclose(
        &self,
        run_id: HarnessRunId,
        grant: &CapabilityGrant,
        manifest_content_hash: &str,
        request_semantic_hash: &str,
        request_json: String,
        provider: &dyn ProviderAdapter,
        now_unix_ms: i64,
    ) -> Result<DiscloseResult, BrokerError> {
        let resource = GrantResourceSelector::ContextManifestHash(manifest_content_hash.to_owned());
        let scope = GrantActionScope::Run;
        self.ensure_grant(
            grant,
            run_id,
            CapabilityType::ProviderDisclose,
            OP_PROVIDER_DISCLOSE_V1,
            &resource,
            &scope,
            None,
            now_unix_ms,
        )?;
        let effect = prepare_effect(
            self.store.as_ref(),
            run_id,
            None,
            None,
            None,
            OP_PROVIDER_DISCLOSE_V1,
            request_semantic_hash,
            grant.id(),
            now_unix_ms,
            None,
            None,
        )
        .map_err(|error| BrokerError::Storage(error.to_string()))?;
        claim_effect(
            self.store.as_ref(),
            run_id,
            effect.id(),
            CLAIMANT,
            now_unix_ms,
        )
        .map_err(|error| BrokerError::Storage(error.to_string()))?;
        // Durable dispatched must precede the external provider boundary.
        let dispatched = dispatch_effect(
            self.store.as_ref(),
            run_id,
            effect.id(),
            grant.id(),
            CLAIMANT,
            now_unix_ms,
        )
        .map_err(|error| BrokerError::Storage(error.to_string()))?;
        let authority = HarnessDisclosureAuthority {
            run_id: run_id.to_string(),
            grant_id: grant.id().to_string(),
            effect_id: dispatched.id().to_string(),
            manifest_content_hash: manifest_content_hash.to_owned(),
            request_semantic_hash: request_semantic_hash.to_owned(),
            registered_operation_id: OP_PROVIDER_DISCLOSE_V1.to_owned(),
            registered_operation_schema: REGISTERED_OPERATION_SCHEMA_V1.to_owned(),
        };
        let events = dispatch_harness_provider(
            provider,
            self.store.as_ref(),
            &authority,
            ProviderRequest {
                session_id: run_id.to_string(),
                request_json,
            },
        )
        .map_err(BrokerError::Provider)?;
        let response_id = events.iter().find_map(|event| match event {
            ProviderEvent::Completed { response_id, .. } => response_id.clone(),
            _ => None,
        });
        settle_effect(
            self.store.as_ref(),
            run_id,
            dispatched.id(),
            CLAIMANT,
            EffectOperationResult::Success,
            EffectCertainty::ConfirmedCommitted,
            now_unix_ms,
        )
        .map_err(|error| BrokerError::Storage(error.to_string()))?;
        Ok(DiscloseResult {
            effect_id: dispatched.id(),
            events,
            response_id,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn ensure_grant(
        &self,
        grant: &CapabilityGrant,
        run_id: HarnessRunId,
        capability: CapabilityType,
        operation_id: &str,
        resource: &GrantResourceSelector,
        action_scope: &GrantActionScope,
        pair: Option<PlanGraphPairBinding>,
        now_unix_ms: i64,
    ) -> Result<(), BrokerError> {
        if grant.run_id() != run_id {
            return Err(BrokerError::AuthorityMismatch("grant run mismatch"));
        }
        let request = GrantEvaluationRequest {
            run_id,
            capability,
            operation_id,
            resource,
            action_scope,
            pair,
            now_unix_ms,
        };
        match require_grant(self.store.as_ref(), grant, &request) {
            Ok(()) => Ok(()),
            Err(GrantUseCaseError::Denied(reason)) => {
                Err(BrokerError::GrantDenied(format!("{reason:?}")))
            }
            Err(error) => Err(BrokerError::Storage(error.to_string())),
        }
    }

    pub(crate) fn find_grant(
        &self,
        run_id: HarnessRunId,
        grant_id: CapabilityGrantId,
    ) -> Result<CapabilityGrant, BrokerError> {
        let reconstructed = self
            .store
            .reconstruct_run(&run_id)
            .map_err(|error| BrokerError::Storage(error.to_string()))?
            .ok_or(BrokerError::MissingRun)?;
        reconstructed
            .grants
            .into_iter()
            .find(|grant| grant.id() == grant_id)
            .ok_or(BrokerError::MissingGrant)
    }

    #[allow(dead_code)]
    pub(crate) fn effect(
        &self,
        run_id: HarnessRunId,
        effect_id: EffectRecordId,
    ) -> Result<EffectRecord, BrokerError> {
        let reconstructed = self
            .store
            .reconstruct_run(&run_id)
            .map_err(|error| BrokerError::Storage(error.to_string()))?
            .ok_or(BrokerError::MissingRun)?;
        reconstructed
            .effects
            .into_iter()
            .find(|effect| effect.id() == effect_id)
            .ok_or(BrokerError::MissingRun)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{ConnectionState, ConnectionStatus, FakeProvider};
    use crate::sqlite::DATABASE_FILENAME;
    use tempfile::TempDir;
    use tule_core::{
        BOOTSTRAP_HEADING_AFTER, BOOTSTRAP_HEADING_BEFORE, CONTROLLED_RELATIVE_TARGET,
        CapabilityEnvelope, Clock, ContextManifest, DisclosurePolicy, FakeClock, GrantActionScope,
        approve_pair, compile_and_freeze_pair, create_run, issue_grant,
    };

    fn preimage() -> String {
        format!("<!doctype html><html><body>{BOOTSTRAP_HEADING_BEFORE}</body></html>")
    }
    fn postimage() -> String {
        format!("<!doctype html><html><body>{BOOTSTRAP_HEADING_AFTER}</body></html>")
    }

    fn open_broker() -> (TempDir, OperationBroker) {
        let directory = tempfile::tempdir().unwrap();
        let store = Arc::new(SqliteStore::open(directory.path().join(DATABASE_FILENAME)).unwrap());
        (directory, OperationBroker::new(store))
    }

    #[test]
    fn unsupported_ops_deny_and_persist_evidence() {
        let (_dir, broker) = open_broker();
        let clock = FakeClock::new(1_000);
        let run = create_run(broker.store.as_ref(), "fixture", None, clock.unix_ms()).unwrap();
        assert!(matches!(
            broker.deny_unsupported(run.id(), "process-exec", clock.unix_ms()),
            BrokerError::UnsupportedOperation("process-exec")
        ));
        assert!(matches!(
            broker.deny_unsupported(run.id(), "git-write", clock.unix_ms()),
            BrokerError::UnsupportedOperation("git-write")
        ));
        assert!(matches!(
            broker.deny_unsupported(run.id(), "publication", clock.unix_ms()),
            BrokerError::UnsupportedOperation("publication")
        ));
        assert!(matches!(
            broker.deny_unsupported(run.id(), "arbitrary-network", clock.unix_ms()),
            BrokerError::UnsupportedOperation("arbitrary-network")
        ));
        let reconstructed = broker.store.reconstruct_run(&run.id()).unwrap().unwrap();
        assert_eq!(reconstructed.denials.len(), 4);
        assert!(
            reconstructed
                .denials
                .iter()
                .any(|denial| { denial.reason().contains("publication") })
        );
        assert!(
            reconstructed
                .events
                .iter()
                .any(|event| { matches!(event.kind(), tule_core::RunEventKind::Denied { .. }) })
        );
    }

    #[test]
    fn provider_disclose_requires_grant_manifest_and_durable_dispatch() {
        let fixture = tempfile::tempdir().unwrap();
        std::fs::write(fixture.path().join("index.html"), preimage()).unwrap();
        let (_dir, broker) = open_broker();
        let clock = FakeClock::new(20_000);
        let run = create_run(broker.store.as_ref(), "fixture", None, clock.unix_ms()).unwrap();
        let manifest = ContextManifest::new(&preimage(), "heading", "preview").unwrap();
        let grant = issue_grant(
            broker.store.as_ref(),
            run.id(),
            CapabilityType::ProviderDisclose,
            GrantResourceSelector::ContextManifestHash(manifest.content_hash().to_owned()),
            GrantActionScope::Run,
            None,
            None,
            "owner",
            clock.unix_ms(),
        )
        .unwrap();
        let provider = FakeProvider::new(
            ConnectionStatus {
                state: ConnectionState::Connected,
                provider_id: "xai-subscription-oauth",
                model: "grok-3",
            },
            Ok(vec![ProviderEvent::Completed {
                response_id: Some("resp-1".to_owned()),
                input_tokens: Some(1),
                output_tokens: Some(2),
            }]),
        );
        let result = broker
            .provider_disclose(
                run.id(),
                &grant,
                manifest.content_hash(),
                manifest.request_semantic_hash(),
                "{\"model\":\"grok-3\"}".to_owned(),
                &provider,
                clock.unix_ms(),
            )
            .unwrap();
        assert_eq!(result.response_id.as_deref(), Some("resp-1"));
        let effect = broker.effect(run.id(), result.effect_id).unwrap();
        assert_eq!(effect.phase(), tule_core::EffectJournalPhase::Settled);

        // Bootstrap grant cannot be reused.
        let denied = broker.provider_disclose(
            run.id(),
            &grant,
            manifest.content_hash(),
            manifest.request_semantic_hash(),
            "{\"model\":\"grok-3\"}".to_owned(),
            &provider,
            clock.unix_ms(),
        );
        assert!(denied.is_err());
    }

    #[test]
    fn local_read_grant_never_authorizes_provider_disclose() {
        let (_dir, broker) = open_broker();
        let clock = FakeClock::new(21_000);
        let run = create_run(broker.store.as_ref(), "fixture", None, clock.unix_ms()).unwrap();
        let grant = issue_grant(
            broker.store.as_ref(),
            run.id(),
            CapabilityType::LocalRead,
            GrantResourceSelector::RelativeTarget(CONTROLLED_RELATIVE_TARGET.to_owned()),
            GrantActionScope::Run,
            None,
            None,
            "owner",
            clock.unix_ms(),
        )
        .unwrap();
        let provider = FakeProvider::new(
            ConnectionStatus {
                state: ConnectionState::Connected,
                provider_id: "xai-subscription-oauth",
                model: "grok-3",
            },
            Ok(vec![]),
        );
        let err = broker
            .provider_disclose(
                run.id(),
                &grant,
                "abc",
                "def",
                "{}".to_owned(),
                &provider,
                clock.unix_ms(),
            )
            .unwrap_err();
        assert!(matches!(err, BrokerError::GrantDenied(_)));
    }

    #[test]
    fn replacement_records_fault_after_dispatch_before_write() {
        let fixture = tempfile::tempdir().unwrap();
        std::fs::write(fixture.path().join("index.html"), preimage()).unwrap();
        let (_dir, broker) = open_broker();
        let clock = FakeClock::new(22_000);
        let run = create_run(broker.store.as_ref(), "fixture", None, clock.unix_ms()).unwrap();
        let manifest = ContextManifest::new(&preimage(), "heading", "preview").unwrap();
        let (plan, graph) = compile_and_freeze_pair(
            broker.store.as_ref(),
            run.id(),
            "change",
            "profile",
            "model",
            DisclosurePolicy::new("d1", "index.html"),
            CapabilityEnvelope::new(vec![CapabilityType::CreateOrReplace], "replace"),
            manifest,
            &preimage(),
            postimage(),
            CONTROLLED_RELATIVE_TARGET,
            "fs",
            None,
            None,
            clock.unix_ms(),
        )
        .unwrap();
        let approval = approve_pair(
            broker.store.as_ref(),
            run.id(),
            &plan,
            &graph,
            "owner",
            clock.unix_ms(),
        )
        .unwrap();
        let grant = issue_grant(
            broker.store.as_ref(),
            run.id(),
            CapabilityType::CreateOrReplace,
            GrantResourceSelector::ReplacementTarget {
                relative_target: CONTROLLED_RELATIVE_TARGET.to_owned(),
                expected_preimage_hash: plan.replacement().preimage_hash().to_owned(),
                expected_postimage_hash: plan.replacement().postimage_hash().to_owned(),
            },
            GrantActionScope::Node(tule_core::NODE_REPLACE_EXISTING_FILE_V1.to_owned()),
            Some(PlanGraphPairBinding {
                plan_version_id: plan.id(),
                graph_version_id: graph.id(),
            }),
            Some(approval.id()),
            "owner",
            clock.unix_ms(),
        )
        .unwrap();
        broker
            .fault_hook()
            .set(BrokerFaultPoint::AfterDispatchedBeforeWrite);
        let err = broker
            .create_or_replace(
                run.id(),
                fixture.path(),
                CONTROLLED_RELATIVE_TARGET,
                None,
                plan.replacement().preimage_hash(),
                plan.replacement().postimage_hash(),
                plan.replacement().postimage_utf8(),
                &grant,
                clock.unix_ms(),
                PlanGraphPairBinding {
                    plan_version_id: plan.id(),
                    graph_version_id: graph.id(),
                },
            )
            .unwrap_err();
        assert!(matches!(
            err,
            BrokerError::InjectedFault(BrokerFaultPoint::AfterDispatchedBeforeWrite)
        ));
        let reconstructed = broker.store.reconstruct_run(&run.id()).unwrap().unwrap();
        assert_eq!(reconstructed.effects.len(), 1);
        assert_eq!(
            reconstructed.effects[0].phase(),
            tule_core::EffectJournalPhase::Dispatched
        );
        assert_eq!(
            std::fs::read_to_string(fixture.path().join("index.html")).unwrap(),
            preimage()
        );
    }

    #[test]
    fn lease_takeover_uses_windows_prior_owner_probe() {
        let (_dir, broker) = open_broker();
        let clock = FakeClock::new(30_000);
        let run = create_run(broker.store.as_ref(), "fixture", None, clock.unix_ms()).unwrap();
        tule_core::acquire_root_lease(
            broker.store.as_ref(),
            run.id(),
            "pid:4294967294",
            clock.unix_ms(),
        )
        .unwrap();
        clock.advance(tule_core::ROOT_LEASE_TTL_MS + 1);
        let lease = broker
            .takeover_root_lease_with_windows_evidence(run.id(), "pid:1", clock.unix_ms())
            .unwrap();
        assert_eq!(lease.owner_process_instance(), "pid:1");

        // Current process is alive — takeover must fail without positive absence.
        let live = create_run(broker.store.as_ref(), "live", None, clock.unix_ms()).unwrap();
        let live_owner = format!("pid:{}", std::process::id());
        tule_core::acquire_root_lease(
            broker.store.as_ref(),
            live.id(),
            &live_owner,
            clock.unix_ms(),
        )
        .unwrap();
        let err = broker
            .takeover_root_lease_with_windows_evidence(live.id(), "pid:2", clock.unix_ms())
            .unwrap_err();
        assert!(matches!(err, BrokerError::Storage(_)));
    }
}
