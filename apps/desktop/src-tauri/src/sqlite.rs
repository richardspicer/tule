//! Shared serialized SQLite persistence for Projects and Agent conversations.

mod agents;
mod artifacts;
mod provider_models;

use std::{
    error::Error,
    fmt,
    path::Path,
    sync::{Mutex, MutexGuard},
    time::{SystemTime, UNIX_EPOCH},
};

use rusqlite::{Connection, OptionalExtension, params};
use rusqlite_migration::{M, Migrations};
use tule_core::{
    Project, ProjectId, ProjectReconstructionError, ProjectRepository, ProviderProfile,
};

use crate::{
    preferences::AppearancePreference,
    provider::{MODEL_ID, PROVIDER_PROFILE_ID},
};

pub(crate) use provider_models::StoredCatalogState;

pub(crate) const DATABASE_FILENAME: &str = "tule.sqlite3";

const MIGRATION_SET: &[M<'static>] = &[
    M::up(include_str!("../migrations/0001_projects.sql")),
    M::up(include_str!("../migrations/0002_project_instructions.sql")),
    M::up(include_str!("../migrations/0003_agent_conversations.sql")),
    M::up(include_str!("../migrations/0004_desktop_preferences.sql")),
    M::up(include_str!("../migrations/0005_provider_models.sql")),
    M::up(include_str!(
        "../migrations/0006_provider_rejected_models.sql"
    )),
    M::up(include_str!(
        "../migrations/0007_provider_catalog_quarantine.sql"
    )),
    M::up(include_str!("../migrations/0008_agent_sources.sql")),
    M::up(include_str!(
        "../migrations/0009_agent_source_member_count.sql"
    )),
    M::up(include_str!(
        "../migrations/0010_xai_subscription_provider.sql"
    )),
    M::up(include_str!(
        "../migrations/0011_agent_source_canonical_url.sql"
    )),
    M::up(include_str!("../migrations/0012_artifacts.sql")),
    M::up(include_str!("../migrations/0013_agent_turn_effort.sql")),
];
const MIGRATIONS: Migrations<'static> = Migrations::from_slice(MIGRATION_SET);

/// The single, synchronized SQLite owner used by all desktop repositories.
pub(crate) struct SqliteStore {
    connection: Mutex<Connection>,
    #[cfg(test)]
    fail_catalog_invalidation: std::sync::atomic::AtomicBool,
    #[cfg(test)]
    fail_model_selection_write: std::sync::atomic::AtomicBool,
    #[cfg(test)]
    fail_catalog_scrub: std::sync::atomic::AtomicBool,
}

impl SqliteStore {
    pub(crate) fn open(path: impl AsRef<Path>) -> Result<Self, SqliteStoreError> {
        let mut connection = Connection::open(path).map_err(SqliteStoreError::Database)?;
        connection
            .pragma_update(None, "foreign_keys", true)
            .map_err(SqliteStoreError::Database)?;
        MIGRATIONS.validate().map_err(SqliteStoreError::Migration)?;
        MIGRATIONS
            .to_latest(&mut connection)
            .map_err(SqliteStoreError::Migration)?;

        let store = Self {
            connection: Mutex::new(connection),
            #[cfg(test)]
            fail_catalog_invalidation: std::sync::atomic::AtomicBool::new(false),
            #[cfg(test)]
            fail_model_selection_write: std::sync::atomic::AtomicBool::new(false),
            #[cfg(test)]
            fail_catalog_scrub: std::sync::atomic::AtomicBool::new(false),
        };
        store.ensure_builtin_provider_profile()?;
        store.ensure_builtin_model_selection()?;
        Ok(store)
    }

    /// Durably hides catalog and selection state before a credential identity
    /// transition. Presence of the row is the seal, so it survives restart.
    pub(crate) fn seal_catalog_reads(&self) -> Result<(), SqliteStoreError> {
        self.connection()?
            .execute(
                "INSERT OR IGNORE INTO provider_model_catalog_quarantine (provider_profile_id)
                 VALUES (?1)",
                params![PROVIDER_PROFILE_ID],
            )
            .map_err(SqliteStoreError::Database)?;
        Ok(())
    }

    pub(crate) fn clear_catalog_read_seal(&self) -> Result<(), SqliteStoreError> {
        self.connection()?
            .execute(
                "DELETE FROM provider_model_catalog_quarantine
                 WHERE provider_profile_id = ?1",
                params![PROVIDER_PROFILE_ID],
            )
            .map_err(SqliteStoreError::Database)?;
        Ok(())
    }

    pub(crate) fn catalog_reads_are_sealed(&self) -> Result<bool, SqliteStoreError> {
        self.connection()?
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM provider_model_catalog_quarantine
                    WHERE provider_profile_id = ?1
                 )",
                params![PROVIDER_PROFILE_ID],
                |row| row.get(0),
            )
            .map_err(SqliteStoreError::Database)
    }

    #[cfg(test)]
    pub(crate) fn set_fail_catalog_invalidation(&self, fail: bool) {
        self.fail_catalog_invalidation
            .store(fail, std::sync::atomic::Ordering::SeqCst);
    }

    #[cfg(test)]
    pub(crate) fn set_fail_model_selection_write(&self, fail: bool) {
        self.fail_model_selection_write
            .store(fail, std::sync::atomic::Ordering::SeqCst);
    }

    #[cfg(test)]
    pub(crate) fn set_fail_catalog_scrub(&self, fail: bool) {
        self.fail_catalog_scrub
            .store(fail, std::sync::atomic::Ordering::SeqCst);
    }

    pub(super) fn connection(&self) -> Result<MutexGuard<'_, Connection>, SqliteStoreError> {
        self.connection
            .lock()
            .map_err(|_| SqliteStoreError::LockPoisoned)
    }

    fn ensure_builtin_provider_profile(&self) -> Result<(), SqliteStoreError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| SqliteStoreError::Clock)?
            .as_millis()
            .try_into()
            .map_err(|_| SqliteStoreError::Clock)?;
        let profile =
            ProviderProfile::built_in(PROVIDER_PROFILE_ID, PROVIDER_PROFILE_ID, MODEL_ID, now);
        <Self as tule_core::AgentRepository>::ensure_provider_profile(self, &profile)
    }

    pub(crate) fn get_appearance_preference(
        &self,
    ) -> Result<AppearancePreference, SqliteStoreError> {
        let connection = self.connection()?;
        let value = connection
            .query_row(
                "SELECT value FROM appearance_preference WHERE id = 1",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(SqliteStoreError::Database)?;

        Ok(value.map_or(AppearancePreference::System, |stored| {
            AppearancePreference::parse(&stored)
        }))
    }

    pub(crate) fn set_appearance_preference(
        &self,
        preference: AppearancePreference,
    ) -> Result<(), SqliteStoreError> {
        self.connection()?
            .execute(
                "INSERT INTO appearance_preference (id, value) VALUES (1, ?1)
                 ON CONFLICT(id) DO UPDATE SET value = excluded.value",
                params![preference.as_str()],
            )
            .map_err(SqliteStoreError::Database)?;
        Ok(())
    }
}

impl ProjectRepository for SqliteStore {
    type Error = SqliteStoreError;

    fn create(&self, project: &Project) -> Result<(), Self::Error> {
        self.connection()?
            .execute(
                "INSERT INTO projects (id, display_name, instructions, created_at_unix_ms) \
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    project.id().to_string(),
                    project.name().as_str(),
                    project.instructions(),
                    project.created_at_unix_ms()
                ],
            )
            .map_err(SqliteStoreError::Database)?;
        Ok(())
    }

    fn list(&self) -> Result<Vec<Project>, Self::Error> {
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(
                "SELECT id, display_name, instructions, created_at_unix_ms \
                 FROM projects \
                 ORDER BY created_at_unix_ms ASC, id ASC",
            )
            .map_err(SqliteStoreError::Database)?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })
            .map_err(SqliteStoreError::Database)?;

        let mut projects = Vec::new();
        for row in rows {
            let (id, name, instructions, created_at_unix_ms) =
                row.map_err(SqliteStoreError::Database)?;
            projects.push(reconstruct_stored_project(
                &id,
                &name,
                &instructions,
                created_at_unix_ms,
            )?);
        }
        Ok(projects)
    }

    fn find_by_id(&self, id: &ProjectId) -> Result<Option<Project>, Self::Error> {
        let connection = self.connection()?;
        let stored = connection
            .query_row(
                "SELECT id, display_name, instructions, created_at_unix_ms \
                 FROM projects WHERE id = ?1",
                [id.to_string()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                },
            )
            .optional()
            .map_err(SqliteStoreError::Database)?;

        stored
            .map(|(stored_id, name, instructions, created_at_unix_ms)| {
                reconstruct_stored_project(&stored_id, &name, &instructions, created_at_unix_ms)
            })
            .transpose()
    }

    fn update_instructions(
        &self,
        id: &ProjectId,
        instructions: &str,
    ) -> Result<Option<Project>, Self::Error> {
        let connection = self.connection()?;
        let updated = connection
            .execute(
                "UPDATE projects SET instructions = ?1 WHERE id = ?2",
                params![instructions, id.to_string()],
            )
            .map_err(SqliteStoreError::Database)?;

        if updated == 0 {
            return Ok(None);
        }

        let (stored_id, name, stored_instructions, created_at_unix_ms) = connection
            .query_row(
                "SELECT id, display_name, instructions, created_at_unix_ms \
                 FROM projects WHERE id = ?1",
                [id.to_string()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                },
            )
            .map_err(SqliteStoreError::Database)?;

        reconstruct_stored_project(&stored_id, &name, &stored_instructions, created_at_unix_ms)
            .map(Some)
    }
}

pub(super) fn reconstruct_stored_project(
    id: &str,
    name: &str,
    instructions: &str,
    created_at_unix_ms: i64,
) -> Result<Project, SqliteStoreError> {
    let project = Project::from_stored_parts(id, name, instructions, created_at_unix_ms).map_err(
        |error| SqliteStoreError::MalformedProject(StoredProjectError::Reconstruction(error)),
    )?;
    if project.id().to_string() != id {
        return Err(SqliteStoreError::MalformedProject(
            StoredProjectError::NonCanonicalProjectId,
        ));
    }

    Ok(project)
}

#[derive(Debug)]
pub(crate) enum StoredProjectError {
    Reconstruction(ProjectReconstructionError),
    NonCanonicalProjectId,
}

impl fmt::Display for StoredProjectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Reconstruction(error) => error.fmt(formatter),
            Self::NonCanonicalProjectId => {
                formatter.write_str("persisted project ID is not in canonical form")
            }
        }
    }
}

impl Error for StoredProjectError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Reconstruction(error) => Some(error),
            Self::NonCanonicalProjectId => None,
        }
    }
}

#[derive(Debug)]
pub(crate) enum SqliteStoreError {
    Database(rusqlite::Error),
    Migration(rusqlite_migration::Error),
    MalformedProject(StoredProjectError),
    MalformedAgent(tule_core::AgentReconstructionError),
    MalformedSource(tule_core::SourceReconstructionError),
    MalformedArtifact(tule_core::ArtifactReconstructionError),
    LockPoisoned,
    Clock,
    Numeric,
}

impl fmt::Display for SqliteStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(error) => write!(formatter, "SQLite storage failed: {error}"),
            Self::Migration(error) => write!(formatter, "SQLite migration failed: {error}"),
            Self::MalformedProject(error) => {
                write!(
                    formatter,
                    "stored project could not be reconstructed: {error}"
                )
            }
            Self::MalformedAgent(error) => {
                write!(
                    formatter,
                    "stored agent record could not be reconstructed: {error}"
                )
            }
            Self::MalformedSource(error) => {
                write!(
                    formatter,
                    "stored source record could not be reconstructed: {error}"
                )
            }
            Self::MalformedArtifact(error) => {
                write!(
                    formatter,
                    "stored artifact record could not be reconstructed: {error}"
                )
            }
            Self::LockPoisoned => formatter.write_str("SQLite storage lock is poisoned"),
            Self::Clock => formatter.write_str("system clock cannot initialize provider profile"),
            Self::Numeric => formatter.write_str("stored numeric value is out of range"),
        }
    }
}

impl Error for SqliteStoreError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Database(error) => Some(error),
            Self::Migration(error) => Some(error),
            Self::MalformedProject(error) => Some(error),
            Self::MalformedAgent(error) => Some(error),
            Self::MalformedSource(error) => Some(error),
            Self::MalformedArtifact(error) => Some(error),
            Self::LockPoisoned | Self::Clock | Self::Numeric => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use rusqlite::ErrorCode;
    use tempfile::TempDir;

    use super::*;

    fn database_path(directory: &TempDir) -> PathBuf {
        directory.path().join(DATABASE_FILENAME)
    }

    fn stored_project(name: &str, created_at_unix_ms: i64) -> Project {
        Project::from_stored_parts(
            &ProjectId::generate().to_string(),
            name,
            "",
            created_at_unix_ms,
        )
        .unwrap()
    }

    fn open_repository(path: &Path) -> SqliteStore {
        SqliteStore::open(path).unwrap()
    }

    #[test]
    fn numbered_migration_set_validates() {
        MIGRATIONS.validate().unwrap();
    }

    #[test]
    fn fresh_and_repeated_initialization_reach_the_latest_schema() {
        let directory = tempfile::tempdir().unwrap();
        let path = database_path(&directory);

        let repository = open_repository(&path);
        let version: i64 = repository
            .connection
            .lock()
            .unwrap()
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, 13);
        drop(repository);

        let reopened = open_repository(&path);
        let version: i64 = reopened
            .connection
            .lock()
            .unwrap()
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, 13);
    }

    #[test]
    fn version_one_projects_migrate_with_empty_instructions() {
        let directory = tempfile::tempdir().unwrap();
        let path = database_path(&directory);
        let id = ProjectId::generate().to_string();
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(include_str!("../migrations/0001_projects.sql"))
            .unwrap();
        connection
            .execute(
                "INSERT INTO projects (id, display_name, created_at_unix_ms) VALUES (?1, ?2, ?3)",
                params![id, "Existing project", 42_i64],
            )
            .unwrap();
        connection
            .pragma_update(None, "user_version", 1_i64)
            .unwrap();
        drop(connection);

        let repository = open_repository(&path);
        let projects = repository.list().unwrap();
        let version: i64 = repository
            .connection
            .lock()
            .unwrap()
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();

        assert_eq!(version, 13);
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].id().to_string(), id);
        assert_eq!(projects[0].name().as_str(), "Existing project");
        assert_eq!(projects[0].instructions(), "");
        assert_eq!(projects[0].created_at_unix_ms(), 42);
    }

    #[test]
    fn version_two_projects_preserve_exact_instructions() {
        let directory = tempfile::tempdir().unwrap();
        let path = database_path(&directory);
        let id = ProjectId::generate().to_string();
        let instructions = "Keep\r\nunicode: 記録";
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(include_str!("../migrations/0001_projects.sql"))
            .unwrap();
        connection
            .execute_batch(include_str!("../migrations/0002_project_instructions.sql"))
            .unwrap();
        connection.execute(
            "INSERT INTO projects (id, display_name, instructions, created_at_unix_ms) VALUES (?1, ?2, ?3, ?4)",
            params![id, "Existing project", instructions, 42_i64],
        ).unwrap();
        connection
            .pragma_update(None, "user_version", 2_i64)
            .unwrap();
        drop(connection);

        let repository = open_repository(&path);
        assert_eq!(repository.list().unwrap()[0].instructions(), instructions);
        let version: i64 = repository
            .connection()
            .unwrap()
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, 13);
    }

    #[test]
    fn initialization_rejects_a_database_from_a_future_schema() {
        let directory = tempfile::tempdir().unwrap();
        let path = database_path(&directory);
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE future_sentinel (value TEXT NOT NULL) STRICT;
                 INSERT INTO future_sentinel (value) VALUES ('preserve me');",
            )
            .unwrap();
        connection
            .pragma_update(None, "user_version", 8_i64)
            .unwrap();
        drop(connection);

        assert!(SqliteStore::open(&path).is_err());

        let connection = Connection::open(&path).unwrap();
        let version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        let sentinel: String = connection
            .query_row("SELECT value FROM future_sentinel", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 8);
        assert_eq!(sentinel, "preserve me");
    }

    #[test]
    fn appearance_preference_persists_and_missing_rows_resolve_to_system() {
        let directory = tempfile::tempdir().unwrap();
        let store = open_repository(&database_path(&directory));

        assert_eq!(
            store.get_appearance_preference().unwrap(),
            crate::preferences::AppearancePreference::System
        );
        store
            .set_appearance_preference(crate::preferences::AppearancePreference::Dark)
            .unwrap();
        assert_eq!(
            store.get_appearance_preference().unwrap(),
            crate::preferences::AppearancePreference::Dark
        );

        {
            let connection = store.connection.lock().unwrap();
            connection
                .execute("DELETE FROM appearance_preference", [])
                .unwrap();
        }
        assert_eq!(
            store.get_appearance_preference().unwrap(),
            crate::preferences::AppearancePreference::System
        );
    }

    #[test]
    fn projects_table_is_strict() {
        let directory = tempfile::tempdir().unwrap();
        let repository = open_repository(&database_path(&directory));
        let connection = repository.connection.lock().unwrap();
        let strict: i64 = connection
            .query_row(
                "SELECT strict FROM pragma_table_list WHERE name = 'projects'",
                [],
                |row| row.get(0),
            )
            .unwrap();

        assert_eq!(strict, 1);
        assert!(
            connection
                .execute(
                    "INSERT INTO projects (id, display_name, created_at_unix_ms) VALUES (?1, ?2, ?3)",
                    params![ProjectId::generate().to_string(), vec![0_u8], 1_i64],
                )
                .is_err()
        );
    }

    #[test]
    fn foreign_keys_are_enabled_and_enforced() {
        let directory = tempfile::tempdir().unwrap();
        let repository = open_repository(&database_path(&directory));
        let connection = repository.connection.lock().unwrap();
        let enabled: i64 = connection
            .pragma_query_value(None, "foreign_keys", |row| row.get(0))
            .unwrap();
        assert_eq!(enabled, 1);

        connection
            .execute_batch(
                "CREATE TABLE parents (id INTEGER PRIMARY KEY) STRICT;
                 CREATE TABLE children (
                     id INTEGER PRIMARY KEY,
                     parent_id INTEGER NOT NULL REFERENCES parents(id)
                 ) STRICT;",
            )
            .unwrap();
        let error = connection
            .execute("INSERT INTO children (id, parent_id) VALUES (1, 999)", [])
            .unwrap_err();
        assert_eq!(
            error.sqlite_error_code(),
            Some(ErrorCode::ConstraintViolation)
        );
    }

    #[test]
    fn list_uses_creation_time_then_identifier_for_deterministic_ordering() {
        let directory = tempfile::tempdir().unwrap();
        let repository = open_repository(&database_path(&directory));
        let first = stored_project("First", 10);
        let tied_a = stored_project("Tied A", 20);
        let tied_b = stored_project("Tied B", 20);

        repository.create(&tied_b).unwrap();
        repository.create(&first).unwrap();
        repository.create(&tied_a).unwrap();

        let mut tied = vec![tied_a, tied_b];
        tied.sort_by_key(Project::id);
        let expected = vec![first, tied.remove(0), tied.remove(0)];
        assert_eq!(repository.list().unwrap(), expected);
    }

    #[test]
    fn malformed_identifier_and_name_rows_are_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let repository = open_repository(&database_path(&directory));

        let uppercase_v7 = "01890F47-64B0-7CC1-98E8-BB5D4A3B1234";
        let hyphenless_v7 = "01890f4764b07cc198e8bb5d4a3b1234";
        assert!(ProjectId::parse(uppercase_v7).is_ok());
        assert!(ProjectId::parse(hyphenless_v7).is_ok());

        for id in ["not-a-uuid", uppercase_v7, hyphenless_v7] {
            {
                let connection = repository.connection.lock().unwrap();
                connection
                    .execute(
                        "INSERT INTO projects (id, display_name, created_at_unix_ms) VALUES (?1, ?2, ?3)",
                        params![id, "Valid", 1_i64],
                    )
                    .unwrap();
            }
            assert!(matches!(
                repository.list(),
                Err(SqliteStoreError::MalformedProject(_))
            ));
            repository
                .connection
                .lock()
                .unwrap()
                .execute("DELETE FROM projects", [])
                .unwrap();
        }

        {
            let connection = repository.connection.lock().unwrap();
            connection
                .execute(
                    "INSERT INTO projects (id, display_name, created_at_unix_ms) VALUES (?1, ?2, ?3)",
                    params![ProjectId::generate().to_string(), "", 2_i64],
                )
                .unwrap();
        }
        assert!(matches!(
            repository.list(),
            Err(SqliteStoreError::MalformedProject(_))
        ));
    }

    #[test]
    fn quoted_sql_shaped_names_round_trip_as_data() {
        let directory = tempfile::tempdir().unwrap();
        let repository = open_repository(&database_path(&directory));
        let project = stored_project("Robert'); DROP TABLE projects; --", 42);

        repository.create(&project).unwrap();

        assert_eq!(repository.list().unwrap(), vec![project]);
    }

    #[test]
    fn instruction_update_with_unicode_and_multiple_lines_survives_reopen() {
        let directory = tempfile::tempdir().unwrap();
        let path = database_path(&directory);
        let repository = open_repository(&path);
        let project = tule_core::create_project(&repository, "Persistent project").unwrap();
        let instructions = "Explore the evidence.\n记录 the open questions.\nKeep ‘why’ visible.";
        let updated = repository
            .update_instructions(&project.id(), instructions)
            .unwrap()
            .unwrap();
        assert_eq!(updated.instructions(), instructions);
        drop(repository);

        let reopened = open_repository(&path);
        assert_eq!(reopened.list().unwrap(), vec![updated.clone()]);
        assert_eq!(reopened.find_by_id(&project.id()).unwrap(), Some(updated));
    }

    #[test]
    fn instruction_update_returns_none_for_a_missing_project() {
        let directory = tempfile::tempdir().unwrap();
        let repository = open_repository(&database_path(&directory));
        let missing_id = ProjectId::generate();

        assert_eq!(
            repository
                .update_instructions(&missing_id, "Do not create a row")
                .unwrap(),
            None
        );
        assert!(repository.list().unwrap().is_empty());
    }
}
