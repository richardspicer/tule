//! Tauri-independent domain and application behavior for TULE.
#![warn(missing_docs)]

mod agent;
mod agent_repository;
mod agent_use_cases;
mod project;
mod repository;
mod use_cases;

pub use agent::{
    AgentContextError, AgentEvent, AgentEventId, AgentEventKind, AgentInputError,
    AgentOutputLimitError, AgentReconstructionError, AgentSession, AgentSessionId, AgentTurn,
    AgentTurnFinishError, AgentTurnId, AgentTurnState, CHECKPOINT_BYTE_THRESHOLD,
    CHECKPOINT_INTERVAL_MS, CompletedTurnContext, FIXED_INSTRUCTION, IllegalAgentTurnTransition,
    InvalidAgentEventKind, InvalidAgentId, InvalidAgentTurnState, MAX_AGENT_OUTPUT_UTF8,
    MAX_CONTEXT_UTF8, MAX_USER_TEXT_UTF8, MODEL_ID, PROMPT_VERSION, PROVIDER_PROFILE_ID,
    ProviderRequestId, TITLE_MAX_SCALARS, assemble_instructions, assemble_responses_request_json,
    derive_session_title, should_checkpoint, validate_user_text,
};
pub use agent_repository::{AgentRepository, ProviderProfile};
pub use agent_use_cases::{
    ApplyAgentDeltaError, FinishAgentTurnError, PrepareAgentSendError, PreparedAgentSend,
    SetSessionProjectError, apply_agent_delta, cancel_agent_turn, checkpoint_agent_turn,
    complete_agent_turn, completed_history_from_turns, fail_agent_turn, interrupt_inflight_turns,
    prepare_agent_send, set_session_project,
};
pub use project::{
    InvalidProjectId, InvalidProjectName, Project, ProjectId, ProjectName,
    ProjectReconstructionError, ProjectTimeError,
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
