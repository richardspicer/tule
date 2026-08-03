//! Provider-neutral model catalog and selection contracts.
//!
//! Transport, credentials, and raw provider payloads stay in the host adapter.
//! This module owns allowlisted catalog metadata, freshness, and selection rules.

use std::{error::Error, fmt};

use crate::MODEL_ID;

/// Catalog cache time-to-live in Unix milliseconds.
pub const CATALOG_TTL_MS: i64 = 5 * 60 * 1000;

/// Maximum retained short description length in Unicode scalar values.
pub const CATALOG_DESCRIPTION_MAX_SCALARS: usize = 280;

/// Maximum accepted model identifier length in UTF-8 bytes.
pub const MODEL_ID_MAX_UTF8: usize = 128;

/// Candidate fields extracted by a host adapter before allowlisting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogCandidate {
    /// Stable model identifier (`slug`).
    pub model_id: String,
    /// Human-readable display name.
    pub display_name: String,
    /// Optional short description when present.
    pub description: Option<String>,
    /// Provider visibility label (`list`, `hide`, …).
    pub visibility: String,
    /// Declared input modalities when present.
    pub input_modalities: Option<Vec<String>>,
    /// Provider sort priority (lower first).
    pub sort_order: i32,
    /// Provider-indicated default when present.
    pub is_provider_default: bool,
}

/// Allowlisted catalog entry retained by TULE.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelCatalogEntry {
    /// Stable model identifier.
    pub model_id: String,
    /// Human-readable display name.
    pub display_name: String,
    /// Optional short description.
    pub description: Option<String>,
    /// Provider sort priority (lower first).
    pub sort_order: i32,
    /// Provider-indicated default when present.
    pub is_provider_default: bool,
}

/// Whether a persisted catalog snapshot is still within its TTL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogFreshness {
    /// Retrieved within the TTL window.
    Current,
    /// Older than the TTL; may still be shown as last-known.
    Stale,
}

impl CatalogFreshness {
    /// Returns the stable snake_case public label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::Stale => "stale",
        }
    }
}

/// Result of resolving the profile selected-model default against a catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectedDefaultResolution {
    /// A concrete model identifier is valid in the catalog.
    Available(String),
    /// The user must choose a new default before creating another session.
    RequiresChoice,
}

/// Reason a model identifier is rejected by TULE.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvalidModelId {
    /// Empty or whitespace-only.
    Empty,
    /// Exceeds the accepted byte ceiling.
    TooLarge {
        /// Observed UTF-8 byte count.
        byte_count: usize,
    },
    /// Contains characters outside the allowlisted identifier alphabet.
    Malformed,
}

impl fmt::Display for InvalidModelId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("model identifier is empty"),
            Self::TooLarge { byte_count } => {
                write!(
                    formatter,
                    "model identifier exceeds {MODEL_ID_MAX_UTF8} bytes ({byte_count})"
                )
            }
            Self::Malformed => formatter.write_str("model identifier is malformed"),
        }
    }
}

impl Error for InvalidModelId {}

/// Validates a model identifier before persistence or request assembly.
pub fn validate_model_id(model_id: &str) -> Result<&str, InvalidModelId> {
    let trimmed = model_id.trim();
    if trimmed.is_empty() {
        return Err(InvalidModelId::Empty);
    }
    if trimmed.len() > MODEL_ID_MAX_UTF8 {
        return Err(InvalidModelId::TooLarge {
            byte_count: trimmed.len(),
        });
    }
    if !trimmed
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_' | '/' | ':'))
    {
        return Err(InvalidModelId::Malformed);
    }
    Ok(trimmed)
}

/// Returns whether a candidate may appear in TULE's picker.
///
/// `supported_in_api` is intentionally not consulted: subscription-backed
/// catalog entries remain eligible even when unavailable through the public API.
#[must_use]
pub fn is_usable_catalog_candidate(candidate: &CatalogCandidate) -> bool {
    if validate_model_id(&candidate.model_id).is_err() {
        return false;
    }
    if candidate.display_name.trim().is_empty() {
        return false;
    }
    let visibility = candidate.visibility.trim().to_ascii_lowercase();
    if !visibility.is_empty() && visibility != "list" {
        return false;
    }
    if let Some(modalities) = candidate.input_modalities.as_ref() {
        if modalities.is_empty() {
            return false;
        }
        if !modalities
            .iter()
            .any(|modality| modality.eq_ignore_ascii_case("text"))
        {
            return false;
        }
    }
    true
}

/// Filters, sanitizes, sorts, and deduplicates catalog candidates.
#[must_use]
pub fn select_usable_catalog_entries(
    candidates: impl IntoIterator<Item = CatalogCandidate>,
) -> Vec<ModelCatalogEntry> {
    let mut entries = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for candidate in candidates {
        if !is_usable_catalog_candidate(&candidate) {
            continue;
        }
        let Ok(model_id) = validate_model_id(&candidate.model_id) else {
            continue;
        };
        if !seen.insert(model_id.to_owned()) {
            continue;
        }
        entries.push(ModelCatalogEntry {
            model_id: model_id.to_owned(),
            display_name: candidate.display_name.trim().to_owned(),
            description: sanitize_description(candidate.description.as_deref()),
            sort_order: candidate.sort_order,
            is_provider_default: candidate.is_provider_default,
        });
    }
    entries.sort_by(|left, right| {
        left.sort_order
            .cmp(&right.sort_order)
            .then_with(|| left.model_id.cmp(&right.model_id))
    });
    entries
}

/// Computes catalog freshness from retrieval time.
#[must_use]
pub fn catalog_freshness(retrieved_at_unix_ms: i64, now_unix_ms: i64) -> CatalogFreshness {
    if now_unix_ms.saturating_sub(retrieved_at_unix_ms) >= CATALOG_TTL_MS {
        CatalogFreshness::Stale
    } else {
        CatalogFreshness::Current
    }
}

/// Returns whether `model_id` appears in the allowlisted catalog.
#[must_use]
pub fn model_id_in_catalog(model_id: &str, entries: &[ModelCatalogEntry]) -> bool {
    entries.iter().any(|entry| entry.model_id == model_id)
}

/// Resolves the persisted selected default against the current catalog.
///
/// When no selection is stored, preserves [`MODEL_ID`] only if it remains in the
/// validated catalog; otherwise requires an explicit new choice.
#[must_use]
pub fn resolve_selected_default(
    selected_model_id: Option<&str>,
    entries: &[ModelCatalogEntry],
) -> SelectedDefaultResolution {
    if let Some(selected) = selected_model_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if model_id_in_catalog(selected, entries) {
            return SelectedDefaultResolution::Available(selected.to_owned());
        }
        return SelectedDefaultResolution::RequiresChoice;
    }
    if model_id_in_catalog(MODEL_ID, entries) {
        SelectedDefaultResolution::Available(MODEL_ID.to_owned())
    } else {
        SelectedDefaultResolution::RequiresChoice
    }
}

fn sanitize_description(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    if value.is_empty() {
        return None;
    }
    if value
        .chars()
        .any(|ch| ch.is_control() && ch != '\n' && ch != '\t')
    {
        return None;
    }
    let count = value.chars().count();
    if count <= CATALOG_DESCRIPTION_MAX_SCALARS {
        return Some(value.to_owned());
    }
    let mut truncated: String = value
        .chars()
        .take(CATALOG_DESCRIPTION_MAX_SCALARS.saturating_sub(1))
        .collect();
    truncated.push('…');
    Some(truncated)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(slug: &str, visibility: &str) -> CatalogCandidate {
        CatalogCandidate {
            model_id: slug.to_owned(),
            display_name: slug.to_owned(),
            description: Some("safe".to_owned()),
            visibility: visibility.to_owned(),
            input_modalities: Some(vec!["text".to_owned()]),
            sort_order: 10,
            is_provider_default: false,
        }
    }

    #[test]
    fn filters_hidden_malformed_and_non_text_entries() {
        let entries = select_usable_catalog_entries([
            candidate("gpt-visible", "list"),
            candidate("gpt-hidden", "hide"),
            CatalogCandidate {
                model_id: String::new(),
                display_name: "Empty".to_owned(),
                description: None,
                visibility: "list".to_owned(),
                input_modalities: Some(vec!["text".to_owned()]),
                sort_order: 1,
                is_provider_default: false,
            },
            CatalogCandidate {
                model_id: "image-only".to_owned(),
                display_name: "Image".to_owned(),
                description: None,
                visibility: "list".to_owned(),
                input_modalities: Some(vec!["image".to_owned()]),
                sort_order: 2,
                is_provider_default: false,
            },
            CatalogCandidate {
                model_id: "api-false".to_owned(),
                display_name: "API false".to_owned(),
                description: None,
                visibility: "list".to_owned(),
                input_modalities: None,
                sort_order: 3,
                is_provider_default: true,
            },
        ]);

        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.model_id.as_str())
                .collect::<Vec<_>>(),
            vec!["api-false", "gpt-visible"]
        );
        assert!(entries[0].is_provider_default);
    }

    #[test]
    fn supported_in_api_is_not_an_exclusion_signal() {
        // Absence of modalities defaults to text-capable; visibility list wins.
        assert!(is_usable_catalog_candidate(&candidate("spark", "list")));
    }

    #[test]
    fn freshness_becomes_stale_after_ttl() {
        assert_eq!(
            catalog_freshness(0, CATALOG_TTL_MS - 1),
            CatalogFreshness::Current
        );
        assert_eq!(
            catalog_freshness(0, CATALOG_TTL_MS),
            CatalogFreshness::Stale
        );
    }

    #[test]
    fn selected_default_preserves_gpt_55_only_when_cataloged() {
        let with_default = select_usable_catalog_entries([candidate(MODEL_ID, "list")]);
        assert_eq!(
            resolve_selected_default(None, &with_default),
            SelectedDefaultResolution::Available(MODEL_ID.to_owned())
        );

        let without_default = select_usable_catalog_entries([candidate("other-model", "list")]);
        assert_eq!(
            resolve_selected_default(None, &without_default),
            SelectedDefaultResolution::RequiresChoice
        );
        assert_eq!(
            resolve_selected_default(Some("missing"), &without_default),
            SelectedDefaultResolution::RequiresChoice
        );
        assert_eq!(
            resolve_selected_default(Some("other-model"), &without_default),
            SelectedDefaultResolution::Available("other-model".to_owned())
        );
    }
}
