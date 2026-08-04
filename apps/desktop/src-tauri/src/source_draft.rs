//! Ephemeral main-window Source drafts. Paths are read once and never retained.

use std::{
    collections::HashMap,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::Mutex,
};

use tule_core::{
    MAX_SOURCE_UTF8, SOURCE_ORIGIN_LOCAL_TEXT_FILE, Source, SourceValidationError,
    validate_source_content, validate_source_display_name,
};

/// In-memory draft captured from one native picker selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SourceDraft {
    pub(crate) display_name: String,
    pub(crate) content: String,
}

impl SourceDraft {
    pub(crate) fn byte_count(&self) -> u64 {
        self.content.len() as u64
    }

    pub(crate) fn into_source(self) -> Result<Source, SourceValidationError> {
        Source::new_local_text(self.display_name, self.content)
    }
}

/// Process-scoped draft handles for the main-window composer.
#[derive(Debug, Default)]
pub(crate) struct SourceDraftStore {
    drafts: Mutex<HashMap<String, SourceDraft>>,
    scope_key: Mutex<String>,
}

impl SourceDraftStore {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Sets the current composer scope; changing scope invalidates all drafts.
    pub(crate) fn set_scope(&self, scope_key: impl Into<String>) {
        let next = scope_key.into();
        let mut scope = self.scope_key.lock().expect("source draft scope lock");
        if *scope != next {
            *scope = next;
            self.drafts.lock().expect("source draft map lock").clear();
        }
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

    pub(crate) fn insert(&self, draft: SourceDraft) -> Result<String, SourceDraftError> {
        let handle = generate_draft_handle()?;
        self.drafts
            .lock()
            .expect("source draft map lock")
            .insert(handle.clone(), draft);
        Ok(handle)
    }

    pub(crate) fn get(&self, handle: &str) -> Option<SourceDraft> {
        self.drafts
            .lock()
            .expect("source draft map lock")
            .get(handle)
            .cloned()
    }

    pub(crate) fn replace_with(&self, draft: SourceDraft) -> Result<String, SourceDraftError> {
        self.clear_all();
        self.insert(draft)
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
    let draft = SourceDraft {
        display_name: display_name.clone(),
        content: content.to_owned(),
    };
    let byte_count = draft.byte_count();
    let draft_handle = store.replace_with(draft)?;
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
    fn handle_invalidation_events_and_no_wall_clock_expiry() {
        let store = SourceDraftStore::new();
        let handle = store
            .insert(SourceDraft {
                display_name: "a.txt".into(),
                content: "x".into(),
            })
            .unwrap();
        assert!(store.get(&handle).is_some());
        store.set_scope("session-1");
        assert!(store.get(&handle).is_none());
        let handle = store
            .insert(SourceDraft {
                display_name: "a.txt".into(),
                content: "x".into(),
            })
            .unwrap();
        store.clear_handle(&handle);
        assert!(store.get(&handle).is_none());
        let handle = store
            .replace_with(SourceDraft {
                display_name: "b.txt".into(),
                content: "y".into(),
            })
            .unwrap();
        store.clear_handle(&handle);
        assert!(store.get(&handle).is_none());
        store.clear_all();
        assert!(store.get("unknown").is_none());
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
