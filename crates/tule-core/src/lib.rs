//! Tauri-independent domain and application behavior for Tule.
#![warn(missing_docs)]

mod project;
mod repository;
mod use_cases;

pub use project::{
    InvalidProjectId, InvalidProjectName, Project, ProjectId, ProjectName,
    ProjectReconstructionError, ProjectTimeError,
};
pub use repository::ProjectRepository;
pub use use_cases::{
    CreateProjectError, OpenProjectError, create_project, list_projects, open_project,
};

/// Stable application identity exposed to Tule hosts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplicationInfo {
    /// Human-readable product name.
    pub name: String,
    /// Product version supplied by the core crate package metadata.
    pub version: String,
}

/// Returns Tule's application identity without depending on a host framework.
#[must_use]
pub fn get_application_info() -> ApplicationInfo {
    ApplicationInfo {
        name: "Tule".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn application_info_reports_the_core_identity() {
        let info = get_application_info();

        assert_eq!(
            info,
            ApplicationInfo {
                name: "Tule".to_owned(),
                version: env!("CARGO_PKG_VERSION").to_owned(),
            }
        );
    }
}
