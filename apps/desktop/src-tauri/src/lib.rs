mod agents;
mod credentials;
mod harness;
mod operation_broker;
mod preferences;
mod projects;
mod provider;
mod settings_window;
mod source_draft;
mod sqlite;
mod windows_fs;
mod xai_subscription;

use std::{fs, sync::Arc};

use agents::{
    AgentState, attach_agent_text_link_source, cancel_agent_turn, clear_agent_text_source_draft,
    create_artifact_from_turn, export_agent_turn_metrics, get_agent_session, get_artifact,
    get_model_request_controls, list_agent_sessions, list_artifacts, pick_agent_text_folder_source,
    pick_agent_text_source, send_agent_message, set_agent_session_project,
    set_agent_source_draft_scope,
};
use credentials::native_store;
use harness::{
    HarnessState, approve_harness_pair, bootstrap_harness_plan, cancel_harness_run,
    create_harness_run, deny_unsupported_harness_operation, execute_harness_run, get_harness_run,
    get_harness_run_detail, issue_harness_execution_grants, pause_harness_run,
    pick_harness_run_root, rebind_harness_run_root, revoke_harness_grant,
    takeover_harness_root_lease,
};
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
use xai_subscription::{
    DevicePairingNotifier, DevicePairingResponse, XAI_DEVICE_PAIRING_CHANGED_EVENT,
    XaiSubscriptionAdapter,
};

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
    state.xai().map_or_else(
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

fn terminal_status_for_emit(
    result: &Result<ConnectionStatus, PublicError>,
    status_on_error: ConnectionStatus,
) -> ConnectionStatus {
    match result {
        Ok(status) => status.clone(),
        Err(_) => status_on_error,
    }
}

fn cancel_xai_connect_inner(
    adapter: Option<&xai_subscription::XaiSubscriptionAdapter>,
) -> Result<(), PublicError> {
    let adapter = adapter.ok_or(PublicError::ProviderUnavailable)?;
    adapter.cancel_connect()
}

fn settle_connect_xai(
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

#[tauri::command]
fn get_xai_device_pairing(
    state: tauri::State<'_, AgentState>,
) -> Result<Option<DevicePairingResponse>, PublicError> {
    Ok(state.xai().and_then(|adapter| adapter.device_pairing()))
}

#[tauri::command]
async fn connect_xai(
    app: tauri::AppHandle,
    state: tauri::State<'_, AgentState>,
) -> Result<ConnectionStatus, PublicError> {
    let _operation = state.try_operation()?;
    let Some(adapter) = state.xai() else {
        let status = state.provider.connection_status();
        emit_connection_status(&app, &status);
        return Ok(status);
    };
    let store = StdArc::clone(&state.store);
    let opener = app.clone();
    let pairing_notifier: DevicePairingNotifier = Arc::new({
        let app = app.clone();
        move |pairing| {
            let payload = pairing.unwrap_or(DevicePairingResponse {
                verification_uri: String::new(),
                user_code: String::new(),
            });
            let _ = app.emit(XAI_DEVICE_PAIRING_CHANGED_EVENT, &payload);
        }
    });
    let result = adapter
        .connect_device_code(
            store,
            move |url| {
                opener
                    .opener()
                    .open_url(url, None::<&str>)
                    .map_err(|_| PublicError::ProviderUnavailable)?;
                Ok(())
            },
            Some(pairing_notifier),
        )
        .await;
    let settled = settle_connect_xai(
        |status| emit_connection_status(&app, status),
        result,
        current_connection_status(&state),
    );
    if settled
        .as_ref()
        .is_ok_and(|status| status.state == provider::ConnectionState::Connected)
        && let Some(adapter) = state.xai()
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
fn cancel_xai_connect(state: tauri::State<'_, AgentState>) -> Result<(), PublicError> {
    cancel_xai_connect_inner(state.xai().as_deref())
}

#[tauri::command]
async fn disconnect_xai(
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
    let Some(adapter) = state.xai() else {
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
    if let Some(adapter) = state.xai() {
        let connected = matches!(
            adapter
                .connection_status_with_store(state.store.as_ref())
                .state,
            provider::ConnectionState::Connected
        );
        if connected {
            match adapter.load_connected_catalog(state.store.as_ref()).await {
                Ok(catalog) => {
                    emit_catalog_status(&app, &catalog);
                    if let Ok(selection) = build_selection_response(state.store.as_ref()) {
                        emit_selection_status(&app, &selection);
                    }
                    return Ok(catalog);
                }
                Err(error) => {
                    emit_stale_model_surfaces(&app, state.store.as_ref());
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
async fn get_persisted_provider_model_catalog(
    state: tauri::State<'_, AgentState>,
) -> Result<ProviderModelCatalogResponse, PublicError> {
    let store = StdArc::clone(&state.store);
    tauri::async_runtime::spawn_blocking(move || build_stale_catalog_response(store.as_ref()))
        .await
        .map_err(|_| PublicError::AgentStorageUnavailable)?
}

#[tauri::command(rename_all = "camelCase")]
async fn refresh_provider_model_catalog(
    app: tauri::AppHandle,
    state: tauri::State<'_, AgentState>,
) -> Result<ProviderModelCatalogResponse, PublicError> {
    let adapter = state.xai().ok_or(PublicError::ProviderUnavailable)?;
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
        .plugin(tauri_plugin_dialog::init())
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
            let xai = Arc::new(XaiSubscriptionAdapter::new(native_store()));
            xai.supersede_legacy_chatgpt_credentials();
            let provider: Arc<dyn provider::ProviderAdapter> = Arc::clone(&xai) as _;
            app.manage(ProjectStorageState::ready_shared(Arc::clone(&store)));
            app.manage(DesktopPreferenceState::ready_shared(Arc::clone(&store)));
            app.manage(HarnessState::new(Arc::clone(&store), Arc::clone(&provider)));
            app.manage(AgentState::new(store, provider, Some(xai)));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_application_info,
            create_project,
            list_projects,
            open_project,
            update_project_instructions,
            create_harness_run,
            pick_harness_run_root,
            rebind_harness_run_root,
            get_harness_run,
            get_harness_run_detail,
            bootstrap_harness_plan,
            approve_harness_pair,
            issue_harness_execution_grants,
            execute_harness_run,
            pause_harness_run,
            cancel_harness_run,
            revoke_harness_grant,
            deny_unsupported_harness_operation,
            takeover_harness_root_lease,
            list_agent_sessions,
            get_agent_session,
            create_artifact_from_turn,
            list_artifacts,
            get_artifact,
            export_agent_turn_metrics,
            pick_agent_text_source,
            pick_agent_text_folder_source,
            attach_agent_text_link_source,
            clear_agent_text_source_draft,
            set_agent_source_draft_scope,
            send_agent_message,
            get_model_request_controls,
            cancel_agent_turn,
            set_agent_session_project,
            connection_status,
            sync_connection_status,
            get_xai_device_pairing,
            connect_xai,
            cancel_xai_connect,
            disconnect_xai,
            get_provider_model_catalog,
            get_persisted_provider_model_catalog,
            refresh_provider_model_catalog,
            get_provider_model_selection,
            set_provider_model_selection,
            get_appearance_preference,
            set_appearance_preference,
            open_settings_window,
            take_settings_launch_category,
            exit_application
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit = event
                && let Some(state) = app.try_state::<AgentState>()
            {
                state.clear_source_drafts();
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credentials::FakeCredentialStore;
    use crate::xai_subscription::{
        MockInference, TestTransport, TokenBundle, TokenValues, XaiSubscriptionAdapter,
    };
    use provider::ConnectionState;
    use reqwest::header::HeaderMap;
    use std::sync::{Arc, Mutex};

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

    fn test_device_response() -> xai_subscription::DeviceCodeResponse {
        xai_subscription::DeviceCodeResponse {
            device_code: "device-code".into(),
            user_code: "ABCD-1234".into(),
            verification_uri: "https://auth.x.ai/device".into(),
            verification_uri_complete: None,
            expires_in: Some(300),
            interval: Some(5),
        }
    }

    struct SuccessDeviceConnect;
    impl TestTransport for SuccessDeviceConnect {
        fn request_device_code(
            &self,
            url: &str,
        ) -> Result<xai_subscription::DeviceCodeResponse, PublicError> {
            assert_eq!(url, xai_subscription::DEVICE_CODE_URL);
            Ok(test_device_response())
        }
        fn poll_device_code_token(&self, url: &str, _: &str) -> Result<TokenBundle, PublicError> {
            assert_eq!(url, xai_subscription::TOKEN_URL);
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
            provider_id: "xai-subscription-oauth",
            model: "grok-3",
        };
        let disconnected = ConnectionStatus {
            state: ConnectionState::Disconnected,
            provider_id: "xai-subscription-oauth",
            model: "grok-3",
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

        {
            let fake = Arc::new(FakeCredentialStore::default());
            let adapter = Arc::new(XaiSubscriptionAdapter::new(fake));
            adapter.set_test_transport(Arc::new(SuccessDeviceConnect));
            let dir = tempfile::Builder::new()
                .prefix("tule-connect-cmd-cancel-")
                .tempdir()
                .unwrap();
            let store = Arc::new(SqliteStore::open(dir.path().join("tule.sqlite3")).unwrap());
            let (entered_tx, entered_rx) = std::sync::mpsc::channel();
            let (release_tx, release_rx) = std::sync::mpsc::channel();

            let connect_adapter = Arc::clone(&adapter);
            let connect_store = Arc::clone(&store);
            let connect = tokio::spawn(async move {
                connect_adapter
                    .connect_device_code(
                        connect_store,
                        move |_url| {
                            entered_tx.send(()).unwrap();
                            release_rx.recv().unwrap();
                            Ok(())
                        },
                        None,
                    )
                    .await
            });

            entered_rx.recv().expect("connect entered open_url");
            let sampled_while_connecting = adapter.connection_status_with_store(store.as_ref());
            assert_eq!(sampled_while_connecting.state, ConnectionState::Connecting);

            assert_eq!(cancel_xai_connect_inner(Some(adapter.as_ref())), Ok(()));
            assert!(events.lock().unwrap().is_empty());

            release_tx.send(()).expect("release open_url");
            let result = connect.await.expect("join");
            assert_eq!(result, Err(PublicError::Cancelled));

            let settled = settle_connect_xai(
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

        {
            events.lock().unwrap().clear();
            let fake = Arc::new(FakeCredentialStore::default());
            let adapter = XaiSubscriptionAdapter::new(fake);
            adapter.set_test_transport(Arc::new(SuccessDeviceConnect));
            let dir = tempfile::Builder::new()
                .prefix("tule-connect-cmd-success-")
                .tempdir()
                .unwrap();
            let store = Arc::new(SqliteStore::open(dir.path().join("tule.sqlite3")).unwrap());

            let result = adapter
                .connect_with_test_device_code(Arc::clone(&store))
                .await;
            assert!(matches!(
                &result,
                Ok(status) if status.state == ConnectionState::Connected
            ));

            let settled = settle_connect_xai(
                |status| events.lock().unwrap().push(status.clone()),
                result,
                adapter.connection_status_with_store(store.as_ref()),
            );
            assert_eq!(settled.as_ref().unwrap().state, ConnectionState::Connected);
            assert_eq!(events.lock().unwrap().len(), 1);

            assert_eq!(
                cancel_xai_connect_inner(Some(&adapter)),
                Err(PublicError::InvalidInput)
            );
            let snapshot = events.lock().unwrap().clone();
            assert_eq!(snapshot.len(), 1);
            assert_eq!(snapshot[0].state, ConnectionState::Connected);
            assert!(!connecting_follows_terminal(&snapshot));
        }

        {
            events.lock().unwrap().clear();
            let adapter = XaiSubscriptionAdapter::new(Arc::new(FakeCredentialStore::default()));
            adapter.set_test_transport(Arc::new(SuccessDeviceConnect));
            let dir = tempfile::Builder::new()
                .prefix("tule-connect-cmd-fail-")
                .tempdir()
                .unwrap();
            let store = Arc::new(SqliteStore::open(dir.path().join("tule.sqlite3")).unwrap());

            let result = adapter
                .connect_device_code(
                    Arc::clone(&store),
                    |_url| Err(PublicError::ProviderUnavailable),
                    None,
                )
                .await;
            assert_eq!(result, Err(PublicError::ProviderUnavailable));

            let settled = settle_connect_xai(
                |status| events.lock().unwrap().push(status.clone()),
                result,
                adapter.connection_status_with_store(store.as_ref()),
            );
            assert_eq!(settled, Err(PublicError::ProviderUnavailable));
            assert_eq!(
                cancel_xai_connect_inner(Some(&adapter)),
                Err(PublicError::InvalidInput)
            );
            let snapshot = events.lock().unwrap().clone();
            assert_eq!(snapshot.len(), 1);
            assert_ne!(snapshot[0].state, ConnectionState::Connecting);
            assert!(!connecting_follows_terminal(&snapshot));
        }
    }
}
