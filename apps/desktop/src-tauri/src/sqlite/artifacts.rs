//! SQLite persistence for Artifacts and immutable Artifact Versions.

use rusqlite::{OptionalExtension, params};
use tule_core::{
    AgentSessionId, Artifact, ArtifactDetail, ArtifactId, ArtifactRepository, ArtifactSummary,
    ArtifactVersion, ArtifactVersionId, ArtifactVersionProvenance, ProjectId,
};

use super::{SqliteStore, SqliteStoreError};

impl ArtifactRepository for SqliteStore {
    type Error = SqliteStoreError;

    fn create_artifact_with_first_version(
        &self,
        artifact: &Artifact,
        version: &ArtifactVersion,
    ) -> Result<(), Self::Error> {
        if version.artifact_id() != artifact.id() {
            return Err(SqliteStoreError::MalformedArtifact(
                tule_core::ArtifactReconstructionError::VersionArtifactMismatch,
            ));
        }
        if version.version_ordinal() != 1 {
            return Err(SqliteStoreError::MalformedArtifact(
                tule_core::ArtifactReconstructionError::InvalidVersionOrdinal,
            ));
        }
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction()
            .map_err(SqliteStoreError::Database)?;
        insert_artifact(&transaction, artifact)?;
        insert_artifact_version(&transaction, version)?;
        transaction.commit().map_err(SqliteStoreError::Database)
    }

    fn get_artifact(&self, id: &ArtifactId) -> Result<Option<ArtifactDetail>, Self::Error> {
        let connection = self.connection()?;
        let artifact = match load_artifact(&connection, id)? {
            Some(artifact) => artifact,
            None => return Ok(None),
        };
        let versions = load_versions_for_artifact(&connection, id)?;
        Ok(Some(
            ArtifactDetail::new(artifact, versions).map_err(SqliteStoreError::MalformedArtifact)?,
        ))
    }

    fn list_artifacts_for_session_context(
        &self,
        session_id: &AgentSessionId,
        project_id: Option<&ProjectId>,
    ) -> Result<Vec<ArtifactSummary>, Self::Error> {
        let connection = self.connection()?;
        let project_id_text = project_id.map(ToString::to_string);
        let mut statement = connection
            .prepare(
                "SELECT a.id, a.title, a.kind, a.project_id, a.created_at_unix_ms,
                        v.id, v.version_ordinal
                 FROM artifacts a
                 INNER JOIN artifact_versions v
                   ON v.artifact_id = a.id
                  AND v.version_ordinal = (
                        SELECT MAX(version_ordinal)
                        FROM artifact_versions
                        WHERE artifact_id = a.id
                  )
                 WHERE a.id IN (
                    SELECT artifact_id FROM artifact_versions WHERE source_session_id = ?1
                    UNION
                    SELECT id FROM artifacts
                    WHERE ?2 IS NOT NULL AND project_id = ?2
                 )
                 ORDER BY a.created_at_unix_ms DESC, a.id DESC",
            )
            .map_err(SqliteStoreError::Database)?;
        let rows = statement
            .query_map(params![session_id.to_string(), project_id_text], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, i64>(6)?,
                ))
            })
            .map_err(SqliteStoreError::Database)?;
        let mut summaries = Vec::new();
        for row in rows {
            let (
                id,
                title,
                kind,
                project_id,
                created_at_unix_ms,
                latest_version_id,
                latest_version_ordinal,
            ) = row.map_err(SqliteStoreError::Database)?;
            let artifact = Artifact::from_stored_parts(
                &id,
                title,
                &kind,
                project_id.as_deref(),
                created_at_unix_ms,
            )
            .map_err(SqliteStoreError::MalformedArtifact)?;
            let latest_version_id =
                ArtifactVersionId::parse(&latest_version_id).map_err(|error| {
                    SqliteStoreError::MalformedArtifact(
                        tule_core::ArtifactReconstructionError::InvalidId(error),
                    )
                })?;
            let latest_version_ordinal =
                u64::try_from(latest_version_ordinal).map_err(|_| SqliteStoreError::Numeric)?;
            summaries.push(
                ArtifactSummary::new(artifact, latest_version_id, latest_version_ordinal)
                    .map_err(SqliteStoreError::MalformedArtifact)?,
            );
        }
        Ok(summaries)
    }
}

fn insert_artifact(
    connection: &rusqlite::Connection,
    artifact: &Artifact,
) -> Result<(), SqliteStoreError> {
    connection
        .execute(
            "INSERT INTO artifacts (id, title, kind, project_id, created_at_unix_ms)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                artifact.id().to_string(),
                artifact.title(),
                artifact.kind().as_str(),
                artifact.project_id().map(|id| id.to_string()),
                artifact.created_at_unix_ms()
            ],
        )
        .map_err(SqliteStoreError::Database)?;
    Ok(())
}

fn insert_artifact_version(
    connection: &rusqlite::Connection,
    version: &ArtifactVersion,
) -> Result<(), SqliteStoreError> {
    let provenance = version.provenance();
    connection
        .execute(
            "INSERT INTO artifact_versions (
                id, artifact_id, version_ordinal, content, content_sha256,
                source_session_id, source_turn_id, provider_profile_id, model_id,
                prompt_version, project_id, provider_request_id, created_at_unix_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                version.id().to_string(),
                version.artifact_id().to_string(),
                i64::try_from(version.version_ordinal()).map_err(|_| SqliteStoreError::Numeric)?,
                version.content(),
                version.content_sha256(),
                provenance.source_session_id().to_string(),
                provenance.source_turn_id().to_string(),
                provenance.provider_profile_id(),
                provenance.model_id(),
                provenance.prompt_version(),
                provenance.project_id().map(|id| id.to_string()),
                provenance.provider_request_id().to_string(),
                version.created_at_unix_ms()
            ],
        )
        .map_err(SqliteStoreError::Database)?;
    Ok(())
}

fn load_artifact(
    connection: &rusqlite::Connection,
    id: &ArtifactId,
) -> Result<Option<Artifact>, SqliteStoreError> {
    let stored = connection
        .query_row(
            "SELECT id, title, kind, project_id, created_at_unix_ms
             FROM artifacts WHERE id = ?1",
            [id.to_string()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            },
        )
        .optional()
        .map_err(SqliteStoreError::Database)?;
    stored
        .map(|(id, title, kind, project_id, created_at_unix_ms)| {
            Artifact::from_stored_parts(
                &id,
                title,
                &kind,
                project_id.as_deref(),
                created_at_unix_ms,
            )
            .map_err(SqliteStoreError::MalformedArtifact)
        })
        .transpose()
}

fn load_versions_for_artifact(
    connection: &rusqlite::Connection,
    artifact_id: &ArtifactId,
) -> Result<Vec<ArtifactVersion>, SqliteStoreError> {
    let mut statement = connection
        .prepare(
            "SELECT id, artifact_id, version_ordinal, content, content_sha256,
                    source_session_id, source_turn_id, provider_profile_id, model_id,
                    prompt_version, project_id, provider_request_id, created_at_unix_ms
             FROM artifact_versions
             WHERE artifact_id = ?1
             ORDER BY version_ordinal ASC",
        )
        .map_err(SqliteStoreError::Database)?;
    let rows = statement
        .query_map([artifact_id.to_string()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, String>(9)?,
                row.get::<_, Option<String>>(10)?,
                row.get::<_, String>(11)?,
                row.get::<_, i64>(12)?,
            ))
        })
        .map_err(SqliteStoreError::Database)?;
    let mut versions = Vec::new();
    for row in rows {
        let (
            id,
            stored_artifact_id,
            version_ordinal,
            content,
            content_sha256,
            source_session_id,
            source_turn_id,
            provider_profile_id,
            model_id,
            prompt_version,
            project_id,
            provider_request_id,
            created_at_unix_ms,
        ) = row.map_err(SqliteStoreError::Database)?;
        let version_ordinal =
            u64::try_from(version_ordinal).map_err(|_| SqliteStoreError::Numeric)?;
        let provenance = ArtifactVersionProvenance::from_stored_parts(
            &source_session_id,
            &source_turn_id,
            provider_profile_id,
            model_id,
            prompt_version,
            project_id.as_deref(),
            &provider_request_id,
        )
        .map_err(SqliteStoreError::MalformedArtifact)?;
        versions.push(
            ArtifactVersion::from_stored_parts(
                &id,
                &stored_artifact_id,
                version_ordinal,
                content,
                content_sha256,
                provenance,
                created_at_unix_ms,
            )
            .map_err(SqliteStoreError::MalformedArtifact)?,
        );
    }
    Ok(versions)
}

#[cfg(test)]
mod tests {
    use tule_core::{
        ArtifactKind, ArtifactRepository, apply_agent_delta, complete_agent_turn,
        create_artifact_from_turn, get_artifact, list_artifacts_for_session_context,
        prepare_agent_send,
    };

    use super::*;
    use crate::sqlite::SqliteStore;

    #[test]
    fn create_list_get_round_trip_and_reopen() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("artifacts.sqlite3");
        let store = SqliteStore::open(&path).unwrap();
        let prepared =
            prepare_agent_send(&store, None, "Save this", None, "", "grok-3", None).unwrap();
        apply_agent_delta(&store, prepared.turn.id(), "Frozen agent result").unwrap();
        let turn = complete_agent_turn(&store, prepared.turn.id(), None, None, None).unwrap();

        let (artifact, version) = create_artifact_from_turn(
            &store,
            &store,
            &turn.id().to_string(),
            Some("Custom title"),
            Some("decision_record"),
        )
        .unwrap();
        assert_eq!(artifact.kind(), ArtifactKind::DecisionRecord);
        assert_eq!(version.content(), "Frozen agent result");

        let listed =
            list_artifacts_for_session_context(&store, &turn.session_id().to_string(), None)
                .unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].artifact().id(), artifact.id());
        assert_eq!(listed[0].latest_version_id(), version.id());

        drop(store);
        let reopened = SqliteStore::open(&path).unwrap();
        let detail = get_artifact(&reopened, &artifact.id().to_string()).unwrap();
        assert_eq!(detail.artifact().title(), "Custom title");
        assert_eq!(detail.versions().len(), 1);
        assert_eq!(detail.versions()[0].content(), "Frozen agent result");
        assert_eq!(
            detail.versions()[0].content_sha256(),
            version.content_sha256()
        );
        assert_eq!(
            detail.versions()[0].provenance().source_turn_id(),
            turn.id()
        );
        assert_eq!(
            detail.versions()[0].provenance().source_session_id(),
            turn.session_id()
        );
        assert_eq!(
            detail.versions()[0].provenance().provider_request_id(),
            turn.provider_request_id()
        );
    }

    #[test]
    fn create_is_atomic_when_version_insert_would_fail() {
        let directory = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(directory.path().join("atomic.sqlite3")).unwrap();
        let prepared = prepare_agent_send(&store, None, "user", None, "", "grok-3", None).unwrap();
        apply_agent_delta(&store, prepared.turn.id(), "body").unwrap();
        let turn = complete_agent_turn(&store, prepared.turn.id(), None, None, None).unwrap();
        let (artifact, version) =
            create_artifact_from_turn(&store, &store, &turn.id().to_string(), None, None).unwrap();

        // Reconstruct a conflicting first version with a new id but same ordinal/artifact.
        let conflicting = ArtifactVersion::from_stored_parts(
            &ArtifactVersionId::generate().to_string(),
            &artifact.id().to_string(),
            1,
            version.content(),
            version.content_sha256(),
            version.provenance().clone(),
            version.created_at_unix_ms(),
        )
        .unwrap();
        let result = store.create_artifact_with_first_version(&artifact, &conflicting);
        assert!(result.is_err());

        // Original rows remain; no partial second artifact write occurred.
        let detail = store.get_artifact(&artifact.id()).unwrap().unwrap();
        assert_eq!(detail.versions().len(), 1);
        assert_eq!(detail.versions()[0].id(), version.id());
    }

    #[test]
    fn reject_paths_do_not_write_rows() {
        let directory = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(directory.path().join("reject.sqlite3")).unwrap();
        let prepared = prepare_agent_send(&store, None, "user", None, "", "grok-3", None).unwrap();
        assert!(
            create_artifact_from_turn(&store, &store, &prepared.turn.id().to_string(), None, None)
                .is_err()
        );

        let count: i64 = store
            .connection()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM artifacts", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
        let version_count: i64 = store
            .connection()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM artifact_versions", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(version_count, 0);
    }

    #[test]
    fn list_includes_project_union_and_session_sourced() {
        let directory = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(directory.path().join("list.sqlite3")).unwrap();
        let project = tule_core::create_project(&store, "Artifact project").unwrap();

        let session_a =
            prepare_agent_send(&store, None, "A", Some(project.id()), "", "grok-3", None).unwrap();
        apply_agent_delta(&store, session_a.turn.id(), "from session A").unwrap();
        let turn_a = complete_agent_turn(&store, session_a.turn.id(), None, None, None).unwrap();
        let (artifact_a, _) =
            create_artifact_from_turn(&store, &store, &turn_a.id().to_string(), None, None)
                .unwrap();

        let session_b =
            prepare_agent_send(&store, None, "B", Some(project.id()), "", "grok-3", None).unwrap();
        apply_agent_delta(&store, session_b.turn.id(), "from session B").unwrap();
        let turn_b = complete_agent_turn(&store, session_b.turn.id(), None, None, None).unwrap();
        let (artifact_b, _) =
            create_artifact_from_turn(&store, &store, &turn_b.id().to_string(), None, None)
                .unwrap();

        let listed_for_a = list_artifacts_for_session_context(
            &store,
            &turn_a.session_id().to_string(),
            Some(&project.id().to_string()),
        )
        .unwrap();
        let ids: Vec<_> = listed_for_a
            .iter()
            .map(|item| item.artifact().id())
            .collect();
        assert!(ids.contains(&artifact_a.id()));
        assert!(ids.contains(&artifact_b.id()));
    }

    #[test]
    fn artifact_tables_are_strict() {
        let directory = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(directory.path().join("strict.sqlite3")).unwrap();
        let connection = store.connection().unwrap();
        for table in ["artifacts", "artifact_versions"] {
            let strict: i64 = connection
                .query_row(
                    "SELECT strict FROM pragma_table_list WHERE name = ?1",
                    [table],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(strict, 1);
        }
    }
}
