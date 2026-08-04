//! Ephemeral main-window Source drafts. Paths are read once and never retained.

use std::{
    collections::HashMap,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::Mutex,
};

use tule_core::{
    AgentSessionId, MAX_SOURCE_UTF8, SOURCE_ORIGIN_LOCAL_TEXT_FILE, Source, SourceValidationError,
    validate_source_content, validate_source_display_name,
};

/// Native-owned composer scope for draft-handle binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ComposerScope {
    /// Bound to a persisted Agent session.
    Session(AgentSessionId),
    /// Bound to one host-generated new-session generation.
    NewSession { generation: u64 },
}

/// In-memory draft captured from one native picker selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SourceDraft {
    pub(crate) display_name: String,
    pub(crate) content: String,
    scope: ComposerScope,
}

impl SourceDraft {
    pub(crate) fn scope(&self) -> &ComposerScope {
        &self.scope
    }

    pub(crate) fn into_source(self) -> Result<Source, SourceValidationError> {
        Source::new_local_text(self.display_name, self.content)
    }
}

/// Process-scoped draft handles for the main-window composer.
#[derive(Debug)]
pub(crate) struct SourceDraftStore {
    drafts: Mutex<HashMap<String, SourceDraft>>,
    current_scope: Mutex<ComposerScope>,
    next_new_session_generation: Mutex<u64>,
}

impl Default for SourceDraftStore {
    fn default() -> Self {
        Self {
            drafts: Mutex::new(HashMap::new()),
            current_scope: Mutex::new(ComposerScope::NewSession { generation: 0 }),
            next_new_session_generation: Mutex::new(1),
        }
    }
}

impl SourceDraftStore {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn current_scope(&self) -> ComposerScope {
        self.current_scope
            .lock()
            .expect("source draft scope lock")
            .clone()
    }

    /// Binds the composer to a validated session scope, invalidating prior drafts.
    pub(crate) fn bind_session_scope(&self, session_id: AgentSessionId) {
        let next = ComposerScope::Session(session_id);
        let mut scope = self.current_scope.lock().expect("source draft scope lock");
        if *scope != next {
            *scope = next;
            self.drafts.lock().expect("source draft map lock").clear();
        }
    }

    /// Advances the host-owned new-session generation and invalidates prior drafts.
    pub(crate) fn begin_new_session_scope(&self) {
        let generation = {
            let mut next = self
                .next_new_session_generation
                .lock()
                .expect("source draft generation lock");
            let generation = *next;
            *next = next.saturating_add(1);
            generation
        };
        *self.current_scope.lock().expect("source draft scope lock") =
            ComposerScope::NewSession { generation };
        self.drafts.lock().expect("source draft map lock").clear();
    }

    pub(crate) fn clear_all(&self) {
        self.drafts.lock().expect("source draft map lock").clear();
    }

    pub(crate) fn clear_handle(&self, handle: &str) {
        self.drafts
            .lock()
            .expect("source draft map lock")
            .remove(handle);
    }

    pub(crate) fn insert(
        &self,
        display_name: String,
        content: String,
    ) -> Result<String, SourceDraftError> {
        let handle = generate_draft_handle()?;
        let scope = self.current_scope();
        self.drafts.lock().expect("source draft map lock").insert(
            handle.clone(),
            SourceDraft {
                display_name,
                content,
                scope,
            },
        );
        Ok(handle)
    }

    pub(crate) fn get(&self, handle: &str) -> Option<SourceDraft> {
        self.drafts
            .lock()
            .expect("source draft map lock")
            .get(handle)
            .cloned()
    }

    /// Resolves a handle only when its bound scope matches the actual send target.
    pub(crate) fn resolve_for_send(
        &self,
        handle: &str,
        send_target: &ComposerScope,
    ) -> Option<SourceDraft> {
        let draft = self.get(handle)?;
        if draft.scope() == send_target {
            Some(draft)
        } else {
            None
        }
    }

    pub(crate) fn replace_with(
        &self,
        display_name: String,
        content: String,
    ) -> Result<String, SourceDraftError> {
        self.clear_all();
        self.insert(display_name, content)
    }

    /// Builds the send-target scope for an optional session identifier.
    ///
    /// A new-session send only matches the current host new-session generation.
    pub(crate) fn send_target_for_session(
        &self,
        session_id: Option<AgentSessionId>,
    ) -> Option<ComposerScope> {
        match session_id {
            Some(id) => Some(ComposerScope::Session(id)),
            None => match self.current_scope() {
                scope @ ComposerScope::NewSession { .. } => Some(scope),
                ComposerScope::Session(_) => None,
            },
        }
    }
}

/// Result of a native picker interaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PickSourceOutcome {
    Cancelled,
    Selected {
        draft_handle: String,
        display_name: String,
        byte_count: u64,
        origin_kind: String,
    },
}

/// Failures while capturing a local text Source draft.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SourceDraftError {
    Unreadable,
    Unsupported,
    TooLarge,
    RandomUnavailable,
}

/// Picker boundary used by production and tests.
pub(crate) trait SourceFilePicker: Send + Sync {
    fn pick_file(&self) -> Option<PathBuf>;
}

/// One-time bounded reader boundary used by production and tests.
pub(crate) trait SourceFileReader: Send + Sync {
    fn read_regular_file(&self, path: &Path) -> Result<Vec<u8>, SourceDraftError>;
}

/// Operating-system file reader with a hard byte ceiling.
#[derive(Debug, Default)]
pub(crate) struct NativeSourceFileReader;

impl SourceFileReader for NativeSourceFileReader {
    fn read_regular_file(&self, path: &Path) -> Result<Vec<u8>, SourceDraftError> {
        let metadata = std::fs::metadata(path).map_err(|_| SourceDraftError::Unreadable)?;
        if !metadata.is_file() {
            return Err(SourceDraftError::Unsupported);
        }
        if metadata.len() > MAX_SOURCE_UTF8 as u64 {
            return Err(SourceDraftError::TooLarge);
        }
        let mut file = File::open(path).map_err(|_| SourceDraftError::Unreadable)?;
        // Re-check after open; reject non-regular reopen races conservatively.
        let opened = file.metadata().map_err(|_| SourceDraftError::Unreadable)?;
        if !opened.is_file() {
            return Err(SourceDraftError::Unsupported);
        }
        let mut buffer = Vec::new();
        file.seek(SeekFrom::Start(0))
            .map_err(|_| SourceDraftError::Unreadable)?;
        let read = file
            .by_ref()
            .take((MAX_SOURCE_UTF8 as u64) + 1)
            .read_to_end(&mut buffer)
            .map_err(|_| SourceDraftError::Unreadable)?;
        if read > MAX_SOURCE_UTF8 {
            return Err(SourceDraftError::TooLarge);
        }
        Ok(buffer)
    }
}

/// Captures one selected file into an ephemeral draft without retaining the path.
pub(crate) fn capture_picked_source<P, R>(
    store: &SourceDraftStore,
    picker: &P,
    reader: &R,
) -> Result<PickSourceOutcome, SourceDraftError>
where
    P: SourceFilePicker + ?Sized,
    R: SourceFileReader + ?Sized,
{
    let Some(path) = picker.pick_file() else {
        return Ok(PickSourceOutcome::Cancelled);
    };
    let display_name = lossless_basename(&path).ok_or(SourceDraftError::Unsupported)?;
    validate_source_display_name(&display_name).map_err(map_validation)?;
    let bytes = reader.read_regular_file(&path)?;
    // Path is dropped when this function returns; never store it.
    drop(path);
    let content = std::str::from_utf8(&bytes).map_err(|_| SourceDraftError::Unsupported)?;
    validate_source_content(content).map_err(map_validation)?;
    let byte_count = content.len() as u64;
    let draft_handle = store.replace_with(display_name.clone(), content.to_owned())?;
    Ok(PickSourceOutcome::Selected {
        draft_handle,
        display_name,
        byte_count,
        origin_kind: SOURCE_ORIGIN_LOCAL_TEXT_FILE.to_owned(),
    })
}

fn lossless_basename(path: &Path) -> Option<String> {
    path.file_name()?.to_str().map(str::to_owned)
}

fn map_validation(error: SourceValidationError) -> SourceDraftError {
    match error {
        SourceValidationError::TooLarge { .. } => SourceDraftError::TooLarge,
        SourceValidationError::UnsafeDisplayName
        | SourceValidationError::ContainsNul
        | SourceValidationError::Time(_) => SourceDraftError::Unsupported,
    }
}

fn generate_draft_handle() -> Result<String, SourceDraftError> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| SourceDraftError::RandomUnavailable)?;
    let mut handle = String::with_capacity(32);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut handle, "{byte:02x}").expect("writing to String cannot fail");
    }
    Ok(handle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct FakePicker {
        path: Mutex<Option<PathBuf>>,
    }

    impl SourceFilePicker for FakePicker {
        fn pick_file(&self) -> Option<PathBuf> {
            self.path.lock().unwrap().clone()
        }
    }

    struct FakeReader {
        bytes: Mutex<Result<Vec<u8>, SourceDraftError>>,
        reads: Mutex<u32>,
    }

    impl SourceFileReader for FakeReader {
        fn read_regular_file(&self, _path: &Path) -> Result<Vec<u8>, SourceDraftError> {
            *self.reads.lock().unwrap() += 1;
            self.bytes.lock().unwrap().clone()
        }
    }

    #[test]
    fn cancel_leaves_store_empty() {
        let store = SourceDraftStore::new();
        let picker = FakePicker {
            path: Mutex::new(None),
        };
        let reader = FakeReader {
            bytes: Mutex::new(Ok(Vec::new())),
            reads: Mutex::new(0),
        };
        assert_eq!(
            capture_picked_source(&store, &picker, &reader).unwrap(),
            PickSourceOutcome::Cancelled
        );
        assert_eq!(*reader.reads.lock().unwrap(), 0);
        assert!(store.get("missing").is_none());
    }

    #[test]
    fn successful_capture_reads_once_and_returns_metadata_only() {
        let store = SourceDraftStore::new();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("notes.txt");
        std::fs::write(&path, "hello\r\n").unwrap();
        let picker = FakePicker {
            path: Mutex::new(Some(path)),
        };
        let reader = NativeSourceFileReader;
        let outcome = capture_picked_source(&store, &picker, &reader).unwrap();
        let PickSourceOutcome::Selected {
            draft_handle,
            display_name,
            byte_count,
            origin_kind,
        } = outcome
        else {
            panic!("expected selection");
        };
        assert_eq!(display_name, "notes.txt");
        assert_eq!(byte_count, 7);
        assert_eq!(origin_kind, SOURCE_ORIGIN_LOCAL_TEXT_FILE);
        assert_eq!(draft_handle.len(), 32);
        let draft = store.get(&draft_handle).unwrap();
        assert_eq!(draft.content, "hello\r\n");
        assert!(matches!(
            draft.scope(),
            ComposerScope::NewSession { generation: 0 }
        ));
    }

    #[test]
    fn rejects_invalid_utf8_nul_oversize_and_unsafe_names() {
        let store = SourceDraftStore::new();
        let picker = FakePicker {
            path: Mutex::new(Some(PathBuf::from("bad\nname.txt"))),
        };
        let reader = FakeReader {
            bytes: Mutex::new(Ok(b"ok".to_vec())),
            reads: Mutex::new(0),
        };
        assert!(matches!(
            capture_picked_source(&store, &picker, &reader),
            Err(SourceDraftError::Unsupported)
        ));

        let picker = FakePicker {
            path: Mutex::new(Some(PathBuf::from("ok.txt"))),
        };
        *reader.bytes.lock().unwrap() = Ok(vec![0xff, 0xfe]);
        assert!(matches!(
            capture_picked_source(&store, &picker, &reader),
            Err(SourceDraftError::Unsupported)
        ));
        *reader.bytes.lock().unwrap() = Ok(b"a\0b".to_vec());
        assert!(matches!(
            capture_picked_source(&store, &picker, &reader),
            Err(SourceDraftError::Unsupported)
        ));
        *reader.bytes.lock().unwrap() = Ok(vec![b'a'; MAX_SOURCE_UTF8 + 1]);
        assert!(matches!(
            capture_picked_source(&store, &picker, &reader),
            Err(SourceDraftError::TooLarge)
        ));
    }

    #[test]
    fn cross_session_handle_cannot_be_substituted() {
        let store = SourceDraftStore::new();
        let session_a = AgentSessionId::generate();
        let session_b = AgentSessionId::generate();
        store.bind_session_scope(session_a);
        let handle = store.insert("a.txt".into(), "from-a".into()).unwrap();
        assert!(
            store
                .resolve_for_send(&handle, &ComposerScope::Session(session_a))
                .is_some()
        );
        assert!(
            store
                .resolve_for_send(&handle, &ComposerScope::Session(session_b))
                .is_none()
        );
        assert!(
            store
                .resolve_for_send(&handle, &ComposerScope::NewSession { generation: 0 })
                .is_none()
        );
    }

    #[test]
    fn repeated_new_session_scope_advances_and_invalidates() {
        let store = SourceDraftStore::new();
        let first = store.insert("a.txt".into(), "one".into()).unwrap();
        store.begin_new_session_scope();
        assert!(store.get(&first).is_none());
        let second = store.insert("b.txt".into(), "two".into()).unwrap();
        let generation = match store.current_scope() {
            ComposerScope::NewSession { generation } => generation,
            ComposerScope::Session(_) => panic!("expected new-session scope"),
        };
        assert!(
            store
                .resolve_for_send(&second, &ComposerScope::NewSession { generation })
                .is_some()
        );
        store.begin_new_session_scope();
        assert!(store.get(&second).is_none());
        assert_ne!(
            store.current_scope(),
            ComposerScope::NewSession { generation }
        );
    }

    #[test]
    fn stale_and_reordered_scope_commands_cannot_revive_handles() {
        let store = SourceDraftStore::new();
        let session_a = AgentSessionId::generate();
        let session_b = AgentSessionId::generate();
        store.bind_session_scope(session_a);
        let handle = store.insert("a.txt".into(), "body".into()).unwrap();
        store.bind_session_scope(session_b);
        assert!(store.get(&handle).is_none());
        store.bind_session_scope(session_a);
        assert!(store.get(&handle).is_none());
        assert!(
            store
                .resolve_for_send(&handle, &ComposerScope::Session(session_a))
                .is_none()
        );
    }

    #[test]
    fn unknown_and_cleared_handles_are_expired() {
        let store = SourceDraftStore::new();
        assert!(
            store
                .resolve_for_send("deadbeef".repeat(4).as_str(), &store.current_scope())
                .is_none()
        );
        let handle = store.insert("a.txt".into(), "x".into()).unwrap();
        store.clear_handle(&handle);
        assert!(
            store
                .resolve_for_send(&handle, &store.current_scope())
                .is_none()
        );
        let handle = store.insert("a.txt".into(), "x".into()).unwrap();
        store.clear_all();
        assert!(
            store
                .resolve_for_send(&handle, &store.current_scope())
                .is_none()
        );
    }

    #[test]
    fn draft_handle_uses_cryptographic_randomness() {
        let first = generate_draft_handle().unwrap();
        let second = generate_draft_handle().unwrap();
        assert_ne!(first, second);
        assert_eq!(first.len(), 32);
        assert!(first.chars().all(|ch| matches!(ch, '0'..='9' | 'a'..='f')));
    }
}
