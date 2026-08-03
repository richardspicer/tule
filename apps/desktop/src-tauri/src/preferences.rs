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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PreferenceCommandError {
    PreferenceStorageUnavailable,
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

/// Persist appearance when possible, always publish the selected value so open
/// windows stay visually synchronized, and still return a bounded storage error
/// when durable write fails.
pub(crate) fn apply_appearance_preference_update<E>(
    value: AppearancePreference,
    persist: impl FnOnce() -> Result<(), PreferenceCommandError>,
    mut emit: impl FnMut(AppearancePreference) -> Result<(), E>,
) -> Result<AppearancePreference, PreferenceCommandError> {
    let persist_result = persist();
    let _ = emit(value);
    persist_result.map(|()| value)
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) fn set_appearance_preference(
    app: AppHandle,
    state: State<'_, DesktopPreferenceState>,
    value: AppearancePreference,
) -> Result<AppearancePreference, PreferenceCommandError> {
    apply_appearance_preference_update(
        value,
        || match state.store() {
            Some(store) => store
                .set_appearance_preference(value)
                .map_err(|_| PreferenceCommandError::PreferenceStorageUnavailable),
            None => Err(PreferenceCommandError::PreferenceStorageUnavailable),
        },
        |preference| app.emit(APPEARANCE_CHANGED_EVENT, preference),
    )
}

#[cfg(test)]
mod tests {
    use super::{AppearancePreference, PreferenceCommandError, apply_appearance_preference_update};

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

    #[test]
    fn preference_storage_error_is_bounded() {
        let json =
            serde_json::to_value(PreferenceCommandError::PreferenceStorageUnavailable).unwrap();
        assert_eq!(json, serde_json::json!("preference_storage_unavailable"));
    }

    #[test]
    fn appearance_changed_emits_even_when_persistence_rejects() {
        let mut emissions = Vec::new();
        let result = apply_appearance_preference_update(
            AppearancePreference::Dark,
            || Err(PreferenceCommandError::PreferenceStorageUnavailable),
            |value| {
                emissions.push(value);
                Ok::<(), ()>(())
            },
        );

        assert_eq!(
            result,
            Err(PreferenceCommandError::PreferenceStorageUnavailable)
        );
        assert_eq!(emissions, vec![AppearancePreference::Dark]);
    }

    #[test]
    fn appearance_changed_emits_after_successful_persistence() {
        let mut emissions = Vec::new();
        let result = apply_appearance_preference_update(
            AppearancePreference::Light,
            || Ok(()),
            |value| {
                emissions.push(value);
                Ok::<(), ()>(())
            },
        );

        assert_eq!(result, Ok(AppearancePreference::Light));
        assert_eq!(emissions, vec![AppearancePreference::Light]);
    }
}
