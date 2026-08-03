//! Typed desktop appearance preference persistence and IPC.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

use crate::sqlite::SqliteStore;

pub(crate) const APPEARANCE_CHANGED_EVENT: &str = "appearance-changed";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum AppearancePreference {
    System,
    Light,
    Dark,
}

impl AppearancePreference {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    pub(crate) fn parse(value: &str) -> Self {
        match value {
            "light" => Self::Light,
            "dark" => Self::Dark,
            _ => Self::System,
        }
    }
}

pub(crate) enum DesktopPreferenceState {
    Ready(Arc<SqliteStore>),
    Unavailable,
}

impl DesktopPreferenceState {
    pub(crate) fn ready_shared(store: Arc<SqliteStore>) -> Self {
        Self::Ready(store)
    }

    pub(crate) fn unavailable() -> Self {
        Self::Unavailable
    }

    fn store(&self) -> Option<Arc<SqliteStore>> {
        match self {
            Self::Ready(store) => Some(Arc::clone(store)),
            Self::Unavailable => None,
        }
    }
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) fn get_appearance_preference(
    state: State<'_, DesktopPreferenceState>,
) -> AppearancePreference {
    state
        .store()
        .and_then(|store| store.get_appearance_preference().ok())
        .unwrap_or(AppearancePreference::System)
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) fn set_appearance_preference(
    app: AppHandle,
    state: State<'_, DesktopPreferenceState>,
    value: AppearancePreference,
) -> AppearancePreference {
    if let Some(store) = state.store() {
        let _ = store.set_appearance_preference(value);
    }
    let _ = app.emit(APPEARANCE_CHANGED_EVENT, value);
    value
}

#[cfg(test)]
mod tests {
    use super::AppearancePreference;

    #[test]
    fn invalid_appearance_values_resolve_to_system() {
        assert_eq!(
            AppearancePreference::parse("light"),
            AppearancePreference::Light
        );
        assert_eq!(
            AppearancePreference::parse("dark"),
            AppearancePreference::Dark
        );
        assert_eq!(
            AppearancePreference::parse("system"),
            AppearancePreference::System
        );
        assert_eq!(
            AppearancePreference::parse(""),
            AppearancePreference::System
        );
        assert_eq!(
            AppearancePreference::parse("nope"),
            AppearancePreference::System
        );
    }

    #[test]
    fn appearance_serializes_as_camel_case_enum_strings() {
        let json = serde_json::to_value(AppearancePreference::Dark).unwrap();
        assert_eq!(json, serde_json::json!("dark"));
    }
}
