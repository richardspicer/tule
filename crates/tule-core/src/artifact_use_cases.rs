//! Create-from-turn and read use cases for Artifacts.

use std::{error::Error, fmt};

use crate::{
    AgentRepository, AgentSessionId, AgentTurnId, AgentTurnState, Artifact, ArtifactDetail,
    ArtifactId, ArtifactKind, ArtifactRepository, ArtifactSummary, ArtifactValidationError,
    ArtifactVersion, ArtifactVersionProvenance, InvalidAgentId, InvalidArtifactKind, ProjectId,
    ProjectTimeError, resolve_artifact_title,
};

/// Creates an Artifact and immutable version 1 from a completed turn in durable storage.
///
/// Loads `agent_text` and provenance by `turn_id`. Client-supplied body or provenance
/// is never trusted. Optional title override and kind are the only caller inputs besides
/// the turn identifier.
pub fn create_artifact_from_turn<A, R>(
    agent_repository: &A,
    artifact_repository: &R,
    turn_id: &str,
    title_override: Option<&str>,
    kind: Option<&str>,
) -> Result<(Artifact, ArtifactVersion), CreateArtifactFromTurnError>
where
    A: AgentRepository + ?Sized,
    R: ArtifactRepository + ?Sized,
{
    let turn_id =
        AgentTurnId::parse(turn_id).map_err(CreateArtifactFromTurnError::InvalidTurnId)?;
    let kind = match kind {
        None => ArtifactKind::DEFAULT,
        Some(value) => {
            ArtifactKind::parse(value).map_err(CreateArtifactFromTurnError::InvalidKind)?
        }
    };
    let turn = agent_repository
        .find_turn(&turn_id)
        .map_err(CreateArtifactFromTurnError::agent_repository)?
        .ok_or(CreateArtifactFromTurnError::TurnNotFound)?;
    if turn.state() != AgentTurnState::Completed {
        return Err(CreateArtifactFromTurnError::TurnNotCompleted);
    }
    if turn.agent_text().is_empty() {
        return Err(CreateArtifactFromTurnError::EmptyAgentText);
    }

    let title = resolve_artifact_title(title_override, turn.agent_text());
    let artifact =
        Artifact::new(title, kind, turn.project_id()).map_err(CreateArtifactFromTurnError::Time)?;
    let provenance = ArtifactVersionProvenance::new(
        turn.session_id(),
        turn.id(),
        turn.provider_profile_id(),
        turn.model_id(),
        turn.prompt_version(),
        turn.project_id(),
        turn.provider_request_id(),
    );
    let version = ArtifactVersion::new_first(artifact.id(), turn.agent_text(), provenance)
        .map_err(CreateArtifactFromTurnError::Validation)?;
    artifact_repository
        .create_artifact_with_first_version(&artifact, &version)
        .map_err(CreateArtifactFromTurnError::artifact_repository)?;
    Ok((artifact, version))
}

/// Lists Artifacts for the open session (session-sourced union optional project).
pub fn list_artifacts_for_session_context<R>(
    repository: &R,
    session_id: &str,
    project_id: Option<&str>,
) -> Result<Vec<ArtifactSummary>, ListArtifactsError>
where
    R: ArtifactRepository + ?Sized,
{
    let session_id =
        AgentSessionId::parse(session_id).map_err(ListArtifactsError::InvalidSessionId)?;
    let project_id = project_id
        .map(ProjectId::parse)
        .transpose()
        .map_err(ListArtifactsError::InvalidProjectId)?;
    repository
        .list_artifacts_for_session_context(&session_id, project_id.as_ref())
        .map_err(ListArtifactsError::repository)
}

/// Loads one Artifact with every version by identifier.
pub fn get_artifact<R>(
    repository: &R,
    artifact_id: &str,
) -> Result<ArtifactDetail, GetArtifactError>
where
    R: ArtifactRepository + ?Sized,
{
    let artifact_id =
        ArtifactId::parse(artifact_id).map_err(GetArtifactError::InvalidArtifactId)?;
    repository
        .get_artifact(&artifact_id)
        .map_err(GetArtifactError::repository)?
        .ok_or(GetArtifactError::NotFound)
}

/// Failure creating an Artifact from a turn.
#[derive(Debug)]
pub enum CreateArtifactFromTurnError {
    /// Turn identifier is not a valid UUID v7.
    InvalidTurnId(InvalidAgentId),
    /// Kind is not allowlisted.
    InvalidKind(InvalidArtifactKind),
    /// No turn exists for the identifier.
    TurnNotFound,
    /// Turn is not in the completed state.
    TurnNotCompleted,
    /// Completed turn has empty agent text.
    EmptyAgentText,
    /// Clock failure while stamping creation time.
    Time(ProjectTimeError),
    /// Content validation failed.
    Validation(ArtifactValidationError),
    /// Agent repository failed while loading the turn.
    AgentRepository(Box<dyn Error + Send + Sync>),
    /// Artifact repository failed while persisting.
    ArtifactRepository(Box<dyn Error + Send + Sync>),
}

impl CreateArtifactFromTurnError {
    fn agent_repository<E>(error: E) -> Self
    where
        E: Error + Send + Sync + 'static,
    {
        Self::AgentRepository(Box::new(error))
    }

    fn artifact_repository<E>(error: E) -> Self
    where
        E: Error + Send + Sync + 'static,
    {
        Self::ArtifactRepository(Box::new(error))
    }
}

impl fmt::Display for CreateArtifactFromTurnError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidTurnId(error) => write!(formatter, "invalid turn id: {error}"),
            Self::InvalidKind(error) => write!(formatter, "invalid artifact kind: {error}"),
            Self::TurnNotFound => formatter.write_str("agent turn was not found"),
            Self::TurnNotCompleted => {
                formatter.write_str("only completed agent turns can be saved as artifacts")
            }
            Self::EmptyAgentText => {
                formatter.write_str("completed agent turn has empty agent text")
            }
            Self::Time(error) => write!(formatter, "could not stamp artifact time: {error}"),
            Self::Validation(error) => write!(formatter, "invalid artifact content: {error}"),
            Self::AgentRepository(error) => {
                write!(formatter, "could not load agent turn: {error}")
            }
            Self::ArtifactRepository(error) => {
                write!(formatter, "could not persist artifact: {error}")
            }
        }
    }
}

impl Error for CreateArtifactFromTurnError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidTurnId(error) => Some(error),
            Self::InvalidKind(error) => Some(error),
            Self::Time(error) => Some(error),
            Self::Validation(error) => Some(error),
            Self::AgentRepository(error) | Self::ArtifactRepository(error) => Some(error.as_ref()),
            Self::TurnNotFound | Self::TurnNotCompleted | Self::EmptyAgentText => None,
        }
    }
}

/// Failure listing Artifacts for a session context.
#[derive(Debug)]
pub enum ListArtifactsError {
    /// Session identifier is invalid.
    InvalidSessionId(InvalidAgentId),
    /// Project identifier is invalid.
    InvalidProjectId(crate::InvalidProjectId),
    /// Repository failure.
    Repository(Box<dyn Error + Send + Sync>),
}

impl ListArtifactsError {
    /// Wraps a typed repository error.
    pub fn repository<E>(error: E) -> Self
    where
        E: Error + Send + Sync + 'static,
    {
        Self::Repository(Box::new(error))
    }
}

impl fmt::Display for ListArtifactsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSessionId(error) => write!(formatter, "invalid session id: {error}"),
            Self::InvalidProjectId(error) => write!(formatter, "invalid project id: {error}"),
            Self::Repository(error) => write!(formatter, "could not list artifacts: {error}"),
        }
    }
}

impl Error for ListArtifactsError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidSessionId(error) => Some(error),
            Self::InvalidProjectId(error) => Some(error),
            Self::Repository(error) => Some(error.as_ref()),
        }
    }
}

/// Failure loading one Artifact.
#[derive(Debug)]
pub enum GetArtifactError {
    /// Artifact identifier is invalid.
    InvalidArtifactId(InvalidAgentId),
    /// No Artifact exists for the identifier.
    NotFound,
    /// Repository failure, including reconstruction failures.
    Repository(Box<dyn Error + Send + Sync>),
}

impl GetArtifactError {
    /// Wraps a typed repository error.
    pub fn repository<E>(error: E) -> Self
    where
        E: Error + Send + Sync + 'static,
    {
        Self::Repository(Box::new(error))
    }
}

impl fmt::Display for GetArtifactError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidArtifactId(error) => write!(formatter, "invalid artifact id: {error}"),
            Self::NotFound => formatter.write_str("artifact was not found"),
            Self::Repository(error) => write!(formatter, "could not load artifact: {error}"),
        }
    }
}

impl Error for GetArtifactError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidArtifactId(error) => Some(error),
            Self::Repository(error) => Some(error.as_ref()),
            Self::NotFound => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, MutexGuard};

    use crate::{
        AgentEvent, AgentSession, AgentTurn, AgentTurnId, ProviderProfile, Source, TurnSource,
        apply_agent_delta, complete_agent_turn, prepare_agent_send,
    };

    use super::*;

    #[derive(Debug)]
    struct TestError;

    impl fmt::Display for TestError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("test error")
        }
    }

    impl Error for TestError {}

    #[derive(Default)]
    struct MemoryStore {
        agent: Mutex<AgentMemory>,
        artifacts: Mutex<Vec<(Artifact, ArtifactVersion)>>,
        fail_create: Mutex<bool>,
    }

    #[derive(Default)]
    struct AgentMemory {
        sessions: Vec<AgentSession>,
        turns: Vec<AgentTurn>,
        events: Vec<AgentEvent>,
        profiles: Vec<ProviderProfile>,
        sources: Vec<TurnSource>,
    }

    impl MemoryStore {
        fn agent(&self) -> MutexGuard<'_, AgentMemory> {
            self.agent.lock().unwrap()
        }

        fn artifacts(&self) -> MutexGuard<'_, Vec<(Artifact, ArtifactVersion)>> {
            self.artifacts.lock().unwrap()
        }
    }

    impl AgentRepository for MemoryStore {
        type Error = TestError;

        fn ensure_provider_profile(&self, profile: &ProviderProfile) -> Result<(), Self::Error> {
            let mut memory = self.agent();
            if memory.profiles.iter().all(|item| item.id() != profile.id()) {
                memory.profiles.push(profile.clone());
            }
            Ok(())
        }

        fn get_provider_profile(&self, id: &str) -> Result<Option<ProviderProfile>, Self::Error> {
            Ok(self
                .agent()
                .profiles
                .iter()
                .find(|item| item.id() == id)
                .cloned())
        }

        fn update_provider_profile(&self, profile: &ProviderProfile) -> Result<(), Self::Error> {
            let mut memory = self.agent();
            if let Some(slot) = memory
                .profiles
                .iter_mut()
                .find(|item| item.id() == profile.id())
            {
                *slot = profile.clone();
            }
            Ok(())
        }

        fn create_session(&self, session: &AgentSession) -> Result<(), Self::Error> {
            self.agent().sessions.push(session.clone());
            Ok(())
        }

        fn update_session(&self, session: &AgentSession) -> Result<(), Self::Error> {
            let mut memory = self.agent();
            if let Some(slot) = memory
                .sessions
                .iter_mut()
                .find(|item| item.id() == session.id())
            {
                *slot = session.clone();
            }
            Ok(())
        }

        fn find_session(&self, id: &AgentSessionId) -> Result<Option<AgentSession>, Self::Error> {
            Ok(self
                .agent()
                .sessions
                .iter()
                .find(|item| item.id() == *id)
                .cloned())
        }

        fn list_sessions(&self) -> Result<Vec<AgentSession>, Self::Error> {
            Ok(self.agent().sessions.clone())
        }

        fn list_sessions_for_project(
            &self,
            project_id: &ProjectId,
        ) -> Result<Vec<AgentSession>, Self::Error> {
            Ok(self
                .agent()
                .sessions
                .iter()
                .filter(|item| item.project_id() == Some(*project_id))
                .cloned()
                .collect())
        }

        fn list_projectless_sessions(&self) -> Result<Vec<AgentSession>, Self::Error> {
            Ok(self
                .agent()
                .sessions
                .iter()
                .filter(|item| item.project_id().is_none())
                .cloned()
                .collect())
        }

        fn most_recent_session(&self) -> Result<Option<AgentSession>, Self::Error> {
            Ok(self.agent().sessions.first().cloned())
        }

        fn create_turn(&self, turn: &AgentTurn) -> Result<(), Self::Error> {
            self.agent().turns.push(turn.clone());
            Ok(())
        }

        fn update_turn(&self, turn: &AgentTurn) -> Result<(), Self::Error> {
            let mut memory = self.agent();
            if let Some(slot) = memory.turns.iter_mut().find(|item| item.id() == turn.id()) {
                *slot = turn.clone();
            }
            Ok(())
        }

        fn find_turn(&self, id: &AgentTurnId) -> Result<Option<AgentTurn>, Self::Error> {
            Ok(self
                .agent()
                .turns
                .iter()
                .find(|item| item.id() == *id)
                .cloned())
        }

        fn list_turns(&self, session_id: &AgentSessionId) -> Result<Vec<AgentTurn>, Self::Error> {
            Ok(self
                .agent()
                .turns
                .iter()
                .filter(|item| item.session_id() == *session_id)
                .cloned()
                .collect())
        }

        fn next_turn_ordinal(&self, session_id: &AgentSessionId) -> Result<u64, Self::Error> {
            Ok(self
                .list_turns(session_id)?
                .iter()
                .map(AgentTurn::ordinal)
                .max()
                .map_or(0, |ordinal| ordinal + 1))
        }

        fn has_inflight_turn(&self) -> Result<bool, Self::Error> {
            Ok(self.agent().turns.iter().any(|turn| {
                matches!(
                    turn.state(),
                    AgentTurnState::Pending | AgentTurnState::Streaming
                )
            }))
        }

        fn list_inflight_turns(&self) -> Result<Vec<AgentTurn>, Self::Error> {
            Ok(self
                .agent()
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
            self.agent().events.push(event.clone());
            Ok(())
        }

        fn update_session_with_event(
            &self,
            session: &AgentSession,
            event: &AgentEvent,
        ) -> Result<(), Self::Error> {
            self.update_session(session)?;
            self.append_event(event)
        }

        fn next_event_sequence(&self, session_id: &AgentSessionId) -> Result<u64, Self::Error> {
            Ok(self
                .agent()
                .events
                .iter()
                .filter(|event| event.session_id() == *session_id)
                .map(AgentEvent::sequence)
                .max()
                .map_or(0, |sequence| sequence + 1))
        }

        fn list_events(&self, session_id: &AgentSessionId) -> Result<Vec<AgentEvent>, Self::Error> {
            Ok(self
                .agent()
                .events
                .iter()
                .filter(|event| event.session_id() == *session_id)
                .cloned()
                .collect())
        }

        fn create_session_with_first_turn(
            &self,
            session: &AgentSession,
            turn: &AgentTurn,
            session_created: &AgentEvent,
            turn_pending: &AgentEvent,
            source: Option<&Source>,
        ) -> Result<(), Self::Error> {
            self.create_session(session)?;
            self.create_turn(turn)?;
            if let Some(source) = source {
                self.agent().sources.push(
                    TurnSource::from_stored_parts(&turn.id().to_string(), source.clone(), 0)
                        .unwrap(),
                );
            }
            self.append_event(session_created)?;
            self.append_event(turn_pending)
        }

        fn append_turn_with_pending_event(
            &self,
            session: &AgentSession,
            turn: &AgentTurn,
            turn_pending: &AgentEvent,
            source: Option<&Source>,
        ) -> Result<(), Self::Error> {
            self.update_session(session)?;
            self.create_turn(turn)?;
            if let Some(source) = source {
                self.agent().sources.push(
                    TurnSource::from_stored_parts(&turn.id().to_string(), source.clone(), 0)
                        .unwrap(),
                );
            }
            self.append_event(turn_pending)
        }

        fn list_turn_sources(
            &self,
            session_id: &AgentSessionId,
        ) -> Result<Vec<TurnSource>, Self::Error> {
            let turn_ids: Vec<_> = self
                .list_turns(session_id)?
                .into_iter()
                .map(|turn| turn.id())
                .collect();
            Ok(self
                .agent()
                .sources
                .iter()
                .filter(|item| turn_ids.contains(&item.turn_id()))
                .cloned()
                .collect())
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
            self.append_event(terminal_event)
        }

        fn finish_turns_with_terminal_events(
            &self,
            updates: &[(AgentSession, AgentTurn, AgentEvent)],
        ) -> Result<(), Self::Error> {
            for (session, turn, event) in updates {
                self.finish_turn_with_terminal_event(session, turn, event)?;
            }
            Ok(())
        }
    }

    impl ArtifactRepository for MemoryStore {
        type Error = TestError;

        fn create_artifact_with_first_version(
            &self,
            artifact: &Artifact,
            version: &ArtifactVersion,
        ) -> Result<(), Self::Error> {
            if *self.fail_create.lock().unwrap() {
                return Err(TestError);
            }
            self.artifacts().push((artifact.clone(), version.clone()));
            Ok(())
        }

        fn get_artifact(&self, id: &ArtifactId) -> Result<Option<ArtifactDetail>, Self::Error> {
            let rows = self.artifacts();
            let mut versions = Vec::new();
            let mut artifact = None;
            for (stored_artifact, version) in rows.iter() {
                if stored_artifact.id() == *id {
                    artifact = Some(stored_artifact.clone());
                    versions.push(version.clone());
                }
            }
            match artifact {
                Some(artifact) => Ok(Some(ArtifactDetail::new(artifact, versions).unwrap())),
                None => Ok(None),
            }
        }

        fn list_artifacts_for_session_context(
            &self,
            session_id: &AgentSessionId,
            project_id: Option<&ProjectId>,
        ) -> Result<Vec<ArtifactSummary>, Self::Error> {
            let rows = self.artifacts();
            let mut summaries = Vec::new();
            for (artifact, version) in rows.iter() {
                let session_match = version.provenance().source_session_id() == *session_id;
                let project_match = match (project_id, artifact.project_id()) {
                    (Some(expected), Some(actual)) => expected == &actual,
                    _ => false,
                };
                if session_match || project_match {
                    summaries.push(
                        ArtifactSummary::new(
                            artifact.clone(),
                            version.id(),
                            version.version_ordinal(),
                        )
                        .unwrap(),
                    );
                }
            }
            Ok(summaries)
        }
    }

    fn completed_turn(store: &MemoryStore, agent_text: &str) -> AgentTurn {
        let prepared = prepare_agent_send(
            store,
            None,
            "user asks",
            None,
            "",
            "xai-subscription-oauth",
            "grok-3",
            None,
            None,
            false,
        )
        .unwrap();
        apply_agent_delta(store, prepared.turn.id(), agent_text).unwrap();
        complete_agent_turn(store, prepared.turn.id(), None, None, None).unwrap()
    }

    #[test]
    fn create_from_completed_turn_freezes_content_and_provenance() {
        let store = MemoryStore::default();
        let turn = completed_turn(&store, "Agent conclusion body");
        let (artifact, version) =
            create_artifact_from_turn(&store, &store, &turn.id().to_string(), None, None).unwrap();

        assert_eq!(artifact.kind(), ArtifactKind::Conclusion);
        assert_eq!(artifact.title(), "Agent conclusion body");
        assert_eq!(version.content(), "Agent conclusion body");
        assert_eq!(version.version_ordinal(), 1);
        assert_eq!(version.provenance().source_turn_id(), turn.id());
        assert_eq!(version.provenance().source_session_id(), turn.session_id());
        assert_eq!(
            version.provenance().provider_profile_id(),
            turn.provider_profile_id()
        );
        assert_eq!(version.provenance().model_id(), turn.model_id());
        assert_eq!(version.provenance().prompt_version(), turn.prompt_version());
        assert_eq!(
            version.provenance().provider_request_id(),
            turn.provider_request_id()
        );
        assert_eq!(store.artifacts().len(), 1);
    }

    #[test]
    fn create_rejects_non_completed_and_empty_text_without_writing() {
        let store = MemoryStore::default();
        let prepared = prepare_agent_send(
            &store,
            None,
            "user asks",
            None,
            "",
            "xai-subscription-oauth",
            "grok-3",
            None,
            None,
            false,
        )
        .unwrap();
        assert!(matches!(
            create_artifact_from_turn(
                &store,
                &store,
                &prepared.turn.id().to_string(),
                None,
                Some("conclusion")
            ),
            Err(CreateArtifactFromTurnError::TurnNotCompleted)
        ));
        assert!(store.artifacts().is_empty());

        let empty_completed =
            complete_agent_turn(&store, prepared.turn.id(), None, None, None).unwrap();
        assert!(empty_completed.agent_text().is_empty());
        assert!(matches!(
            create_artifact_from_turn(
                &store,
                &store,
                &empty_completed.id().to_string(),
                None,
                None
            ),
            Err(CreateArtifactFromTurnError::EmptyAgentText)
        ));
        assert!(store.artifacts().is_empty());
    }

    #[test]
    fn create_does_not_trust_absent_client_body_and_uses_stored_turn_text() {
        let store = MemoryStore::default();
        let turn = completed_turn(&store, "durable stored text");
        let (_, version) = create_artifact_from_turn(
            &store,
            &store,
            &turn.id().to_string(),
            Some("  My title  "),
            Some("research_brief"),
        )
        .unwrap();
        assert_eq!(version.content(), "durable stored text");
        assert_eq!(store.artifacts()[0].0.kind(), ArtifactKind::ResearchBrief);
        assert_eq!(store.artifacts()[0].0.title(), "My title");
    }

    #[test]
    fn create_rolls_back_when_persist_fails() {
        let store = MemoryStore::default();
        let turn = completed_turn(&store, "body");
        *store.fail_create.lock().unwrap() = true;
        assert!(matches!(
            create_artifact_from_turn(&store, &store, &turn.id().to_string(), None, None),
            Err(CreateArtifactFromTurnError::ArtifactRepository(_))
        ));
        assert!(store.artifacts().is_empty());
    }

    #[test]
    fn list_and_get_round_trip_after_create() {
        let store = MemoryStore::default();
        let turn = completed_turn(&store, "retrieve me");
        let (artifact, version) =
            create_artifact_from_turn(&store, &store, &turn.id().to_string(), None, None).unwrap();

        let listed =
            list_artifacts_for_session_context(&store, &turn.session_id().to_string(), None)
                .unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].artifact().id(), artifact.id());
        assert_eq!(listed[0].latest_version_id(), version.id());

        let detail = get_artifact(&store, &artifact.id().to_string()).unwrap();
        assert_eq!(detail.artifact().id(), artifact.id());
        assert_eq!(detail.versions().len(), 1);
        assert_eq!(detail.versions()[0].content(), "retrieve me");
        assert_eq!(
            detail.versions()[0].provenance().source_turn_id(),
            turn.id()
        );
    }

    #[test]
    fn invalid_kind_rejected_before_persist() {
        let store = MemoryStore::default();
        let turn = completed_turn(&store, "body");
        assert!(matches!(
            create_artifact_from_turn(
                &store,
                &store,
                &turn.id().to_string(),
                None,
                Some("not_a_kind")
            ),
            Err(CreateArtifactFromTurnError::InvalidKind(_))
        ));
        assert!(store.artifacts().is_empty());
    }
}
