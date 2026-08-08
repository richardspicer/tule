//! Provider-neutral native contracts. Provider implementations never expose secrets to IPC.

use std::{future::Future, pin::Pin};

use serde::Serialize;
use tokio_util::sync::CancellationToken;
use tule_core::{
    SelectedDefaultResolution, catalog_freshness, model_id_in_catalog, resolve_selected_default,
    validate_model_id,
};

use crate::sqlite::SqliteStore;

pub(crate) const PROVIDER_MODEL_CATALOG_CHANGED_EVENT: &str = "provider-model-catalog-changed";
pub(crate) const PROVIDER_MODEL_SELECTION_CHANGED_EVENT: &str = "provider-model-selection-changed";

/// Built-in xAI subscription OAuth provider-profile identifier.
pub(crate) const PROVIDER_PROFILE_ID: &str = "xai-subscription-oauth";

/// Upgrade-compatible default model identifier when still present in the catalog.
pub(crate) const MODEL_ID: &str = "grok-3";

/// Stored selected-default marker that forces an explicit new choice without
/// falling back to the built-in catalog default.
const REQUIRES_EXPLICIT_CHOICE: &str = "__requires_choice__";

/// Every error which may cross the native IPC boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PublicError {
    NotConnected,
    InvalidInput,
    ContextLimit,
    SessionBusy,
    AuthenticationRequired,
    EntitlementUnavailable,
    RateLimited,
    ProviderUnavailable,
    UnsupportedProviderOutput,
    OutputLimit,
    Cancelled,
    #[allow(dead_code)]
    Interrupted,
    CredentialStoreUnavailable,
    AgentStorageUnavailable,
    ModelUnavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
    ReconnectRequired,
    UnavailableInThisBuild,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConnectionStatus {
    pub(crate) state: ConnectionState,
    pub(crate) provider_id: &'static str,
    pub(crate) model: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProviderModelEntryResponse {
    pub(crate) id: String,
    pub(crate) display_name: String,
    pub(crate) description: Option<String>,
    pub(crate) is_provider_default: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProviderModelCatalogResponse {
    pub(crate) provider_id: String,
    pub(crate) models: Vec<ProviderModelEntryResponse>,
    pub(crate) freshness: String,
    pub(crate) retrieved_at_unix_ms: Option<i64>,
    pub(crate) compatibility_revision: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProviderModelSelectionResponse {
    pub(crate) provider_id: String,
    pub(crate) selected_model_id: Option<String>,
    pub(crate) requires_selection: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderRequest {
    pub(crate) session_id: String,
    pub(crate) request_json: String,
}

/// Required identity for Harness provider disclosure. Absence is not permitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HarnessDisclosureAuthority {
    pub(crate) run_id: String,
    pub(crate) grant_id: String,
    pub(crate) effect_id: String,
    pub(crate) manifest_content_hash: String,
    pub(crate) request_semantic_hash: String,
    pub(crate) registered_operation_id: String,
    pub(crate) registered_operation_schema: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProviderEvent {
    Delta(String),
    Completed {
        response_id: Option<String>,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
    },
}

pub(crate) type ProviderFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Vec<ProviderEvent>, PublicError>> + Send + 'a>>;

/// Sink for ordered provider events during an in-flight stream.
pub(crate) type ProviderEventSink = Box<dyn FnMut(ProviderEvent) -> Result<(), PublicError> + Send>;

/// A replaceable adapter boundary. Implementations own all HTTP and credential access.
pub(crate) trait ProviderAdapter: Send + Sync {
    fn connection_status(&self) -> ConnectionStatus;
    fn stream<'a>(
        &'a self,
        request: ProviderRequest,
        cancel: CancellationToken,
        on_event: ProviderEventSink,
    ) -> ProviderFuture<'a>;
}

fn unix_now_ms() -> Result<i64, PublicError> {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| PublicError::AgentStorageUnavailable)?
        .as_millis()
        .try_into()
        .map_err(|_| PublicError::AgentStorageUnavailable)
}

/// Revalidates Harness grant/effect/manifest identity and only then crosses the provider adapter.
pub(crate) fn dispatch_harness_provider(
    adapter: &dyn ProviderAdapter,
    store: &SqliteStore,
    authority: &HarnessDisclosureAuthority,
    request: ProviderRequest,
) -> Result<Vec<ProviderEvent>, PublicError> {
    revalidate_harness_disclosure(store, authority)?;
    let cancel = tokio_util::sync::CancellationToken::new();
    let collected = std::sync::Arc::new(std::sync::Mutex::new(Vec::<ProviderEvent>::new()));
    let sink_collected = std::sync::Arc::clone(&collected);
    let future = adapter.stream(
        request,
        cancel,
        Box::new(move |event| {
            sink_collected
                .lock()
                .map_err(|_| PublicError::ProviderUnavailable)?
                .push(event);
            Ok(())
        }),
    );
    let runtime_result = match tokio::runtime::Handle::try_current() {
        Ok(handle) => tokio::task::block_in_place(|| handle.block_on(future)),
        Err(_) => {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|_| PublicError::ProviderUnavailable)?;
            runtime.block_on(future)
        }
    };
    runtime_result?;
    collected
        .lock()
        .map(|guard| guard.clone())
        .map_err(|_| PublicError::ProviderUnavailable)
}

fn revalidate_harness_disclosure(
    store: &SqliteStore,
    authority: &HarnessDisclosureAuthority,
) -> Result<(), PublicError> {
    use tule_core::{
        CapabilityType, EffectJournalPhase, OP_PROVIDER_DISCLOSE_V1,
        REGISTERED_OPERATION_SCHEMA_V1, RunRepository,
    };
    if authority.registered_operation_id != OP_PROVIDER_DISCLOSE_V1
        || authority.registered_operation_schema != REGISTERED_OPERATION_SCHEMA_V1
    {
        return Err(PublicError::InvalidInput);
    }
    let run_id =
        tule_core::HarnessRunId::parse(&authority.run_id).map_err(|_| PublicError::InvalidInput)?;
    let grant_id = tule_core::CapabilityGrantId::parse(&authority.grant_id)
        .map_err(|_| PublicError::InvalidInput)?;
    let effect_id = tule_core::EffectRecordId::parse(&authority.effect_id)
        .map_err(|_| PublicError::InvalidInput)?;
    let reconstructed = store
        .reconstruct_run(&run_id)
        .map_err(|_| PublicError::AgentStorageUnavailable)?
        .ok_or(PublicError::InvalidInput)?;
    let effect = reconstructed
        .effects
        .iter()
        .find(|effect| effect.id() == effect_id)
        .ok_or(PublicError::InvalidInput)?;
    if effect.phase() != EffectJournalPhase::Dispatched {
        return Err(PublicError::InvalidInput);
    }
    if effect.grant_id() != grant_id {
        return Err(PublicError::InvalidInput);
    }
    if effect.operation_id() != OP_PROVIDER_DISCLOSE_V1 {
        return Err(PublicError::InvalidInput);
    }
    if effect.target_hash() != authority.request_semantic_hash {
        return Err(PublicError::InvalidInput);
    }
    let grant = reconstructed
        .grants
        .iter()
        .find(|grant| grant.id() == grant_id)
        .ok_or(PublicError::InvalidInput)?;
    if grant.capability() != CapabilityType::ProviderDisclose {
        return Err(PublicError::InvalidInput);
    }
    match grant.resource() {
        tule_core::GrantResourceSelector::ContextManifestHash(hash)
            if *hash == authority.manifest_content_hash => {}
        _ => return Err(PublicError::InvalidInput),
    }
    Ok(())
}

/// Marks a persisted catalog as visibly stale for failure/recovery surfaces.
pub(crate) fn build_stale_catalog_response(
    store: &SqliteStore,
) -> Result<ProviderModelCatalogResponse, PublicError> {
    let mut response = build_catalog_response(store)?;
    response.freshness = "stale".to_owned();
    Ok(response)
}

pub(crate) fn build_catalog_response(
    store: &SqliteStore,
) -> Result<ProviderModelCatalogResponse, PublicError> {
    if store
        .catalog_reads_are_sealed()
        .map_err(|_| PublicError::AgentStorageUnavailable)?
    {
        return Ok(ProviderModelCatalogResponse {
            provider_id: PROVIDER_PROFILE_ID.to_owned(),
            models: Vec::new(),
            freshness: "stale".to_owned(),
            retrieved_at_unix_ms: None,
            compatibility_revision: None,
        });
    }
    let snapshot = store
        .get_catalog_snapshot(PROVIDER_PROFILE_ID)
        .map_err(|_| PublicError::AgentStorageUnavailable)?;
    let Some(snapshot) = snapshot else {
        return Ok(ProviderModelCatalogResponse {
            provider_id: PROVIDER_PROFILE_ID.to_owned(),
            models: Vec::new(),
            freshness: "stale".to_owned(),
            retrieved_at_unix_ms: None,
            compatibility_revision: None,
        });
    };
    let now = unix_now_ms()?;
    let freshness = catalog_freshness(snapshot.state.retrieved_at_unix_ms, now);
    Ok(ProviderModelCatalogResponse {
        provider_id: PROVIDER_PROFILE_ID.to_owned(),
        models: snapshot
            .entries
            .into_iter()
            .map(|entry| ProviderModelEntryResponse {
                id: entry.model_id,
                display_name: entry.display_name,
                description: entry.description,
                is_provider_default: entry.is_provider_default,
            })
            .collect(),
        freshness: freshness.as_str().to_owned(),
        retrieved_at_unix_ms: Some(snapshot.state.retrieved_at_unix_ms),
        compatibility_revision: Some(snapshot.state.compatibility_revision),
    })
}

pub(crate) fn build_selection_response(
    store: &SqliteStore,
) -> Result<ProviderModelSelectionResponse, PublicError> {
    if store
        .catalog_reads_are_sealed()
        .map_err(|_| PublicError::AgentStorageUnavailable)?
    {
        return Ok(ProviderModelSelectionResponse {
            provider_id: PROVIDER_PROFILE_ID.to_owned(),
            selected_model_id: None,
            requires_selection: false,
        });
    }
    let selection = store
        .get_model_selection(PROVIDER_PROFILE_ID)
        .map_err(|_| PublicError::AgentStorageUnavailable)?;
    let entries = store
        .get_catalog_snapshot(PROVIDER_PROFILE_ID)
        .map_err(|_| PublicError::AgentStorageUnavailable)?
        .map(|snapshot| snapshot.entries)
        .unwrap_or_default();
    if selection.selected_model_id.as_deref() == Some(REQUIRES_EXPLICIT_CHOICE) {
        return Ok(ProviderModelSelectionResponse {
            provider_id: PROVIDER_PROFILE_ID.to_owned(),
            selected_model_id: None,
            requires_selection: !entries.is_empty(),
        });
    }
    let resolution =
        resolve_selected_default(selection.selected_model_id.as_deref(), &entries, MODEL_ID);
    match resolution {
        SelectedDefaultResolution::Available(model_id) => Ok(ProviderModelSelectionResponse {
            provider_id: PROVIDER_PROFILE_ID.to_owned(),
            selected_model_id: Some(model_id),
            requires_selection: false,
        }),
        SelectedDefaultResolution::RequiresChoice => {
            let requires_selection = !entries.is_empty() || selection.selected_model_id.is_some();
            Ok(ProviderModelSelectionResponse {
                provider_id: PROVIDER_PROFILE_ID.to_owned(),
                selected_model_id: selection
                    .selected_model_id
                    .filter(|model_id| model_id != REQUIRES_EXPLICIT_CHOICE),
                requires_selection,
            })
        }
    }
}

pub(crate) fn persist_model_selection(
    store: &SqliteStore,
    model_id: &str,
) -> Result<ProviderModelSelectionResponse, PublicError> {
    let model_id = validate_model_id(model_id)
        .map(str::to_owned)
        .map_err(|_| PublicError::ModelUnavailable)?;
    let snapshot = store
        .get_catalog_snapshot(PROVIDER_PROFILE_ID)
        .map_err(|_| PublicError::AgentStorageUnavailable)?;
    let entries = snapshot
        .as_ref()
        .map(|item| item.entries.as_slice())
        .unwrap_or(&[]);
    if store
        .is_model_rejected(PROVIDER_PROFILE_ID, &model_id)
        .map_err(|_| PublicError::AgentStorageUnavailable)?
    {
        return Err(PublicError::ModelUnavailable);
    }
    if !model_id_in_catalog(&model_id, entries) {
        return Err(PublicError::ModelUnavailable);
    }
    let now = unix_now_ms()?;
    // Selected-default persistence is authoritative and separate from profile
    // display metadata (`visible_model_id`).
    store
        .set_model_selection(PROVIDER_PROFILE_ID, Some(&model_id), now)
        .map_err(|_| PublicError::AgentStorageUnavailable)?;
    build_selection_response(store)
}

/// Records a provider model rejection and clears any matching selected default.
///
/// The rejected identifier remains unavailable for new-session choice for the
/// current credential generation, even when a later catalog refresh returns it.
pub(crate) fn apply_model_rejection(
    store: &SqliteStore,
    rejected_model_id: &str,
) -> Result<(ProviderModelCatalogResponse, ProviderModelSelectionResponse), PublicError> {
    let rejected_model_id = validate_model_id(rejected_model_id)
        .map(str::to_owned)
        .map_err(|_| PublicError::ModelUnavailable)?;
    let now = unix_now_ms()?;
    store
        .record_rejected_model(PROVIDER_PROFILE_ID, &rejected_model_id, now)
        .map_err(|_| PublicError::AgentStorageUnavailable)?;
    let selection = store
        .get_model_selection(PROVIDER_PROFILE_ID)
        .map_err(|_| PublicError::AgentStorageUnavailable)?;
    let resolved = build_selection_response(store)?;
    if resolved.selected_model_id.as_deref() == Some(rejected_model_id.as_str())
        || selection.selected_model_id.as_deref() == Some(rejected_model_id.as_str())
    {
        store
            .set_model_selection(PROVIDER_PROFILE_ID, Some(REQUIRES_EXPLICIT_CHOICE), now)
            .map_err(|_| PublicError::AgentStorageUnavailable)?;
    }
    let catalog = build_catalog_response(store)?;
    let selection = build_selection_response(store)?;
    Ok((catalog, selection))
}

/// Ensures a new-session model is present in the last validated catalog.
pub(crate) fn validate_new_session_model(
    store: &SqliteStore,
    model_id: &str,
) -> Result<String, PublicError> {
    let model_id = validate_model_id(model_id)
        .map(str::to_owned)
        .map_err(|_| PublicError::ModelUnavailable)?;
    if store
        .is_model_rejected(PROVIDER_PROFILE_ID, &model_id)
        .map_err(|_| PublicError::AgentStorageUnavailable)?
    {
        return Err(PublicError::ModelUnavailable);
    }
    let snapshot = store
        .get_catalog_snapshot(PROVIDER_PROFILE_ID)
        .map_err(|_| PublicError::AgentStorageUnavailable)?;
    let Some(snapshot) = snapshot else {
        return Err(PublicError::ModelUnavailable);
    };
    if !model_id_in_catalog(&model_id, &snapshot.entries) {
        return Err(PublicError::ModelUnavailable);
    }
    Ok(model_id)
}

#[cfg(test)]
pub(crate) struct FakeProvider {
    status: ConnectionStatus,
    result: Result<Vec<ProviderEvent>, PublicError>,
}

#[cfg(test)]
impl FakeProvider {
    pub(crate) fn new(
        status: ConnectionStatus,
        result: Result<Vec<ProviderEvent>, PublicError>,
    ) -> Self {
        Self { status, result }
    }
}

#[cfg(test)]
impl ProviderAdapter for FakeProvider {
    fn connection_status(&self) -> ConnectionStatus {
        self.status.clone()
    }
    fn stream<'a>(
        &'a self,
        _: ProviderRequest,
        cancel: CancellationToken,
        mut on_event: ProviderEventSink,
    ) -> ProviderFuture<'a> {
        let result = self.result.clone();
        Box::pin(async move {
            if cancel.is_cancelled() {
                return Err(PublicError::Cancelled);
            }
            match result {
                Ok(events) => {
                    for event in events {
                        if cancel.is_cancelled() {
                            return Err(PublicError::Cancelled);
                        }
                        on_event(event)?;
                    }
                    Ok(Vec::new())
                }
                Err(error) => Err(error),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite::{SqliteStore, StoredCatalogState};
    use tule_core::{AgentRepository, ModelCatalogEntry};

    #[test]
    fn public_errors_serialize_without_internal_detail() {
        assert_eq!(
            serde_json::to_string(&PublicError::UnsupportedProviderOutput).unwrap(),
            "\"unsupported_provider_output\""
        );
    }

    fn tempfile_dir() -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "tule-provider-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn selected_default_write_is_atomic_and_separate_from_display_metadata() {
        let dir = tempfile_dir();
        let store = SqliteStore::open(dir.join("selection.sqlite3")).unwrap();
        let before = store
            .get_provider_profile(PROVIDER_PROFILE_ID)
            .unwrap()
            .unwrap()
            .visible_model_id()
            .to_owned();
        store
            .replace_catalog_snapshot(
                PROVIDER_PROFILE_ID,
                &StoredCatalogState {
                    credential_generation: 0,
                    compatibility_revision: "1.0.0".into(),
                    etag: None,
                    retrieved_at_unix_ms: 1,
                    updated_at_unix_ms: 1,
                },
                &[ModelCatalogEntry {
                    model_id: "other-model".into(),
                    display_name: "Other".into(),
                    description: None,
                    sort_order: 1,
                    is_provider_default: false,
                }],
            )
            .unwrap();

        let selection = persist_model_selection(&store, "other-model").unwrap();
        assert_eq!(selection.selected_model_id.as_deref(), Some("other-model"));
        assert_eq!(
            store
                .get_provider_profile(PROVIDER_PROFILE_ID)
                .unwrap()
                .unwrap()
                .visible_model_id(),
            before
        );

        store.set_fail_model_selection_write(true);
        assert_eq!(
            persist_model_selection(&store, "other-model"),
            Err(PublicError::AgentStorageUnavailable)
        );
        assert_eq!(
            store
                .get_model_selection(PROVIDER_PROFILE_ID)
                .unwrap()
                .selected_model_id
                .as_deref(),
            Some("other-model")
        );
    }

    #[test]
    fn model_rejection_recovery_clears_default_and_blocks_repeated_selection() {
        let dir = tempfile_dir();
        let store = SqliteStore::open(dir.join("reject-recovery.sqlite3")).unwrap();
        let entries = vec![
            ModelCatalogEntry {
                model_id: "bad-model".into(),
                display_name: "Bad".into(),
                description: None,
                sort_order: 1,
                is_provider_default: false,
            },
            ModelCatalogEntry {
                model_id: "good-model".into(),
                display_name: "Good".into(),
                description: None,
                sort_order: 2,
                is_provider_default: true,
            },
        ];
        store
            .replace_catalog_snapshot(
                PROVIDER_PROFILE_ID,
                &StoredCatalogState {
                    credential_generation: 0,
                    compatibility_revision: "1.0.0".into(),
                    etag: None,
                    retrieved_at_unix_ms: 1,
                    updated_at_unix_ms: 1,
                },
                &entries,
            )
            .unwrap();
        persist_model_selection(&store, "bad-model").unwrap();

        let (catalog, selection) = apply_model_rejection(&store, "bad-model").unwrap();
        assert!(selection.selected_model_id.is_none());
        assert!(selection.requires_selection);
        assert_eq!(
            catalog
                .models
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            vec!["good-model"]
        );
        assert_eq!(
            validate_new_session_model(&store, "bad-model"),
            Err(PublicError::ModelUnavailable)
        );
        assert_eq!(
            persist_model_selection(&store, "bad-model"),
            Err(PublicError::ModelUnavailable)
        );

        // Refresh returning the same identifier must not re-offer it.
        store
            .replace_catalog_snapshot(
                PROVIDER_PROFILE_ID,
                &StoredCatalogState {
                    credential_generation: 0,
                    compatibility_revision: "1.0.0".into(),
                    etag: Some("\"v2\"".into()),
                    retrieved_at_unix_ms: 2,
                    updated_at_unix_ms: 2,
                },
                &entries,
            )
            .unwrap();
        let after_refresh = build_catalog_response(&store).unwrap();
        assert!(
            !after_refresh
                .models
                .iter()
                .any(|model| model.id == "bad-model")
        );
        assert_eq!(
            validate_new_session_model(&store, "bad-model"),
            Err(PublicError::ModelUnavailable)
        );
        assert_eq!(
            validate_new_session_model(&store, "good-model").as_deref(),
            Ok("good-model")
        );
    }

    #[test]
    fn harness_provider_dispatch_requires_dispatched_effect_and_manifest_binding() {
        use std::sync::Arc;
        use tule_core::{
            CapabilityType, Clock, ContextManifest, FakeClock, GrantActionScope,
            GrantResourceSelector, OP_PROVIDER_DISCLOSE_V1, REGISTERED_OPERATION_SCHEMA_V1,
            claim_effect, create_run, dispatch_effect, issue_grant, prepare_effect,
        };

        let dir = tempfile_dir();
        let store = SqliteStore::open(dir.join("harness-provider.sqlite3")).unwrap();
        let clock = FakeClock::new(30_000);
        let run = create_run(&store, "fixture", None, clock.unix_ms()).unwrap();
        let manifest = ContextManifest::new("<h1>Ready</h1>", "heading", "preview").unwrap();
        let grant = issue_grant(
            &store,
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
        let effect = prepare_effect(
            &store,
            run.id(),
            None,
            None,
            None,
            OP_PROVIDER_DISCLOSE_V1,
            manifest.request_semantic_hash(),
            grant.id(),
            clock.unix_ms(),
            None,
            None,
        )
        .unwrap();
        claim_effect(&store, run.id(), effect.id(), "broker", clock.unix_ms()).unwrap();
        let authority = HarnessDisclosureAuthority {
            run_id: run.id().to_string(),
            grant_id: grant.id().to_string(),
            effect_id: effect.id().to_string(),
            manifest_content_hash: manifest.content_hash().to_owned(),
            request_semantic_hash: manifest.request_semantic_hash().to_owned(),
            registered_operation_id: OP_PROVIDER_DISCLOSE_V1.to_owned(),
            registered_operation_schema: REGISTERED_OPERATION_SCHEMA_V1.to_owned(),
        };
        let provider = FakeProvider::new(
            ConnectionStatus {
                state: ConnectionState::Connected,
                provider_id: PROVIDER_PROFILE_ID,
                model: MODEL_ID,
            },
            Ok(vec![ProviderEvent::Completed {
                response_id: Some("r1".into()),
                input_tokens: Some(1),
                output_tokens: Some(1),
            }]),
        );
        // Not yet dispatched — must deny before crossing the adapter.
        assert_eq!(
            dispatch_harness_provider(
                &provider,
                &store,
                &authority,
                ProviderRequest {
                    session_id: run.id().to_string(),
                    request_json: "{}".into(),
                },
            ),
            Err(PublicError::InvalidInput)
        );
        dispatch_effect(
            &store,
            run.id(),
            effect.id(),
            grant.id(),
            "broker",
            clock.unix_ms(),
        )
        .unwrap();
        let events = dispatch_harness_provider(
            &provider,
            &store,
            &authority,
            ProviderRequest {
                session_id: run.id().to_string(),
                request_json: "{}".into(),
            },
        )
        .unwrap();
        assert!(matches!(
            events.as_slice(),
            [ProviderEvent::Completed {
                response_id: Some(_),
                ..
            }]
        ));
        let _ = Arc::new(store);
    }
}
