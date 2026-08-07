//! Narrow typed Agent IPC commands and ordered stream channel messages.

use std::{
    future::Future,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use serde::Serialize;
use tauri::{AppHandle, Emitter, State, Webview, ipc::Channel};
use tauri_plugin_dialog::{DialogExt, FilePath};
use tokio_util::sync::CancellationToken;
use tule_core::{
    AgentEvent, AgentRepository, AgentSession, AgentSessionId, AgentTurn, AgentTurnId,
    AgentTurnState, Artifact, ArtifactDetail, ArtifactSummary, ArtifactVersion, ProjectId,
    ProjectRepository, Source, TurnSource,
};

use crate::{
    provider::{
        PROVIDER_MODEL_CATALOG_CHANGED_EVENT, PROVIDER_MODEL_SELECTION_CHANGED_EVENT,
        ProviderAdapter, ProviderEvent, PublicError, apply_model_rejection,
        build_selection_response, build_stale_catalog_response,
    },
    source_draft::{
        NativeSourceFileReader, NativeSourceFolderReader, NativeSourceUrlFetcher,
        PickSourceOutcome, SourceDraftError, SourceDraftStore, SourceFilePicker,
        SourceFolderPicker, capture_link_source, capture_picked_folder, capture_picked_source,
    },
    sqlite::SqliteStore,
};

/// Allowlisted Agent IPC failures, including Source capture codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AgentIpcError {
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
    Interrupted,
    CredentialStoreUnavailable,
    AgentStorageUnavailable,
    ModelUnavailable,
    SourceUnreadable,
    SourceUnsupported,
    SourceTooLarge,
    SourceDraftExpired,
}

impl From<PublicError> for AgentIpcError {
    fn from(error: PublicError) -> Self {
        match error {
            PublicError::NotConnected => Self::NotConnected,
            PublicError::InvalidInput => Self::InvalidInput,
            PublicError::ContextLimit => Self::ContextLimit,
            PublicError::SessionBusy => Self::SessionBusy,
            PublicError::AuthenticationRequired => Self::AuthenticationRequired,
            PublicError::EntitlementUnavailable => Self::EntitlementUnavailable,
            PublicError::RateLimited => Self::RateLimited,
            PublicError::ProviderUnavailable => Self::ProviderUnavailable,
            PublicError::UnsupportedProviderOutput => Self::UnsupportedProviderOutput,
            PublicError::OutputLimit => Self::OutputLimit,
            PublicError::Cancelled => Self::Cancelled,
            PublicError::Interrupted => Self::Interrupted,
            PublicError::CredentialStoreUnavailable => Self::CredentialStoreUnavailable,
            PublicError::AgentStorageUnavailable => Self::AgentStorageUnavailable,
            PublicError::ModelUnavailable => Self::ModelUnavailable,
        }
    }
}

impl From<SourceDraftError> for AgentIpcError {
    fn from(error: SourceDraftError) -> Self {
        match error {
            SourceDraftError::Unreadable => Self::SourceUnreadable,
            SourceDraftError::Unsupported | SourceDraftError::RandomUnavailable => {
                Self::SourceUnsupported
            }
            SourceDraftError::TooLarge => Self::SourceTooLarge,
        }
    }
}

pub(crate) struct AgentState {
    pub(crate) store: Arc<SqliteStore>,
    pub(crate) provider: Arc<dyn ProviderAdapter>,
    pub(crate) xai: Option<Arc<crate::xai_subscription::XaiSubscriptionAdapter>>,
    pub(crate) source_drafts: Arc<SourceDraftStore>,
    source_url_fetcher: Arc<NativeSourceUrlFetcher>,
    operation_gate: Arc<OperationGate>,
    cancellation: Arc<Mutex<Option<(String, CancellationToken)>>>,
}

#[derive(Default)]
struct OperationGate {
    active: AtomicBool,
}

pub(crate) struct OperationGuard {
    gate: Arc<OperationGate>,
}

impl Drop for OperationGuard {
    fn drop(&mut self) {
        self.gate.active.store(false, Ordering::Release);
    }
}

struct SendOperationGuard {
    _operation: OperationGuard,
    cancellation: Arc<Mutex<Option<(String, CancellationToken)>>>,
}

impl Drop for SendOperationGuard {
    fn drop(&mut self) {
        if let Ok(mut slot) = self.cancellation.lock() {
            *slot = None;
        }
    }
}

impl AgentState {
    pub(crate) fn new(
        store: Arc<SqliteStore>,
        provider: Arc<dyn ProviderAdapter>,
        xai: Option<Arc<crate::xai_subscription::XaiSubscriptionAdapter>>,
    ) -> Self {
        Self {
            store,
            provider,
            xai,
            source_drafts: Arc::new(SourceDraftStore::new()),
            source_url_fetcher: Arc::new(
                NativeSourceUrlFetcher::new().expect("native link fetcher initialization"),
            ),
            operation_gate: Arc::new(OperationGate::default()),
            cancellation: Arc::new(Mutex::new(None)),
        }
    }

    pub(crate) fn clear_source_drafts(&self) {
        self.source_drafts.clear_all();
    }

    pub(crate) fn xai(&self) -> Option<Arc<crate::xai_subscription::XaiSubscriptionAdapter>> {
        self.xai.clone()
    }

    pub(crate) fn try_operation(&self) -> Result<OperationGuard, PublicError> {
        if self
            .operation_gate
            .active
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            return Err(PublicError::SessionBusy);
        }
        Ok(OperationGuard {
            gate: Arc::clone(&self.operation_gate),
        })
    }

    fn try_send_operation(&self) -> Result<SendOperationGuard, PublicError> {
        Ok(SendOperationGuard {
            _operation: self.try_operation()?,
            cancellation: Arc::clone(&self.cancellation),
        })
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SessionResponse {
    id: String,
    title: String,
    project_id: Option<String>,
    model_id: String,
}
impl From<AgentSession> for SessionResponse {
    fn from(value: AgentSession) -> Self {
        Self {
            id: value.id().to_string(),
            title: value.title().into(),
            project_id: value.project_id().map(|id| id.to_string()),
            model_id: value.model_id().into(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SourceMetadataResponse {
    id: String,
    origin_kind: String,
    display_name: String,
    byte_count: u64,
    content_sha256: String,
    member_count: u32,
    canonical_url: Option<String>,
}

impl From<&Source> for SourceMetadataResponse {
    fn from(value: &Source) -> Self {
        Self {
            id: value.id().to_string(),
            origin_kind: value.origin_kind().into(),
            display_name: value.display_name().into(),
            byte_count: value.byte_count(),
            content_sha256: value.content_sha256().into(),
            member_count: value.member_count(),
            canonical_url: value.canonical_url().map(str::to_owned),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TurnResponse {
    id: String,
    ordinal: u64,
    user_text: String,
    agent_text: String,
    state: String,
    error_code: Option<String>,
    /// Product Effort used for the turn when available; null when omitted.
    effort: Option<String>,
    /// Durable turn start time from persisted state.
    started_at_unix_ms: i64,
    /// Durable finish time when the turn is terminal; null while in flight.
    finished_at_unix_ms: Option<i64>,
    /// Provider-reported input tokens when present; null when unreported.
    usage_input_tokens: Option<u64>,
    /// Provider-reported output tokens when present; null when unreported.
    usage_output_tokens: Option<u64>,
    sources: Vec<SourceMetadataResponse>,
}

/// Structured per-turn metrics snapshot for clipboard / typed IPC export.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) struct TurnMetricsExportResponse {
    turn_id: String,
    session_id: String,
    ordinal: u64,
    state: String,
    provider_profile_id: String,
    model_id: String,
    effort: Option<String>,
    started_at_unix_ms: i64,
    finished_at_unix_ms: Option<i64>,
    duration_ms: Option<i64>,
    usage_input_tokens: Option<u64>,
    usage_output_tokens: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModelRequestControlsResponse {
    model_id: String,
    effort_available: bool,
    effort_values: Vec<&'static str>,
    effort_default: Option<&'static str>,
    speed_available: bool,
}

fn turn_response(value: &AgentTurn, sources: &[TurnSource]) -> TurnResponse {
    let mut turn_sources: Vec<&TurnSource> = sources
        .iter()
        .filter(|item| item.turn_id() == value.id())
        .collect();
    turn_sources.sort_by_key(|item| item.attachment_order());
    TurnResponse {
        id: value.id().to_string(),
        ordinal: value.ordinal(),
        user_text: value.user_text().into(),
        agent_text: value.agent_text().into(),
        state: value.state().as_str().into(),
        error_code: value.error_code().map(str::to_owned),
        effort: value.effort().map(|effort| effort.as_str().to_owned()),
        started_at_unix_ms: value.started_at_unix_ms(),
        finished_at_unix_ms: value.finished_at_unix_ms(),
        usage_input_tokens: value.usage_input_tokens(),
        usage_output_tokens: value.usage_output_tokens(),
        sources: turn_sources
            .into_iter()
            .map(|item| SourceMetadataResponse::from(item.source()))
            .collect(),
    }
}

fn turn_metrics_export(value: &AgentTurn) -> TurnMetricsExportResponse {
    let finished_at_unix_ms = value.finished_at_unix_ms();
    let duration_ms =
        finished_at_unix_ms.map(|finished| finished.saturating_sub(value.started_at_unix_ms()));
    TurnMetricsExportResponse {
        turn_id: value.id().to_string(),
        session_id: value.session_id().to_string(),
        ordinal: value.ordinal(),
        state: value.state().as_str().into(),
        provider_profile_id: value.provider_profile_id().into(),
        model_id: value.model_id().into(),
        effort: value.effort().map(|effort| effort.as_str().to_owned()),
        started_at_unix_ms: value.started_at_unix_ms(),
        finished_at_unix_ms,
        duration_ms,
        usage_input_tokens: value.usage_input_tokens(),
        usage_output_tokens: value.usage_output_tokens(),
    }
}

fn model_request_controls_response(model_id: &str) -> ModelRequestControlsResponse {
    match crate::xai_subscription::effort_capability_for_model(model_id) {
        Some(capability) => ModelRequestControlsResponse {
            model_id: model_id.to_owned(),
            effort_available: true,
            effort_values: vec!["low", "medium", "high"],
            effort_default: Some(capability.default.as_str()),
            speed_available: false,
        },
        None => ModelRequestControlsResponse {
            model_id: model_id.to_owned(),
            effort_available: false,
            effort_values: Vec::new(),
            effort_default: None,
            speed_available: false,
        },
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum AgentStreamEvent {
    Started { session_id: String, turn_id: String },
    Delta { turn_id: String, text: String },
    Terminal { turn: TurnResponse },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PickAgentTextSourceResponse {
    status: String,
    draft_handle: Option<String>,
    display_name: Option<String>,
    byte_count: Option<u64>,
    origin_kind: Option<String>,
    member_count: Option<u32>,
    canonical_url: Option<String>,
}

fn require_main_window(webview: &Webview) -> Result<(), AgentIpcError> {
    if webview.label() != "main" {
        return Err(AgentIpcError::InvalidInput);
    }
    Ok(())
}

struct AppDialogPicker {
    app: AppHandle,
}

impl SourceFilePicker for AppDialogPicker {
    fn pick_file(&self) -> Option<std::path::PathBuf> {
        match self.app.dialog().file().blocking_pick_file()? {
            FilePath::Path(path) => Some(path),
            FilePath::Url(_) => None,
        }
    }
}

impl SourceFolderPicker for AppDialogPicker {
    fn pick_folder(&self) -> Option<std::path::PathBuf> {
        match self.app.dialog().file().blocking_pick_folder()? {
            FilePath::Path(path) => Some(path),
            FilePath::Url(_) => None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EventResponse {
    id: String,
    session_id: String,
    turn_id: Option<String>,
    sequence: u64,
    kind: String,
    created_at_unix_ms: i64,
}

impl From<&AgentEvent> for EventResponse {
    fn from(event: &AgentEvent) -> Self {
        Self {
            id: event.id().to_string(),
            session_id: event.session_id().to_string(),
            turn_id: event.turn_id().map(|turn_id| turn_id.to_string()),
            sequence: event.sequence(),
            kind: event.kind().as_str().to_owned(),
            created_at_unix_ms: event.created_at_unix_ms(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SessionDetailResponse {
    session: SessionResponse,
    turns: Vec<TurnResponse>,
    events: Vec<EventResponse>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ArtifactVersionProvenanceResponse {
    source_session_id: String,
    source_turn_id: String,
    provider_profile_id: String,
    model_id: String,
    prompt_version: String,
    project_id: Option<String>,
    provider_request_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ArtifactVersionResponse {
    id: String,
    artifact_id: String,
    version_ordinal: u64,
    content: String,
    content_sha256: String,
    provenance: ArtifactVersionProvenanceResponse,
    created_at_unix_ms: i64,
}

impl From<&ArtifactVersion> for ArtifactVersionResponse {
    fn from(version: &ArtifactVersion) -> Self {
        let provenance = version.provenance();
        Self {
            id: version.id().to_string(),
            artifact_id: version.artifact_id().to_string(),
            version_ordinal: version.version_ordinal(),
            content: version.content().to_owned(),
            content_sha256: version.content_sha256().to_owned(),
            provenance: ArtifactVersionProvenanceResponse {
                source_session_id: provenance.source_session_id().to_string(),
                source_turn_id: provenance.source_turn_id().to_string(),
                provider_profile_id: provenance.provider_profile_id().to_owned(),
                model_id: provenance.model_id().to_owned(),
                prompt_version: provenance.prompt_version().to_owned(),
                project_id: provenance.project_id().map(|id| id.to_string()),
                provider_request_id: provenance.provider_request_id().to_string(),
            },
            created_at_unix_ms: version.created_at_unix_ms(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ArtifactResponse {
    id: String,
    title: String,
    kind: String,
    project_id: Option<String>,
    created_at_unix_ms: i64,
}

impl From<&Artifact> for ArtifactResponse {
    fn from(artifact: &Artifact) -> Self {
        Self {
            id: artifact.id().to_string(),
            title: artifact.title().to_owned(),
            kind: artifact.kind().as_str().to_owned(),
            project_id: artifact.project_id().map(|id| id.to_string()),
            created_at_unix_ms: artifact.created_at_unix_ms(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ArtifactSummaryResponse {
    id: String,
    title: String,
    kind: String,
    project_id: Option<String>,
    created_at_unix_ms: i64,
    latest_version_id: String,
    latest_version_ordinal: u64,
}

impl From<&ArtifactSummary> for ArtifactSummaryResponse {
    fn from(summary: &ArtifactSummary) -> Self {
        let artifact = summary.artifact();
        Self {
            id: artifact.id().to_string(),
            title: artifact.title().to_owned(),
            kind: artifact.kind().as_str().to_owned(),
            project_id: artifact.project_id().map(|id| id.to_string()),
            created_at_unix_ms: artifact.created_at_unix_ms(),
            latest_version_id: summary.latest_version_id().to_string(),
            latest_version_ordinal: summary.latest_version_ordinal(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ArtifactDetailResponse {
    artifact: ArtifactResponse,
    versions: Vec<ArtifactVersionResponse>,
}

impl From<&ArtifactDetail> for ArtifactDetailResponse {
    fn from(detail: &ArtifactDetail) -> Self {
        Self {
            artifact: ArtifactResponse::from(detail.artifact()),
            versions: detail
                .versions()
                .iter()
                .map(ArtifactVersionResponse::from)
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateArtifactResponse {
    artifact: ArtifactResponse,
    version: ArtifactVersionResponse,
}

fn map_create_artifact(error: tule_core::CreateArtifactFromTurnError) -> AgentIpcError {
    match error {
        tule_core::CreateArtifactFromTurnError::InvalidTurnId(_)
        | tule_core::CreateArtifactFromTurnError::InvalidKind(_)
        | tule_core::CreateArtifactFromTurnError::TurnNotFound
        | tule_core::CreateArtifactFromTurnError::TurnNotCompleted
        | tule_core::CreateArtifactFromTurnError::EmptyAgentText
        | tule_core::CreateArtifactFromTurnError::Validation(_) => AgentIpcError::InvalidInput,
        tule_core::CreateArtifactFromTurnError::Time(_)
        | tule_core::CreateArtifactFromTurnError::AgentRepository(_)
        | tule_core::CreateArtifactFromTurnError::ArtifactRepository(_) => {
            AgentIpcError::AgentStorageUnavailable
        }
    }
}

fn map_list_artifacts(error: tule_core::ListArtifactsError) -> AgentIpcError {
    match error {
        tule_core::ListArtifactsError::InvalidSessionId(_)
        | tule_core::ListArtifactsError::InvalidProjectId(_) => AgentIpcError::InvalidInput,
        tule_core::ListArtifactsError::Repository(_) => AgentIpcError::AgentStorageUnavailable,
    }
}

fn map_get_artifact(error: tule_core::GetArtifactError) -> AgentIpcError {
    match error {
        tule_core::GetArtifactError::InvalidArtifactId(_)
        | tule_core::GetArtifactError::NotFound => AgentIpcError::InvalidInput,
        tule_core::GetArtifactError::Repository(_) => AgentIpcError::AgentStorageUnavailable,
    }
}

fn map_prepare(error: tule_core::PrepareAgentSendError) -> AgentIpcError {
    match error {
        tule_core::PrepareAgentSendError::InvalidInput(_) => AgentIpcError::InvalidInput,
        tule_core::PrepareAgentSendError::ContextLimit { .. } => {
            AgentIpcError::from(PublicError::ContextLimit)
        }
        tule_core::PrepareAgentSendError::SessionBusy => AgentIpcError::SessionBusy,
        tule_core::PrepareAgentSendError::SessionNotFound => AgentIpcError::InvalidInput,
        tule_core::PrepareAgentSendError::ProjectAssociationMismatch => AgentIpcError::InvalidInput,
        tule_core::PrepareAgentSendError::ProviderProfileUnavailable(_) => {
            AgentIpcError::InvalidInput
        }
        tule_core::PrepareAgentSendError::ModelUnavailable(_) => AgentIpcError::ModelUnavailable,
        tule_core::PrepareAgentSendError::UnsupportedRequestControl => AgentIpcError::InvalidInput,
        tule_core::PrepareAgentSendError::Time(_)
        | tule_core::PrepareAgentSendError::Repository(_) => AgentIpcError::AgentStorageUnavailable,
    }
}

fn map_finish(_: tule_core::FinishAgentTurnError) -> AgentIpcError {
    AgentIpcError::AgentStorageUnavailable
}

fn public_error_code(error: PublicError) -> &'static str {
    match error {
        PublicError::NotConnected => "not_connected",
        PublicError::InvalidInput => "invalid_input",
        PublicError::ContextLimit => "context_limit",
        PublicError::SessionBusy => "session_busy",
        PublicError::AuthenticationRequired => "authentication_required",
        PublicError::EntitlementUnavailable => "entitlement_unavailable",
        PublicError::RateLimited => "rate_limited",
        PublicError::ProviderUnavailable => "provider_unavailable",
        PublicError::UnsupportedProviderOutput => "unsupported_provider_output",
        PublicError::OutputLimit => "output_limit",
        PublicError::Cancelled => "cancelled",
        PublicError::Interrupted => "interrupted",
        PublicError::CredentialStoreUnavailable => "credential_store_unavailable",
        PublicError::AgentStorageUnavailable => "agent_storage_unavailable",
        PublicError::ModelUnavailable => "model_unavailable",
    }
}

fn turn_sources_for(
    store: &SqliteStore,
    session_id: &AgentSessionId,
) -> Result<Vec<TurnSource>, AgentIpcError> {
    store
        .list_turn_sources(session_id)
        .map_err(|_| AgentIpcError::AgentStorageUnavailable)
}

fn turn_response_for_store(
    store: &SqliteStore,
    turn: &AgentTurn,
) -> Result<TurnResponse, AgentIpcError> {
    let sources = turn_sources_for(store, &turn.session_id())?;
    Ok(turn_response(turn, &sources))
}

fn fail_with_public_error(
    store: &SqliteStore,
    turn_id: tule_core::AgentTurnId,
    error: PublicError,
) -> Result<AgentTurn, AgentIpcError> {
    tule_core::fail_agent_turn(store, turn_id, public_error_code(error)).map_err(map_finish)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PreStreamRefresh {
    Ready,
    Cancelled,
}

async fn refresh_or_cancel<F>(
    token: &CancellationToken,
    refresh: F,
) -> Result<PreStreamRefresh, PublicError>
where
    F: Future<Output = Result<(), PublicError>>,
{
    tokio::select! {
        biased;
        _ = token.cancelled() => Ok(PreStreamRefresh::Cancelled),
        result = refresh => result.map(|()| PreStreamRefresh::Ready),
    }
}

struct StreamFinalization {
    result: Result<(), PublicError>,
    completed: bool,
    response_id: Option<String>,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    already_terminal: Option<AgentTurn>,
    cancelled: bool,
}

fn finalize_stream_result(
    store: &SqliteStore,
    turn_id: tule_core::AgentTurnId,
    finalization: StreamFinalization,
) -> Result<AgentTurn, AgentIpcError> {
    let StreamFinalization {
        result,
        completed,
        response_id,
        input_tokens,
        output_tokens,
        already_terminal,
        cancelled,
    } = finalization;
    if let Some(turn) = already_terminal {
        return Ok(turn);
    }
    match result {
        Ok(()) if cancelled => tule_core::cancel_agent_turn(store, turn_id).map_err(map_finish),
        Ok(()) if completed => {
            tule_core::complete_agent_turn(store, turn_id, response_id, input_tokens, output_tokens)
                .map_err(map_finish)
        }
        Ok(()) => fail_with_public_error(store, turn_id, PublicError::ProviderUnavailable),
        Err(PublicError::Cancelled) => {
            tule_core::cancel_agent_turn(store, turn_id).map_err(map_finish)
        }
        Err(error) => fail_with_public_error(store, turn_id, error),
    }
}

#[tauri::command]
pub(crate) async fn list_agent_sessions(
    state: State<'_, AgentState>,
) -> Result<Vec<SessionResponse>, AgentIpcError> {
    let store = Arc::clone(&state.store);
    tauri::async_runtime::spawn_blocking(move || {
        store
            .list_sessions()
            .map(|items| items.into_iter().map(SessionResponse::from).collect())
            .map_err(|_| AgentIpcError::AgentStorageUnavailable)
    })
    .await
    .map_err(|_| AgentIpcError::AgentStorageUnavailable)?
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn get_agent_session(
    session_id: String,
    state: State<'_, AgentState>,
) -> Result<SessionDetailResponse, AgentIpcError> {
    let store = Arc::clone(&state.store);
    tauri::async_runtime::spawn_blocking(move || {
        let id = AgentSessionId::parse(&session_id).map_err(|_| AgentIpcError::InvalidInput)?;
        let session = store
            .find_session(&id)
            .map_err(|_| AgentIpcError::AgentStorageUnavailable)?
            .ok_or(AgentIpcError::InvalidInput)?;
        let turns = store
            .list_turns(&id)
            .map_err(|_| AgentIpcError::AgentStorageUnavailable)?;
        let events = store
            .list_events(&id)
            .map_err(|_| AgentIpcError::AgentStorageUnavailable)?;
        let sources = turn_sources_for(store.as_ref(), &id)?;
        Ok(SessionDetailResponse {
            session: session.into(),
            turns: turns
                .iter()
                .map(|turn| turn_response(turn, &sources))
                .collect(),
            events: events.iter().map(EventResponse::from).collect(),
        })
    })
    .await
    .map_err(|_| AgentIpcError::AgentStorageUnavailable)?
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn pick_agent_text_source(
    app: AppHandle,
    webview: Webview,
    state: State<'_, AgentState>,
) -> Result<PickAgentTextSourceResponse, AgentIpcError> {
    require_main_window(&webview)?;
    let _operation = begin_draft_mutation(&state)?;
    let drafts = Arc::clone(&state.source_drafts);
    let picker = AppDialogPicker { app };
    let outcome = tauri::async_runtime::spawn_blocking(move || {
        capture_picked_source(&drafts, &picker, &NativeSourceFileReader)
    })
    .await
    .map_err(|_| AgentIpcError::SourceUnreadable)??;
    Ok(match outcome {
        PickSourceOutcome::Cancelled => PickAgentTextSourceResponse {
            status: "cancelled".into(),
            draft_handle: None,
            display_name: None,
            byte_count: None,
            origin_kind: None,
            member_count: None,
            canonical_url: None,
        },
        PickSourceOutcome::Selected {
            draft_handle,
            display_name,
            byte_count,
            origin_kind,
            member_count,
            canonical_url,
        } => PickAgentTextSourceResponse {
            status: "selected".into(),
            draft_handle: Some(draft_handle),
            display_name: Some(display_name),
            byte_count: Some(byte_count),
            origin_kind: Some(origin_kind),
            member_count: Some(member_count),
            canonical_url,
        },
    })
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn attach_agent_text_link_source(
    webview: Webview,
    url: String,
    state: State<'_, AgentState>,
) -> Result<PickAgentTextSourceResponse, AgentIpcError> {
    require_main_window(&webview)?;
    let _operation = begin_draft_mutation(&state)?;
    let drafts = Arc::clone(&state.source_drafts);
    let fetcher = Arc::clone(&state.source_url_fetcher);
    let trimmed = url.trim().to_owned();
    let outcome = tauri::async_runtime::spawn_blocking(move || {
        capture_link_source(&drafts, &trimmed, fetcher.as_ref())
    })
    .await
    .map_err(|_| AgentIpcError::SourceUnreadable)??;
    Ok(match outcome {
        PickSourceOutcome::Cancelled => PickAgentTextSourceResponse {
            status: "cancelled".into(),
            draft_handle: None,
            display_name: None,
            byte_count: None,
            origin_kind: None,
            member_count: None,
            canonical_url: None,
        },
        PickSourceOutcome::Selected {
            draft_handle,
            display_name,
            byte_count,
            origin_kind,
            member_count,
            canonical_url,
        } => PickAgentTextSourceResponse {
            status: "selected".into(),
            draft_handle: Some(draft_handle),
            display_name: Some(display_name),
            byte_count: Some(byte_count),
            origin_kind: Some(origin_kind),
            member_count: Some(member_count),
            canonical_url,
        },
    })
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn pick_agent_text_folder_source(
    app: AppHandle,
    webview: Webview,
    state: State<'_, AgentState>,
) -> Result<PickAgentTextSourceResponse, AgentIpcError> {
    require_main_window(&webview)?;
    let _operation = begin_draft_mutation(&state)?;
    let drafts = Arc::clone(&state.source_drafts);
    let picker = AppDialogPicker { app };
    let outcome = tauri::async_runtime::spawn_blocking(move || {
        capture_picked_folder(&drafts, &picker, &NativeSourceFolderReader)
    })
    .await
    .map_err(|_| AgentIpcError::SourceUnreadable)??;
    Ok(match outcome {
        PickSourceOutcome::Cancelled => PickAgentTextSourceResponse {
            status: "cancelled".into(),
            draft_handle: None,
            display_name: None,
            byte_count: None,
            origin_kind: None,
            member_count: None,
            canonical_url: None,
        },
        PickSourceOutcome::Selected {
            draft_handle,
            display_name,
            byte_count,
            origin_kind,
            member_count,
            canonical_url,
        } => PickAgentTextSourceResponse {
            status: "selected".into(),
            draft_handle: Some(draft_handle),
            display_name: Some(display_name),
            byte_count: Some(byte_count),
            origin_kind: Some(origin_kind),
            member_count: Some(member_count),
            canonical_url,
        },
    })
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn clear_agent_text_source_draft(
    webview: Webview,
    draft_handle: Option<String>,
    state: State<'_, AgentState>,
) -> Result<(), AgentIpcError> {
    require_main_window(&webview)?;
    clear_agent_text_source_draft_inner(draft_handle.as_deref(), &state)
}

fn begin_draft_mutation(state: &AgentState) -> Result<OperationGuard, AgentIpcError> {
    state.try_operation().map_err(AgentIpcError::from)
}

fn clear_agent_text_source_draft_inner(
    draft_handle: Option<&str>,
    state: &AgentState,
) -> Result<(), AgentIpcError> {
    let _operation = begin_draft_mutation(state)?;
    if let Some(handle) = draft_handle {
        state.source_drafts.clear_handle(handle);
    } else {
        state.source_drafts.clear_all();
    }
    Ok(())
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn set_agent_source_draft_scope(
    webview: Webview,
    session_id: Option<String>,
    state: State<'_, AgentState>,
) -> Result<(), AgentIpcError> {
    require_main_window(&webview)?;
    set_agent_source_draft_scope_inner(session_id.as_deref(), &state)
}

fn set_agent_source_draft_scope_inner(
    session_id: Option<&str>,
    state: &AgentState,
) -> Result<(), AgentIpcError> {
    let _operation = begin_draft_mutation(state)?;
    match session_id {
        None => {
            state.source_drafts.begin_new_session_scope();
            Ok(())
        }
        Some(value) => {
            let id = AgentSessionId::parse(value).map_err(|_| AgentIpcError::InvalidInput)?;
            state.source_drafts.bind_session_scope(id);
            Ok(())
        }
    }
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn get_model_request_controls(
    webview: Webview,
    model_id: String,
) -> Result<ModelRequestControlsResponse, AgentIpcError> {
    require_main_window(&webview)?;
    let model_id =
        tule_core::validate_model_id(&model_id).map_err(|_| AgentIpcError::InvalidInput)?;
    Ok(model_request_controls_response(model_id))
}

#[tauri::command(rename_all = "camelCase")]
#[allow(clippy::too_many_arguments)]
pub(crate) async fn send_agent_message(
    app: AppHandle,
    webview: Webview,
    session_id: Option<String>,
    user_text: String,
    project_id: Option<String>,
    model_id: Option<String>,
    effort: Option<String>,
    source_draft_handle: Option<String>,
    channel: Channel<AgentStreamEvent>,
    state: State<'_, AgentState>,
) -> Result<(), AgentIpcError> {
    require_main_window(&webview)?;
    let _operation = state.try_send_operation().map_err(AgentIpcError::from)?;
    let session_id = session_id
        .map(|value| AgentSessionId::parse(&value).map_err(|_| AgentIpcError::InvalidInput))
        .transpose()?;
    let project_id = project_id
        .map(|value| ProjectId::parse(&value).map_err(|_| AgentIpcError::InvalidInput))
        .transpose()?;
    let store = Arc::clone(&state.store);
    let project_instructions = if let Some(id) = project_id {
        let project = store
            .find_by_id(&id)
            .map_err(|_| AgentIpcError::AgentStorageUnavailable)?
            .ok_or(AgentIpcError::InvalidInput)?;
        project.instructions().to_owned()
    } else {
        String::new()
    };
    let connection_state = state.xai().map_or_else(
        || state.provider.connection_status().state,
        |adapter| adapter.connection_status_with_store(store.as_ref()).state,
    );
    match connection_state {
        crate::provider::ConnectionState::Connected => {}
        crate::provider::ConnectionState::UnavailableInThisBuild => {
            return Err(AgentIpcError::ProviderUnavailable);
        }
        crate::provider::ConnectionState::ReconnectRequired => {
            return Err(AgentIpcError::AuthenticationRequired);
        }
        _ => return Err(AgentIpcError::NotConnected),
    }
    let frozen_model_id = if let Some(id) = session_id {
        // Existing sessions ignore client-supplied model identifiers.
        let session = store
            .find_session(&id)
            .map_err(|_| AgentIpcError::AgentStorageUnavailable)?
            .ok_or(AgentIpcError::InvalidInput)?;
        session.model_id().to_owned()
    } else {
        let requested = model_id.ok_or(AgentIpcError::ModelUnavailable)?;
        crate::provider::validate_new_session_model(store.as_ref(), &requested)
            .map_err(AgentIpcError::from)?
    };
    let (effort_available, resolved_effort) =
        crate::xai_subscription::resolve_effort_for_send(&frozen_model_id, effort.as_deref())
            .map_err(AgentIpcError::from)?;
    let pending_source = if let Some(handle) = source_draft_handle.as_ref() {
        let send_target = state
            .source_drafts
            .send_target_for_session(session_id)
            .ok_or(AgentIpcError::SourceDraftExpired)?;
        let draft = state
            .source_drafts
            .resolve_for_send(handle, &send_target)
            .ok_or(AgentIpcError::SourceDraftExpired)?;
        Some(
            draft
                .into_source()
                .map_err(|_| AgentIpcError::SourceUnsupported)?,
        )
    } else {
        None
    };
    let prepared = tule_core::prepare_agent_send(
        store.as_ref(),
        session_id,
        &user_text,
        project_id,
        &project_instructions,
        crate::provider::PROVIDER_PROFILE_ID,
        &frozen_model_id,
        pending_source.as_ref(),
        resolved_effort,
        effort_available,
    )
    .map_err(map_prepare)?;
    let request_json = match crate::xai_subscription::assemble_chat_completions_request_json(
        &prepared.request_context,
    ) {
        Ok(json) => json,
        Err(tule_core::AgentContextError::ContextLimit { .. }) => {
            let _ = fail_with_public_error(
                store.as_ref(),
                prepared.turn.id(),
                PublicError::ContextLimit,
            );
            return Err(AgentIpcError::from(PublicError::ContextLimit));
        }
        Err(tule_core::AgentContextError::InvalidInput(_)) => {
            let _ = fail_with_public_error(
                store.as_ref(),
                prepared.turn.id(),
                PublicError::InvalidInput,
            );
            return Err(AgentIpcError::InvalidInput);
        }
    };
    if let Some(handle) = source_draft_handle.as_ref() {
        state.source_drafts.clear_handle(handle);
    }
    // Adopt the send's persisted session as the composer scope so an immediate
    // follow-up attachment binds to the same target without a navigation round-trip.
    state
        .source_drafts
        .bind_session_scope(prepared.session.id());
    let turn_id = prepared.turn.id();
    let token = CancellationToken::new();
    *state
        .cancellation
        .lock()
        .map_err(|_| AgentIpcError::AgentStorageUnavailable)? =
        Some((turn_id.to_string(), token.clone()));
    if channel
        .send(AgentStreamEvent::Started {
            session_id: prepared.session.id().to_string(),
            turn_id: turn_id.to_string(),
        })
        .is_err()
    {
        fail_with_public_error(store.as_ref(), turn_id, PublicError::ProviderUnavailable)?;
        return Err(AgentIpcError::ProviderUnavailable);
    }

    if let Some(adapter) = state.xai() {
        match refresh_or_cancel(
            &token,
            adapter.ensure_fresh_access_cancellable_public(store.as_ref(), token.clone()),
        )
        .await
        {
            Ok(PreStreamRefresh::Ready) => {}
            Ok(PreStreamRefresh::Cancelled) => {
                let terminal =
                    tule_core::cancel_agent_turn(store.as_ref(), turn_id).map_err(map_finish)?;
                channel
                    .send(AgentStreamEvent::Terminal {
                        turn: turn_response_for_store(store.as_ref(), &terminal)?,
                    })
                    .map_err(|_| AgentIpcError::ProviderUnavailable)?;
                return Ok(());
            }
            Err(error) => {
                let terminal = fail_with_public_error(store.as_ref(), turn_id, error)?;
                channel
                    .send(AgentStreamEvent::Terminal {
                        turn: turn_response_for_store(store.as_ref(), &terminal)?,
                    })
                    .map_err(|_| AgentIpcError::ProviderUnavailable)?;
                return Ok(());
            }
        }
    }
    if token.is_cancelled() {
        let terminal = tule_core::cancel_agent_turn(store.as_ref(), turn_id).map_err(map_finish)?;
        channel
            .send(AgentStreamEvent::Terminal {
                turn: turn_response_for_store(store.as_ref(), &terminal)?,
            })
            .map_err(|_| AgentIpcError::ProviderUnavailable)?;
        return Ok(());
    }

    let store_for_stream = Arc::clone(&store);
    let channel_for_stream = channel.clone();
    let terminal_meta = Arc::new(Mutex::new((
        false,
        None::<String>,
        None::<u64>,
        None::<u64>,
    )));
    let terminal_meta_cb = Arc::clone(&terminal_meta);
    let output_limit_terminal = Arc::new(Mutex::new(None::<AgentTurn>));
    let output_limit_terminal_cb = Arc::clone(&output_limit_terminal);
    let rejected_model_id = prepared.session.model_id().to_owned();
    let result = state
        .provider
        .stream(
            crate::provider::ProviderRequest {
                session_id: prepared.session.id().to_string(),
                request_json,
            },
            token.clone(),
            Box::new(move |event| {
                match event {
                    ProviderEvent::Delta(text) => {
                        let turn = match tule_core::apply_agent_delta(
                            store_for_stream.as_ref(),
                            turn_id,
                            &text,
                        ) {
                            Ok(turn) => turn,
                            Err(tule_core::ApplyAgentDeltaError::OutputLimit(turn)) => {
                                if let Ok(mut terminal) = output_limit_terminal_cb.lock() {
                                    *terminal = Some(*turn);
                                }
                                return Err(PublicError::OutputLimit);
                            }
                            Err(_) => return Err(PublicError::AgentStorageUnavailable),
                        };
                        channel_for_stream
                            .send(AgentStreamEvent::Delta {
                                turn_id: turn.id().to_string(),
                                text,
                            })
                            .map_err(|_| PublicError::ProviderUnavailable)?;
                    }
                    ProviderEvent::Completed {
                        response_id: id,
                        input_tokens,
                        output_tokens,
                    } => {
                        if let Ok(mut guard) = terminal_meta_cb.lock() {
                            guard.0 = true;
                            if id.is_some() {
                                guard.1 = id;
                            }
                            if input_tokens.is_some() {
                                guard.2 = input_tokens;
                            }
                            if output_tokens.is_some() {
                                guard.3 = output_tokens;
                            }
                        }
                    }
                }
                Ok(())
            }),
        )
        .await;
    let model_rejected = matches!(result, Err(PublicError::ModelUnavailable));
    let (completed, response_id, input_tokens, output_tokens) = terminal_meta
        .lock()
        .map(|guard| (guard.0, guard.1.clone(), guard.2, guard.3))
        .unwrap_or((false, None, None, None));
    let already_terminal = output_limit_terminal
        .lock()
        .map_err(|_| AgentIpcError::AgentStorageUnavailable)?
        .take();
    let terminal = finalize_stream_result(
        store.as_ref(),
        turn_id,
        StreamFinalization {
            result: result.map(|_| ()),
            completed,
            response_id,
            input_tokens,
            output_tokens,
            already_terminal,
            cancelled: token.is_cancelled(),
        },
    )?;
    if model_rejected {
        let adapter = state.xai();
        recover_after_model_rejection(&app, adapter.as_deref(), store.as_ref(), &rejected_model_id)
            .await;
    }
    channel
        .send(AgentStreamEvent::Terminal {
            turn: turn_response_for_store(store.as_ref(), &terminal)?,
        })
        .map_err(|_| AgentIpcError::ProviderUnavailable)?;
    Ok(())
}

async fn recover_after_model_rejection(
    app: &AppHandle,
    adapter: Option<&crate::xai_subscription::XaiSubscriptionAdapter>,
    store: &SqliteStore,
    rejected_model_id: &str,
) {
    let Ok((mut catalog, mut selection)) = apply_model_rejection(store, rejected_model_id) else {
        return;
    };
    if let Some(adapter) = adapter {
        match adapter.refresh_model_catalog(store, true).await {
            Ok(refreshed) => {
                catalog = refreshed;
            }
            Err(_) => {
                if let Ok(stale) = build_stale_catalog_response(store) {
                    catalog = stale;
                }
            }
        }
        if let Ok(next_selection) = build_selection_response(store) {
            selection = next_selection;
        }
    }
    let _ = app.emit(PROVIDER_MODEL_CATALOG_CHANGED_EVENT, &catalog);
    let _ = app.emit(PROVIDER_MODEL_SELECTION_CHANGED_EVENT, &selection);
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn create_artifact_from_turn(
    turn_id: String,
    title: Option<String>,
    kind: Option<String>,
    state: State<'_, AgentState>,
) -> Result<CreateArtifactResponse, AgentIpcError> {
    let store = Arc::clone(&state.store);
    tauri::async_runtime::spawn_blocking(move || {
        let title_override = title.as_deref();
        let kind = kind.as_deref();
        let (artifact, version) = tule_core::create_artifact_from_turn(
            store.as_ref(),
            store.as_ref(),
            &turn_id,
            title_override,
            kind,
        )
        .map_err(map_create_artifact)?;
        Ok(CreateArtifactResponse {
            artifact: ArtifactResponse::from(&artifact),
            version: ArtifactVersionResponse::from(&version),
        })
    })
    .await
    .map_err(|_| AgentIpcError::AgentStorageUnavailable)?
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn list_artifacts(
    session_id: String,
    project_id: Option<String>,
    state: State<'_, AgentState>,
) -> Result<Vec<ArtifactSummaryResponse>, AgentIpcError> {
    let store = Arc::clone(&state.store);
    tauri::async_runtime::spawn_blocking(move || {
        tule_core::list_artifacts_for_session_context(
            store.as_ref(),
            &session_id,
            project_id.as_deref(),
        )
        .map(|items| items.iter().map(ArtifactSummaryResponse::from).collect())
        .map_err(map_list_artifacts)
    })
    .await
    .map_err(|_| AgentIpcError::AgentStorageUnavailable)?
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn get_artifact(
    artifact_id: String,
    state: State<'_, AgentState>,
) -> Result<ArtifactDetailResponse, AgentIpcError> {
    let store = Arc::clone(&state.store);
    tauri::async_runtime::spawn_blocking(move || {
        tule_core::get_artifact(store.as_ref(), &artifact_id)
            .map(|detail| ArtifactDetailResponse::from(&detail))
            .map_err(map_get_artifact)
    })
    .await
    .map_err(|_| AgentIpcError::AgentStorageUnavailable)?
}

/// Loads a completed turn's durable metrics snapshot for clipboard / typed export.
pub(crate) fn export_agent_turn_metrics_inner(
    store: &SqliteStore,
    turn_id: &str,
) -> Result<TurnMetricsExportResponse, AgentIpcError> {
    let id = AgentTurnId::parse(turn_id).map_err(|_| AgentIpcError::InvalidInput)?;
    let turn = store
        .find_turn(&id)
        .map_err(|_| AgentIpcError::AgentStorageUnavailable)?
        .ok_or(AgentIpcError::InvalidInput)?;
    if turn.state() != AgentTurnState::Completed {
        return Err(AgentIpcError::InvalidInput);
    }
    Ok(turn_metrics_export(&turn))
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn export_agent_turn_metrics(
    turn_id: String,
    state: State<'_, AgentState>,
) -> Result<TurnMetricsExportResponse, AgentIpcError> {
    let store = Arc::clone(&state.store);
    tauri::async_runtime::spawn_blocking(move || {
        export_agent_turn_metrics_inner(store.as_ref(), &turn_id)
    })
    .await
    .map_err(|_| AgentIpcError::AgentStorageUnavailable)?
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn cancel_agent_turn(
    turn_id: String,
    state: State<'_, AgentState>,
) -> Result<(), AgentIpcError> {
    let guard = state
        .cancellation
        .lock()
        .map_err(|_| AgentIpcError::AgentStorageUnavailable)?;
    if let Some((active, token)) = guard.as_ref()
        && active == &turn_id
    {
        token.cancel();
        return Ok(());
    }
    Err(AgentIpcError::InvalidInput)
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn set_agent_session_project(
    session_id: String,
    project_id: Option<String>,
    state: State<'_, AgentState>,
) -> Result<SessionResponse, AgentIpcError> {
    set_agent_session_project_inner(session_id, project_id, &state).await
}

async fn set_agent_session_project_inner(
    session_id: String,
    project_id: Option<String>,
    state: &AgentState,
) -> Result<SessionResponse, AgentIpcError> {
    let _operation = state.try_operation().map_err(AgentIpcError::from)?;
    let session_id = AgentSessionId::parse(&session_id).map_err(|_| AgentIpcError::InvalidInput)?;
    let project_id = project_id
        .map(|value| ProjectId::parse(&value).map_err(|_| AgentIpcError::InvalidInput))
        .transpose()?;
    let store = Arc::clone(&state.store);
    tauri::async_runtime::spawn_blocking(move || {
        tule_core::set_session_project(store.as_ref(), session_id, project_id)
            .map(SessionResponse::from)
            .map_err(|_| AgentIpcError::AgentStorageUnavailable)
    })
    .await
    .map_err(|_| AgentIpcError::AgentStorageUnavailable)?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{ConnectionState, ConnectionStatus, FakeProvider};
    use tule_core::{AgentEventKind, AgentRepository, AgentTurnState, MAX_AGENT_OUTPUT_UTF8};

    fn test_store() -> (tempfile::TempDir, Arc<SqliteStore>) {
        let directory = tempfile::tempdir().unwrap();
        let store = Arc::new(SqliteStore::open(directory.path().join("tule.sqlite3")).unwrap());
        (directory, store)
    }

    fn test_state() -> (tempfile::TempDir, AgentState) {
        let (directory, store) = test_store();
        let provider = Arc::new(FakeProvider::new(
            ConnectionStatus {
                state: ConnectionState::Connected,
                provider_id: "fake",
                model: "gpt-5.5",
            },
            Ok(Vec::new()),
        ));
        (directory, AgentState::new(store, provider, None))
    }

    #[test]
    fn operation_gate_is_try_locked_and_send_drop_clears_cancellation() {
        let (directory, state) = test_state();
        let first = state.try_send_operation().unwrap();
        assert!(matches!(
            state.try_operation(),
            Err(PublicError::SessionBusy)
        ));
        *state.cancellation.lock().unwrap() = Some(("turn".to_owned(), CancellationToken::new()));

        drop(first);

        assert!(state.cancellation.lock().unwrap().is_none());
        assert!(state.try_operation().is_ok());
        drop(state);
        drop(directory);
    }

    #[test]
    fn accumulated_output_limit_reuses_the_existing_terminal_turn() {
        let (directory, store) = test_store();
        let prepared = tule_core::prepare_agent_send(
            store.as_ref(),
            None,
            "Hello",
            None,
            "",
            crate::provider::PROVIDER_PROFILE_ID,
            "gpt-5.5",
            None,
            None,
            false,
        )
        .unwrap();
        tule_core::apply_agent_delta(
            store.as_ref(),
            prepared.turn.id(),
            &"a".repeat(MAX_AGENT_OUTPUT_UTF8),
        )
        .unwrap();
        let already_terminal =
            match tule_core::apply_agent_delta(store.as_ref(), prepared.turn.id(), "b") {
                Err(tule_core::ApplyAgentDeltaError::OutputLimit(turn)) => Some(*turn),
                other => panic!("expected output limit, got {other:?}"),
            };

        let terminal = finalize_stream_result(
            store.as_ref(),
            prepared.turn.id(),
            StreamFinalization {
                result: Err(PublicError::OutputLimit),
                completed: false,
                response_id: None,
                input_tokens: None,
                output_tokens: None,
                already_terminal,
                cancelled: false,
            },
        )
        .unwrap();

        assert_eq!(terminal.state(), AgentTurnState::Failed);
        assert_eq!(terminal.error_code(), Some("output_limit"));
        assert_eq!(
            store
                .list_events(&prepared.session.id())
                .unwrap()
                .iter()
                .filter(|event| event.kind() == AgentEventKind::TurnFailed)
                .count(),
            1
        );
        assert!(
            tule_core::prepare_agent_send(
                store.as_ref(),
                Some(prepared.session.id()),
                "Try again",
                None,
                "",
                crate::provider::PROVIDER_PROFILE_ID,
                "gpt-5.5",
                None,
                None,
                false,
            )
            .is_ok()
        );
        drop(store);
        drop(directory);
    }

    #[test]
    fn pre_stream_failure_terminalizes_pending_turn_once() {
        let (directory, store) = test_store();
        let prepared = tule_core::prepare_agent_send(
            store.as_ref(),
            None,
            "Hello",
            None,
            "",
            crate::provider::PROVIDER_PROFILE_ID,
            "gpt-5.5",
            None,
            None,
            false,
        )
        .unwrap();

        let terminal = fail_with_public_error(
            store.as_ref(),
            prepared.turn.id(),
            PublicError::ProviderUnavailable,
        )
        .unwrap();

        assert_eq!(terminal.state(), AgentTurnState::Failed);
        assert_eq!(terminal.error_code(), Some("provider_unavailable"));
        assert_eq!(
            store
                .list_events(&prepared.session.id())
                .unwrap()
                .iter()
                .filter(|event| event.kind() == AgentEventKind::TurnFailed)
                .count(),
            1
        );
        drop(store);
        drop(directory);
    }

    #[tokio::test]
    async fn project_association_change_uses_the_application_operation_gate() {
        let (directory, state) = test_state();
        let prepared = tule_core::prepare_agent_send(
            state.store.as_ref(),
            None,
            "Hello",
            None,
            "",
            crate::provider::PROVIDER_PROFILE_ID,
            "gpt-5.5",
            None,
            None,
            false,
        )
        .unwrap();
        tule_core::complete_agent_turn(state.store.as_ref(), prepared.turn.id(), None, None, None)
            .unwrap();
        let project = tule_core::create_project(state.store.as_ref(), "Context").unwrap();
        let held = state.try_operation().unwrap();

        let blocked = set_agent_session_project_inner(
            prepared.session.id().to_string(),
            Some(project.id().to_string()),
            &state,
        )
        .await;

        assert!(matches!(blocked, Err(AgentIpcError::SessionBusy)));
        assert_eq!(
            state
                .store
                .find_session(&prepared.session.id())
                .unwrap()
                .unwrap()
                .project_id(),
            None
        );
        drop(held);

        let changed = set_agent_session_project_inner(
            prepared.session.id().to_string(),
            Some(project.id().to_string()),
            &state,
        )
        .await
        .unwrap();
        assert_eq!(changed.project_id, Some(project.id().to_string()));
        drop(state);
        drop(directory);
    }

    #[test]
    fn send_adopts_persisted_session_scope_for_immediate_follow_up_attachment() {
        use crate::source_draft::ComposerScope;

        let (directory, state) = test_state();
        let first_handle = state
            .source_drafts
            .insert(
                "first.txt".into(),
                "first-body".into(),
                tule_core::SOURCE_ORIGIN_LOCAL_TEXT_FILE.into(),
                1,
                None,
            )
            .unwrap();
        let new_session_scope = state.source_drafts.current_scope();
        assert!(matches!(
            new_session_scope,
            ComposerScope::NewSession { .. }
        ));
        let draft = state
            .source_drafts
            .resolve_for_send(&first_handle, &new_session_scope)
            .unwrap();
        let source = draft.into_source().unwrap();
        let prepared = tule_core::prepare_agent_send(
            state.store.as_ref(),
            None,
            "First turn",
            None,
            "",
            crate::provider::PROVIDER_PROFILE_ID,
            "gpt-5.5",
            Some(&source),
            None,
            false,
        )
        .unwrap();
        state.source_drafts.clear_handle(&first_handle);
        state
            .source_drafts
            .bind_session_scope(prepared.session.id());

        let follow_handle = state
            .source_drafts
            .insert(
                "follow.txt".into(),
                "follow-body".into(),
                tule_core::SOURCE_ORIGIN_LOCAL_TEXT_FILE.into(),
                1,
                None,
            )
            .unwrap();
        assert!(
            state
                .source_drafts
                .resolve_for_send(
                    &follow_handle,
                    &ComposerScope::Session(prepared.session.id())
                )
                .is_some()
        );
        assert!(
            state
                .source_drafts
                .resolve_for_send(&follow_handle, &new_session_scope)
                .is_none()
        );
        let other = AgentSessionId::generate();
        assert!(
            state
                .source_drafts
                .resolve_for_send(&follow_handle, &ComposerScope::Session(other))
                .is_none()
        );
        drop(state);
        drop(directory);
    }

    #[test]
    fn draft_mutations_are_gated_while_send_holds_the_operation() {
        let (directory, state) = test_state();
        let handle = state
            .source_drafts
            .insert(
                "notes.txt".into(),
                "original-body".into(),
                tule_core::SOURCE_ORIGIN_LOCAL_TEXT_FILE.into(),
                1,
                None,
            )
            .unwrap();
        let send_target = state.source_drafts.current_scope();
        let held = state.try_send_operation().unwrap();

        assert!(matches!(
            begin_draft_mutation(&state),
            Err(AgentIpcError::SessionBusy)
        ));
        assert!(matches!(
            clear_agent_text_source_draft_inner(Some(&handle), &state),
            Err(AgentIpcError::SessionBusy)
        ));
        assert!(matches!(
            clear_agent_text_source_draft_inner(None, &state),
            Err(AgentIpcError::SessionBusy)
        ));
        assert!(matches!(
            set_agent_source_draft_scope_inner(None, &state),
            Err(AgentIpcError::SessionBusy)
        ));
        assert!(matches!(
            set_agent_source_draft_scope_inner(Some("not-a-uuid"), &state),
            Err(AgentIpcError::SessionBusy)
        ));

        let draft = state.source_drafts.get(&handle).unwrap();
        assert_eq!(draft.content, "original-body");
        assert_eq!(draft.display_name, "notes.txt");
        assert_eq!(
            state
                .source_drafts
                .resolve_for_send(&handle, &send_target)
                .unwrap()
                .content,
            "original-body"
        );
        drop(held);
        drop(state);
        drop(directory);
    }

    #[test]
    fn session_detail_events_are_ordered_and_include_cancelled_terminal_kind() {
        let (directory, store) = test_store();
        let prepared = tule_core::prepare_agent_send(
            store.as_ref(),
            None,
            "Hello",
            None,
            "",
            crate::provider::PROVIDER_PROFILE_ID,
            "gpt-5.5",
            None,
            None,
            false,
        )
        .unwrap();
        tule_core::cancel_agent_turn(store.as_ref(), prepared.turn.id()).unwrap();

        let events = store.list_events(&prepared.session.id()).unwrap();
        let mapped: Vec<EventResponse> = events.iter().map(EventResponse::from).collect();

        assert_eq!(mapped.len(), 3);
        assert_eq!(mapped[0].kind, "session_created");
        assert_eq!(mapped[1].kind, "turn_pending");
        assert_eq!(mapped[2].kind, "turn_cancelled");
        assert_eq!(mapped[0].session_id, prepared.session.id().to_string());
        assert_eq!(
            mapped[1].turn_id.as_deref(),
            Some(prepared.turn.id().to_string().as_str())
        );
        assert!(mapped[0].sequence < mapped[1].sequence);
        assert!(mapped[1].sequence < mapped[2].sequence);
        drop(store);
        drop(directory);
    }

    #[test]
    fn session_detail_events_include_interrupted_terminal_kind_after_recovery() {
        let (directory, store) = test_store();
        let prepared = tule_core::prepare_agent_send(
            store.as_ref(),
            None,
            "Hello",
            None,
            "",
            crate::provider::PROVIDER_PROFILE_ID,
            "gpt-5.5",
            None,
            None,
            false,
        )
        .unwrap();
        let interrupted = tule_core::interrupt_inflight_turns(store.as_ref()).unwrap();

        assert_eq!(interrupted.len(), 1);
        let mapped: Vec<EventResponse> = store
            .list_events(&prepared.session.id())
            .unwrap()
            .iter()
            .map(EventResponse::from)
            .collect();
        assert!(mapped.iter().any(|event| event.kind == "turn_interrupted"));
        drop(store);
        drop(directory);
    }

    #[test]
    fn artifact_dto_mapping_exposes_allowlisted_fields_only() {
        let (directory, store) = test_store();
        let prepared = tule_core::prepare_agent_send(
            store.as_ref(),
            None,
            "Hello",
            None,
            "",
            crate::provider::PROVIDER_PROFILE_ID,
            "gpt-5.5",
            None,
            None,
            false,
        )
        .unwrap();
        tule_core::apply_agent_delta(store.as_ref(), prepared.turn.id(), "Saved body").unwrap();
        let turn =
            tule_core::complete_agent_turn(store.as_ref(), prepared.turn.id(), None, None, None)
                .unwrap();
        let (artifact, version) = tule_core::create_artifact_from_turn(
            store.as_ref(),
            store.as_ref(),
            &turn.id().to_string(),
            None,
            Some("critique"),
        )
        .unwrap();

        let create = CreateArtifactResponse {
            artifact: ArtifactResponse::from(&artifact),
            version: ArtifactVersionResponse::from(&version),
        };
        assert_eq!(create.artifact.kind, "critique");
        assert_eq!(create.version.content, "Saved body");
        assert_eq!(
            create.version.provenance.source_turn_id,
            turn.id().to_string()
        );
        assert_eq!(
            create.version.provenance.provider_request_id,
            turn.provider_request_id().to_string()
        );

        let listed = tule_core::list_artifacts_for_session_context(
            store.as_ref(),
            &turn.session_id().to_string(),
            None,
        )
        .unwrap();
        let summary = ArtifactSummaryResponse::from(&listed[0]);
        assert_eq!(summary.id, artifact.id().to_string());
        assert_eq!(summary.latest_version_id, version.id().to_string());
        assert_eq!(summary.latest_version_ordinal, 1);

        let detail = ArtifactDetailResponse::from(
            &tule_core::get_artifact(store.as_ref(), &artifact.id().to_string()).unwrap(),
        );
        assert_eq!(detail.versions.len(), 1);
        assert_eq!(detail.versions[0].content_sha256, version.content_sha256());
        drop(store);
        drop(directory);
    }

    #[test]
    fn create_artifact_reject_paths_map_to_invalid_input() {
        assert!(matches!(
            map_create_artifact(tule_core::CreateArtifactFromTurnError::TurnNotCompleted),
            AgentIpcError::InvalidInput
        ));
        assert!(matches!(
            map_create_artifact(tule_core::CreateArtifactFromTurnError::EmptyAgentText),
            AgentIpcError::InvalidInput
        ));
        assert!(matches!(
            map_create_artifact(tule_core::CreateArtifactFromTurnError::InvalidKind(
                tule_core::InvalidArtifactKind
            )),
            AgentIpcError::InvalidInput
        ));
        assert!(matches!(
            map_get_artifact(tule_core::GetArtifactError::NotFound),
            AgentIpcError::InvalidInput
        ));
    }

    #[test]
    fn turn_response_exposes_timing_and_nullable_usage_from_durable_state() {
        let (directory, store) = test_store();
        let prepared = tule_core::prepare_agent_send(
            store.as_ref(),
            None,
            "Hello",
            None,
            "",
            crate::provider::PROVIDER_PROFILE_ID,
            "gpt-5.5",
            None,
            None,
            false,
        )
        .unwrap();
        let pending = turn_response_for_store(store.as_ref(), &prepared.turn).unwrap();
        assert_eq!(
            pending.started_at_unix_ms,
            prepared.turn.started_at_unix_ms()
        );
        assert_eq!(pending.finished_at_unix_ms, None);
        assert_eq!(pending.usage_input_tokens, None);
        assert_eq!(pending.usage_output_tokens, None);

        tule_core::apply_agent_delta(store.as_ref(), prepared.turn.id(), "Hi").unwrap();
        let completed = tule_core::complete_agent_turn(
            store.as_ref(),
            prepared.turn.id(),
            Some("resp-1".into()),
            Some(12),
            Some(34),
        )
        .unwrap();
        let response = turn_response_for_store(store.as_ref(), &completed).unwrap();
        assert_eq!(response.started_at_unix_ms, completed.started_at_unix_ms());
        assert_eq!(
            response.finished_at_unix_ms,
            completed.finished_at_unix_ms()
        );
        assert_eq!(response.usage_input_tokens, Some(12));
        assert_eq!(response.usage_output_tokens, Some(34));

        let without_usage = tule_core::prepare_agent_send(
            store.as_ref(),
            Some(prepared.session.id()),
            "Again",
            None,
            "",
            crate::provider::PROVIDER_PROFILE_ID,
            "gpt-5.5",
            None,
            None,
            false,
        )
        .unwrap();
        tule_core::apply_agent_delta(store.as_ref(), without_usage.turn.id(), "Ok").unwrap();
        let completed_null_tokens = tule_core::complete_agent_turn(
            store.as_ref(),
            without_usage.turn.id(),
            None,
            None,
            None,
        )
        .unwrap();
        let null_usage = turn_response_for_store(store.as_ref(), &completed_null_tokens).unwrap();
        assert_eq!(null_usage.usage_input_tokens, None);
        assert_eq!(null_usage.usage_output_tokens, None);
        assert!(null_usage.finished_at_unix_ms.is_some());
        drop(store);
        drop(directory);
    }

    #[test]
    fn non_null_usage_survives_sqlite_reopen_export_with_one_completion_event() {
        let directory = tempfile::tempdir().unwrap();
        let database_path = directory.path().join("tule.sqlite3");
        let store = SqliteStore::open(&database_path).unwrap();
        let prepared = tule_core::prepare_agent_send(
            &store,
            None,
            "Hello",
            None,
            "",
            crate::provider::PROVIDER_PROFILE_ID,
            "gpt-5.5",
            None,
            None,
            false,
        )
        .unwrap();
        let turn_id = prepared.turn.id();
        let session_id = prepared.session.id();
        tule_core::apply_agent_delta(&store, turn_id, "Hi").unwrap();
        tule_core::complete_agent_turn(
            &store,
            turn_id,
            Some("resp-usage".into()),
            Some(12),
            Some(34),
        )
        .unwrap();
        drop(store);

        let reopened = SqliteStore::open(database_path).unwrap();
        let turn = reopened.find_turn(&turn_id).unwrap().unwrap();
        let response = turn_response_for_store(&reopened, &turn).unwrap();
        assert_eq!(response.usage_input_tokens, Some(12));
        assert_eq!(response.usage_output_tokens, Some(34));

        let snapshot = export_agent_turn_metrics_inner(&reopened, &turn_id.to_string()).unwrap();
        assert_eq!(snapshot.usage_input_tokens, Some(12));
        assert_eq!(snapshot.usage_output_tokens, Some(34));

        let events = reopened.list_events(&session_id).unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|event| event.kind() == AgentEventKind::TurnCompleted)
                .count(),
            1
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| {
                    matches!(
                        event.kind(),
                        AgentEventKind::TurnCompleted
                            | AgentEventKind::TurnCancelled
                            | AgentEventKind::TurnFailed
                            | AgentEventKind::TurnInterrupted
                    )
                })
                .count(),
            1
        );
    }

    #[test]
    fn export_agent_turn_metrics_returns_durable_snapshot_and_rejects_unknown_turns() {
        let (directory, store) = test_store();
        let prepared = tule_core::prepare_agent_send(
            store.as_ref(),
            None,
            "Hello",
            None,
            "",
            crate::provider::PROVIDER_PROFILE_ID,
            "gpt-5.5",
            None,
            None,
            false,
        )
        .unwrap();
        assert!(matches!(
            export_agent_turn_metrics_inner(store.as_ref(), &prepared.turn.id().to_string()),
            Err(AgentIpcError::InvalidInput)
        ));

        tule_core::apply_agent_delta(store.as_ref(), prepared.turn.id(), "Hi").unwrap();
        let completed = tule_core::complete_agent_turn(
            store.as_ref(),
            prepared.turn.id(),
            Some("resp-1".into()),
            Some(100),
            None,
        )
        .unwrap();
        let snapshot =
            export_agent_turn_metrics_inner(store.as_ref(), &completed.id().to_string()).unwrap();
        assert_eq!(snapshot.turn_id, completed.id().to_string());
        assert_eq!(snapshot.session_id, completed.session_id().to_string());
        assert_eq!(snapshot.ordinal, completed.ordinal());
        assert_eq!(snapshot.state, "completed");
        assert_eq!(
            snapshot.provider_profile_id,
            completed.provider_profile_id()
        );
        assert_eq!(snapshot.model_id, completed.model_id());
        assert_eq!(snapshot.effort, None);
        assert_eq!(snapshot.started_at_unix_ms, completed.started_at_unix_ms());
        assert_eq!(
            snapshot.finished_at_unix_ms,
            completed.finished_at_unix_ms()
        );
        assert_eq!(
            snapshot.duration_ms,
            completed
                .finished_at_unix_ms()
                .map(|finished| finished.saturating_sub(completed.started_at_unix_ms()))
        );
        assert_eq!(snapshot.usage_input_tokens, Some(100));
        assert_eq!(snapshot.usage_output_tokens, None);

        assert!(matches!(
            export_agent_turn_metrics_inner(store.as_ref(), "01900000-0000-7000-8000-000000000099"),
            Err(AgentIpcError::InvalidInput)
        ));
        assert!(matches!(
            export_agent_turn_metrics_inner(store.as_ref(), "not-a-turn-id"),
            Err(AgentIpcError::InvalidInput)
        ));
        drop(store);
        drop(directory);
    }

    #[tokio::test]
    async fn cancellation_wins_while_pre_stream_refresh_is_pending() {
        let (directory, store) = test_store();
        let prepared = tule_core::prepare_agent_send(
            store.as_ref(),
            None,
            "Hello",
            None,
            "",
            crate::provider::PROVIDER_PROFILE_ID,
            "gpt-5.5",
            None,
            None,
            false,
        )
        .unwrap();
        let token = CancellationToken::new();
        let cancel = token.clone();
        let outcome = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            refresh_or_cancel(&token, async move {
                cancel.cancel();
                std::future::pending::<Result<(), PublicError>>().await
            }),
        )
        .await
        .expect("cancellation should not wait for refresh")
        .unwrap();

        assert_eq!(outcome, PreStreamRefresh::Cancelled);
        let terminal = tule_core::cancel_agent_turn(store.as_ref(), prepared.turn.id()).unwrap();
        assert_eq!(terminal.state(), AgentTurnState::Cancelled);
        assert_eq!(
            store
                .list_events(&prepared.session.id())
                .unwrap()
                .iter()
                .filter(|event| event.kind() == AgentEventKind::TurnCancelled)
                .count(),
            1
        );
        drop(store);
        drop(directory);
    }
}
