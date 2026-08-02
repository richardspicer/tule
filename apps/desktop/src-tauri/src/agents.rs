//! Narrow typed Agent IPC commands and ordered stream channel messages.

use std::{
    future::Future,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use serde::Serialize;
use tauri::{State, ipc::Channel};
use tokio_util::sync::CancellationToken;
use tule_core::{
    AgentRepository, AgentSession, AgentSessionId, AgentTurn, ProjectId, ProjectRepository,
};

use crate::{
    provider::{ProviderAdapter, ProviderEvent, PublicError},
    sqlite::SqliteStore,
};

pub(crate) struct AgentState {
    pub(crate) store: Arc<SqliteStore>,
    pub(crate) provider: Arc<dyn ProviderAdapter>,
    pub(crate) chatgpt: Option<Arc<crate::openai_chatgpt::ChatGptAdapter>>,
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
            operation_gate: Arc::new(OperationGate::default()),
            cancellation: Arc::new(Mutex::new(None)),
        }
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
pub(crate) struct TurnResponse {
    id: String,
    ordinal: u64,
    user_text: String,
    agent_text: String,
    state: String,
    error_code: Option<String>,
}
impl From<AgentTurn> for TurnResponse {
    fn from(value: AgentTurn) -> Self {
        Self {
            id: value.id().to_string(),
            ordinal: value.ordinal(),
            user_text: value.user_text().into(),
            agent_text: value.agent_text().into(),
            state: value.state().as_str().into(),
            error_code: value.error_code().map(str::to_owned),
        }
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
pub(crate) struct SessionDetailResponse {
    session: SessionResponse,
    turns: Vec<TurnResponse>,
}

fn map_prepare(error: tule_core::PrepareAgentSendError) -> PublicError {
    match error {
        tule_core::PrepareAgentSendError::InvalidInput(_) => PublicError::InvalidInput,
        tule_core::PrepareAgentSendError::ContextLimit { .. } => PublicError::ContextLimit,
        tule_core::PrepareAgentSendError::SessionBusy => PublicError::SessionBusy,
        tule_core::PrepareAgentSendError::SessionNotFound => PublicError::InvalidInput,
        tule_core::PrepareAgentSendError::ProjectAssociationMismatch => PublicError::InvalidInput,
        tule_core::PrepareAgentSendError::Time(_)
        | tule_core::PrepareAgentSendError::Repository(_) => PublicError::AgentStorageUnavailable,
    }
}

fn map_finish(_: tule_core::FinishAgentTurnError) -> PublicError {
    PublicError::AgentStorageUnavailable
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
    }
}

fn fail_with_public_error(
    store: &SqliteStore,
    turn_id: tule_core::AgentTurnId,
    error: PublicError,
) -> Result<AgentTurn, PublicError> {
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
) -> Result<AgentTurn, PublicError> {
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
) -> Result<Vec<SessionResponse>, PublicError> {
    let store = Arc::clone(&state.store);
    tauri::async_runtime::spawn_blocking(move || {
        store
            .list_sessions()
            .map(|items| items.into_iter().map(SessionResponse::from).collect())
            .map_err(|_| PublicError::AgentStorageUnavailable)
    })
    .await
    .map_err(|_| PublicError::AgentStorageUnavailable)?
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn get_agent_session(
    session_id: String,
    state: State<'_, AgentState>,
) -> Result<SessionDetailResponse, PublicError> {
    let store = Arc::clone(&state.store);
    tauri::async_runtime::spawn_blocking(move || {
        let id = AgentSessionId::parse(&session_id).map_err(|_| PublicError::InvalidInput)?;
        let session = store
            .find_session(&id)
            .map_err(|_| PublicError::AgentStorageUnavailable)?
            .ok_or(PublicError::InvalidInput)?;
        let turns = store
            .list_turns(&id)
            .map_err(|_| PublicError::AgentStorageUnavailable)?;
        Ok(SessionDetailResponse {
            session: session.into(),
            turns: turns.into_iter().map(TurnResponse::from).collect(),
        })
    })
    .await
    .map_err(|_| PublicError::AgentStorageUnavailable)?
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn send_agent_message(
    session_id: Option<String>,
    user_text: String,
    project_id: Option<String>,
    channel: Channel<AgentStreamEvent>,
    state: State<'_, AgentState>,
) -> Result<(), PublicError> {
    let _operation = state.try_send_operation()?;
    let session_id = session_id
        .map(|value| AgentSessionId::parse(&value).map_err(|_| PublicError::InvalidInput))
        .transpose()?;
    let project_id = project_id
        .map(|value| ProjectId::parse(&value).map_err(|_| PublicError::InvalidInput))
        .transpose()?;
    let store = Arc::clone(&state.store);
    let project_instructions = if let Some(id) = project_id {
        let project = store
            .find_by_id(&id)
            .map_err(|_| PublicError::AgentStorageUnavailable)?
            .ok_or(PublicError::InvalidInput)?;
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
            return Err(PublicError::ProviderUnavailable);
        }
        crate::provider::ConnectionState::ReconnectRequired => {
            return Err(PublicError::AuthenticationRequired);
        }
        _ => return Err(PublicError::NotConnected),
    }
    let prepared = tule_core::prepare_agent_send(
        store.as_ref(),
        session_id,
        &user_text,
        project_id,
        &project_instructions,
    )
    .map_err(map_prepare)?;
    let turn_id = prepared.turn.id();
    let token = CancellationToken::new();
    *state
        .cancellation
        .lock()
        .map_err(|_| PublicError::AgentStorageUnavailable)? =
        Some((turn_id.to_string(), token.clone()));
    if channel
        .send(AgentStreamEvent::Started {
            session_id: prepared.session.id().to_string(),
            turn_id: turn_id.to_string(),
        })
        .is_err()
    {
        fail_with_public_error(store.as_ref(), turn_id, PublicError::ProviderUnavailable)?;
        return Err(PublicError::ProviderUnavailable);
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
                        turn: terminal.into(),
                    })
                    .map_err(|_| PublicError::ProviderUnavailable)?;
                return Ok(());
            }
            Err(error) => {
                let terminal = fail_with_public_error(store.as_ref(), turn_id, error)?;
                channel
                    .send(AgentStreamEvent::Terminal {
                        turn: terminal.into(),
                    })
                    .map_err(|_| PublicError::ProviderUnavailable)?;
                return Ok(());
            }
        }
    }
    if token.is_cancelled() {
        let terminal = tule_core::cancel_agent_turn(store.as_ref(), turn_id).map_err(map_finish)?;
        channel
            .send(AgentStreamEvent::Terminal {
                turn: terminal.into(),
            })
            .map_err(|_| PublicError::ProviderUnavailable)?;
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
    let (completed, response_id, input_tokens, output_tokens) = terminal_meta
        .lock()
        .map(|guard| (guard.0, guard.1.clone(), guard.2, guard.3))
        .unwrap_or((false, None, None, None));
    let already_terminal = output_limit_terminal
        .lock()
        .map_err(|_| PublicError::AgentStorageUnavailable)?
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
    channel
        .send(AgentStreamEvent::Terminal {
            turn: terminal.into(),
        })
        .map_err(|_| PublicError::ProviderUnavailable)?;
    Ok(())
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn cancel_agent_turn(
    turn_id: String,
    state: State<'_, AgentState>,
) -> Result<(), PublicError> {
    let guard = state
        .cancellation
        .lock()
        .map_err(|_| PublicError::AgentStorageUnavailable)?;
    if let Some((active, token)) = guard.as_ref()
        && active == &turn_id
    {
        token.cancel();
        return Ok(());
    }
    Err(PublicError::InvalidInput)
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn set_agent_session_project(
    session_id: String,
    project_id: Option<String>,
    state: State<'_, AgentState>,
) -> Result<SessionResponse, PublicError> {
    set_agent_session_project_inner(session_id, project_id, &state).await
}

async fn set_agent_session_project_inner(
    session_id: String,
    project_id: Option<String>,
    state: &AgentState,
) -> Result<SessionResponse, PublicError> {
    let _operation = state.try_operation()?;
    let session_id = AgentSessionId::parse(&session_id).map_err(|_| PublicError::InvalidInput)?;
    let project_id = project_id
        .map(|value| ProjectId::parse(&value).map_err(|_| PublicError::InvalidInput))
        .transpose()?;
    let store = Arc::clone(&state.store);
    tauri::async_runtime::spawn_blocking(move || {
        tule_core::set_session_project(store.as_ref(), session_id, project_id)
            .map(SessionResponse::from)
            .map_err(|_| PublicError::AgentStorageUnavailable)
    })
    .await
    .map_err(|_| PublicError::AgentStorageUnavailable)?
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
            tule_core::prepare_agent_send(store.as_ref(), None, "Hello", None, "").unwrap();
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
            tule_core::prepare_agent_send(store.as_ref(), None, "Hello", None, "").unwrap();

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
        let prepared =
            tule_core::prepare_agent_send(state.store.as_ref(), None, "Hello", None, "").unwrap();
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

        assert!(matches!(blocked, Err(PublicError::SessionBusy)));
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
            tule_core::prepare_agent_send(store.as_ref(), None, "Hello", None, "").unwrap();
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
