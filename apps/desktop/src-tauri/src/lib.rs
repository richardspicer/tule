mod agents;
mod credentials;
mod openai_chatgpt;
mod projects;
mod provider;
mod sqlite;

use std::{fs, sync::Arc};

use agents::{
    AgentState, cancel_agent_turn, get_agent_session, list_agent_sessions, send_agent_message,
    set_agent_session_project,
};
use credentials::native_store;
use openai_chatgpt::ChatGptAdapter;
use projects::{
    ProjectStorageState, create_project, list_projects, open_project, update_project_instructions,
};
use provider::{ConnectionStatus, PublicError};
use sqlite::{DATABASE_FILENAME, SqliteStore};
use std::sync::Arc as StdArc;
use tauri::Manager;
use tauri_plugin_opener::OpenerExt;
use tule_core::AgentRepository;

#[derive(Debug, serde::Serialize)]
struct ApplicationInfoResponse {
    name: String,
    version: String,
}

impl From<tule_core::ApplicationInfo> for ApplicationInfoResponse {
    fn from(info: tule_core::ApplicationInfo) -> Self {
        Self {
            name: info.name,
            version: info.version,
        }
    }
}

#[tauri::command]
fn get_application_info() -> ApplicationInfoResponse {
    tule_core::get_application_info().into()
}

fn initialize_store<R: tauri::Runtime>(app: &tauri::App<R>) -> Option<Arc<SqliteStore>> {
    let Ok(directory) = app.path().app_local_data_dir() else {
        return None;
    };
    if fs::create_dir_all(&directory).is_err() {
        return None;
    }

    SqliteStore::open(directory.join(DATABASE_FILENAME))
        .ok()
        .map(Arc::new)
}

#[tauri::command]
fn connection_status(state: tauri::State<'_, AgentState>) -> ConnectionStatus {
    state.chatgpt().map_or_else(
        || state.provider.connection_status(),
        |adapter| adapter.connection_status_with_store(state.store.as_ref()),
    )
}

/// Connection setup is intentionally native-only. The frontend is never given
/// URLs, callback data, PKCE state, or credential material.
#[tauri::command]
async fn connect_chatgpt(
    app: tauri::AppHandle,
    state: tauri::State<'_, AgentState>,
) -> Result<ConnectionStatus, PublicError> {
    let _operation = state.try_operation()?;
    let Some(adapter) = state.chatgpt() else {
        return Ok(state.provider.connection_status());
    };
    let store = StdArc::clone(&state.store);
    adapter
        .connect_in_browser(store, move |url| {
            app.opener()
                .open_url(url, None::<&str>)
                .map_err(|_| PublicError::ProviderUnavailable)?;
            Ok(())
        })
        .await
}

#[tauri::command]
fn cancel_chatgpt_connect(state: tauri::State<'_, AgentState>) -> Result<(), PublicError> {
    let Some(adapter) = state.chatgpt() else {
        return Err(PublicError::ProviderUnavailable);
    };
    adapter.cancel_connect()
}

#[tauri::command]
async fn disconnect_chatgpt(
    state: tauri::State<'_, AgentState>,
) -> Result<ConnectionStatus, PublicError> {
    let _operation = state.try_operation()?;
    if state
        .store
        .has_inflight_turn()
        .map_err(|_| PublicError::AgentStorageUnavailable)?
    {
        return Err(PublicError::SessionBusy);
    }
    let Some(adapter) = state.chatgpt() else {
        return Ok(state.provider.connection_status());
    };
    adapter.disconnect(state.store.as_ref())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let Some(store) = initialize_store(app) else {
                app.manage(ProjectStorageState::unavailable());
                return Ok(());
            };
            tule_core::interrupt_inflight_turns(store.as_ref()).map_err(|_| {
                std::io::Error::other("Agent storage recovery failed during startup")
            })?;
            let chatgpt = Arc::new(ChatGptAdapter::new(native_store()));
            let provider: Arc<dyn provider::ProviderAdapter> = Arc::clone(&chatgpt) as _;
            app.manage(ProjectStorageState::ready_shared(Arc::clone(&store)));
            app.manage(AgentState::new(store, provider, Some(chatgpt)));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_application_info,
            create_project,
            list_projects,
            open_project,
            update_project_instructions,
            list_agent_sessions,
            get_agent_session,
            send_agent_message,
            cancel_agent_turn,
            set_agent_session_project,
            connection_status,
            connect_chatgpt,
            cancel_chatgpt_connect,
            disconnect_chatgpt
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn application_info_command_preserves_the_typed_core_shape() {
        let expected = tule_core::get_application_info();
        let response = get_application_info();
        let json = serde_json::to_value(response).expect("response should serialize");

        assert_eq!(
            json,
            serde_json::json!({
                "name": expected.name,
                "version": expected.version,
            })
        );
    }
}
