//! Singleton modeless Settings window lifecycle.

use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{
    AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
    WindowEvent,
};

pub(crate) const SETTINGS_WINDOW_LABEL: &str = "settings";
pub(crate) const SETTINGS_NAVIGATE_EVENT: &str = "settings-navigate";
pub(crate) const CONNECTION_STATUS_CHANGED_EVENT: &str = "connection-status-changed";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SettingsCategory {
    Connections,
    Appearance,
}

#[derive(Default)]
pub(crate) struct SettingsLaunchState {
    pending_category: Mutex<Option<SettingsCategory>>,
}

impl SettingsLaunchState {
    fn set_pending(&self, category: Option<SettingsCategory>) {
        if let Ok(mut pending) = self.pending_category.lock() {
            *pending = category;
        }
    }

    fn take_pending(&self) -> Option<SettingsCategory> {
        self.pending_category
            .lock()
            .ok()
            .and_then(|mut pending| pending.take())
    }
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn open_settings_window(
    app: AppHandle,
    launch: State<'_, SettingsLaunchState>,
    category: Option<SettingsCategory>,
) -> Result<(), String> {
    launch.set_pending(category);

    let created = app.get_webview_window(SETTINGS_WINDOW_LABEL).is_none();
    let window = match app.get_webview_window(SETTINGS_WINDOW_LABEL) {
        Some(existing) => {
            existing
                .show()
                .map_err(|_| "settings_window_unavailable".to_owned())?;
            existing
                .unminimize()
                .map_err(|_| "settings_window_unavailable".to_owned())?;
            existing
                .set_focus()
                .map_err(|_| "settings_window_unavailable".to_owned())?;
            existing
        }
        None => create_settings_window(&app)?,
    };

    // Existing webviews already listen; newly created windows read the pending
    // category during bootstrap via take_settings_launch_category.
    if !created && let Some(category) = category {
        let _ = window.emit(SETTINGS_NAVIGATE_EVENT, category);
    }

    Ok(())
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) fn take_settings_launch_category(
    launch: State<'_, SettingsLaunchState>,
) -> Option<SettingsCategory> {
    launch.take_pending()
}

fn create_settings_window(app: &AppHandle) -> Result<WebviewWindow, String> {
    let window = WebviewWindowBuilder::new(
        app,
        SETTINGS_WINDOW_LABEL,
        WebviewUrl::App("index.html".into()),
    )
    .title("TULE — Settings")
    .inner_size(720.0, 600.0)
    .min_inner_size(560.0, 440.0)
    .resizable(true)
    .center()
    .build()
    .map_err(|_| "settings_window_unavailable".to_owned())?;

    let hide_window = window.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let _ = hide_window.hide();
        }
    });

    Ok(window)
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) fn exit_application(app: AppHandle) {
    app.exit(0);
}

#[cfg(test)]
mod tests {
    use super::{SettingsCategory, SettingsLaunchState};

    #[test]
    fn settings_category_serializes_as_camel_case_enum_strings() {
        assert_eq!(
            serde_json::to_value(SettingsCategory::Connections).unwrap(),
            serde_json::json!("connections")
        );
        assert_eq!(
            serde_json::to_value(SettingsCategory::Appearance).unwrap(),
            serde_json::json!("appearance")
        );
    }

    #[test]
    fn pending_launch_category_is_taken_once() {
        let state = SettingsLaunchState::default();
        state.set_pending(Some(SettingsCategory::Appearance));
        assert_eq!(state.take_pending(), Some(SettingsCategory::Appearance));
        assert_eq!(state.take_pending(), None);
    }
}
