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
    Providers,
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

/// Decide whether an already-created Settings window should navigate.
///
/// - Explicit deep links always select their category.
/// - Reopening a hidden window without a deep link starts on Providers.
/// - Refocusing a visible window without a deep link preserves the selection.
/// - Newly created windows use the pending launch category instead.
pub(crate) fn navigation_for_existing_settings(
    was_visible: bool,
    category: Option<SettingsCategory>,
) -> Option<SettingsCategory> {
    match category {
        Some(category) => Some(category),
        None if !was_visible => Some(SettingsCategory::Providers),
        None => None,
    }
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn open_settings_window(
    app: AppHandle,
    launch: State<'_, SettingsLaunchState>,
    category: Option<SettingsCategory>,
) -> Result<(), String> {
    launch.set_pending(category);

    if let Some(existing) = app.get_webview_window(SETTINGS_WINDOW_LABEL) {
        let was_visible = existing.is_visible().unwrap_or(false);
        existing
            .show()
            .map_err(|_| "settings_window_unavailable".to_owned())?;
        existing
            .unminimize()
            .map_err(|_| "settings_window_unavailable".to_owned())?;
        existing
            .set_focus()
            .map_err(|_| "settings_window_unavailable".to_owned())?;

        if let Some(target) = navigation_for_existing_settings(was_visible, category) {
            let _ = existing.emit(SETTINGS_NAVIGATE_EVENT, target);
        }

        return Ok(());
    }

    create_settings_window(&app)?;
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
    use super::{SettingsCategory, SettingsLaunchState, navigation_for_existing_settings};

    #[test]
    fn settings_category_serializes_as_camel_case_enum_strings() {
        assert_eq!(
            serde_json::to_value(SettingsCategory::Providers).unwrap(),
            serde_json::json!("providers")
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

    #[test]
    fn reopen_after_hidden_starts_on_providers() {
        assert_eq!(
            navigation_for_existing_settings(false, None),
            Some(SettingsCategory::Providers)
        );
    }

    #[test]
    fn refocus_of_visible_settings_preserves_category() {
        assert_eq!(navigation_for_existing_settings(true, None), None);
    }

    #[test]
    fn contextual_deep_link_selects_target_category() {
        assert_eq!(
            navigation_for_existing_settings(true, Some(SettingsCategory::Appearance)),
            Some(SettingsCategory::Appearance)
        );
        assert_eq!(
            navigation_for_existing_settings(false, Some(SettingsCategory::Providers)),
            Some(SettingsCategory::Providers)
        );
    }
}
