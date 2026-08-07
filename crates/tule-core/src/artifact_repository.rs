//! Persistence interface for Artifacts and immutable Artifact Versions.

use std::error::Error;

use crate::{
    AgentSessionId, Artifact, ArtifactDetail, ArtifactId, ArtifactSummary, ArtifactVersion,
    ProjectId,
};

/// Storage operations required by Artifact use cases.
///
/// Implementations own synchronization and transactions.
pub trait ArtifactRepository: Send + Sync {
    /// Implementation-specific storage failure.
    type Error: Error + Send + Sync + 'static;

    /// Atomically inserts an Artifact and its first immutable version.
    fn create_artifact_with_first_version(
        &self,
        artifact: &Artifact,
        version: &ArtifactVersion,
    ) -> Result<(), Self::Error>;

    /// Loads an Artifact and every version by identifier.
    fn get_artifact(&self, id: &ArtifactId) -> Result<Option<ArtifactDetail>, Self::Error>;

    /// Lists Artifacts for the open session context.
    ///
    /// Includes Artifacts with any version whose `source_session_id` equals
    /// `session_id`, union Artifacts whose `project_id` equals `project_id`
    /// when that Project association is present.
    fn list_artifacts_for_session_context(
        &self,
        session_id: &AgentSessionId,
        project_id: Option<&ProjectId>,
    ) -> Result<Vec<ArtifactSummary>, Self::Error>;
}
