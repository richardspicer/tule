use std::{
    error::Error,
    fmt,
    path::Path,
    sync::{Mutex, MutexGuard},
};

use rusqlite::{Connection, OptionalExtension, params};
use rusqlite_migration::{M, Migrations};
use tule_core::{Project, ProjectId, ProjectReconstructionError, ProjectRepository};

pub(crate) const DATABASE_FILENAME: &str = "tule.sqlite3";

const MIGRATION_SET: &[M<'static>] = &[M::up(include_str!("../migrations/0001_projects.sql"))];
const MIGRATIONS: Migrations<'static> = Migrations::from_slice(MIGRATION_SET);

pub(crate) struct SqliteProjectRepository {
    connection: Mutex<Connection>,
}

impl SqliteProjectRepository {
    pub(crate) fn open(path: impl AsRef<Path>) -> Result<Self, SqliteProjectRepositoryError> {
        let mut connection =
            Connection::open(path).map_err(SqliteProjectRepositoryError::Database)?;
        connection
            .pragma_update(None, "foreign_keys", true)
            .map_err(SqliteProjectRepositoryError::Database)?;

        MIGRATIONS
            .validate()
            .map_err(SqliteProjectRepositoryError::Migration)?;
        MIGRATIONS
            .to_latest(&mut connection)
            .map_err(SqliteProjectRepositoryError::Migration)?;

        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    fn connection(&self) -> Result<MutexGuard<'_, Connection>, SqliteProjectRepositoryError> {
        self.connection
            .lock()
            .map_err(|_| SqliteProjectRepositoryError::LockPoisoned)
    }
}

impl ProjectRepository for SqliteProjectRepository {
    type Error = SqliteProjectRepositoryError;

    fn create(&self, project: &Project) -> Result<(), Self::Error> {
        self.connection()?
            .execute(
                "INSERT INTO projects (id, display_name, created_at_unix_ms) VALUES (?1, ?2, ?3)",
                params![
                    project.id().to_string(),
                    project.name().as_str(),
                    project.created_at_unix_ms()
                ],
            )
            .map_err(SqliteProjectRepositoryError::Database)?;
        Ok(())
    }

    fn list(&self) -> Result<Vec<Project>, Self::Error> {
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(
                "SELECT id, display_name, created_at_unix_ms \
                 FROM projects \
                 ORDER BY created_at_unix_ms ASC, id ASC",
            )
            .map_err(SqliteProjectRepositoryError::Database)?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })
            .map_err(SqliteProjectRepositoryError::Database)?;

        let mut projects = Vec::new();
        for row in rows {
            let (id, name, created_at_unix_ms) =
                row.map_err(SqliteProjectRepositoryError::Database)?;
            projects.push(reconstruct_stored_project(&id, &name, created_at_unix_ms)?);
        }
        Ok(projects)
    }

    fn find_by_id(&self, id: &ProjectId) -> Result<Option<Project>, Self::Error> {
        let connection = self.connection()?;
        let stored = connection
            .query_row(
                "SELECT id, display_name, created_at_unix_ms FROM projects WHERE id = ?1",
                [id.to_string()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(SqliteProjectRepositoryError::Database)?;

        stored
            .map(|(stored_id, name, created_at_unix_ms)| {
                reconstruct_stored_project(&stored_id, &name, created_at_unix_ms)
            })
            .transpose()
    }
}

fn reconstruct_stored_project(
    id: &str,
    name: &str,
    created_at_unix_ms: i64,
) -> Result<Project, SqliteProjectRepositoryError> {
    let project = Project::from_stored_parts(id, name, created_at_unix_ms).map_err(|error| {
        SqliteProjectRepositoryError::MalformedProject(StoredProjectError::Reconstruction(error))
    })?;
    if project.id().to_string() != id {
        return Err(SqliteProjectRepositoryError::MalformedProject(
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
pub(crate) enum SqliteProjectRepositoryError {
    Database(rusqlite::Error),
    Migration(rusqlite_migration::Error),
    MalformedProject(StoredProjectError),
    LockPoisoned,
}

impl fmt::Display for SqliteProjectRepositoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(error) => write!(formatter, "SQLite project storage failed: {error}"),
            Self::Migration(error) => write!(formatter, "project migration failed: {error}"),
            Self::MalformedProject(error) => {
                write!(
                    formatter,
                    "stored project could not be reconstructed: {error}"
                )
            }
            Self::LockPoisoned => formatter.write_str("project storage lock is poisoned"),
        }
    }
}

impl Error for SqliteProjectRepositoryError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Database(error) => Some(error),
            Self::Migration(error) => Some(error),
            Self::MalformedProject(error) => Some(error),
            Self::LockPoisoned => None,
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
        Project::from_stored_parts(&ProjectId::generate().to_string(), name, created_at_unix_ms)
            .unwrap()
    }

    fn open_repository(path: &Path) -> SqliteProjectRepository {
        SqliteProjectRepository::open(path).unwrap()
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
        assert_eq!(version, 1);
        drop(repository);

        let reopened = open_repository(&path);
        let version: i64 = reopened
            .connection
            .lock()
            .unwrap()
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, 1);
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
            .pragma_update(None, "user_version", 2_i64)
            .unwrap();
        drop(connection);

        assert!(SqliteProjectRepository::open(&path).is_err());

        let connection = Connection::open(&path).unwrap();
        let version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        let sentinel: String = connection
            .query_row("SELECT value FROM future_sentinel", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 2);
        assert_eq!(sentinel, "preserve me");
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
                Err(SqliteProjectRepositoryError::MalformedProject(_))
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
            Err(SqliteProjectRepositoryError::MalformedProject(_))
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
    fn create_list_and_open_survive_drop_and_reopen() {
        let directory = tempfile::tempdir().unwrap();
        let path = database_path(&directory);
        let repository = open_repository(&path);
        let project = tule_core::create_project(&repository, "Persistent project").unwrap();
        drop(repository);

        let reopened = open_repository(&path);
        assert_eq!(reopened.list().unwrap(), vec![project.clone()]);
        assert_eq!(reopened.find_by_id(&project.id()).unwrap(), Some(project));
    }
}
