use std::ops::Deref;

use rusqlite::{OptionalExtension, params};
use tule_core::{
    AgentEvent, AgentRepository, AgentSession, AgentSessionId, AgentTurn, AgentTurnId, ProjectId,
    ProviderProfile,
};

use super::{SqliteStore, SqliteStoreError};

impl AgentRepository for SqliteStore {
    type Error = SqliteStoreError;

    fn ensure_provider_profile(&self, profile: &ProviderProfile) -> Result<(), Self::Error> {
        self.connection()?
            .execute(
                "INSERT INTO provider_profiles (
                id, provider_kind, visible_model_id, credential_handle, expires_at_unix_ms,
                created_at_unix_ms, updated_at_unix_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(id) DO NOTHING",
                params![
                    profile.id(),
                    profile.provider_kind(),
                    profile.visible_model_id(),
                    profile.credential_handle(),
                    profile.access_token_expires_at_unix_ms(),
                    profile.created_at_unix_ms(),
                    profile.updated_at_unix_ms()
                ],
            )
            .map_err(SqliteStoreError::Database)?;
        Ok(())
    }

    fn get_provider_profile(&self, id: &str) -> Result<Option<ProviderProfile>, Self::Error> {
        let stored = self
            .connection()?
            .query_row(
                "SELECT id, provider_kind, visible_model_id, credential_handle, expires_at_unix_ms,
                    created_at_unix_ms, updated_at_unix_ms
             FROM provider_profiles WHERE id = ?1",
                [id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<i64>>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, i64>(6)?,
                    ))
                },
            )
            .optional()
            .map_err(SqliteStoreError::Database)?;
        Ok(stored.map(|row| {
            ProviderProfile::from_stored_parts(row.0, row.1, row.2, row.3, row.4, row.5, row.6)
        }))
    }

    fn update_provider_profile(&self, profile: &ProviderProfile) -> Result<(), Self::Error> {
        self.connection()?
            .execute(
                "UPDATE provider_profiles
             SET provider_kind = ?2, visible_model_id = ?3, credential_handle = ?4,
                 expires_at_unix_ms = ?5, updated_at_unix_ms = ?6
             WHERE id = ?1",
                params![
                    profile.id(),
                    profile.provider_kind(),
                    profile.visible_model_id(),
                    profile.credential_handle(),
                    profile.access_token_expires_at_unix_ms(),
                    profile.updated_at_unix_ms()
                ],
            )
            .map_err(SqliteStoreError::Database)?;
        Ok(())
    }

    fn create_session(&self, session: &AgentSession) -> Result<(), Self::Error> {
        insert_session(&self.connection()?, session)
    }

    fn update_session(&self, session: &AgentSession) -> Result<(), Self::Error> {
        update_session(&self.connection()?, session)
    }

    fn find_session(&self, id: &AgentSessionId) -> Result<Option<AgentSession>, Self::Error> {
        let stored = self
            .connection()?
            .query_row(
                "SELECT id, title, project_id, provider_profile_id, model_id,
                    created_at_unix_ms, updated_at_unix_ms
             FROM agent_sessions WHERE id = ?1",
                [id.to_string()],
                session_row,
            )
            .optional()
            .map_err(SqliteStoreError::Database)?;
        stored.map(reconstruct_session).transpose()
    }

    fn list_sessions(&self) -> Result<Vec<AgentSession>, Self::Error> {
        list_sessions(&self.connection()?, "")
    }

    fn list_sessions_for_project(
        &self,
        project_id: &ProjectId,
    ) -> Result<Vec<AgentSession>, Self::Error> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT id, title, project_id, provider_profile_id, model_id, created_at_unix_ms, updated_at_unix_ms
             FROM agent_sessions WHERE project_id = ?1 ORDER BY updated_at_unix_ms DESC, id DESC",
        ).map_err(SqliteStoreError::Database)?;
        let rows = statement
            .query_map([project_id.to_string()], session_row)
            .map_err(SqliteStoreError::Database)?;
        collect_sessions(rows)
    }

    fn list_projectless_sessions(&self) -> Result<Vec<AgentSession>, Self::Error> {
        list_sessions(&self.connection()?, " WHERE project_id IS NULL")
    }

    fn most_recent_session(&self) -> Result<Option<AgentSession>, Self::Error> {
        Ok(self.list_sessions()?.into_iter().next())
    }

    fn create_turn(&self, turn: &AgentTurn) -> Result<(), Self::Error> {
        insert_turn(&self.connection()?, turn)
    }

    fn update_turn(&self, turn: &AgentTurn) -> Result<(), Self::Error> {
        update_turn(&self.connection()?, turn)
    }

    fn find_turn(&self, id: &AgentTurnId) -> Result<Option<AgentTurn>, Self::Error> {
        let stored = self
            .connection()?
            .query_row(&turn_select(" WHERE id = ?1"), [id.to_string()], turn_row)
            .optional()
            .map_err(SqliteStoreError::Database)?;
        stored.map(reconstruct_turn).transpose()
    }

    fn list_turns(&self, session_id: &AgentSessionId) -> Result<Vec<AgentTurn>, Self::Error> {
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(&turn_select(" WHERE session_id = ?1 ORDER BY ordinal ASC"))
            .map_err(SqliteStoreError::Database)?;
        let rows = statement
            .query_map([session_id.to_string()], turn_row)
            .map_err(SqliteStoreError::Database)?;
        collect_turns(rows)
    }

    fn next_turn_ordinal(&self, session_id: &AgentSessionId) -> Result<u64, Self::Error> {
        let ordinal: Option<i64> = self
            .connection()?
            .query_row(
                "SELECT MAX(ordinal) FROM agent_turns WHERE session_id = ?1",
                [session_id.to_string()],
                |row| row.get(0),
            )
            .map_err(SqliteStoreError::Database)?;
        ordinal
            .map(|value| u64::try_from(value + 1).map_err(|_| SqliteStoreError::Numeric))
            .transpose()
            .map(|value| value.unwrap_or(0))
    }

    fn has_inflight_turn(&self) -> Result<bool, Self::Error> {
        let count: i64 = self
            .connection()?
            .query_row(
                "SELECT COUNT(*) FROM agent_turns WHERE state IN ('pending', 'streaming')",
                [],
                |row| row.get(0),
            )
            .map_err(SqliteStoreError::Database)?;
        Ok(count > 0)
    }

    fn list_inflight_turns(&self) -> Result<Vec<AgentTurn>, Self::Error> {
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(&turn_select(
                " WHERE state IN ('pending', 'streaming') ORDER BY started_at_unix_ms ASC, id ASC",
            ))
            .map_err(SqliteStoreError::Database)?;
        let rows = statement
            .query_map([], turn_row)
            .map_err(SqliteStoreError::Database)?;
        collect_turns(rows)
    }

    fn append_event(&self, event: &AgentEvent) -> Result<(), Self::Error> {
        insert_event(&self.connection()?, event)
    }

    fn update_session_with_event(
        &self,
        session: &AgentSession,
        event: &AgentEvent,
    ) -> Result<(), Self::Error> {
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction()
            .map_err(SqliteStoreError::Database)?;
        update_session(&transaction, session)?;
        insert_event(&transaction, event)?;
        transaction.commit().map_err(SqliteStoreError::Database)
    }

    fn next_event_sequence(&self, session_id: &AgentSessionId) -> Result<u64, Self::Error> {
        let sequence: Option<i64> = self
            .connection()?
            .query_row(
                "SELECT MAX(sequence) FROM agent_events WHERE session_id = ?1",
                [session_id.to_string()],
                |row| row.get(0),
            )
            .map_err(SqliteStoreError::Database)?;
        sequence
            .map(|value| u64::try_from(value + 1).map_err(|_| SqliteStoreError::Numeric))
            .transpose()
            .map(|value| value.unwrap_or(0))
    }

    fn list_events(&self, session_id: &AgentSessionId) -> Result<Vec<AgentEvent>, Self::Error> {
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(
                "SELECT id, session_id, turn_id, sequence, kind, created_at_unix_ms
             FROM agent_events WHERE session_id = ?1 ORDER BY sequence ASC",
            )
            .map_err(SqliteStoreError::Database)?;
        let rows = statement
            .query_map([session_id.to_string()], event_row)
            .map_err(SqliteStoreError::Database)?;
        let mut events = Vec::new();
        for row in rows {
            events.push(reconstruct_event(row.map_err(SqliteStoreError::Database)?)?);
        }
        Ok(events)
    }

    fn create_session_with_first_turn(
        &self,
        session: &AgentSession,
        turn: &AgentTurn,
        session_created: &AgentEvent,
        turn_pending: &AgentEvent,
    ) -> Result<(), Self::Error> {
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction()
            .map_err(SqliteStoreError::Database)?;
        insert_session(&transaction, session)?;
        insert_turn(&transaction, turn)?;
        insert_event(&transaction, session_created)?;
        insert_event(&transaction, turn_pending)?;
        transaction.commit().map_err(SqliteStoreError::Database)
    }

    fn append_turn_with_pending_event(
        &self,
        session: &AgentSession,
        turn: &AgentTurn,
        turn_pending: &AgentEvent,
    ) -> Result<(), Self::Error> {
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction()
            .map_err(SqliteStoreError::Database)?;
        update_session(&transaction, session)?;
        insert_turn(&transaction, turn)?;
        insert_event(&transaction, turn_pending)?;
        transaction.commit().map_err(SqliteStoreError::Database)
    }

    fn checkpoint_turn(
        &self,
        turn: &AgentTurn,
        streaming_event: Option<&AgentEvent>,
    ) -> Result<(), Self::Error> {
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction()
            .map_err(SqliteStoreError::Database)?;
        update_inflight_turn(&transaction, turn)?;
        if let Some(event) = streaming_event {
            insert_event(&transaction, event)?;
        }
        transaction.commit().map_err(SqliteStoreError::Database)
    }

    fn finish_turn_with_terminal_event(
        &self,
        session: &AgentSession,
        turn: &AgentTurn,
        terminal_event: &AgentEvent,
    ) -> Result<(), Self::Error> {
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction()
            .map_err(SqliteStoreError::Database)?;
        update_session(&transaction, session)?;
        update_inflight_turn(&transaction, turn)?;
        insert_event(&transaction, terminal_event)?;
        transaction.commit().map_err(SqliteStoreError::Database)
    }

    fn finish_turns_with_terminal_events(
        &self,
        updates: &[(AgentSession, AgentTurn, AgentEvent)],
    ) -> Result<(), Self::Error> {
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction()
            .map_err(SqliteStoreError::Database)?;
        for (session, turn, terminal_event) in updates {
            update_session(&transaction, session)?;
            update_inflight_turn(&transaction, turn)?;
            insert_event(&transaction, terminal_event)?;
        }
        transaction.commit().map_err(SqliteStoreError::Database)
    }
}

fn insert_session(
    connection: &impl Deref<Target = rusqlite::Connection>,
    session: &AgentSession,
) -> Result<(), SqliteStoreError> {
    connection.execute(
        "INSERT INTO agent_sessions (id, title, project_id, provider_profile_id, model_id, created_at_unix_ms, updated_at_unix_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![session.id().to_string(), session.title(), session.project_id().map(|id| id.to_string()),
            session.provider_profile_id(), session.model_id(), session.created_at_unix_ms(), session.updated_at_unix_ms()],
    ).map_err(SqliteStoreError::Database)?;
    Ok(())
}

fn update_session(
    connection: &impl Deref<Target = rusqlite::Connection>,
    session: &AgentSession,
) -> Result<(), SqliteStoreError> {
    connection.execute(
        "UPDATE agent_sessions SET title = ?2, project_id = ?3, provider_profile_id = ?4, model_id = ?5, updated_at_unix_ms = ?6 WHERE id = ?1",
        params![session.id().to_string(), session.title(), session.project_id().map(|id| id.to_string()),
            session.provider_profile_id(), session.model_id(), session.updated_at_unix_ms()],
    ).map_err(SqliteStoreError::Database)?;
    Ok(())
}

fn insert_turn(
    connection: &impl Deref<Target = rusqlite::Connection>,
    turn: &AgentTurn,
) -> Result<(), SqliteStoreError> {
    let ordinal = i64::try_from(turn.ordinal()).map_err(|_| SqliteStoreError::Numeric)?;
    let input = turn
        .usage_input_tokens()
        .map(i64::try_from)
        .transpose()
        .map_err(|_| SqliteStoreError::Numeric)?;
    let output = turn
        .usage_output_tokens()
        .map(i64::try_from)
        .transpose()
        .map_err(|_| SqliteStoreError::Numeric)?;
    connection.execute(
        "INSERT INTO agent_turns (id, session_id, ordinal, user_text, agent_text, state, error_code, provider_profile_id, model_id, provider_request_id, provider_response_id, usage_input_tokens, usage_output_tokens, project_id, project_instructions, prompt_version, started_at_unix_ms, finished_at_unix_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
        params![turn.id().to_string(), turn.session_id().to_string(), ordinal, turn.user_text(), turn.agent_text(),
            turn.state().as_str(), turn.error_code(), turn.provider_profile_id(), turn.model_id(),
            turn.provider_request_id().to_string(), turn.provider_response_id(), input, output,
            turn.project_id().map(|id| id.to_string()), turn.project_instructions(), turn.prompt_version(),
            turn.started_at_unix_ms(), turn.finished_at_unix_ms()],
    ).map_err(SqliteStoreError::Database)?;
    Ok(())
}

fn update_turn(
    connection: &impl Deref<Target = rusqlite::Connection>,
    turn: &AgentTurn,
) -> Result<(), SqliteStoreError> {
    let input = turn
        .usage_input_tokens()
        .map(i64::try_from)
        .transpose()
        .map_err(|_| SqliteStoreError::Numeric)?;
    let output = turn
        .usage_output_tokens()
        .map(i64::try_from)
        .transpose()
        .map_err(|_| SqliteStoreError::Numeric)?;
    connection.execute(
        "UPDATE agent_turns SET agent_text = ?2, state = ?3, error_code = ?4, provider_response_id = ?5,
             usage_input_tokens = ?6, usage_output_tokens = ?7, finished_at_unix_ms = ?8 WHERE id = ?1",
        params![turn.id().to_string(), turn.agent_text(), turn.state().as_str(), turn.error_code(),
            turn.provider_response_id(), input, output, turn.finished_at_unix_ms()],
    ).map_err(SqliteStoreError::Database)?;
    Ok(())
}

fn update_inflight_turn(
    connection: &impl Deref<Target = rusqlite::Connection>,
    turn: &AgentTurn,
) -> Result<(), SqliteStoreError> {
    let input = turn
        .usage_input_tokens()
        .map(i64::try_from)
        .transpose()
        .map_err(|_| SqliteStoreError::Numeric)?;
    let output = turn
        .usage_output_tokens()
        .map(i64::try_from)
        .transpose()
        .map_err(|_| SqliteStoreError::Numeric)?;
    let updated = connection.execute(
        "UPDATE agent_turns SET agent_text = ?2, state = ?3, error_code = ?4, provider_response_id = ?5,
             usage_input_tokens = ?6, usage_output_tokens = ?7, finished_at_unix_ms = ?8
         WHERE id = ?1 AND state IN ('pending', 'streaming')",
        params![turn.id().to_string(), turn.agent_text(), turn.state().as_str(), turn.error_code(),
            turn.provider_response_id(), input, output, turn.finished_at_unix_ms()],
    ).map_err(SqliteStoreError::Database)?;
    if updated != 1 {
        return Err(SqliteStoreError::Database(
            rusqlite::Error::QueryReturnedNoRows,
        ));
    }
    Ok(())
}

fn insert_event(
    connection: &impl Deref<Target = rusqlite::Connection>,
    event: &AgentEvent,
) -> Result<(), SqliteStoreError> {
    let sequence = i64::try_from(event.sequence()).map_err(|_| SqliteStoreError::Numeric)?;
    connection.execute(
        "INSERT INTO agent_events (id, session_id, turn_id, sequence, kind, created_at_unix_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![event.id().to_string(), event.session_id().to_string(), event.turn_id().map(|id| id.to_string()),
            sequence, event.kind().as_str(), event.created_at_unix_ms()],
    ).map_err(SqliteStoreError::Database)?;
    Ok(())
}

fn list_sessions(
    connection: &impl Deref<Target = rusqlite::Connection>,
    filter: &str,
) -> Result<Vec<AgentSession>, SqliteStoreError> {
    let mut statement = connection.prepare(&format!(
        "SELECT id, title, project_id, provider_profile_id, model_id, created_at_unix_ms, updated_at_unix_ms
         FROM agent_sessions{filter} ORDER BY updated_at_unix_ms DESC, id DESC"
    )).map_err(SqliteStoreError::Database)?;
    collect_sessions(
        statement
            .query_map([], session_row)
            .map_err(SqliteStoreError::Database)?,
    )
}

type SessionRow = (String, String, Option<String>, String, String, i64, i64);

fn session_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SessionRow> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
    ))
}
fn reconstruct_session(row: SessionRow) -> Result<AgentSession, SqliteStoreError> {
    AgentSession::from_stored_parts(&row.0, row.1, row.2.as_deref(), row.3, row.4, row.5, row.6)
        .map_err(SqliteStoreError::MalformedAgent)
}
fn collect_sessions(
    rows: impl Iterator<Item = rusqlite::Result<SessionRow>>,
) -> Result<Vec<AgentSession>, SqliteStoreError> {
    rows.map(|row| {
        row.map_err(SqliteStoreError::Database)
            .and_then(reconstruct_session)
    })
    .collect()
}

type TurnRow = (
    String,
    String,
    i64,
    String,
    String,
    String,
    Option<String>,
    String,
    String,
    String,
    Option<String>,
    Option<i64>,
    Option<i64>,
    Option<String>,
    String,
    String,
    i64,
    Option<i64>,
);
fn turn_select(suffix: &str) -> String {
    format!(
        "SELECT id, session_id, ordinal, user_text, agent_text, state, error_code, provider_profile_id, model_id, provider_request_id, provider_response_id, usage_input_tokens, usage_output_tokens, project_id, project_instructions, prompt_version, started_at_unix_ms, finished_at_unix_ms FROM agent_turns{suffix}"
    )
}
fn turn_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TurnRow> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
        row.get(9)?,
        row.get(10)?,
        row.get(11)?,
        row.get(12)?,
        row.get(13)?,
        row.get(14)?,
        row.get(15)?,
        row.get(16)?,
        row.get(17)?,
    ))
}
fn reconstruct_turn(row: TurnRow) -> Result<AgentTurn, SqliteStoreError> {
    AgentTurn::from_stored_parts(
        &row.0,
        &row.1,
        u64::try_from(row.2).map_err(|_| SqliteStoreError::Numeric)?,
        row.3,
        row.4,
        &row.5,
        row.6,
        row.7,
        row.8,
        &row.9,
        row.10,
        row.11
            .map(u64::try_from)
            .transpose()
            .map_err(|_| SqliteStoreError::Numeric)?,
        row.12
            .map(u64::try_from)
            .transpose()
            .map_err(|_| SqliteStoreError::Numeric)?,
        row.13.as_deref(),
        row.14,
        row.15,
        row.16,
        row.17,
    )
    .map_err(SqliteStoreError::MalformedAgent)
}
fn collect_turns(
    rows: impl Iterator<Item = rusqlite::Result<TurnRow>>,
) -> Result<Vec<AgentTurn>, SqliteStoreError> {
    rows.map(|row| {
        row.map_err(SqliteStoreError::Database)
            .and_then(reconstruct_turn)
    })
    .collect()
}

type EventRow = (String, String, Option<String>, i64, String, i64);
fn event_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<EventRow> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
    ))
}
fn reconstruct_event(row: EventRow) -> Result<AgentEvent, SqliteStoreError> {
    AgentEvent::from_stored_parts(
        &row.0,
        &row.1,
        row.2.as_deref(),
        u64::try_from(row.3).map_err(|_| SqliteStoreError::Numeric)?,
        &row.4,
        row.5,
    )
    .map_err(SqliteStoreError::MalformedAgent)
}

#[cfg(test)]
mod tests {
    use tule_core::{
        AgentEvent, AgentEventKind, AgentRepository, AgentSession, AgentTurn, AgentTurnState,
        PROVIDER_PROFILE_ID, ProviderRequestId,
    };

    use super::*;

    #[test]
    fn built_in_profile_and_partial_unicode_checkpoint_round_trip() {
        let directory = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(directory.path().join("agent.sqlite3")).unwrap();
        assert_eq!(
            store
                .get_provider_profile(PROVIDER_PROFILE_ID)
                .unwrap()
                .unwrap()
                .visible_model_id(),
            "gpt-5.5"
        );

        let session = AgentSession::new("Unicode session", None).unwrap();
        let mut turn = AgentTurn::new_pending(
            session.id(),
            0,
            "line one\r\n記録",
            None,
            "keep\r\nexact",
            ProviderRequestId::generate(),
        )
        .unwrap();
        let created =
            AgentEvent::new(session.id(), None, 0, AgentEventKind::SessionCreated).unwrap();
        let pending = AgentEvent::new(
            session.id(),
            Some(turn.id()),
            1,
            AgentEventKind::TurnPending,
        )
        .unwrap();
        store
            .create_session_with_first_turn(&session, &turn, &created, &pending)
            .unwrap();

        turn.append_agent_text("partial\r\n応答").unwrap();
        turn.mark_streaming().unwrap();
        let streaming = AgentEvent::new(
            session.id(),
            Some(turn.id()),
            2,
            AgentEventKind::TurnStreaming,
        )
        .unwrap();
        store.checkpoint_turn(&turn, Some(&streaming)).unwrap();

        assert_eq!(store.find_turn(&turn.id()).unwrap().unwrap(), turn);
        assert_eq!(
            store.list_events(&session.id()).unwrap(),
            vec![created, pending, streaming]
        );
    }

    #[test]
    fn agent_tables_are_strict_and_foreign_keyed() {
        let directory = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(directory.path().join("constraints.sqlite3")).unwrap();
        let connection = store.connection().unwrap();
        for table in [
            "provider_profiles",
            "agent_sessions",
            "agent_turns",
            "agent_events",
        ] {
            let strict: i64 = connection
                .query_row(
                    "SELECT strict FROM pragma_table_list WHERE name = ?1",
                    [table],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(strict, 1);
        }
        assert!(connection.execute(
            "INSERT INTO agent_sessions (id, title, provider_profile_id, model_id, created_at_unix_ms, updated_at_unix_ms)
             VALUES ('missing-profile', 'No profile', 'missing', 'model', 1, 1)", [],
        ).is_err());
    }

    #[test]
    fn migration_sql_contains_no_secret_value_columns() {
        let migration = include_str!("../../migrations/0003_agent_conversations.sql");
        for forbidden in [
            "access_token",
            "refresh_token",
            "authorization_code",
            "pkce",
            "jwt",
            "cookie",
            "auth_header",
        ] {
            assert!(
                !migration.contains(forbidden),
                "migration contains {forbidden}"
            );
        }
        assert!(migration.contains("credential_handle"));
        assert!(migration.contains("expires_at_unix_ms"));
    }

    #[test]
    fn project_association_and_event_roll_back_together() {
        let directory = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(directory.path().join("association.sqlite3")).unwrap();
        let project = tule_core::create_project(&store, "Project context").unwrap();
        let prepared = tule_core::prepare_agent_send(&store, None, "Hello", None, "").unwrap();
        tule_core::complete_agent_turn(&store, prepared.turn.id(), None, None, None).unwrap();

        let mut changed = store.find_session(&prepared.session.id()).unwrap().unwrap();
        changed.set_project_id(Some(project.id())).unwrap();
        let duplicate_sequence = store
            .list_events(&prepared.session.id())
            .unwrap()
            .last()
            .unwrap()
            .sequence();
        let conflicting = AgentEvent::new(
            prepared.session.id(),
            None,
            duplicate_sequence,
            AgentEventKind::ProjectAssociationChanged,
        )
        .unwrap();

        assert!(
            store
                .update_session_with_event(&changed, &conflicting)
                .is_err()
        );
        assert_eq!(
            store
                .find_session(&prepared.session.id())
                .unwrap()
                .unwrap()
                .project_id(),
            None
        );
    }

    #[test]
    fn stale_terminal_write_cannot_change_terminal_kind() {
        let directory = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(directory.path().join("terminal-cas.sqlite3")).unwrap();
        let prepared = tule_core::prepare_agent_send(&store, None, "Hello", None, "").unwrap();
        let mut stale = prepared.turn.clone();
        let completed =
            tule_core::complete_agent_turn(&store, prepared.turn.id(), None, None, None).unwrap();
        stale.cancel().unwrap();
        let session = store.find_session(&prepared.session.id()).unwrap().unwrap();
        let event = AgentEvent::new(
            prepared.session.id(),
            Some(stale.id()),
            store.next_event_sequence(&prepared.session.id()).unwrap(),
            AgentEventKind::TurnCancelled,
        )
        .unwrap();

        assert!(
            store
                .finish_turn_with_terminal_event(&session, &stale, &event)
                .is_err()
        );
        assert_eq!(
            store.find_turn(&stale.id()).unwrap().unwrap().state(),
            AgentTurnState::Completed
        );
        assert_eq!(
            store
                .list_events(&prepared.session.id())
                .unwrap()
                .iter()
                .filter(|event| matches!(
                    event.kind(),
                    AgentEventKind::TurnCompleted | AgentEventKind::TurnCancelled
                ))
                .count(),
            1
        );
        assert_eq!(completed.state(), AgentTurnState::Completed);
    }

    #[test]
    fn batch_interruption_rolls_back_every_turn_on_failure() {
        let directory = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(directory.path().join("recovery-atomic.sqlite3")).unwrap();
        let mut updates = Vec::new();
        let mut turn_ids = Vec::new();

        for index in 0..2 {
            let mut session = AgentSession::new(format!("Session {index}"), None).unwrap();
            let mut turn = AgentTurn::new_pending(
                session.id(),
                0,
                format!("Message {index}"),
                None,
                "",
                ProviderRequestId::generate(),
            )
            .unwrap();
            let created =
                AgentEvent::new(session.id(), None, 0, AgentEventKind::SessionCreated).unwrap();
            let pending = AgentEvent::new(
                session.id(),
                Some(turn.id()),
                1,
                AgentEventKind::TurnPending,
            )
            .unwrap();
            store
                .create_session_with_first_turn(&session, &turn, &created, &pending)
                .unwrap();
            turn_ids.push(turn.id());
            turn.interrupt().unwrap();
            session.touch_updated_at().unwrap();
            let sequence = if index == 0 { 2 } else { 1 };
            let interrupted = AgentEvent::new(
                session.id(),
                Some(turn.id()),
                sequence,
                AgentEventKind::TurnInterrupted,
            )
            .unwrap();
            updates.push((session, turn, interrupted));
        }

        assert!(store.finish_turns_with_terminal_events(&updates).is_err());
        for turn_id in turn_ids {
            assert_eq!(
                store.find_turn(&turn_id).unwrap().unwrap().state(),
                AgentTurnState::Pending
            );
        }
    }
}
