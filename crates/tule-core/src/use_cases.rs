use std::{error::Error, fmt};

use crate::{
    InvalidProjectId, InvalidProjectName, Project, ProjectId, ProjectName, ProjectRepository,
    ProjectTimeError,
};

/// Creates and persists a project after validating its user-provided name.
///
/// The core generates both the UUID version 7 identifier and the creation time.
/// Invalid input returns before the repository is called.
pub fn create_project<R>(
    repository: &R,
    name: &str,
) -> Result<Project, CreateProjectError<R::Error>>
where
    R: ProjectRepository + ?Sized,
{
    let name = ProjectName::new(name).map_err(CreateProjectError::InvalidName)?;
    let project = Project::create(name).map_err(CreateProjectError::Time)?;
    repository
        .create(&project)
        .map_err(CreateProjectError::Repository)?;
    Ok(project)
}

/// Lists the projects available from a repository.
pub fn list_projects<R>(repository: &R) -> Result<Vec<Project>, R::Error>
where
    R: ProjectRepository + ?Sized,
{
    repository.list()
}

/// Opens a project after validating its persisted identifier representation.
///
/// A syntactically valid UUID of another version is also rejected. A valid
/// identifier absent from the repository produces the distinct
/// [`OpenProjectError::NotFound`] outcome.
pub fn open_project<R>(repository: &R, id: &str) -> Result<Project, OpenProjectError<R::Error>>
where
    R: ProjectRepository + ?Sized,
{
    let id = ProjectId::parse(id).map_err(OpenProjectError::InvalidId)?;
    repository
        .find_by_id(&id)
        .map_err(OpenProjectError::Repository)?
        .ok_or(OpenProjectError::NotFound(id))
}

/// A failure to create a project.
#[derive(Debug)]
pub enum CreateProjectError<E> {
    /// The user-provided project name is invalid.
    InvalidName(InvalidProjectName),
    /// The current creation time cannot be represented as Unix milliseconds.
    Time(ProjectTimeError),
    /// The repository failed to persist the valid project.
    Repository(E),
}

impl<E> fmt::Display for CreateProjectError<E>
where
    E: fmt::Display,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidName(error) => write!(formatter, "invalid project name: {error}"),
            Self::Time(error) => {
                write!(formatter, "could not obtain project creation time: {error}")
            }
            Self::Repository(error) => write!(formatter, "could not persist project: {error}"),
        }
    }
}

impl<E> Error for CreateProjectError<E>
where
    E: Error + 'static,
{
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidName(error) => Some(error),
            Self::Time(error) => Some(error),
            Self::Repository(error) => Some(error),
        }
    }
}

/// A failure to open a project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenProjectError<E> {
    /// The supplied project identifier is malformed or not UUID version 7.
    InvalidId(InvalidProjectId),
    /// No stored project has the validated identifier.
    NotFound(ProjectId),
    /// The repository failed while looking up the project.
    Repository(E),
}

impl<E> fmt::Display for OpenProjectError<E>
where
    E: fmt::Display,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidId(error) => write!(formatter, "invalid project ID: {error}"),
            Self::NotFound(id) => write!(formatter, "project {id} was not found"),
            Self::Repository(error) => write!(formatter, "could not load project: {error}"),
        }
    }
}

impl<E> Error for OpenProjectError<E>
where
    E: Error + 'static,
{
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidId(error) => Some(error),
            Self::NotFound(_) => None,
            Self::Repository(error) => Some(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::Mutex,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum FakeRepositoryError {
        Forced,
    }

    impl fmt::Display for FakeRepositoryError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("forced fake repository failure")
        }
    }

    impl Error for FakeRepositoryError {}

    #[derive(Default)]
    struct FakeRepository {
        state: Mutex<FakeState>,
    }

    #[derive(Default)]
    struct FakeState {
        projects: Vec<Project>,
        create_calls: usize,
        list_calls: usize,
        find_calls: usize,
        fail_create: bool,
        fail_list: bool,
        fail_find: bool,
    }

    impl FakeRepository {
        fn projects(&self) -> Vec<Project> {
            self.state.lock().unwrap().projects.clone()
        }

        fn create_calls(&self) -> usize {
            self.state.lock().unwrap().create_calls
        }

        fn list_calls(&self) -> usize {
            self.state.lock().unwrap().list_calls
        }

        fn find_calls(&self) -> usize {
            self.state.lock().unwrap().find_calls
        }

        fn seed(&self, project: Project) {
            self.state.lock().unwrap().projects.push(project);
        }

        fn force_create_failure(&self) {
            self.state.lock().unwrap().fail_create = true;
        }

        fn force_list_failure(&self) {
            self.state.lock().unwrap().fail_list = true;
        }

        fn force_find_failure(&self) {
            self.state.lock().unwrap().fail_find = true;
        }
    }

    impl ProjectRepository for FakeRepository {
        type Error = FakeRepositoryError;

        fn create(&self, project: &Project) -> Result<(), Self::Error> {
            let mut state = self.state.lock().unwrap();
            state.create_calls += 1;
            if state.fail_create {
                return Err(FakeRepositoryError::Forced);
            }
            state.projects.push(project.clone());
            Ok(())
        }

        fn list(&self) -> Result<Vec<Project>, Self::Error> {
            let mut state = self.state.lock().unwrap();
            state.list_calls += 1;
            if state.fail_list {
                return Err(FakeRepositoryError::Forced);
            }
            Ok(state.projects.clone())
        }

        fn find_by_id(&self, id: &ProjectId) -> Result<Option<Project>, Self::Error> {
            let mut state = self.state.lock().unwrap();
            state.find_calls += 1;
            if state.fail_find {
                return Err(FakeRepositoryError::Forced);
            }
            Ok(state
                .projects
                .iter()
                .find(|project| project.id() == *id)
                .cloned())
        }
    }

    fn stored_project(name: &str, created_at_unix_ms: i64) -> Project {
        Project::from_stored_parts(&ProjectId::generate().to_string(), name, created_at_unix_ms)
            .unwrap()
    }

    fn current_unix_milliseconds() -> i64 {
        i64::try_from(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis(),
        )
        .unwrap()
    }

    #[test]
    fn repository_contract_is_send_and_sync() {
        fn assert_send_and_sync<T: Send + Sync>() {}

        assert_send_and_sync::<FakeRepository>();
    }

    #[test]
    fn create_validates_before_calling_the_repository() {
        let repository = FakeRepository::default();

        let empty = create_project(&repository, "  ");
        let too_long = create_project(&repository, &"a".repeat(101));
        let control = create_project(&repository, "alpha\u{0000}beta");

        assert!(matches!(
            empty,
            Err(CreateProjectError::InvalidName(InvalidProjectName::Empty))
        ));
        assert!(matches!(
            too_long,
            Err(CreateProjectError::InvalidName(
                InvalidProjectName::TooLong { scalar_count: 101 }
            ))
        ));
        assert!(matches!(
            control,
            Err(CreateProjectError::InvalidName(
                InvalidProjectName::ContainsControlCharacter
            ))
        ));
        assert_eq!(repository.create_calls(), 0);
        assert!(repository.projects().is_empty());
    }

    #[test]
    fn create_generates_identity_time_and_persists_the_normalized_project() {
        let repository = FakeRepository::default();
        let before = current_unix_milliseconds();

        let project = create_project(&repository, "  First project  ").unwrap();

        let after = current_unix_milliseconds();
        assert_eq!(project.name().as_str(), "First project");
        assert_eq!(
            ProjectId::parse(&project.id().to_string()),
            Ok(project.id())
        );
        assert!(project.created_at_unix_ms() >= before);
        assert!(project.created_at_unix_ms() <= after);
        assert_eq!(repository.create_calls(), 1);
        assert_eq!(repository.projects(), vec![project]);
    }

    #[test]
    fn duplicate_project_names_are_allowed() {
        let repository = FakeRepository::default();

        let first = create_project(&repository, "Repeated").unwrap();
        let second = create_project(&repository, "Repeated").unwrap();
        let listed = list_projects(&repository).unwrap();

        assert_ne!(first.id(), second.id());
        assert_eq!(first.name(), second.name());
        assert_eq!(listed, vec![first, second]);
    }

    #[test]
    fn create_reports_the_repository_specific_error() {
        let repository = FakeRepository::default();
        repository.force_create_failure();

        let result = create_project(&repository, "Valid");

        assert!(matches!(
            result,
            Err(CreateProjectError::Repository(FakeRepositoryError::Forced))
        ));
        assert_eq!(repository.create_calls(), 1);
        assert!(repository.projects().is_empty());
    }

    #[test]
    fn list_returns_repository_projects_in_order() {
        let repository = FakeRepository::default();
        let first = stored_project("First", 10);
        let second = stored_project("Second", 20);
        repository.seed(first.clone());
        repository.seed(second.clone());

        assert_eq!(list_projects(&repository), Ok(vec![first, second]));
        assert_eq!(repository.list_calls(), 1);
    }

    #[test]
    fn list_returns_an_empty_collection_when_no_projects_exist() {
        let repository = FakeRepository::default();

        assert_eq!(list_projects(&repository), Ok(Vec::new()));
        assert_eq!(repository.list_calls(), 1);
    }

    #[test]
    fn list_preserves_the_repository_specific_error() {
        let repository = FakeRepository::default();
        repository.force_list_failure();

        assert_eq!(list_projects(&repository), Err(FakeRepositoryError::Forced));
        assert_eq!(repository.list_calls(), 1);
    }

    #[test]
    fn open_rejects_invalid_ids_before_calling_the_repository() {
        let repository = FakeRepository::default();

        let malformed = open_project(&repository, "not-a-uuid");
        let wrong_version = open_project(&repository, "550e8400-e29b-41d4-a716-446655440000");

        assert_eq!(
            malformed,
            Err(OpenProjectError::InvalidId(InvalidProjectId::Malformed))
        );
        assert_eq!(
            wrong_version,
            Err(OpenProjectError::InvalidId(
                InvalidProjectId::NotVersionSeven
            ))
        );
        assert_eq!(repository.find_calls(), 0);
    }

    #[test]
    fn open_returns_a_stored_project() {
        let repository = FakeRepository::default();
        let project = stored_project("Stored", 42);
        repository.seed(project.clone());

        assert_eq!(
            open_project(&repository, &project.id().to_string()),
            Ok(project)
        );
        assert_eq!(repository.find_calls(), 1);
    }

    #[test]
    fn open_distinguishes_a_missing_project() {
        let repository = FakeRepository::default();
        let id = ProjectId::generate();

        assert_eq!(
            open_project(&repository, &id.to_string()),
            Err(OpenProjectError::NotFound(id))
        );
        assert_eq!(repository.find_calls(), 1);
    }

    #[test]
    fn open_reports_the_repository_specific_error() {
        let repository = FakeRepository::default();
        repository.force_find_failure();
        let id = ProjectId::generate();

        assert_eq!(
            open_project(&repository, &id.to_string()),
            Err(OpenProjectError::Repository(FakeRepositoryError::Forced))
        );
        assert_eq!(repository.find_calls(), 1);
    }
}
