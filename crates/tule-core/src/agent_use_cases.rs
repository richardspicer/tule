//! Provider-neutral Agent lifecycle, context selection, and recovery use cases.

use std::{error::Error, fmt};

use crate::{
    AgentContextError, AgentEvent, AgentEventKind, AgentInputError, AgentOutputLimitError,
    AgentRepository, AgentSession, AgentSessionId, AgentTurn, AgentTurnFinishError, AgentTurnId,
    AgentTurnState, CompletedTurnContext, IllegalAgentTurnTransition, ProjectId, ProjectTimeError,
    ProviderRequestId, assemble_responses_request_json, derive_session_title, validate_user_text,
};

/// Result of preparing a first or follow-up send before network transmission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedAgentSend {
    /// Session after persistence.
    pub session: AgentSession,
    /// Pending turn after persistence.
    pub turn: AgentTurn,
    /// Deterministic Responses JSON body.
    pub request_json: String,
    /// Whether this send created the durable session.
    pub created_session: bool,
}

/// Starts a send against an existing session or creates one on first valid send.
pub fn prepare_agent_send<R>(
    repository: &R,
    session_id: Option<AgentSessionId>,
    user_text: &str,
    project_id: Option<ProjectId>,
    saved_project_instructions: &str,
) -> Result<PreparedAgentSend, PrepareAgentSendError>
where
    R: AgentRepository + ?Sized,
{
    validate_user_text(user_text).map_err(PrepareAgentSendError::InvalidInput)?;
    if repository
        .has_inflight_turn()
        .map_err(PrepareAgentSendError::repository)?
    {
        return Err(PrepareAgentSendError::SessionBusy);
    }

    let completed_history = match session_id {
        Some(id) => {
            let turns = repository
                .list_turns(&id)
                .map_err(PrepareAgentSendError::repository)?;
            completed_history_from_turns(&turns)
        }
        None => Vec::new(),
    };

    let request_json = assemble_responses_request_json(
        &completed_history,
        user_text,
        if saved_project_instructions.is_empty() {
            None
        } else {
            Some(saved_project_instructions)
        },
    )
    .map_err(PrepareAgentSendError::from)?;

    let provider_request_id = ProviderRequestId::generate();

    if let Some(session_id) = session_id {
        let mut session = repository
            .find_session(&session_id)
            .map_err(PrepareAgentSendError::repository)?
            .ok_or(PrepareAgentSendError::SessionNotFound)?;
        // Prospective association for this send uses the caller's selected project.
        let ordinal = repository
            .next_turn_ordinal(&session_id)
            .map_err(PrepareAgentSendError::repository)?;
        let turn = AgentTurn::new_pending(
            session_id,
            ordinal,
            user_text,
            project_id,
            saved_project_instructions,
            provider_request_id,
        )
        .map_err(PrepareAgentSendError::Time)?;
        session
            .touch_updated_at()
            .map_err(PrepareAgentSendError::Time)?;
        let sequence = repository
            .next_event_sequence(&session_id)
            .map_err(PrepareAgentSendError::repository)?;
        let turn_pending = AgentEvent::new(
            session_id,
            Some(turn.id()),
            sequence,
            AgentEventKind::TurnPending,
        )
        .map_err(PrepareAgentSendError::Time)?;
        repository
            .append_turn_with_pending_event(&session, &turn, &turn_pending)
            .map_err(PrepareAgentSendError::repository)?;
        return Ok(PreparedAgentSend {
            session,
            turn,
            request_json,
            created_session: false,
        });
    }

    let title = derive_session_title(user_text);
    let session = AgentSession::new(title, project_id).map_err(PrepareAgentSendError::Time)?;
    let turn = AgentTurn::new_pending(
        session.id(),
        0,
        user_text,
        project_id,
        saved_project_instructions,
        provider_request_id,
    )
    .map_err(PrepareAgentSendError::Time)?;
    let session_created = AgentEvent::new(session.id(), None, 0, AgentEventKind::SessionCreated)
        .map_err(PrepareAgentSendError::Time)?;
    let turn_pending = AgentEvent::new(
        session.id(),
        Some(turn.id()),
        1,
        AgentEventKind::TurnPending,
    )
    .map_err(PrepareAgentSendError::Time)?;
    repository
        .create_session_with_first_turn(&session, &turn, &session_created, &turn_pending)
        .map_err(PrepareAgentSendError::repository)?;

    Ok(PreparedAgentSend {
        session,
        turn,
        request_json,
        created_session: true,
    })
}

/// Applies a text delta, transitioning pending→streaming on the first delta.
pub fn apply_agent_delta<R>(
    repository: &R,
    turn_id: AgentTurnId,
    delta: &str,
) -> Result<AgentTurn, ApplyAgentDeltaError>
where
    R: AgentRepository + ?Sized,
{
    let mut turn = repository
        .find_turn(&turn_id)
        .map_err(ApplyAgentDeltaError::repository)?
        .ok_or(ApplyAgentDeltaError::TurnNotFound)?;

    if turn.state().is_terminal() {
        return Err(ApplyAgentDeltaError::IllegalTransition(
            IllegalAgentTurnTransition {
                from: turn.state(),
                to: AgentTurnState::Streaming,
            },
        ));
    }

    let first_delta = turn.state() == AgentTurnState::Pending && !delta.is_empty();
    if let Err(AgentOutputLimitError) = turn.append_agent_text(delta) {
        let failed =
            fail_turn(repository, &mut turn, "output_limit").map_err(ApplyAgentDeltaError::from)?;
        return Err(ApplyAgentDeltaError::OutputLimit(Box::new(failed)));
    }

    let streaming_event = if first_delta {
        turn.mark_streaming()
            .map_err(ApplyAgentDeltaError::IllegalTransition)?;
        let sequence = repository
            .next_event_sequence(&turn.session_id())
            .map_err(ApplyAgentDeltaError::repository)?;
        Some(
            AgentEvent::new(
                turn.session_id(),
                Some(turn.id()),
                sequence,
                AgentEventKind::TurnStreaming,
            )
            .map_err(ApplyAgentDeltaError::Time)?,
        )
    } else {
        None
    };

    repository
        .checkpoint_turn(&turn, streaming_event.as_ref())
        .map_err(ApplyAgentDeltaError::repository)?;
    Ok(turn)
}

/// Checkpoints accumulated text without changing lifecycle state.
pub fn checkpoint_agent_turn<R>(
    repository: &R,
    turn: &AgentTurn,
) -> Result<(), ApplyAgentDeltaError>
where
    R: AgentRepository + ?Sized,
{
    repository
        .checkpoint_turn(turn, None)
        .map_err(ApplyAgentDeltaError::repository)
}

/// Completes a turn successfully with exactly one terminal event.
pub fn complete_agent_turn<R>(
    repository: &R,
    turn_id: AgentTurnId,
    provider_response_id: Option<String>,
    usage_input_tokens: Option<u64>,
    usage_output_tokens: Option<u64>,
) -> Result<AgentTurn, FinishAgentTurnError>
where
    R: AgentRepository + ?Sized,
{
    let mut turn = repository
        .find_turn(&turn_id)
        .map_err(FinishAgentTurnError::repository)?
        .ok_or(FinishAgentTurnError::TurnNotFound)?;
    turn.complete(
        provider_response_id,
        usage_input_tokens,
        usage_output_tokens,
    )
    .map_err(FinishAgentTurnError::from)?;
    persist_terminal(repository, turn)
}

/// Cancels a turn locally with exactly one terminal event.
pub fn cancel_agent_turn<R>(
    repository: &R,
    turn_id: AgentTurnId,
) -> Result<AgentTurn, FinishAgentTurnError>
where
    R: AgentRepository + ?Sized,
{
    let mut turn = repository
        .find_turn(&turn_id)
        .map_err(FinishAgentTurnError::repository)?
        .ok_or(FinishAgentTurnError::TurnNotFound)?;
    turn.cancel().map_err(FinishAgentTurnError::from)?;
    persist_terminal(repository, turn)
}

/// Fails a turn with an allowlisted code and exactly one terminal event.
pub fn fail_agent_turn<R>(
    repository: &R,
    turn_id: AgentTurnId,
    error_code: &str,
) -> Result<AgentTurn, FinishAgentTurnError>
where
    R: AgentRepository + ?Sized,
{
    let mut turn = repository
        .find_turn(&turn_id)
        .map_err(FinishAgentTurnError::repository)?
        .ok_or(FinishAgentTurnError::TurnNotFound)?;
    fail_turn(repository, &mut turn, error_code)
}

/// Updates a session's prospective Project association.
pub fn set_session_project<R>(
    repository: &R,
    session_id: AgentSessionId,
    project_id: Option<ProjectId>,
) -> Result<AgentSession, SetSessionProjectError>
where
    R: AgentRepository + ?Sized,
{
    let mut session = repository
        .find_session(&session_id)
        .map_err(SetSessionProjectError::repository)?
        .ok_or(SetSessionProjectError::SessionNotFound)?;
    session
        .set_project_id(project_id)
        .map_err(SetSessionProjectError::Time)?;
    let sequence = repository
        .next_event_sequence(&session_id)
        .map_err(SetSessionProjectError::repository)?;
    let event = AgentEvent::new(
        session_id,
        None,
        sequence,
        AgentEventKind::ProjectAssociationChanged,
    )
    .map_err(SetSessionProjectError::Time)?;
    repository
        .update_session(&session)
        .map_err(SetSessionProjectError::repository)?;
    repository
        .append_event(&event)
        .map_err(SetSessionProjectError::repository)?;
    Ok(session)
}

/// Marks every pending/streaming turn interrupted exactly once.
pub fn interrupt_inflight_turns<R>(repository: &R) -> Result<Vec<AgentTurn>, FinishAgentTurnError>
where
    R: AgentRepository + ?Sized,
{
    let inflight = repository
        .list_inflight_turns()
        .map_err(FinishAgentTurnError::repository)?;
    let mut finished = Vec::with_capacity(inflight.len());
    for mut turn in inflight {
        turn.interrupt().map_err(FinishAgentTurnError::from)?;
        finished.push(persist_terminal(repository, turn)?);
    }
    Ok(finished)
}

/// Selects completed turns only for later provider context.
#[must_use]
pub fn completed_history_from_turns(turns: &[AgentTurn]) -> Vec<CompletedTurnContext> {
    turns
        .iter()
        .filter(|turn| turn.state() == AgentTurnState::Completed)
        .map(|turn| CompletedTurnContext {
            user_text: turn.user_text().to_owned(),
            agent_text: turn.agent_text().to_owned(),
        })
        .collect()
}

fn fail_turn<R>(
    repository: &R,
    turn: &mut AgentTurn,
    error_code: &str,
) -> Result<AgentTurn, FinishAgentTurnError>
where
    R: AgentRepository + ?Sized,
{
    turn.fail(error_code).map_err(FinishAgentTurnError::from)?;
    persist_terminal(repository, turn.clone())
}

fn persist_terminal<R>(repository: &R, turn: AgentTurn) -> Result<AgentTurn, FinishAgentTurnError>
where
    R: AgentRepository + ?Sized,
{
    let mut session = repository
        .find_session(&turn.session_id())
        .map_err(FinishAgentTurnError::repository)?
        .ok_or(FinishAgentTurnError::SessionNotFound)?;
    session
        .touch_updated_at()
        .map_err(FinishAgentTurnError::Time)?;
    let kind = AgentEventKind::for_terminal_state(turn.state())
        .map_err(FinishAgentTurnError::IllegalTransition)?;
    let sequence = repository
        .next_event_sequence(&turn.session_id())
        .map_err(FinishAgentTurnError::repository)?;
    let terminal_event = AgentEvent::new(turn.session_id(), Some(turn.id()), sequence, kind)
        .map_err(FinishAgentTurnError::Time)?;
    repository
        .finish_turn_with_terminal_event(&session, &turn, &terminal_event)
        .map_err(FinishAgentTurnError::repository)?;
    Ok(turn)
}

/// Failure preparing an Agent send.
#[derive(Debug)]
pub enum PrepareAgentSendError {
    /// User text is invalid.
    InvalidInput(AgentInputError),
    /// Assembled context exceeds the ceiling.
    ContextLimit {
        /// Observed byte count.
        byte_count: usize,
    },
    /// Another turn is already in flight.
    SessionBusy,
    /// The referenced session does not exist.
    SessionNotFound,
    /// Clock failure.
    Time(ProjectTimeError),
    /// Repository failure.
    Repository(Box<dyn Error + Send + Sync>),
}

impl From<AgentContextError> for PrepareAgentSendError {
    fn from(error: AgentContextError) -> Self {
        match error {
            AgentContextError::InvalidInput(error) => Self::InvalidInput(error),
            AgentContextError::ContextLimit { byte_count } => Self::ContextLimit { byte_count },
        }
    }
}

impl PrepareAgentSendError {
    fn repository<E>(error: E) -> Self
    where
        E: Error + Send + Sync + 'static,
    {
        Self::Repository(Box::new(error))
    }
}

impl fmt::Display for PrepareAgentSendError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidInput(error) => error.fmt(formatter),
            Self::ContextLimit { byte_count } => {
                write!(
                    formatter,
                    "agent context limit exceeded ({byte_count} bytes)"
                )
            }
            Self::SessionBusy => formatter.write_str("an agent turn is already in flight"),
            Self::SessionNotFound => formatter.write_str("agent session not found"),
            Self::Time(error) => error.fmt(formatter),
            Self::Repository(error) => error.fmt(formatter),
        }
    }
}

impl Error for PrepareAgentSendError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidInput(error) => Some(error),
            Self::Time(error) => Some(error),
            Self::Repository(error) => Some(error.as_ref()),
            Self::ContextLimit { .. } | Self::SessionBusy | Self::SessionNotFound => None,
        }
    }
}

/// Failure applying a stream delta.
#[derive(Debug)]
pub enum ApplyAgentDeltaError {
    /// Turn does not exist.
    TurnNotFound,
    /// Illegal lifecycle transition.
    IllegalTransition(IllegalAgentTurnTransition),
    /// Output ceiling reached; turn was failed as `output_limit`.
    OutputLimit(Box<AgentTurn>),
    /// Clock failure.
    Time(ProjectTimeError),
    /// Finishing after output limit failed.
    Finish(FinishAgentTurnError),
    /// Repository failure.
    Repository(Box<dyn Error + Send + Sync>),
}

impl ApplyAgentDeltaError {
    fn repository<E>(error: E) -> Self
    where
        E: Error + Send + Sync + 'static,
    {
        Self::Repository(Box::new(error))
    }
}

impl From<FinishAgentTurnError> for ApplyAgentDeltaError {
    fn from(error: FinishAgentTurnError) -> Self {
        Self::Finish(error)
    }
}

impl fmt::Display for ApplyAgentDeltaError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TurnNotFound => formatter.write_str("agent turn not found"),
            Self::IllegalTransition(error) => error.fmt(formatter),
            Self::OutputLimit(_) => formatter.write_str("agent output limit exceeded"),
            Self::Time(error) => error.fmt(formatter),
            Self::Finish(error) => error.fmt(formatter),
            Self::Repository(error) => error.fmt(formatter),
        }
    }
}

impl Error for ApplyAgentDeltaError {}

/// Failure finishing a turn.
#[derive(Debug)]
pub enum FinishAgentTurnError {
    /// Turn does not exist.
    TurnNotFound,
    /// Session does not exist.
    SessionNotFound,
    /// Illegal lifecycle transition.
    IllegalTransition(IllegalAgentTurnTransition),
    /// Nested finish error from AgentTurn methods.
    Finish(AgentTurnFinishError),
    /// Clock failure.
    Time(ProjectTimeError),
    /// Repository failure.
    Repository(Box<dyn Error + Send + Sync>),
}

impl FinishAgentTurnError {
    fn repository<E>(error: E) -> Self
    where
        E: Error + Send + Sync + 'static,
    {
        Self::Repository(Box::new(error))
    }
}

impl From<AgentTurnFinishError> for FinishAgentTurnError {
    fn from(error: AgentTurnFinishError) -> Self {
        match error {
            AgentTurnFinishError::IllegalTransition(error) => Self::IllegalTransition(error),
            AgentTurnFinishError::Time(error) => Self::Time(error),
        }
    }
}

impl From<IllegalAgentTurnTransition> for FinishAgentTurnError {
    fn from(error: IllegalAgentTurnTransition) -> Self {
        Self::IllegalTransition(error)
    }
}

impl fmt::Display for FinishAgentTurnError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TurnNotFound => formatter.write_str("agent turn not found"),
            Self::SessionNotFound => formatter.write_str("agent session not found"),
            Self::IllegalTransition(error) => error.fmt(formatter),
            Self::Finish(error) => error.fmt(formatter),
            Self::Time(error) => error.fmt(formatter),
            Self::Repository(error) => error.fmt(formatter),
        }
    }
}

impl Error for FinishAgentTurnError {}

/// Failure updating prospective Project association.
#[derive(Debug)]
pub enum SetSessionProjectError {
    /// Session does not exist.
    SessionNotFound,
    /// Clock failure.
    Time(ProjectTimeError),
    /// Repository failure.
    Repository(Box<dyn Error + Send + Sync>),
}

impl SetSessionProjectError {
    fn repository<E>(error: E) -> Self
    where
        E: Error + Send + Sync + 'static,
    {
        Self::Repository(Box::new(error))
    }
}

impl fmt::Display for SetSessionProjectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SessionNotFound => formatter.write_str("agent session not found"),
            Self::Time(error) => error.fmt(formatter),
            Self::Repository(error) => error.fmt(formatter),
        }
    }
}

impl Error for SetSessionProjectError {}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::{AgentEventId, MODEL_ID, PROVIDER_PROFILE_ID, ProviderProfile};

    #[derive(Debug)]
    struct FakeError(&'static str);

    impl fmt::Display for FakeError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(self.0)
        }
    }

    impl Error for FakeError {}

    #[derive(Default)]
    struct FakeAgentRepository {
        inner: Mutex<FakeState>,
    }

    #[derive(Default)]
    struct FakeState {
        profiles: Vec<ProviderProfile>,
        sessions: Vec<AgentSession>,
        turns: Vec<AgentTurn>,
        events: Vec<AgentEvent>,
    }

    impl AgentRepository for FakeAgentRepository {
        type Error = FakeError;

        fn ensure_provider_profile(&self, profile: &ProviderProfile) -> Result<(), Self::Error> {
            let mut state = self.inner.lock().unwrap();
            if !state.profiles.iter().any(|item| item.id() == profile.id()) {
                state.profiles.push(profile.clone());
            }
            Ok(())
        }

        fn get_provider_profile(&self, id: &str) -> Result<Option<ProviderProfile>, Self::Error> {
            Ok(self
                .inner
                .lock()
                .unwrap()
                .profiles
                .iter()
                .find(|profile| profile.id() == id)
                .cloned())
        }

        fn update_provider_profile(&self, profile: &ProviderProfile) -> Result<(), Self::Error> {
            let mut state = self.inner.lock().unwrap();
            if let Some(existing) = state
                .profiles
                .iter_mut()
                .find(|item| item.id() == profile.id())
            {
                *existing = profile.clone();
                Ok(())
            } else {
                Err(FakeError("missing profile"))
            }
        }

        fn create_session(&self, session: &AgentSession) -> Result<(), Self::Error> {
            self.inner.lock().unwrap().sessions.push(session.clone());
            Ok(())
        }

        fn update_session(&self, session: &AgentSession) -> Result<(), Self::Error> {
            let mut state = self.inner.lock().unwrap();
            if let Some(existing) = state
                .sessions
                .iter_mut()
                .find(|item| item.id() == session.id())
            {
                *existing = session.clone();
                Ok(())
            } else {
                Err(FakeError("missing session"))
            }
        }

        fn find_session(&self, id: &AgentSessionId) -> Result<Option<AgentSession>, Self::Error> {
            Ok(self
                .inner
                .lock()
                .unwrap()
                .sessions
                .iter()
                .find(|session| session.id() == *id)
                .cloned())
        }

        fn list_sessions(&self) -> Result<Vec<AgentSession>, Self::Error> {
            let mut sessions = self.inner.lock().unwrap().sessions.clone();
            sessions.sort_by(|left, right| {
                right
                    .updated_at_unix_ms()
                    .cmp(&left.updated_at_unix_ms())
                    .then_with(|| right.id().to_string().cmp(&left.id().to_string()))
            });
            Ok(sessions)
        }

        fn list_sessions_for_project(
            &self,
            project_id: &ProjectId,
        ) -> Result<Vec<AgentSession>, Self::Error> {
            Ok(self
                .list_sessions()?
                .into_iter()
                .filter(|session| session.project_id() == Some(*project_id))
                .collect())
        }

        fn list_projectless_sessions(&self) -> Result<Vec<AgentSession>, Self::Error> {
            Ok(self
                .list_sessions()?
                .into_iter()
                .filter(|session| session.project_id().is_none())
                .collect())
        }

        fn most_recent_session(&self) -> Result<Option<AgentSession>, Self::Error> {
            Ok(self.list_sessions()?.into_iter().next())
        }

        fn create_turn(&self, turn: &AgentTurn) -> Result<(), Self::Error> {
            self.inner.lock().unwrap().turns.push(turn.clone());
            Ok(())
        }

        fn update_turn(&self, turn: &AgentTurn) -> Result<(), Self::Error> {
            let mut state = self.inner.lock().unwrap();
            if let Some(existing) = state.turns.iter_mut().find(|item| item.id() == turn.id()) {
                *existing = turn.clone();
                Ok(())
            } else {
                Err(FakeError("missing turn"))
            }
        }

        fn find_turn(&self, id: &AgentTurnId) -> Result<Option<AgentTurn>, Self::Error> {
            Ok(self
                .inner
                .lock()
                .unwrap()
                .turns
                .iter()
                .find(|turn| turn.id() == *id)
                .cloned())
        }

        fn list_turns(&self, session_id: &AgentSessionId) -> Result<Vec<AgentTurn>, Self::Error> {
            let mut turns: Vec<_> = self
                .inner
                .lock()
                .unwrap()
                .turns
                .iter()
                .filter(|turn| turn.session_id() == *session_id)
                .cloned()
                .collect();
            turns.sort_by_key(AgentTurn::ordinal);
            Ok(turns)
        }

        fn next_turn_ordinal(&self, session_id: &AgentSessionId) -> Result<u64, Self::Error> {
            Ok(self
                .list_turns(session_id)?
                .last()
                .map(|turn| turn.ordinal() + 1)
                .unwrap_or(0))
        }

        fn has_inflight_turn(&self) -> Result<bool, Self::Error> {
            Ok(self.inner.lock().unwrap().turns.iter().any(|turn| {
                matches!(
                    turn.state(),
                    AgentTurnState::Pending | AgentTurnState::Streaming
                )
            }))
        }

        fn list_inflight_turns(&self) -> Result<Vec<AgentTurn>, Self::Error> {
            Ok(self
                .inner
                .lock()
                .unwrap()
                .turns
                .iter()
                .filter(|turn| {
                    matches!(
                        turn.state(),
                        AgentTurnState::Pending | AgentTurnState::Streaming
                    )
                })
                .cloned()
                .collect())
        }

        fn append_event(&self, event: &AgentEvent) -> Result<(), Self::Error> {
            self.inner.lock().unwrap().events.push(event.clone());
            Ok(())
        }

        fn next_event_sequence(&self, session_id: &AgentSessionId) -> Result<u64, Self::Error> {
            Ok(self
                .list_events(session_id)?
                .last()
                .map(|event| event.sequence() + 1)
                .unwrap_or(0))
        }

        fn list_events(&self, session_id: &AgentSessionId) -> Result<Vec<AgentEvent>, Self::Error> {
            let mut events: Vec<_> = self
                .inner
                .lock()
                .unwrap()
                .events
                .iter()
                .filter(|event| event.session_id() == *session_id)
                .cloned()
                .collect();
            events.sort_by_key(AgentEvent::sequence);
            Ok(events)
        }

        fn create_session_with_first_turn(
            &self,
            session: &AgentSession,
            turn: &AgentTurn,
            session_created: &AgentEvent,
            turn_pending: &AgentEvent,
        ) -> Result<(), Self::Error> {
            self.create_session(session)?;
            self.create_turn(turn)?;
            self.append_event(session_created)?;
            self.append_event(turn_pending)?;
            Ok(())
        }

        fn append_turn_with_pending_event(
            &self,
            session: &AgentSession,
            turn: &AgentTurn,
            turn_pending: &AgentEvent,
        ) -> Result<(), Self::Error> {
            self.update_session(session)?;
            self.create_turn(turn)?;
            self.append_event(turn_pending)?;
            Ok(())
        }

        fn checkpoint_turn(
            &self,
            turn: &AgentTurn,
            streaming_event: Option<&AgentEvent>,
        ) -> Result<(), Self::Error> {
            self.update_turn(turn)?;
            if let Some(event) = streaming_event {
                self.append_event(event)?;
            }
            Ok(())
        }

        fn finish_turn_with_terminal_event(
            &self,
            session: &AgentSession,
            turn: &AgentTurn,
            terminal_event: &AgentEvent,
        ) -> Result<(), Self::Error> {
            self.update_session(session)?;
            self.update_turn(turn)?;
            self.append_event(terminal_event)?;
            Ok(())
        }
    }

    #[test]
    fn first_send_creates_session_pending_turn_and_events() {
        let repository = FakeAgentRepository::default();
        let prepared =
            prepare_agent_send(&repository, None, "Hello Agent", None, "").expect("prepare");
        assert!(prepared.created_session);
        assert_eq!(prepared.session.title(), "Hello Agent");
        assert_eq!(prepared.turn.state(), AgentTurnState::Pending);
        assert!(prepared.request_json.contains("\"stream\":true"));
        let events = repository.list_events(&prepared.session.id()).unwrap();
        assert_eq!(events[0].kind(), AgentEventKind::SessionCreated);
        assert_eq!(events[1].kind(), AgentEventKind::TurnPending);
        assert_eq!(events[1].sequence(), 1);
    }

    #[test]
    fn one_inflight_turn_blocks_second_send() {
        let repository = FakeAgentRepository::default();
        prepare_agent_send(&repository, None, "First", None, "").unwrap();
        let error = prepare_agent_send(&repository, None, "Second", None, "").unwrap_err();
        assert!(matches!(error, PrepareAgentSendError::SessionBusy));
    }

    #[test]
    fn streaming_completion_and_history_selection() {
        let repository = FakeAgentRepository::default();
        let prepared = prepare_agent_send(&repository, None, "First", None, "").unwrap();
        let turn = apply_agent_delta(&repository, prepared.turn.id(), "Hi").unwrap();
        assert_eq!(turn.state(), AgentTurnState::Streaming);
        let completed =
            complete_agent_turn(&repository, turn.id(), Some("resp".into()), None, None).unwrap();
        assert_eq!(completed.state(), AgentTurnState::Completed);
        let events = repository.list_events(&prepared.session.id()).unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|event| event.kind() == AgentEventKind::TurnCompleted)
                .count(),
            1
        );
        let cancelled_prep =
            prepare_agent_send(&repository, Some(prepared.session.id()), "Second", None, "")
                .unwrap();
        cancel_agent_turn(&repository, cancelled_prep.turn.id()).unwrap();
        let history =
            completed_history_from_turns(&repository.list_turns(&prepared.session.id()).unwrap());
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].user_text, "First");
    }

    #[test]
    fn startup_interruption_marks_inflight_once() {
        let repository = FakeAgentRepository::default();
        let prepared = prepare_agent_send(&repository, None, "Hang", None, "").unwrap();
        apply_agent_delta(&repository, prepared.turn.id(), "partial").unwrap();
        let interrupted = interrupt_inflight_turns(&repository).unwrap();
        assert_eq!(interrupted.len(), 1);
        assert_eq!(interrupted[0].state(), AgentTurnState::Interrupted);
        assert_eq!(interrupted[0].agent_text(), "partial");
        assert!(interrupt_inflight_turns(&repository).unwrap().is_empty());
    }

    #[test]
    fn prospective_project_change_records_event() {
        let repository = FakeAgentRepository::default();
        let prepared = prepare_agent_send(&repository, None, "Hello", None, "").unwrap();
        complete_agent_turn(&repository, prepared.turn.id(), None, None, None).unwrap();
        let project_id = ProjectId::generate();
        let session =
            set_session_project(&repository, prepared.session.id(), Some(project_id)).unwrap();
        assert_eq!(session.project_id(), Some(project_id));
        assert!(
            repository
                .list_events(&session.id())
                .unwrap()
                .iter()
                .any(|event| event.kind() == AgentEventKind::ProjectAssociationChanged)
        );
        let _ = (PROVIDER_PROFILE_ID, MODEL_ID, AgentEventId::generate());
    }
}
