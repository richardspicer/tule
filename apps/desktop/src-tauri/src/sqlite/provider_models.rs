//! Non-secret provider model catalog and selected-default persistence.

use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{OptionalExtension, Transaction, params};
use tule_core::{ModelCatalogEntry, PROVIDER_PROFILE_ID};

use super::{SqliteStore, SqliteStoreError};

/// Persisted selected-model default for one provider profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StoredModelSelection {
    pub(crate) selected_model_id: Option<String>,
    pub(crate) updated_at_unix_ms: i64,
}

/// Persisted catalog cache metadata scoped to a credential generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StoredCatalogState {
    pub(crate) credential_generation: i64,
    pub(crate) compatibility_revision: String,
    pub(crate) etag: Option<String>,
    pub(crate) retrieved_at_unix_ms: i64,
    pub(crate) updated_at_unix_ms: i64,
}

/// Last validated catalog snapshot for the current credential generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StoredCatalogSnapshot {
    pub(crate) state: StoredCatalogState,
    pub(crate) entries: Vec<ModelCatalogEntry>,
}

impl SqliteStore {
    pub(crate) fn get_model_selection(
        &self,
        provider_profile_id: &str,
    ) -> Result<StoredModelSelection, SqliteStoreError> {
        let connection = self.connection()?;
        let row = connection
            .query_row(
                "SELECT selected_model_id, updated_at_unix_ms
                 FROM provider_model_selection
                 WHERE provider_profile_id = ?1",
                params![provider_profile_id],
                |row| {
                    Ok(StoredModelSelection {
                        selected_model_id: row.get(0)?,
                        updated_at_unix_ms: row.get(1)?,
                    })
                },
            )
            .optional()
            .map_err(SqliteStoreError::Database)?;
        Ok(row.unwrap_or(StoredModelSelection {
            selected_model_id: None,
            updated_at_unix_ms: 0,
        }))
    }

    pub(crate) fn set_model_selection(
        &self,
        provider_profile_id: &str,
        selected_model_id: Option<&str>,
        updated_at_unix_ms: i64,
    ) -> Result<(), SqliteStoreError> {
        #[cfg(test)]
        if self
            .fail_model_selection_write
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            return Err(SqliteStoreError::Database(rusqlite::Error::InvalidQuery));
        }
        self.connection()?
            .execute(
                "INSERT INTO provider_model_selection (
                provider_profile_id, selected_model_id, updated_at_unix_ms
             ) VALUES (?1, ?2, ?3)
             ON CONFLICT(provider_profile_id) DO UPDATE SET
                selected_model_id = excluded.selected_model_id,
                updated_at_unix_ms = excluded.updated_at_unix_ms",
                params![provider_profile_id, selected_model_id, updated_at_unix_ms],
            )
            .map_err(SqliteStoreError::Database)?;
        Ok(())
    }

    pub(crate) fn get_catalog_snapshot(
        &self,
        provider_profile_id: &str,
    ) -> Result<Option<StoredCatalogSnapshot>, SqliteStoreError> {
        let connection = self.connection()?;
        let state = connection
            .query_row(
                "SELECT credential_generation, compatibility_revision, etag,
                        retrieved_at_unix_ms, updated_at_unix_ms
                 FROM provider_model_catalog_state
                 WHERE provider_profile_id = ?1",
                params![provider_profile_id],
                |row| {
                    Ok(StoredCatalogState {
                        credential_generation: row.get(0)?,
                        compatibility_revision: row.get(1)?,
                        etag: row.get(2)?,
                        retrieved_at_unix_ms: row.get(3)?,
                        updated_at_unix_ms: row.get(4)?,
                    })
                },
            )
            .optional()
            .map_err(SqliteStoreError::Database)?;
        let Some(state) = state else {
            return Ok(None);
        };

        let mut statement = connection
            .prepare(
                "SELECT model_id, display_name, description, sort_order, is_provider_default
                 FROM provider_model_catalog_entries
                 WHERE provider_profile_id = ?1 AND credential_generation = ?2
                 ORDER BY sort_order ASC, model_id ASC",
            )
            .map_err(SqliteStoreError::Database)?;
        let entries = statement
            .query_map(
                params![provider_profile_id, state.credential_generation],
                |row| {
                    let is_provider_default: i64 = row.get(4)?;
                    Ok(ModelCatalogEntry {
                        model_id: row.get(0)?,
                        display_name: row.get(1)?,
                        description: row.get(2)?,
                        sort_order: row.get(3)?,
                        is_provider_default: is_provider_default != 0,
                    })
                },
            )
            .map_err(SqliteStoreError::Database)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(SqliteStoreError::Database)?;
        drop(statement);
        drop(connection);

        let rejected = self.rejected_model_ids(provider_profile_id)?;
        let entries = entries
            .into_iter()
            .filter(|entry| !rejected.iter().any(|model_id| model_id == &entry.model_id))
            .collect::<Vec<_>>();

        if entries.is_empty() {
            return Ok(None);
        }
        Ok(Some(StoredCatalogSnapshot { state, entries }))
    }

    pub(crate) fn replace_catalog_snapshot(
        &self,
        provider_profile_id: &str,
        state: &StoredCatalogState,
        entries: &[ModelCatalogEntry],
    ) -> Result<(), SqliteStoreError> {
        let rejected = self.rejected_model_ids(provider_profile_id)?;
        let entries = entries
            .iter()
            .filter(|entry| !rejected.iter().any(|model_id| model_id == &entry.model_id))
            .cloned()
            .collect::<Vec<_>>();
        if entries.is_empty() {
            // Refuse to persist an empty post-filter snapshot over a usable one.
            return Err(SqliteStoreError::Database(rusqlite::Error::InvalidQuery));
        }
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction()
            .map_err(SqliteStoreError::Database)?;
        replace_catalog_snapshot_tx(&transaction, provider_profile_id, state, &entries)?;
        transaction.commit().map_err(SqliteStoreError::Database)?;
        Ok(())
    }

    pub(crate) fn filter_rejected_catalog_entries(
        &self,
        provider_profile_id: &str,
        entries: Vec<ModelCatalogEntry>,
    ) -> Result<Vec<ModelCatalogEntry>, SqliteStoreError> {
        let rejected = self.rejected_model_ids(provider_profile_id)?;
        Ok(entries
            .into_iter()
            .filter(|entry| !rejected.iter().any(|model_id| model_id == &entry.model_id))
            .collect())
    }

    pub(crate) fn touch_catalog_retrieval(
        &self,
        provider_profile_id: &str,
        retrieved_at_unix_ms: i64,
        updated_at_unix_ms: i64,
        etag: Option<&str>,
    ) -> Result<(), SqliteStoreError> {
        self.connection()?
            .execute(
                "UPDATE provider_model_catalog_state
                 SET retrieved_at_unix_ms = ?2,
                     updated_at_unix_ms = ?3,
                     etag = COALESCE(?4, etag)
                 WHERE provider_profile_id = ?1",
                params![
                    provider_profile_id,
                    retrieved_at_unix_ms,
                    updated_at_unix_ms,
                    etag
                ],
            )
            .map_err(SqliteStoreError::Database)?;
        Ok(())
    }

    pub(crate) fn current_credential_generation(
        &self,
        provider_profile_id: &str,
    ) -> Result<i64, SqliteStoreError> {
        let connection = self.connection()?;
        let generation = connection
            .query_row(
                "SELECT credential_generation
                 FROM provider_model_catalog_state
                 WHERE provider_profile_id = ?1",
                params![provider_profile_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(SqliteStoreError::Database)?;
        Ok(generation.unwrap_or(0))
    }

    /// Invalidates the catalog for a credential lifecycle change without clearing
    /// the separately persisted selected-model default.
    pub(crate) fn invalidate_catalog_for_credential_change(
        &self,
        provider_profile_id: &str,
        updated_at_unix_ms: i64,
    ) -> Result<i64, SqliteStoreError> {
        #[cfg(test)]
        if self
            .fail_catalog_invalidation
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            return Err(SqliteStoreError::Database(rusqlite::Error::InvalidQuery));
        }
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction()
            .map_err(SqliteStoreError::Database)?;
        let previous = transaction
            .query_row(
                "SELECT credential_generation
                 FROM provider_model_catalog_state
                 WHERE provider_profile_id = ?1",
                params![provider_profile_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(SqliteStoreError::Database)?
            .unwrap_or(0);
        let next = previous.saturating_add(1);
        transaction
            .execute(
                "DELETE FROM provider_model_catalog_entries
                 WHERE provider_profile_id = ?1",
                params![provider_profile_id],
            )
            .map_err(SqliteStoreError::Database)?;
        transaction
            .execute(
                "DELETE FROM provider_rejected_models
                 WHERE provider_profile_id = ?1",
                params![provider_profile_id],
            )
            .map_err(SqliteStoreError::Database)?;
        transaction
            .execute(
                "INSERT INTO provider_model_catalog_state (
                    provider_profile_id, credential_generation, compatibility_revision,
                    etag, retrieved_at_unix_ms, updated_at_unix_ms
                 ) VALUES (?1, ?2, '', NULL, 0, ?3)
                 ON CONFLICT(provider_profile_id) DO UPDATE SET
                    credential_generation = excluded.credential_generation,
                    compatibility_revision = '',
                    etag = NULL,
                    retrieved_at_unix_ms = 0,
                    updated_at_unix_ms = excluded.updated_at_unix_ms",
                params![provider_profile_id, next, updated_at_unix_ms],
            )
            .map_err(SqliteStoreError::Database)?;
        // Clear the pre-transition quarantine in the same transaction that
        // makes the prior credential generation structurally unreadable.
        transaction
            .execute(
                "DELETE FROM provider_model_catalog_quarantine
                 WHERE provider_profile_id = ?1",
                params![provider_profile_id],
            )
            .map_err(SqliteStoreError::Database)?;
        transaction.commit().map_err(SqliteStoreError::Database)?;
        Ok(next)
    }

    /// Deletes catalog entries without advancing generation metadata. Used only
    /// when compensation cannot restore the prior credential generation.
    pub(crate) fn scrub_catalog_entries(
        &self,
        provider_profile_id: &str,
    ) -> Result<(), SqliteStoreError> {
        #[cfg(test)]
        if self
            .fail_catalog_scrub
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            return Err(SqliteStoreError::Database(rusqlite::Error::InvalidQuery));
        }
        let connection = self.connection()?;
        connection
            .execute(
                "DELETE FROM provider_model_catalog_entries
                 WHERE provider_profile_id = ?1",
                params![provider_profile_id],
            )
            .map_err(SqliteStoreError::Database)?;
        connection
            .execute(
                "DELETE FROM provider_rejected_models
                 WHERE provider_profile_id = ?1",
                params![provider_profile_id],
            )
            .map_err(SqliteStoreError::Database)?;
        connection
            .execute(
                "UPDATE provider_model_catalog_state
                 SET compatibility_revision = '',
                     etag = NULL,
                     retrieved_at_unix_ms = 0
                 WHERE provider_profile_id = ?1",
                params![provider_profile_id],
            )
            .map_err(SqliteStoreError::Database)?;
        Ok(())
    }

    pub(crate) fn record_rejected_model(
        &self,
        provider_profile_id: &str,
        model_id: &str,
        rejected_at_unix_ms: i64,
    ) -> Result<(), SqliteStoreError> {
        let generation = self.current_credential_generation(provider_profile_id)?;
        self.connection()?
            .execute(
                "INSERT INTO provider_rejected_models (
                    provider_profile_id, model_id, credential_generation, rejected_at_unix_ms
                 ) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(provider_profile_id, model_id) DO UPDATE SET
                    credential_generation = excluded.credential_generation,
                    rejected_at_unix_ms = excluded.rejected_at_unix_ms",
                params![
                    provider_profile_id,
                    model_id,
                    generation,
                    rejected_at_unix_ms
                ],
            )
            .map_err(SqliteStoreError::Database)?;
        Ok(())
    }

    pub(crate) fn rejected_model_ids(
        &self,
        provider_profile_id: &str,
    ) -> Result<Vec<String>, SqliteStoreError> {
        let generation = self.current_credential_generation(provider_profile_id)?;
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(
                "SELECT model_id
                 FROM provider_rejected_models
                 WHERE provider_profile_id = ?1 AND credential_generation = ?2
                 ORDER BY model_id ASC",
            )
            .map_err(SqliteStoreError::Database)?;
        let ids = statement
            .query_map(params![provider_profile_id, generation], |row| row.get(0))
            .map_err(SqliteStoreError::Database)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(SqliteStoreError::Database)?;
        Ok(ids)
    }

    pub(crate) fn is_model_rejected(
        &self,
        provider_profile_id: &str,
        model_id: &str,
    ) -> Result<bool, SqliteStoreError> {
        let generation = self.current_credential_generation(provider_profile_id)?;
        let exists = self
            .connection()?
            .query_row(
                "SELECT 1
                 FROM provider_rejected_models
                 WHERE provider_profile_id = ?1
                   AND model_id = ?2
                   AND credential_generation = ?3",
                params![provider_profile_id, model_id, generation],
                |_| Ok(()),
            )
            .optional()
            .map_err(SqliteStoreError::Database)?;
        Ok(exists.is_some())
    }

    pub(crate) fn ensure_builtin_model_selection(&self) -> Result<(), SqliteStoreError> {
        let now: i64 = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| SqliteStoreError::Clock)?
            .as_millis()
            .try_into()
            .map_err(|_| SqliteStoreError::Clock)?;
        let connection = self.connection()?;
        connection
            .execute(
                "INSERT INTO provider_model_selection (
                    provider_profile_id, selected_model_id, updated_at_unix_ms
                 ) VALUES (?1, 'grok-3', ?2)
                 ON CONFLICT(provider_profile_id) DO NOTHING",
                params![PROVIDER_PROFILE_ID, now],
            )
            .map_err(SqliteStoreError::Database)?;
        Ok(())
    }
}

fn replace_catalog_snapshot_tx(
    transaction: &Transaction<'_>,
    provider_profile_id: &str,
    state: &StoredCatalogState,
    entries: &[ModelCatalogEntry],
) -> Result<(), SqliteStoreError> {
    transaction
        .execute(
            "DELETE FROM provider_model_catalog_entries
             WHERE provider_profile_id = ?1",
            params![provider_profile_id],
        )
        .map_err(SqliteStoreError::Database)?;
    transaction
        .execute(
            "INSERT INTO provider_model_catalog_state (
                provider_profile_id, credential_generation, compatibility_revision,
                etag, retrieved_at_unix_ms, updated_at_unix_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(provider_profile_id) DO UPDATE SET
                credential_generation = excluded.credential_generation,
                compatibility_revision = excluded.compatibility_revision,
                etag = excluded.etag,
                retrieved_at_unix_ms = excluded.retrieved_at_unix_ms,
                updated_at_unix_ms = excluded.updated_at_unix_ms",
            params![
                provider_profile_id,
                state.credential_generation,
                state.compatibility_revision,
                state.etag,
                state.retrieved_at_unix_ms,
                state.updated_at_unix_ms
            ],
        )
        .map_err(SqliteStoreError::Database)?;
    for entry in entries {
        transaction
            .execute(
                "INSERT INTO provider_model_catalog_entries (
                    provider_profile_id, credential_generation, model_id, display_name,
                    description, sort_order, is_provider_default
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    provider_profile_id,
                    state.credential_generation,
                    entry.model_id,
                    entry.display_name,
                    entry.description,
                    entry.sort_order,
                    i64::from(entry.is_provider_default)
                ],
            )
            .map_err(SqliteStoreError::Database)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn selection_survives_catalog_invalidation() {
        let directory = tempdir().unwrap();
        let store = SqliteStore::open(directory.path().join("models.sqlite3")).unwrap();
        store
            .set_model_selection(PROVIDER_PROFILE_ID, Some("gpt-5.5"), 10)
            .unwrap();
        store.seal_catalog_reads().unwrap();
        assert!(store.catalog_reads_are_sealed().unwrap());
        let generation = store
            .invalidate_catalog_for_credential_change(PROVIDER_PROFILE_ID, 20)
            .unwrap();
        assert_eq!(generation, 1);
        assert!(!store.catalog_reads_are_sealed().unwrap());
        assert_eq!(
            store
                .get_model_selection(PROVIDER_PROFILE_ID)
                .unwrap()
                .selected_model_id
                .as_deref(),
            Some("gpt-5.5")
        );
        assert!(
            store
                .get_catalog_snapshot(PROVIDER_PROFILE_ID)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn catalog_snapshot_round_trips_allowlisted_entries() {
        let directory = tempdir().unwrap();
        let store = SqliteStore::open(directory.path().join("catalog.sqlite3")).unwrap();
        let entries = vec![
            ModelCatalogEntry {
                model_id: "a-model".into(),
                display_name: "A".into(),
                description: Some("desc".into()),
                sort_order: 2,
                is_provider_default: false,
            },
            ModelCatalogEntry {
                model_id: "b-model".into(),
                display_name: "B".into(),
                description: None,
                sort_order: 1,
                is_provider_default: true,
            },
        ];
        store
            .replace_catalog_snapshot(
                PROVIDER_PROFILE_ID,
                &StoredCatalogState {
                    credential_generation: 3,
                    compatibility_revision: "1.0.0".into(),
                    etag: Some("\"etag\"".into()),
                    retrieved_at_unix_ms: 100,
                    updated_at_unix_ms: 100,
                },
                &entries,
            )
            .unwrap();
        let snapshot = store
            .get_catalog_snapshot(PROVIDER_PROFILE_ID)
            .unwrap()
            .expect("snapshot");
        assert_eq!(snapshot.state.credential_generation, 3);
        assert_eq!(snapshot.state.etag.as_deref(), Some("\"etag\""));
        assert_eq!(snapshot.entries[0].model_id, "b-model");
        assert_eq!(snapshot.entries[1].model_id, "a-model");
    }
}
