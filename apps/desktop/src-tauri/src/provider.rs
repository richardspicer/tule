//! Provider-neutral native contracts. Provider implementations never expose secrets to IPC.

use std::{future::Future, pin::Pin};

use serde::Serialize;
use tokio_util::sync::CancellationToken;
use tule_core::{
    PROVIDER_PROFILE_ID, SelectedDefaultResolution, catalog_freshness, model_id_in_catalog,
    resolve_selected_default, validate_model_id,
};

use crate::sqlite::SqliteStore;

pub(crate) const PROVIDER_MODEL_CATALOG_CHANGED_EVENT: &str = "provider-model-catalog-changed";
pub(crate) const PROVIDER_MODEL_SELECTION_CHANGED_EVENT: &str = "provider-model-selection-changed";

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
    let resolution = resolve_selected_default(selection.selected_model_id.as_deref(), &entries);
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

/// Clears the selected default when it matches a rejected model id.
///
/// Writes an explicit requires-choice marker so the built-in catalog default
/// cannot silently reappear after rejection.
pub(crate) fn clear_selected_default_if_matches(
    store: &SqliteStore,
    rejected_model_id: &str,
) -> Result<ProviderModelSelectionResponse, PublicError> {
    let selection = store
        .get_model_selection(PROVIDER_PROFILE_ID)
        .map_err(|_| PublicError::AgentStorageUnavailable)?;
    let resolved = build_selection_response(store)?;
    if resolved.selected_model_id.as_deref() == Some(rejected_model_id)
        || selection.selected_model_id.as_deref() == Some(rejected_model_id)
    {
        let now = unix_now_ms()?;
        store
            .set_model_selection(PROVIDER_PROFILE_ID, Some(REQUIRES_EXPLICIT_CHOICE), now)
            .map_err(|_| PublicError::AgentStorageUnavailable)?;
    }
    build_selection_response(store)
}

/// Ensures a new-session model is present in the last validated catalog.
pub(crate) fn validate_new_session_model(
    store: &SqliteStore,
    model_id: &str,
) -> Result<String, PublicError> {
    let model_id = validate_model_id(model_id)
        .map(str::to_owned)
        .map_err(|_| PublicError::ModelUnavailable)?;
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
#[allow(dead_code)]
pub(crate) struct FakeProvider {
    status: ConnectionStatus,
    result: Result<Vec<ProviderEvent>, PublicError>,
}

#[cfg(test)]
#[allow(dead_code)]
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
    fn clear_selected_default_requires_new_choice() {
        let dir = tempfile_dir();
        let store = SqliteStore::open(dir.join("clear-selection.sqlite3")).unwrap();
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
                    model_id: "gpt-5.5".into(),
                    display_name: "GPT-5.5".into(),
                    description: None,
                    sort_order: 1,
                    is_provider_default: true,
                }],
            )
            .unwrap();
        persist_model_selection(&store, "gpt-5.5").unwrap();
        let cleared = clear_selected_default_if_matches(&store, "gpt-5.5").unwrap();
        assert!(cleared.selected_model_id.is_none());
        assert!(cleared.requires_selection);
    }
}
