//! Tauri-independent domain and application behavior for TULE.
#![warn(missing_docs)]

mod agent;
mod agent_repository;
mod agent_source;
mod agent_use_cases;
mod artifact;
mod artifact_repository;
mod artifact_use_cases;
mod project;
mod provider_catalog;
mod repository;
mod use_cases;

pub use agent::{
    AgentContextError, AgentEvent, AgentEventId, AgentEventKind, AgentInputError,
    AgentOutputLimitError, AgentReconstructionError, AgentSession, AgentSessionId, AgentTurn,
    AgentTurnFinishError, AgentTurnId, AgentTurnState, CHECKPOINT_BYTE_THRESHOLD,
    CHECKPOINT_INTERVAL_MS, CompletedTurnContext, FIXED_INSTRUCTION, IllegalAgentTurnTransition,
    InvalidAgentEventKind, InvalidAgentId, InvalidAgentTurnState, MAX_AGENT_OUTPUT_UTF8,
    MAX_CONTEXT_UTF8, MAX_USER_TEXT_UTF8, MODEL_ID, PROMPT_VERSION, PROVIDER_PROFILE_ID,
    ProviderRequestId, TITLE_MAX_SCALARS, assemble_chat_completions_request_json,
    assemble_instructions, assemble_responses_request_json, derive_session_title,
    should_checkpoint, validate_user_text,
};
pub use agent_repository::{AgentRepository, ProviderProfile};
pub use agent_source::{
    ATTACHED_SOURCE_FRAME_VERSION, MAX_CANONICAL_URL_UTF8, MAX_FOLDER_MEMBERS, MAX_SOURCE_UTF8,
    SOURCE_ORIGIN_LOCAL_TEXT_FILE, SOURCE_ORIGIN_LOCAL_TEXT_FOLDER, SOURCE_ORIGIN_REMOTE_TEXT_URL,
    Source, SourceContext, SourceId, SourceReconstructionError, SourceValidationError, TurnSource,
    count_folder_members, derive_remote_source_display_name, format_turn_user_content,
    frame_folder_members, hash_source_bytes, validate_canonical_https_url, validate_source_content,
    validate_source_display_name,
};
pub use agent_use_cases::{
    ApplyAgentDeltaError, FinishAgentTurnError, PrepareAgentSendError, PreparedAgentSend,
    SetSessionProjectError, apply_agent_delta, cancel_agent_turn, checkpoint_agent_turn,
    complete_agent_turn, completed_history_from_turns, fail_agent_turn, interrupt_inflight_turns,
    prepare_agent_send, set_session_project,
};
pub use artifact::{
    Artifact, ArtifactDetail, ArtifactId, ArtifactKind, ArtifactReconstructionError,
    ArtifactSummary, ArtifactValidationError, ArtifactVersion, ArtifactVersionId,
    ArtifactVersionProvenance, InvalidArtifactKind, MAX_ARTIFACT_CONTENT_UTF8,
    derive_artifact_title, resolve_artifact_title, validate_artifact_content,
};
pub use artifact_repository::ArtifactRepository;
pub use artifact_use_cases::{
    CreateArtifactFromTurnError, GetArtifactError, ListArtifactsError, create_artifact_from_turn,
    get_artifact, list_artifacts_for_session_context,
};
pub use project::{
    InvalidProjectId, InvalidProjectName, Project, ProjectId, ProjectName,
    ProjectReconstructionError, ProjectTimeError,
};
pub use provider_catalog::{
    CATALOG_DESCRIPTION_MAX_SCALARS, CATALOG_TTL_MS, CatalogCandidate, CatalogFreshness,
    InvalidModelId, MODEL_ID_MAX_UTF8, ModelCatalogEntry, SelectedDefaultResolution,
    catalog_freshness, is_usable_catalog_candidate, model_id_in_catalog, resolve_selected_default,
    select_usable_catalog_entries, validate_model_id,
};
pub use repository::ProjectRepository;
pub use use_cases::{
    CreateProjectError, OpenProjectError, UpdateProjectInstructionsError, create_project,
    list_projects, open_project, update_project_instructions,
};

/// Stable application identity exposed to TULE hosts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplicationInfo {
    /// Human-readable product name.
    pub name: String,
    /// Product version supplied by the core crate package metadata.
    pub version: String,
}

/// Returns TULE's application identity without depending on a host framework.
#[must_use]
pub fn get_application_info() -> ApplicationInfo {
    ApplicationInfo {
        name: "TULE".to_owned(),
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
                name: "TULE".to_owned(),
                version: env!("CARGO_PKG_VERSION").to_owned(),
            }
        );
    }
}
