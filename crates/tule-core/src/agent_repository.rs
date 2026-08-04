//! Provider-neutral persistence interfaces for Agent Sessions and provider profiles.

use std::error::Error;

use crate::{AgentEvent, AgentSession, AgentSessionId, AgentTurn, AgentTurnId, ProjectId};

/// Non-secret provider-profile metadata persisted in application storage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderProfile {
    id: String,
    provider_kind: String,
    visible_model_id: String,
    credential_handle: Option<String>,
    access_token_expires_at_unix_ms: Option<i64>,
    created_at_unix_ms: i64,
    updated_at_unix_ms: i64,
}

impl ProviderProfile {
    /// Creates a built-in profile definition without credentials.
    #[must_use]
    pub fn built_in(
        id: impl Into<String>,
        provider_kind: impl Into<String>,
        visible_model_id: impl Into<String>,
        created_at_unix_ms: i64,
    ) -> Self {
        Self {
            id: id.into(),
            provider_kind: provider_kind.into(),
            visible_model_id: visible_model_id.into(),
            credential_handle: None,
            access_token_expires_at_unix_ms: None,
            created_at_unix_ms,
            updated_at_unix_ms: created_at_unix_ms,
        }
    }

    /// Reconstructs a persisted profile.
    #[must_use]
    pub fn from_stored_parts(
        id: impl Into<String>,
        provider_kind: impl Into<String>,
        visible_model_id: impl Into<String>,
        credential_handle: Option<impl Into<String>>,
        access_token_expires_at_unix_ms: Option<i64>,
        created_at_unix_ms: i64,
        updated_at_unix_ms: i64,
    ) -> Self {
        Self {
            id: id.into(),
            provider_kind: provider_kind.into(),
            visible_model_id: visible_model_id.into(),
            credential_handle: credential_handle.map(Into::into),
            access_token_expires_at_unix_ms,
            created_at_unix_ms,
            updated_at_unix_ms,
        }
    }

    /// Returns the stable profile identifier.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Returns the provider kind label.
    #[must_use]
    pub fn provider_kind(&self) -> &str {
        &self.provider_kind
    }

    /// Returns the visible model identifier.
    #[must_use]
    pub fn visible_model_id(&self) -> &str {
        &self.visible_model_id
    }

    /// Returns the opaque credential handle when connected.
    #[must_use]
    pub fn credential_handle(&self) -> Option<&str> {
        self.credential_handle.as_deref()
    }

    /// Returns access-token expiry metadata when known.
    #[must_use]
    pub const fn access_token_expires_at_unix_ms(&self) -> Option<i64> {
        self.access_token_expires_at_unix_ms
    }

    /// Returns creation time.
    #[must_use]
    pub const fn created_at_unix_ms(&self) -> i64 {
        self.created_at_unix_ms
    }

    /// Returns update time.
    #[must_use]
    pub const fn updated_at_unix_ms(&self) -> i64 {
        self.updated_at_unix_ms
    }

    /// Updates opaque credential handle and expiry metadata.
    pub fn set_credential_metadata(
        &mut self,
        credential_handle: Option<String>,
        access_token_expires_at_unix_ms: Option<i64>,
        updated_at_unix_ms: i64,
    ) {
        self.credential_handle = credential_handle;
        self.access_token_expires_at_unix_ms = access_token_expires_at_unix_ms;
        self.updated_at_unix_ms = updated_at_unix_ms;
    }

    /// Updates the non-secret visible model label used as display metadata.
    pub fn set_visible_model_id(
        &mut self,
        visible_model_id: impl Into<String>,
        updated_at_unix_ms: i64,
    ) {
        self.visible_model_id = visible_model_id.into();
        self.updated_at_unix_ms = updated_at_unix_ms;
    }
}

/// Storage operations required by Agent use cases.
///
/// Implementations own synchronization and transactions. Network waits must not
/// hold a database mutex.
pub trait AgentRepository: Send + Sync {
    /// Implementation-specific storage failure.
    type Error: Error + Send + Sync + 'static;

    /// Ensures the built-in provider profile exists.
    fn ensure_provider_profile(&self, profile: &ProviderProfile) -> Result<(), Self::Error>;

    /// Loads a provider profile by identifier.
    fn get_provider_profile(&self, id: &str) -> Result<Option<ProviderProfile>, Self::Error>;

    /// Replaces non-secret credential metadata for a profile.
    fn update_provider_profile(&self, profile: &ProviderProfile) -> Result<(), Self::Error>;

    /// Inserts a newly created session.
    fn create_session(&self, session: &AgentSession) -> Result<(), Self::Error>;

    /// Updates an existing session's mutable fields.
    fn update_session(&self, session: &AgentSession) -> Result<(), Self::Error>;

    /// Finds a session by identifier.
    fn find_session(&self, id: &AgentSessionId) -> Result<Option<AgentSession>, Self::Error>;

    /// Lists sessions newest-updated first.
    fn list_sessions(&self) -> Result<Vec<AgentSession>, Self::Error>;

    /// Lists sessions associated with a Project, newest-updated first.
    fn list_sessions_for_project(
        &self,
        project_id: &ProjectId,
    ) -> Result<Vec<AgentSession>, Self::Error>;

    /// Lists projectless sessions, newest-updated first.
    fn list_projectless_sessions(&self) -> Result<Vec<AgentSession>, Self::Error>;

    /// Returns the most recently updated session when any exist.
    fn most_recent_session(&self) -> Result<Option<AgentSession>, Self::Error>;

    /// Inserts a pending turn.
    fn create_turn(&self, turn: &AgentTurn) -> Result<(), Self::Error>;

    /// Updates turn text/state/provenance fields.
    fn update_turn(&self, turn: &AgentTurn) -> Result<(), Self::Error>;

    /// Finds a turn by identifier.
    fn find_turn(&self, id: &AgentTurnId) -> Result<Option<AgentTurn>, Self::Error>;

    /// Lists turns for a session in ascending ordinal order.
    fn list_turns(&self, session_id: &AgentSessionId) -> Result<Vec<AgentTurn>, Self::Error>;

    /// Returns the next ordinal for a session.
    fn next_turn_ordinal(&self, session_id: &AgentSessionId) -> Result<u64, Self::Error>;

    /// Returns whether any turn is currently pending or streaming.
    fn has_inflight_turn(&self) -> Result<bool, Self::Error>;

    /// Lists every pending or streaming turn for startup recovery.
    fn list_inflight_turns(&self) -> Result<Vec<AgentTurn>, Self::Error>;

    /// Appends an event. Sequence must be unique and increasing per session.
    fn append_event(&self, event: &AgentEvent) -> Result<(), Self::Error>;

    /// Atomically updates a session and appends its provenance event.
    fn update_session_with_event(
        &self,
        session: &AgentSession,
        event: &AgentEvent,
    ) -> Result<(), Self::Error>;

    /// Returns the next event sequence for a session.
    fn next_event_sequence(&self, session_id: &AgentSessionId) -> Result<u64, Self::Error>;

    /// Lists events for a session in ascending sequence order.
    fn list_events(&self, session_id: &AgentSessionId) -> Result<Vec<AgentEvent>, Self::Error>;

    /// Atomically creates a first-send session, pending turn, and events.
    fn create_session_with_first_turn(
        &self,
        session: &AgentSession,
        turn: &AgentTurn,
        session_created: &AgentEvent,
        turn_pending: &AgentEvent,
    ) -> Result<(), Self::Error>;

    /// Atomically appends a turn and its pending event to an existing session.
    fn append_turn_with_pending_event(
        &self,
        session: &AgentSession,
        turn: &AgentTurn,
        turn_pending: &AgentEvent,
    ) -> Result<(), Self::Error>;

    /// Atomically checkpoints turn text and optionally records first streaming.
    fn checkpoint_turn(
        &self,
        turn: &AgentTurn,
        streaming_event: Option<&AgentEvent>,
    ) -> Result<(), Self::Error>;

    /// Atomically finishes a turn and appends its single terminal event.
    fn finish_turn_with_terminal_event(
        &self,
        session: &AgentSession,
        turn: &AgentTurn,
        terminal_event: &AgentEvent,
    ) -> Result<(), Self::Error>;

    /// Atomically finishes every supplied in-flight turn and appends each terminal event.
    fn finish_turns_with_terminal_events(
        &self,
        updates: &[(AgentSession, AgentTurn, AgentEvent)],
    ) -> Result<(), Self::Error>;
}
