//! Narrow typed Agent IPC commands and ordered stream channel messages.

use std::sync::{Arc, Mutex};

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
    cancellation: Mutex<Option<(String, CancellationToken)>>,
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
            cancellation: Mutex::new(None),
        }
    }

    pub(crate) fn chatgpt(&self) -> Option<Arc<crate::openai_chatgpt::ChatGptAdapter>> {
        self.chatgpt.clone()
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
        tule_core::PrepareAgentSendError::Time(_)
        | tule_core::PrepareAgentSendError::Repository(_) => PublicError::AgentStorageUnavailable,
    }
}

fn map_finish(_: tule_core::FinishAgentTurnError) -> PublicError {
    PublicError::AgentStorageUnavailable
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
    match state.provider.connection_status().state {
        crate::provider::ConnectionState::Connected => {}
        crate::provider::ConnectionState::UnavailableInThisBuild => {
            return Err(PublicError::ProviderUnavailable);
        }
        crate::provider::ConnectionState::ReconnectRequired => {
            return Err(PublicError::AuthenticationRequired);
        }
        _ => return Err(PublicError::NotConnected),
    }
    if let Some(adapter) = state.chatgpt() {
        adapter.ensure_fresh_access_public(store.as_ref()).await?;
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
    channel
        .send(AgentStreamEvent::Started {
            session_id: prepared.session.id().to_string(),
            turn_id: turn_id.to_string(),
        })
        .map_err(|_| PublicError::ProviderUnavailable)?;

    let store_for_stream = Arc::clone(&store);
    let channel_for_stream = channel.clone();
    let terminal_meta = Arc::new(Mutex::new((false, None::<String>)));
    let terminal_meta_cb = Arc::clone(&terminal_meta);
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
                        let turn =
                            tule_core::apply_agent_delta(store_for_stream.as_ref(), turn_id, &text)
                                .map_err(|_| PublicError::AgentStorageUnavailable)?;
                        channel_for_stream
                            .send(AgentStreamEvent::Delta {
                                turn_id: turn.id().to_string(),
                                text,
                            })
                            .map_err(|_| PublicError::ProviderUnavailable)?;
                    }
                    ProviderEvent::Completed {
                        response_id: id, ..
                    } => {
                        if let Ok(mut guard) = terminal_meta_cb.lock() {
                            guard.0 = true;
                            guard.1 = id;
                        }
                    }
                }
                Ok(())
            }),
        )
        .await;
    let (completed, response_id) = terminal_meta
        .lock()
        .map(|guard| (guard.0, guard.1.clone()))
        .unwrap_or((false, None));
    let terminal = match result {
        Ok(_) if token.is_cancelled() => {
            tule_core::cancel_agent_turn(store.as_ref(), turn_id).map_err(map_finish)?
        }
        Ok(_) if completed => {
            tule_core::complete_agent_turn(store.as_ref(), turn_id, response_id, None, None)
                .map_err(map_finish)?
        }
        Ok(_) => tule_core::fail_agent_turn(store.as_ref(), turn_id, "provider_unavailable")
            .map_err(map_finish)?,
        Err(PublicError::Cancelled) => {
            tule_core::cancel_agent_turn(store.as_ref(), turn_id).map_err(map_finish)?
        }
        Err(PublicError::OutputLimit) => {
            tule_core::fail_agent_turn(store.as_ref(), turn_id, "output_limit")
                .map_err(map_finish)?
        }
        Err(error) => tule_core::fail_agent_turn(
            store.as_ref(),
            turn_id,
            serde_json::to_string(&error)
                .unwrap_or_default()
                .trim_matches('"'),
        )
        .map_err(map_finish)?,
    };
    *state
        .cancellation
        .lock()
        .map_err(|_| PublicError::AgentStorageUnavailable)? = None;
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
