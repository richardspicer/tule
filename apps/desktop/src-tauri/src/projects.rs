use std::sync::Arc;

use serde::Serialize;
use tauri::State;
use tule_core::{
    CreateProjectError, OpenProjectError, Project, ProjectRepository,
    UpdateProjectInstructionsError,
};

use crate::sqlite::SqliteProjectRepository;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProjectResponse {
    id: String,
    display_name: String,
    instructions: String,
}

impl From<Project> for ProjectResponse {
    fn from(project: Project) -> Self {
        Self {
            id: project.id().to_string(),
            display_name: project.name().as_str().to_owned(),
            instructions: project.instructions().to_owned(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProjectCommandError {
    InvalidProjectName,
    InvalidProjectId,
    ProjectNotFound,
    ProjectStorageUnavailable,
}

pub(crate) enum ProjectStorageState {
    Ready(Arc<SqliteProjectRepository>),
    Unavailable,
}

impl ProjectStorageState {
    pub(crate) fn ready(repository: SqliteProjectRepository) -> Self {
        Self::Ready(Arc::new(repository))
    }

    pub(crate) fn unavailable() -> Self {
        Self::Unavailable
    }

    fn repository(&self) -> Result<Arc<SqliteProjectRepository>, ProjectCommandError> {
        match self {
            Self::Ready(repository) => Ok(Arc::clone(repository)),
            Self::Unavailable => Err(ProjectCommandError::ProjectStorageUnavailable),
        }
    }
}

pub(crate) fn handle_create_project<R>(
    repository: &R,
    display_name: &str,
) -> Result<ProjectResponse, ProjectCommandError>
where
    R: ProjectRepository + ?Sized,
{
    tule_core::create_project(repository, display_name)
        .map(ProjectResponse::from)
        .map_err(|error| match error {
            CreateProjectError::InvalidName(_) => ProjectCommandError::InvalidProjectName,
            CreateProjectError::Time(_) | CreateProjectError::Repository(_) => {
                ProjectCommandError::ProjectStorageUnavailable
            }
        })
}

pub(crate) fn handle_list_projects<R>(
    repository: &R,
) -> Result<Vec<ProjectResponse>, ProjectCommandError>
where
    R: ProjectRepository + ?Sized,
{
    tule_core::list_projects(repository)
        .map(|projects| projects.into_iter().map(ProjectResponse::from).collect())
        .map_err(|_| ProjectCommandError::ProjectStorageUnavailable)
}

pub(crate) fn handle_open_project<R>(
    repository: &R,
    project_id: &str,
) -> Result<ProjectResponse, ProjectCommandError>
where
    R: ProjectRepository + ?Sized,
{
    tule_core::open_project(repository, project_id)
        .map(ProjectResponse::from)
        .map_err(|error| match error {
            OpenProjectError::InvalidId(_) => ProjectCommandError::InvalidProjectId,
            OpenProjectError::NotFound(_) => ProjectCommandError::ProjectNotFound,
            OpenProjectError::Repository(_) => ProjectCommandError::ProjectStorageUnavailable,
        })
}

pub(crate) fn handle_update_project_instructions<R>(
    repository: &R,
    project_id: &str,
    instructions: &str,
) -> Result<ProjectResponse, ProjectCommandError>
where
    R: ProjectRepository + ?Sized,
{
    tule_core::update_project_instructions(repository, project_id, instructions)
        .map(ProjectResponse::from)
        .map_err(|error| match error {
            UpdateProjectInstructionsError::InvalidId(_) => ProjectCommandError::InvalidProjectId,
            UpdateProjectInstructionsError::NotFound(_) => ProjectCommandError::ProjectNotFound,
            UpdateProjectInstructionsError::Repository(_) => {
                ProjectCommandError::ProjectStorageUnavailable
            }
        })
}

async fn dispatch_project_operation<T, F>(operation: F) -> Result<T, ProjectCommandError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, ProjectCommandError> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(operation)
        .await
        .map_err(|_| ProjectCommandError::ProjectStorageUnavailable)?
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn create_project(
    display_name: String,
    state: State<'_, ProjectStorageState>,
) -> Result<ProjectResponse, ProjectCommandError> {
    let repository = state.repository()?;
    dispatch_project_operation(move || handle_create_project(repository.as_ref(), &display_name))
        .await
}

#[tauri::command]
pub(crate) async fn list_projects(
    state: State<'_, ProjectStorageState>,
) -> Result<Vec<ProjectResponse>, ProjectCommandError> {
    let repository = state.repository()?;
    dispatch_project_operation(move || handle_list_projects(repository.as_ref())).await
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn open_project(
    project_id: String,
    state: State<'_, ProjectStorageState>,
) -> Result<ProjectResponse, ProjectCommandError> {
    let repository = state.repository()?;
    dispatch_project_operation(move || handle_open_project(repository.as_ref(), &project_id)).await
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn update_project_instructions(
    project_id: String,
    instructions: String,
    state: State<'_, ProjectStorageState>,
) -> Result<ProjectResponse, ProjectCommandError> {
    let repository = state.repository()?;
    dispatch_project_operation(move || {
        handle_update_project_instructions(repository.as_ref(), &project_id, &instructions)
    })
    .await
}

#[cfg(test)]
mod tests {
    use std::{
        error::Error,
        fmt,
        sync::{Arc, Mutex},
    };

    use serde::Serialize;
    use serde_json::{Value, json};
    use tule_core::{ProjectId, ProjectRepository};

    use super::*;

    const SENTINEL_INTERNAL_ERROR: &str =
        "SELECT secret FROM sqlite_master; C:\\Users\\operator\\private\\tule.sqlite3";
    const WRONG_VARIANT_VERSION_SEVEN_ID: &str = "01890f47-64b0-7cc1-18e8-bb5d4a3b1234";

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct SentinelRepositoryError;

    impl fmt::Display for SentinelRepositoryError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str(SENTINEL_INTERNAL_ERROR)
        }
    }

    impl Error for SentinelRepositoryError {}

    #[derive(Default)]
    struct FakeRepository {
        projects: Mutex<Vec<Project>>,
        fail: bool,
    }

    impl FakeRepository {
        fn with_projects(projects: Vec<Project>) -> Self {
            Self {
                projects: Mutex::new(projects),
                fail: false,
            }
        }

        fn failing() -> Self {
            Self {
                projects: Mutex::new(Vec::new()),
                fail: true,
            }
        }
    }

    impl ProjectRepository for FakeRepository {
        type Error = SentinelRepositoryError;

        fn create(&self, project: &Project) -> Result<(), Self::Error> {
            if self.fail {
                return Err(SentinelRepositoryError);
            }
            self.projects.lock().unwrap().push(project.clone());
            Ok(())
        }

        fn list(&self) -> Result<Vec<Project>, Self::Error> {
            if self.fail {
                return Err(SentinelRepositoryError);
            }
            Ok(self.projects.lock().unwrap().clone())
        }

        fn find_by_id(&self, id: &ProjectId) -> Result<Option<Project>, Self::Error> {
            if self.fail {
                return Err(SentinelRepositoryError);
            }
            Ok(self
                .projects
                .lock()
                .unwrap()
                .iter()
                .find(|project| project.id() == *id)
                .cloned())
        }

        fn update_instructions(
            &self,
            id: &ProjectId,
            instructions: &str,
        ) -> Result<Option<Project>, Self::Error> {
            if self.fail {
                return Err(SentinelRepositoryError);
            }

            let mut projects = self.projects.lock().unwrap();
            let Some(index) = projects.iter().position(|project| project.id() == *id) else {
                return Ok(None);
            };
            let current = &projects[index];
            let updated = Project::from_stored_parts(
                &current.id().to_string(),
                current.name().as_str(),
                instructions,
                current.created_at_unix_ms(),
            )
            .unwrap();
            projects[index] = updated.clone();
            Ok(Some(updated))
        }
    }

    fn stored_project(name: &str, created_at_unix_ms: i64) -> Project {
        stored_project_with_instructions(name, "", created_at_unix_ms)
    }

    fn stored_project_with_instructions(
        name: &str,
        instructions: &str,
        created_at_unix_ms: i64,
    ) -> Project {
        Project::from_stored_parts(
            &ProjectId::generate().to_string(),
            name,
            instructions,
            created_at_unix_ms,
        )
        .unwrap()
    }

    fn serialized(value: &impl Serialize) -> Value {
        serde_json::to_value(value).unwrap()
    }

    #[test]
    fn public_error_codes_are_exact_snake_case_strings() {
        let cases = [
            (
                ProjectCommandError::InvalidProjectName,
                json!("invalid_project_name"),
            ),
            (
                ProjectCommandError::InvalidProjectId,
                json!("invalid_project_id"),
            ),
            (
                ProjectCommandError::ProjectNotFound,
                json!("project_not_found"),
            ),
            (
                ProjectCommandError::ProjectStorageUnavailable,
                json!("project_storage_unavailable"),
            ),
        ];

        for (error, expected) in cases {
            assert_eq!(serialized(&error), expected);
        }
    }

    #[test]
    fn create_handler_returns_exact_project_json() {
        let repository = FakeRepository::default();
        let response = handle_create_project(&repository, "  First project  ").unwrap();

        assert_eq!(
            serialized(&response),
            json!({
                "id": response.id,
                "displayName": "First project",
                "instructions": "",
            })
        );
    }

    #[test]
    fn list_handler_returns_exact_ordered_project_json() {
        let first = stored_project("First", 10);
        let second = stored_project_with_instructions("Second", "Continue here.", 20);
        let repository = FakeRepository::with_projects(vec![first.clone(), second.clone()]);

        let response = handle_list_projects(&repository).unwrap();

        assert_eq!(
            serialized(&response),
            json!([
                { "id": first.id().to_string(), "displayName": "First", "instructions": "" },
                {
                    "id": second.id().to_string(),
                    "displayName": "Second",
                    "instructions": "Continue here."
                },
            ])
        );
    }

    #[test]
    fn open_handler_returns_exact_project_json() {
        let project = stored_project_with_instructions("Stored", "Review the record.", 42);
        let repository = FakeRepository::with_projects(vec![project.clone()]);

        let response = handle_open_project(&repository, &project.id().to_string()).unwrap();

        assert_eq!(
            serialized(&response),
            json!({
                "id": project.id().to_string(),
                "displayName": "Stored",
                "instructions": "Review the record.",
            })
        );
    }

    #[test]
    fn update_instructions_handler_returns_exact_project_json() {
        let project = stored_project("Stored", 42);
        let project_id = project.id().to_string();
        let repository = FakeRepository::with_projects(vec![project]);
        let instructions = "Ask why.\n保留 evidence.";

        let response =
            handle_update_project_instructions(&repository, &project_id, instructions).unwrap();

        assert_eq!(
            serialized(&response),
            json!({
                "id": project_id,
                "displayName": "Stored",
                "instructions": instructions,
            })
        );
    }

    #[test]
    fn handlers_return_exact_input_and_not_found_errors() {
        let repository = FakeRepository::default();
        let missing_id = ProjectId::generate().to_string();

        assert_eq!(
            serialized(&handle_create_project(&repository, " ").unwrap_err()),
            json!("invalid_project_name")
        );
        assert_eq!(
            serialized(&handle_open_project(&repository, "not-a-uuid").unwrap_err()),
            json!("invalid_project_id")
        );
        assert_eq!(
            serialized(
                &handle_open_project(&repository, WRONG_VARIANT_VERSION_SEVEN_ID).unwrap_err()
            ),
            json!("invalid_project_id")
        );
        assert_eq!(
            serialized(&handle_open_project(&repository, &missing_id).unwrap_err()),
            json!("project_not_found")
        );
        assert_eq!(
            serialized(
                &handle_update_project_instructions(&repository, "not-a-uuid", "Anything")
                    .unwrap_err()
            ),
            json!("invalid_project_id")
        );
        assert_eq!(
            serialized(
                &handle_update_project_instructions(&repository, &missing_id, "Anything")
                    .unwrap_err()
            ),
            json!("project_not_found")
        );
    }

    #[test]
    fn internal_repository_details_never_serialize() {
        let repository = FakeRepository::failing();
        let project_id = ProjectId::generate().to_string();
        let errors = [
            handle_create_project(&repository, "Valid").unwrap_err(),
            handle_list_projects(&repository).unwrap_err(),
            handle_open_project(&repository, &project_id).unwrap_err(),
            handle_update_project_instructions(&repository, &project_id, "Anything").unwrap_err(),
        ];

        for error in errors {
            let json = serde_json::to_string(&error).unwrap();
            assert_eq!(json, "\"project_storage_unavailable\"");
            assert!(!json.contains("SELECT secret"));
            assert!(!json.contains("C:\\\\Users"));
            assert!(!json.contains(SENTINEL_INTERNAL_ERROR));
        }
    }

    #[test]
    fn unavailable_state_returns_only_the_storage_error_code() {
        let state = ProjectStorageState::unavailable();
        let error = match state.repository() {
            Ok(_) => panic!("unavailable storage unexpectedly returned a repository"),
            Err(error) => error,
        };

        assert_eq!(serialized(&error), json!("project_storage_unavailable"));
    }

    #[test]
    fn blocking_task_join_failures_map_to_storage_unavailable() {
        let result: Result<(), ProjectCommandError> =
            tauri::async_runtime::block_on(dispatch_project_operation(|| {
                panic!("forced blocking task failure")
            }));

        assert_eq!(
            serialized(&result.unwrap_err()),
            json!("project_storage_unavailable")
        );
    }

    #[test]
    fn ready_state_clones_the_single_repository_owner() {
        let directory = tempfile::tempdir().unwrap();
        let repository =
            SqliteProjectRepository::open(directory.path().join("state.sqlite3")).unwrap();
        let state = ProjectStorageState::ready(repository);

        let first = state.repository().unwrap();
        let second = state.repository().unwrap();

        assert!(Arc::ptr_eq(&first, &second));
    }
}
