use crate::{Project, ProjectId};

/// Storage operations required by Tule's project use cases.
///
/// Implementations own their concrete error type and any synchronization or
/// transaction details. The interface uses only core values, which keeps SQL,
/// Tauri, filesystem, and provider types outside the domain boundary.
///
/// Project names are not unique. Implementations must permit multiple projects
/// with the same validated name.
pub trait ProjectRepository: Send + Sync {
    /// The implementation-specific storage failure.
    type Error;

    /// Persists a newly created project.
    fn create(&self, project: &Project) -> Result<(), Self::Error>;

    /// Returns all persisted projects in repository-defined stable order.
    fn list(&self) -> Result<Vec<Project>, Self::Error>;

    /// Finds a project by its stable identifier.
    fn find_by_id(&self, id: &ProjectId) -> Result<Option<Project>, Self::Error>;

    /// Replaces a project's instructions and returns the updated project.
    ///
    /// Implementations must preserve `instructions` exactly, without trimming,
    /// normalization, or interpretation. [`None`] indicates that no project has
    /// the supplied identifier.
    fn update_instructions(
        &self,
        id: &ProjectId,
        instructions: &str,
    ) -> Result<Option<Project>, Self::Error>;
}
