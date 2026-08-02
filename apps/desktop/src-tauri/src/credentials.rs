//! Native-only credential storage for provider adapter secrets.

#[cfg(test)]
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

/// Windows Credential Manager's per-entry blob limit.
pub(crate) const CRED_MAX_CREDENTIAL_BLOB_SIZE: usize = 2560;

/// The three protected values required by the compatibility adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum CredentialKind {
    AccessToken,
    RefreshToken,
    AccountId,
}

impl CredentialKind {
    const fn suffix(self) -> &'static str {
        match self {
            Self::AccessToken => "access",
            Self::RefreshToken => "refresh",
            Self::AccountId => "account",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CredentialStoreError {
    Unavailable,
    ValueTooLarge,
}

/// Opaque native credential storage. Values never cross the IPC boundary.
pub(crate) trait CredentialStore: Send + Sync {
    fn read(
        &self,
        handle: &str,
        kind: CredentialKind,
    ) -> Result<Option<Vec<u8>>, CredentialStoreError>;
    fn replace(
        &self,
        handle: &str,
        kind: CredentialKind,
        value: &[u8],
    ) -> Result<(), CredentialStoreError>;
    fn delete(&self, handle: &str, kind: CredentialKind) -> Result<(), CredentialStoreError>;
}

/// Production store selection. Unsupported hosts fail closed.
pub(crate) fn native_store() -> Arc<dyn CredentialStore> {
    #[cfg(windows)]
    {
        Arc::new(WindowsCredentialStore::new())
    }
    #[cfg(not(windows))]
    {
        Arc::new(UnavailableCredentialStore)
    }
}

#[cfg(not(windows))]
pub(crate) struct UnavailableCredentialStore;

#[cfg(not(windows))]
impl CredentialStore for UnavailableCredentialStore {
    fn read(&self, _: &str, _: CredentialKind) -> Result<Option<Vec<u8>>, CredentialStoreError> {
        Err(CredentialStoreError::Unavailable)
    }
    fn replace(&self, _: &str, _: CredentialKind, _: &[u8]) -> Result<(), CredentialStoreError> {
        Err(CredentialStoreError::Unavailable)
    }
    fn delete(&self, _: &str, _: CredentialKind) -> Result<(), CredentialStoreError> {
        Err(CredentialStoreError::Unavailable)
    }
}

#[cfg(windows)]
struct WindowsCredentialStore {
    store: std::sync::Arc<windows_native_keyring_store::Store>,
    lock: Mutex<()>,
}

#[cfg(windows)]
impl WindowsCredentialStore {
    fn new() -> Self {
        use windows_native_keyring_store::Store;
        Self {
            store: Store::new()
                .expect("Windows Credential Manager store construction is infallible"),
            lock: Mutex::new(()),
        }
    }

    fn entry(
        &self,
        handle: &str,
        kind: CredentialKind,
    ) -> Result<keyring_core::Entry, CredentialStoreError> {
        use keyring_core::api::CredentialStoreApi;
        self.store
            .build(
                "build.tule.desktop",
                &format!("{handle}.{}", kind.suffix()),
                None,
            )
            .map_err(|_| CredentialStoreError::Unavailable)
    }
}

#[cfg(windows)]
impl CredentialStore for WindowsCredentialStore {
    fn read(
        &self,
        handle: &str,
        kind: CredentialKind,
    ) -> Result<Option<Vec<u8>>, CredentialStoreError> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| CredentialStoreError::Unavailable)?;
        match self.entry(handle, kind)?.get_secret() {
            Ok(value) => Ok(Some(value)),
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(_) => Err(CredentialStoreError::Unavailable),
        }
    }

    fn replace(
        &self,
        handle: &str,
        kind: CredentialKind,
        value: &[u8],
    ) -> Result<(), CredentialStoreError> {
        if value.len() > CRED_MAX_CREDENTIAL_BLOB_SIZE {
            return Err(CredentialStoreError::ValueTooLarge);
        }
        let _guard = self
            .lock
            .lock()
            .map_err(|_| CredentialStoreError::Unavailable)?;
        self.entry(handle, kind)?
            .set_secret(value)
            .map_err(|_| CredentialStoreError::Unavailable)
    }

    fn delete(&self, handle: &str, kind: CredentialKind) -> Result<(), CredentialStoreError> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| CredentialStoreError::Unavailable)?;
        match self.entry(handle, kind)?.delete_credential() {
            Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
            Err(_) => Err(CredentialStoreError::Unavailable),
        }
    }
}

/// Test-only deterministic in-memory credential store with failure injection.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct FakeCredentialStore {
    values: Mutex<HashMap<(String, CredentialKind), Vec<u8>>>,
    fail_at: Mutex<HashSet<usize>>,
    operations: Mutex<usize>,
}

#[cfg(test)]
impl FakeCredentialStore {
    pub(crate) fn fail_on_operation(&self, operation: usize) {
        self.fail_at.lock().unwrap().insert(operation);
    }

    pub(crate) fn fail_on_operations(&self, operations: impl IntoIterator<Item = usize>) {
        self.fail_at.lock().unwrap().extend(operations);
    }

    pub(crate) fn reset_operations(&self) {
        *self.operations.lock().unwrap() = 0;
        self.fail_at.lock().unwrap().clear();
    }

    pub(crate) fn peek(&self, handle: &str, kind: CredentialKind) -> Option<Vec<u8>> {
        self.values
            .lock()
            .unwrap()
            .get(&(handle.into(), kind))
            .cloned()
    }

    fn may_fail(&self) -> Result<(), CredentialStoreError> {
        let mut operations = self.operations.lock().unwrap();
        *operations += 1;
        if self.fail_at.lock().unwrap().contains(&*operations) {
            return Err(CredentialStoreError::Unavailable);
        }
        Ok(())
    }
}

#[cfg(test)]
impl CredentialStore for FakeCredentialStore {
    fn read(
        &self,
        handle: &str,
        kind: CredentialKind,
    ) -> Result<Option<Vec<u8>>, CredentialStoreError> {
        self.may_fail()?;
        Ok(self
            .values
            .lock()
            .unwrap()
            .get(&(handle.into(), kind))
            .cloned())
    }
    fn replace(
        &self,
        handle: &str,
        kind: CredentialKind,
        value: &[u8],
    ) -> Result<(), CredentialStoreError> {
        self.may_fail()?;
        if value.len() > CRED_MAX_CREDENTIAL_BLOB_SIZE {
            return Err(CredentialStoreError::ValueTooLarge);
        }
        self.values
            .lock()
            .unwrap()
            .insert((handle.into(), kind), value.to_vec());
        Ok(())
    }
    fn delete(&self, handle: &str, kind: CredentialKind) -> Result<(), CredentialStoreError> {
        self.may_fail()?;
        self.values.lock().unwrap().remove(&(handle.into(), kind));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    struct WindowsCredentialCleanup<'a> {
        store: &'a WindowsCredentialStore,
        handle: String,
    }

    #[cfg(windows)]
    impl Drop for WindowsCredentialCleanup<'_> {
        fn drop(&mut self) {
            for kind in [
                CredentialKind::AccessToken,
                CredentialKind::RefreshToken,
                CredentialKind::AccountId,
            ] {
                let _ = self.store.delete(&self.handle, kind);
            }
        }
    }

    #[test]
    fn fake_enforces_windows_blob_limit() {
        let store = FakeCredentialStore::default();
        assert_eq!(
            store.replace(
                "h",
                CredentialKind::AccessToken,
                &vec![0; CRED_MAX_CREDENTIAL_BLOB_SIZE + 1]
            ),
            Err(CredentialStoreError::ValueTooLarge)
        );
    }

    #[test]
    fn fake_round_trips_and_deletes() {
        let store = FakeCredentialStore::default();
        store
            .replace("h", CredentialKind::RefreshToken, b"secret")
            .unwrap();
        assert_eq!(
            store.read("h", CredentialKind::RefreshToken).unwrap(),
            Some(b"secret".to_vec())
        );
        store.delete("h", CredentialKind::RefreshToken).unwrap();
        assert_eq!(store.read("h", CredentialKind::RefreshToken).unwrap(), None);
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "uses the current Windows user's Credential Manager"]
    fn windows_credential_manager_round_trips_and_deletes() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let store = WindowsCredentialStore::new();
        let handle = format!(
            "test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock must be after the Unix epoch")
                .as_nanos()
        );
        let cleanup = WindowsCredentialCleanup {
            store: &store,
            handle,
        };

        store
            .replace(
                &cleanup.handle,
                CredentialKind::RefreshToken,
                b"tule-credential-round-trip",
            )
            .expect("temporary credential must be writable");
        assert_eq!(
            store
                .read(&cleanup.handle, CredentialKind::RefreshToken)
                .expect("temporary credential must be readable"),
            Some(b"tule-credential-round-trip".to_vec())
        );
        store
            .delete(&cleanup.handle, CredentialKind::RefreshToken)
            .expect("temporary credential must be deletable");
        assert_eq!(
            store
                .read(&cleanup.handle, CredentialKind::RefreshToken)
                .expect("deleted credential lookup must succeed"),
            None
        );
    }
}
