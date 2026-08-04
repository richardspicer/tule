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
    AgentRepository, AgentSession, AgentSessionId, AgentTurn, ProjectId, ProjectRepository, Source,
    TurnSource,
};

use crate::{
    provider::{
        PROVIDER_MODEL_CATALOG_CHANGED_EVENT, PROVIDER_MODEL_SELECTION_CHANGED_EVENT,
        ProviderAdapter, ProviderEvent, PublicError, apply_model_rejection,
        build_selection_response, build_stale_catalog_response,
    },
    source_draft::{
        NativeSourceFileReader, PickSourceOutcome, SourceDraftError, SourceDraftStore,
        SourceFilePicker, capture_picked_source,
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
    pub(crate) chatgpt: Option<Arc<crate::openai_chatgpt::ChatGptAdapter>>,
    pub(crate) source_drafts: Arc<SourceDraftStore>,
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
        chatgpt: Option<Arc<crate::openai_chatgpt::ChatGptAdapter>>,
    ) -> Self {
        Self {
            store,
            provider,
            chatgpt,
            source_drafts: Arc::new(SourceDraftStore::new()),
            operation_gate: Arc::new(OperationGate::default()),
            cancellation: Arc::new(Mutex::new(None)),
        }
    }

    pub(crate) fn clear_source_drafts(&self) {
        self.source_drafts.clear_all();
    }

    pub(crate) fn chatgpt(&self) -> Option<Arc<crate::openai_chatgpt::ChatGptAdapter>> {
        self.chatgpt.clone()
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
}

impl From<&Source> for SourceMetadataResponse {
    fn from(value: &Source) -> Self {
        Self {
            id: value.id().to_string(),
            origin_kind: value.origin_kind().into(),
            display_name: value.display_name().into(),
            byte_count: value.byte_count(),
            content_sha256: value.content_sha256().into(),
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
    sources: Vec<SourceMetadataResponse>,
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
        sources: turn_sources
            .into_iter()
            .map(|item| SourceMetadataResponse::from(item.source()))
            .collect(),
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SessionDetailResponse {
    session: SessionResponse,
    turns: Vec<TurnResponse>,
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
        tule_core::PrepareAgentSendError::ModelUnavailable(_) => AgentIpcError::ModelUnavailable,
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
        let sources = turn_sources_for(store.as_ref(), &id)?;
        Ok(SessionDetailResponse {
            session: session.into(),
            turns: turns
                .iter()
                .map(|turn| turn_response(turn, &sources))
                .collect(),
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
        },
        PickSourceOutcome::Selected {
            draft_handle,
            display_name,
            byte_count,
            origin_kind,
        } => PickAgentTextSourceResponse {
            status: "selected".into(),
            draft_handle: Some(draft_handle),
            display_name: Some(display_name),
            byte_count: Some(byte_count),
            origin_kind: Some(origin_kind),
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
    if let Some(handle) = draft_handle {
        state.source_drafts.clear_handle(&handle);
    } else {
        state.source_drafts.clear_all();
    }
    Ok(())
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn set_agent_source_draft_scope(
    webview: Webview,
    scope_key: String,
    state: State<'_, AgentState>,
) -> Result<(), AgentIpcError> {
    require_main_window(&webview)?;
    state.source_drafts.set_scope(scope_key);
    Ok(())
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
    let connection_state = state.chatgpt().map_or_else(
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
    let frozen_model_id = if session_id.is_none() {
        let requested = model_id.ok_or(AgentIpcError::ModelUnavailable)?;
        crate::provider::validate_new_session_model(store.as_ref(), &requested)
            .map_err(AgentIpcError::from)?
    } else {
        // Existing sessions ignore client-supplied model identifiers.
        String::new()
    };
    let pending_source = if let Some(handle) = source_draft_handle.as_ref() {
        let draft = state
            .source_drafts
            .get(handle)
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
        &frozen_model_id,
        pending_source.as_ref(),
    )
    .map_err(map_prepare)?;
    if let Some(handle) = source_draft_handle.as_ref() {
        state.source_drafts.clear_handle(handle);
    }
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

    if let Some(adapter) = state.chatgpt() {
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
                request_json: prepared.request_json,
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
        let adapter = state.chatgpt();
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
    adapter: Option<&crate::openai_chatgpt::ChatGptAdapter>,
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
        let prepared =
            tule_core::prepare_agent_send(store.as_ref(), None, "Hello", None, "", "gpt-5.5", None)
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
                "gpt-5.5",
                None,
            )
            .is_ok()
        );
        drop(store);
        drop(directory);
    }

    #[test]
    fn pre_stream_failure_terminalizes_pending_turn_once() {
        let (directory, store) = test_store();
        let prepared =
            tule_core::prepare_agent_send(store.as_ref(), None, "Hello", None, "", "gpt-5.5", None)
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
            "gpt-5.5",
            None,
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

    #[tokio::test]
    async fn cancellation_wins_while_pre_stream_refresh_is_pending() {
        let (directory, store) = test_store();
        let prepared =
            tule_core::prepare_agent_send(store.as_ref(), None, "Hello", None, "", "gpt-5.5", None)
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
