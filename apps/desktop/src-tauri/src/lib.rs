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
use provider::{
    ConnectionStatus, PROVIDER_MODEL_CATALOG_CHANGED_EVENT, PROVIDER_MODEL_SELECTION_CHANGED_EVENT,
    ProviderModelCatalogResponse, ProviderModelSelectionResponse, PublicError,
    build_catalog_response, build_selection_response, build_stale_catalog_response,
    persist_model_selection,
};
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

fn emit_catalog_status(app: &tauri::AppHandle, catalog: &ProviderModelCatalogResponse) {
    let _ = app.emit(PROVIDER_MODEL_CATALOG_CHANGED_EVENT, catalog);
}

fn emit_selection_status(app: &tauri::AppHandle, selection: &ProviderModelSelectionResponse) {
    let _ = app.emit(PROVIDER_MODEL_SELECTION_CHANGED_EVENT, selection);
}

fn emit_model_surfaces(app: &tauri::AppHandle, store: &SqliteStore) {
    if let Ok(catalog) = build_catalog_response(store) {
        emit_catalog_status(app, &catalog);
    }
    if let Ok(selection) = build_selection_response(store) {
        emit_selection_status(app, &selection);
    }
}

fn emit_stale_model_surfaces(app: &tauri::AppHandle, store: &SqliteStore) {
    if let Ok(catalog) = build_stale_catalog_response(store) {
        emit_catalog_status(app, &catalog);
    }
    if let Ok(selection) = build_selection_response(store) {
        emit_selection_status(app, &selection);
    }
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

/// Production cancel path used by `cancel_chatgpt_connect`.
/// Requests cancellation only — never samples or emits connection status.
fn cancel_chatgpt_connect_inner(
    adapter: Option<&openai_chatgpt::ChatGptAdapter>,
) -> Result<(), PublicError> {
    let adapter = adapter.ok_or(PublicError::ProviderUnavailable)?;
    adapter.cancel_connect()
}

/// Production settle path used by `connect_chatgpt` after the connect future
/// completes. This is the sole terminal-status emitter for connect/cancel races.
fn settle_connect_chatgpt(
    emit: impl FnOnce(&ConnectionStatus),
    result: Result<ConnectionStatus, PublicError>,
    status_on_error: ConnectionStatus,
) -> Result<ConnectionStatus, PublicError> {
    let status = terminal_status_for_emit(&result, status_on_error);
    emit(&status);
    result
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
    let settled = settle_connect_chatgpt(
        |status| emit_connection_status(&app, status),
        result,
        current_connection_status(&state),
    );
    if settled
        .as_ref()
        .is_ok_and(|status| status.state == provider::ConnectionState::Connected)
        && let Some(adapter) = state.chatgpt()
    {
        match adapter
            .refresh_model_catalog(state.store.as_ref(), true)
            .await
        {
            Ok(catalog) => {
                emit_catalog_status(&app, &catalog);
                if let Ok(selection) = build_selection_response(state.store.as_ref()) {
                    emit_selection_status(&app, &selection);
                }
            }
            Err(_) => emit_stale_model_surfaces(&app, state.store.as_ref()),
        }
    }
    settled
}

#[tauri::command]
fn cancel_chatgpt_connect(state: tauri::State<'_, AgentState>) -> Result<(), PublicError> {
    // Request cancellation only. Do not sample or emit connection status here:
    // a stale Connecting snapshot can race ahead of the in-flight connect
    // command's terminal emission and strand windows that consume the event
    // stream. connect_chatgpt emits the authoritative terminal status after it
    // settles; late Cancel keeps its original safe error for frontend
    // reconciliation.
    cancel_chatgpt_connect_inner(state.chatgpt().as_deref())
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
    emit_model_surfaces(&app, state.store.as_ref());
    Ok(status)
}

#[tauri::command(rename_all = "camelCase")]
async fn get_provider_model_catalog(
    app: tauri::AppHandle,
    state: tauri::State<'_, AgentState>,
) -> Result<ProviderModelCatalogResponse, PublicError> {
    if let Some(adapter) = state.chatgpt() {
        let connected = matches!(
            adapter
                .connection_status_with_store(state.store.as_ref())
                .state,
            provider::ConnectionState::Connected
        );
        if connected
            && adapter
                .catalog_needs_refresh(state.store.as_ref())
                .unwrap_or(true)
        {
            match adapter
                .refresh_model_catalog(state.store.as_ref(), false)
                .await
            {
                Ok(catalog) => {
                    emit_catalog_status(&app, &catalog);
                    if let Ok(selection) = build_selection_response(state.store.as_ref()) {
                        emit_selection_status(&app, &selection);
                    }
                    return Ok(catalog);
                }
                Err(error) => {
                    emit_stale_model_surfaces(&app, state.store.as_ref());
                    if let Ok(stale) = build_stale_catalog_response(state.store.as_ref())
                        && !stale.models.is_empty()
                    {
                        // Last-known remains visible; recovery stays available via refresh.
                        return Ok(stale);
                    }
                    return Err(error);
                }
            }
        }
    }
    let store = StdArc::clone(&state.store);
    tauri::async_runtime::spawn_blocking(move || build_catalog_response(store.as_ref()))
        .await
        .map_err(|_| PublicError::AgentStorageUnavailable)?
}

#[tauri::command(rename_all = "camelCase")]
async fn refresh_provider_model_catalog(
    app: tauri::AppHandle,
    state: tauri::State<'_, AgentState>,
) -> Result<ProviderModelCatalogResponse, PublicError> {
    let adapter = state.chatgpt().ok_or(PublicError::ProviderUnavailable)?;
    match adapter
        .refresh_model_catalog(state.store.as_ref(), true)
        .await
    {
        Ok(catalog) => {
            emit_catalog_status(&app, &catalog);
            if let Ok(selection) = build_selection_response(state.store.as_ref()) {
                emit_selection_status(&app, &selection);
            }
            Ok(catalog)
        }
        Err(error) => {
            emit_stale_model_surfaces(&app, state.store.as_ref());
            Err(error)
        }
    }
}

#[tauri::command(rename_all = "camelCase")]
async fn get_provider_model_selection(
    state: tauri::State<'_, AgentState>,
) -> Result<ProviderModelSelectionResponse, PublicError> {
    let store = StdArc::clone(&state.store);
    tauri::async_runtime::spawn_blocking(move || build_selection_response(store.as_ref()))
        .await
        .map_err(|_| PublicError::AgentStorageUnavailable)?
}

#[tauri::command(rename_all = "camelCase")]
async fn set_provider_model_selection(
    app: tauri::AppHandle,
    state: tauri::State<'_, AgentState>,
    model_id: String,
) -> Result<ProviderModelSelectionResponse, PublicError> {
    let store = StdArc::clone(&state.store);
    let selection = tauri::async_runtime::spawn_blocking(move || {
        persist_model_selection(store.as_ref(), &model_id)
    })
    .await
    .map_err(|_| PublicError::AgentStorageUnavailable)??;
    emit_selection_status(&app, &selection);
    Ok(selection)
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
            get_provider_model_catalog,
            refresh_provider_model_catalog,
            get_provider_model_selection,
            set_provider_model_selection,
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
    use crate::credentials::FakeCredentialStore;
    use crate::openai_chatgpt::{
        ChatGptAdapter, MockInference, TestTransport, TokenBundle, TokenValues,
    };
    use provider::ConnectionState;
    use reqwest::header::HeaderMap;
    use std::sync::{Arc, Mutex};

    // Fixed callback ports cannot be shared across parallel browser-connect tests.
    static CONNECT_COMMAND_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    fn connecting_follows_terminal(events: &[ConnectionStatus]) -> bool {
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

    struct SuccessExchange;
    impl TestTransport for SuccessExchange {
        fn exchange_token(
            &self,
            _: &str,
            _: &str,
            _: &str,
            _: &str,
        ) -> Result<TokenBundle, PublicError> {
            Ok(TokenBundle::Success(TokenValues::for_test(
                "access", "refresh", "account",
            )))
        }
        fn refresh_token(&self, _: &str, _: &str) -> Result<TokenBundle, PublicError> {
            unreachable!()
        }
        fn inference(&self, _: &str, _: &HeaderMap, _: &str) -> Result<MockInference, PublicError> {
            unreachable!()
        }
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

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn production_cancel_path_does_not_emit_and_connect_settle_is_sole_terminal_emitter() {
        let _guard = CONNECT_COMMAND_TEST_LOCK.lock().await;
        let events = Arc::new(Mutex::new(Vec::<ConnectionStatus>::new()));

        // --- Active cancellation: Cancel requests only; settle emits terminal. ---
        {
            let fake = Arc::new(FakeCredentialStore::default());
            let adapter = Arc::new(ChatGptAdapter::new(fake));
            let dir = tempfile::Builder::new()
                .prefix("tule-connect-cmd-cancel-")
                .tempdir()
                .unwrap();
            let store = Arc::new(SqliteStore::open(dir.path().join("tule.sqlite3")).unwrap());
            // std channels: open_url is synchronous. Multi-thread runtime lets the
            // test task progress while connect blocks in open_url without sleeps.
            let (entered_tx, entered_rx) = std::sync::mpsc::channel();
            let (release_tx, release_rx) = std::sync::mpsc::channel();

            let connect_adapter = Arc::clone(&adapter);
            let connect_store = Arc::clone(&store);
            let connect = tokio::spawn(async move {
                connect_adapter
                    .connect_in_browser(connect_store, move |_url| {
                        entered_tx.send(()).unwrap();
                        release_rx.recv().unwrap();
                        Ok(())
                    })
                    .await
            });

            entered_rx.recv().expect("connect entered open_url");
            let sampled_while_connecting = adapter.connection_status_with_store(store.as_ref());
            assert_eq!(sampled_while_connecting.state, ConnectionState::Connecting);

            // Production cancel path used by the command: request only.
            assert_eq!(cancel_chatgpt_connect_inner(Some(adapter.as_ref())), Ok(()));
            assert!(
                events.lock().unwrap().is_empty(),
                "cancel must not emit; a reintroduced sample would be Connecting ({sampled_while_connecting:?})"
            );

            release_tx.send(()).expect("release open_url");
            let result = connect.await.expect("join");
            assert_eq!(result, Err(PublicError::Cancelled));

            let settled = settle_connect_chatgpt(
                |status| events.lock().unwrap().push(status.clone()),
                result,
                adapter.connection_status_with_store(store.as_ref()),
            );
            assert_eq!(settled, Err(PublicError::Cancelled));
            let snapshot = events.lock().unwrap().clone();
            assert_eq!(snapshot.len(), 1);
            assert_eq!(snapshot[0].state, ConnectionState::Disconnected);
            assert!(!connecting_follows_terminal(&snapshot));
        }

        // --- Success then late Cancel: terminal Connected is preserved. ---
        {
            events.lock().unwrap().clear();
            let fake = Arc::new(FakeCredentialStore::default());
            let adapter = ChatGptAdapter::new(fake);
            adapter.set_test_transport(Arc::new(SuccessExchange));
            let dir = tempfile::Builder::new()
                .prefix("tule-connect-cmd-success-")
                .tempdir()
                .unwrap();
            let store = Arc::new(SqliteStore::open(dir.path().join("tule.sqlite3")).unwrap());

            let result = adapter
                .connect_in_browser_with_test_callback(Arc::clone(&store), "ok")
                .await;
            assert!(matches!(
                &result,
                Ok(status) if status.state == ConnectionState::Connected
            ));

            let settled = settle_connect_chatgpt(
                |status| events.lock().unwrap().push(status.clone()),
                result,
                adapter.connection_status_with_store(store.as_ref()),
            );
            assert_eq!(settled.as_ref().unwrap().state, ConnectionState::Connected);
            assert_eq!(events.lock().unwrap().len(), 1);

            // Late Cancel keeps InvalidInput and must not emit over Connected.
            assert_eq!(
                cancel_chatgpt_connect_inner(Some(&adapter)),
                Err(PublicError::InvalidInput)
            );
            let snapshot = events.lock().unwrap().clone();
            assert_eq!(snapshot.len(), 1);
            assert_eq!(snapshot[0].state, ConnectionState::Connected);
            assert!(!connecting_follows_terminal(&snapshot));
        }

        // --- Provider failure: settle emits terminal; Cancel still does not. ---
        {
            events.lock().unwrap().clear();
            let adapter = ChatGptAdapter::new(Arc::new(FakeCredentialStore::default()));
            let dir = tempfile::Builder::new()
                .prefix("tule-connect-cmd-fail-")
                .tempdir()
                .unwrap();
            let store = Arc::new(SqliteStore::open(dir.path().join("tule.sqlite3")).unwrap());

            let result = adapter
                .connect_in_browser(Arc::clone(&store), |_url| {
                    Err(PublicError::ProviderUnavailable)
                })
                .await;
            assert_eq!(result, Err(PublicError::ProviderUnavailable));

            let settled = settle_connect_chatgpt(
                |status| events.lock().unwrap().push(status.clone()),
                result,
                adapter.connection_status_with_store(store.as_ref()),
            );
            assert_eq!(settled, Err(PublicError::ProviderUnavailable));
            assert_eq!(
                cancel_chatgpt_connect_inner(Some(&adapter)),
                Err(PublicError::InvalidInput)
            );
            let snapshot = events.lock().unwrap().clone();
            assert_eq!(snapshot.len(), 1);
            assert_ne!(snapshot[0].state, ConnectionState::Connecting);
            assert!(!connecting_follows_terminal(&snapshot));
        }
    }
}
