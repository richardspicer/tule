mod agents;
mod credentials;
mod openai_chatgpt;
mod preferences;
mod projects;
mod provider;
mod settings_window;
mod sqlite;

use std::{fs, sync::Arc};

use agents::{
    AgentState, cancel_agent_turn, get_agent_session, list_agent_sessions, send_agent_message,
    set_agent_session_project,
};
use credentials::native_store;
use openai_chatgpt::ChatGptAdapter;
use preferences::{DesktopPreferenceState, get_appearance_preference, set_appearance_preference};
use projects::{
    ProjectStorageState, create_project, list_projects, open_project, update_project_instructions,
};
use provider::{ConnectionStatus, PublicError};
use settings_window::{
    CONNECTION_STATUS_CHANGED_EVENT, SettingsLaunchState, exit_application, open_settings_window,
    take_settings_launch_category,
};
use sqlite::{DATABASE_FILENAME, SqliteStore};
use std::sync::Arc as StdArc;
use tauri::{Emitter, Manager};
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

fn current_connection_status(state: &AgentState) -> ConnectionStatus {
    state.chatgpt().map_or_else(
        || state.provider.connection_status(),
        |adapter| adapter.connection_status_with_store(state.store.as_ref()),
    )
}

fn emit_connection_status(app: &tauri::AppHandle, status: &ConnectionStatus) {
    let _ = app.emit(CONNECTION_STATUS_CHANGED_EVENT, status);
}

/// Selects the status that connect terminal paths emit without altering
/// the original command result.
fn terminal_status_for_emit(
    result: &Result<ConnectionStatus, PublicError>,
    status_on_error: ConnectionStatus,
) -> ConnectionStatus {
    match result {
        Ok(status) => status.clone(),
        Err(_) => status_on_error,
    }
}

#[tauri::command]
fn connection_status(state: tauri::State<'_, AgentState>) -> ConnectionStatus {
    current_connection_status(&state)
}

#[tauri::command]
fn sync_connection_status(
    app: tauri::AppHandle,
    state: tauri::State<'_, AgentState>,
) -> ConnectionStatus {
    let status = current_connection_status(&state);
    emit_connection_status(&app, &status);
    status
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
        let status = state.provider.connection_status();
        emit_connection_status(&app, &status);
        return Ok(status);
    };
    let store = StdArc::clone(&state.store);
    let opener = app.clone();
    let result = adapter
        .connect_in_browser(store, move |url| {
            opener
                .opener()
                .open_url(url, None::<&str>)
                .map_err(|_| PublicError::ProviderUnavailable)?;
            Ok(())
        })
        .await;
    // Emit the authoritative terminal status on success and failure without
    // replacing the original command error.
    let status = terminal_status_for_emit(&result, current_connection_status(&state));
    emit_connection_status(&app, &status);
    result
}

#[tauri::command]
fn cancel_chatgpt_connect(state: tauri::State<'_, AgentState>) -> Result<(), PublicError> {
    let Some(adapter) = state.chatgpt() else {
        return Err(PublicError::ProviderUnavailable);
    };
    // Request cancellation only. Do not sample or emit connection status here:
    // a stale Connecting snapshot can race ahead of the in-flight connect
    // command's terminal emission and strand windows that consume the event
    // stream. connect_chatgpt emits the authoritative terminal status after it
    // settles; late Cancel keeps its original safe error for frontend
    // reconciliation.
    adapter.cancel_connect()
}

#[tauri::command]
async fn disconnect_chatgpt(
    app: tauri::AppHandle,
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
        let status = state.provider.connection_status();
        emit_connection_status(&app, &status);
        return Ok(status);
    };
    let status = adapter.disconnect(state.store.as_ref())?;
    emit_connection_status(&app, &status);
    Ok(status)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            if let Some(main) = app.get_webview_window("main") {
                let _ = main.set_title("");
            }
            app.manage(SettingsLaunchState::default());

            let Some(store) = initialize_store(app) else {
                app.manage(ProjectStorageState::unavailable());
                app.manage(DesktopPreferenceState::unavailable());
                return Ok(());
            };
            tule_core::interrupt_inflight_turns(store.as_ref()).map_err(|_| {
                std::io::Error::other("Agent storage recovery failed during startup")
            })?;
            let chatgpt = Arc::new(ChatGptAdapter::new(native_store()));
            let provider: Arc<dyn provider::ProviderAdapter> = Arc::clone(&chatgpt) as _;
            app.manage(ProjectStorageState::ready_shared(Arc::clone(&store)));
            app.manage(DesktopPreferenceState::ready_shared(Arc::clone(&store)));
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
            sync_connection_status,
            connect_chatgpt,
            cancel_chatgpt_connect,
            disconnect_chatgpt,
            get_appearance_preference,
            set_appearance_preference,
            open_settings_window,
            take_settings_launch_category,
            exit_application
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;
    use provider::ConnectionState;

    /// Cancel never emits connection status. Only the settled connect command
    /// emits the authoritative terminal status, so a Connecting snapshot
    /// sampled during Cancel cannot overtake success, cancellation, or failure.
    fn connection_status_events_after_connect_cancel_race(
        connect_terminal: ConnectionStatus,
        cancel_sampled_while_connecting: ConnectionStatus,
        cancel_emits_sampled_status: bool,
    ) -> Vec<ConnectionStatus> {
        let mut events = vec![connect_terminal];
        if cancel_emits_sampled_status {
            // Models the defective ordering: Cancel sampled Connecting, then
            // emitted that snapshot after connect already published terminal.
            events.push(cancel_sampled_while_connecting);
        }
        events
    }

    fn connecting_overtakes_terminal(events: &[ConnectionStatus]) -> bool {
        let mut saw_terminal = false;
        for status in events {
            if status.state != ConnectionState::Connecting {
                saw_terminal = true;
            } else if saw_terminal {
                return true;
            }
        }
        false
    }

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

    #[test]
    fn terminal_status_emit_preserves_success_and_uses_authoritative_error_status() {
        let connected = ConnectionStatus {
            state: ConnectionState::Connected,
            provider_id: "openai-chatgpt-compat",
            model: "gpt-5.5",
        };
        let disconnected = ConnectionStatus {
            state: ConnectionState::Disconnected,
            provider_id: "openai-chatgpt-compat",
            model: "gpt-5.5",
        };

        assert_eq!(
            terminal_status_for_emit(&Ok(connected.clone()), disconnected.clone()),
            connected
        );
        assert_eq!(
            terminal_status_for_emit(&Err(PublicError::Cancelled), disconnected.clone()),
            disconnected
        );
        assert_eq!(
            terminal_status_for_emit(&Err(PublicError::InvalidInput), connected.clone()),
            connected
        );
    }

    #[test]
    fn cancel_status_emission_cannot_overtake_terminal_connect_events() {
        let connecting = ConnectionStatus {
            state: ConnectionState::Connecting,
            provider_id: "openai-chatgpt-compat",
            model: "gpt-5.5",
        };
        let terminals = [
            ConnectionStatus {
                state: ConnectionState::Connected,
                provider_id: "openai-chatgpt-compat",
                model: "gpt-5.5",
            },
            ConnectionStatus {
                state: ConnectionState::Disconnected,
                provider_id: "openai-chatgpt-compat",
                model: "gpt-5.5",
            },
            ConnectionStatus {
                state: ConnectionState::ReconnectRequired,
                provider_id: "openai-chatgpt-compat",
                model: "gpt-5.5",
            },
        ];

        for terminal in terminals {
            // Production policy: Cancel does not emit; connect emits the terminal.
            let events = connection_status_events_after_connect_cancel_race(
                terminal.clone(),
                connecting.clone(),
                false,
            );
            assert_eq!(events, vec![terminal.clone()]);
            assert!(!connecting_overtakes_terminal(&events));

            // Defective policy: Cancel emits a Connecting snapshot after the
            // terminal event and strands event consumers.
            let raced = connection_status_events_after_connect_cancel_race(
                terminal,
                connecting.clone(),
                true,
            );
            assert!(connecting_overtakes_terminal(&raced));
        }
    }
}
