//! Ephemeral main-window Source drafts. Paths are read once and never retained.

use std::{
    collections::HashMap,
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::Mutex,
};

use tule_core::{
    AgentSessionId, MAX_FOLDER_MEMBERS, MAX_SOURCE_UTF8, SOURCE_ORIGIN_LOCAL_TEXT_FILE,
    SOURCE_ORIGIN_LOCAL_TEXT_FOLDER, SOURCE_ORIGIN_REMOTE_TEXT_URL, Source, SourceValidationError,
    derive_remote_source_display_name, frame_folder_members, validate_canonical_https_url,
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
    pub(crate) origin_kind: String,
    pub(crate) member_count: u32,
    pub(crate) canonical_url: Option<String>,
    scope: ComposerScope,
}

impl SourceDraft {
    pub(crate) fn scope(&self) -> &ComposerScope {
        &self.scope
    }

    pub(crate) fn into_source(self) -> Result<Source, SourceValidationError> {
        match self.origin_kind.as_str() {
            SOURCE_ORIGIN_LOCAL_TEXT_FILE => {
                Source::new_local_text(self.display_name, self.content)
            }
            SOURCE_ORIGIN_LOCAL_TEXT_FOLDER => {
                let members = parse_folder_draft_members(&self.content)?;
                Source::new_local_text_folder(self.display_name, &members)
            }
            SOURCE_ORIGIN_REMOTE_TEXT_URL => {
                let canonical_url = self
                    .canonical_url
                    .clone()
                    .ok_or(SourceValidationError::InvalidCanonicalUrl)?;
                Source::new_remote_text_url(self.display_name, self.content, canonical_url)
            }
            _ => Err(SourceValidationError::UnsafeDisplayName),
        }
    }
}

fn parse_folder_draft_members(
    content: &str,
) -> Result<Vec<(String, String)>, SourceValidationError> {
    let mut members = Vec::new();
    let mut offset = 0;
    while offset < content.len() {
        let rest = content
            .get(offset..)
            .ok_or(SourceValidationError::UnsafeDisplayName)?;
        let Some(after_member) = rest.strip_prefix("member: ") else {
            return Err(SourceValidationError::UnsafeDisplayName);
        };
        let Some((basename, after_basename)) = after_member.split_once('\n') else {
            return Err(SourceValidationError::UnsafeDisplayName);
        };
        validate_source_display_name(basename)?;
        let Some(after_bytes_label) = after_basename.strip_prefix("content-bytes: ") else {
            return Err(SourceValidationError::UnsafeDisplayName);
        };
        let Some((len_text, after_len)) = after_bytes_label.split_once('\n') else {
            return Err(SourceValidationError::UnsafeDisplayName);
        };
        let byte_len: usize = len_text
            .parse()
            .map_err(|_| SourceValidationError::UnsafeDisplayName)?;
        let header_len = rest.len() - after_len.len();
        let body_start = offset + header_len;
        let body_end = body_start
            .checked_add(byte_len)
            .ok_or(SourceValidationError::TooLarge {
                byte_count: content.len(),
            })?;
        let body = content
            .get(body_start..body_end)
            .ok_or(SourceValidationError::UnsafeDisplayName)?
            .to_owned();
        validate_source_content(&body)?;
        members.push((basename.to_owned(), body));
        offset = body_end;
    }
    Ok(members)
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
        origin_kind: String,
        member_count: u32,
        canonical_url: Option<String>,
    ) -> Result<String, SourceDraftError> {
        let handle = generate_draft_handle()?;
        let scope = self.current_scope();
        self.drafts.lock().expect("source draft map lock").insert(
            handle.clone(),
            SourceDraft {
                display_name,
                content,
                origin_kind,
                member_count,
                canonical_url,
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
        origin_kind: String,
        member_count: u32,
        canonical_url: Option<String>,
    ) -> Result<String, SourceDraftError> {
        self.clear_all();
        self.insert(
            display_name,
            content,
            origin_kind,
            member_count,
            canonical_url,
        )
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
        member_count: u32,
        canonical_url: Option<String>,
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

/// Folder picker boundary used by production and tests.
pub(crate) trait SourceFolderPicker: Send + Sync {
    fn pick_folder(&self) -> Option<PathBuf>;
}

/// One-time bounded reader boundary used by production and tests.
pub(crate) trait SourceFileReader: Send + Sync {
    fn read_regular_file(&self, path: &Path) -> Result<Vec<u8>, SourceDraftError>;
}

/// Shallow folder enumeration boundary used by production and tests.
pub(crate) trait SourceFolderReader: Send + Sync {
    fn read_shallow_folder(&self, path: &Path) -> Result<Vec<(String, Vec<u8>)>, SourceDraftError>;
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

/// Operating-system shallow folder reader with aggregate and member ceilings.
#[derive(Debug, Default)]
pub(crate) struct NativeSourceFolderReader;

impl SourceFolderReader for NativeSourceFolderReader {
    fn read_shallow_folder(&self, path: &Path) -> Result<Vec<(String, Vec<u8>)>, SourceDraftError> {
        collect_shallow_folder_members(path, &NativeSourceFileReader)
    }
}

fn collect_shallow_folder_members(
    path: &Path,
    file_reader: &impl SourceFileReader,
) -> Result<Vec<(String, Vec<u8>)>, SourceDraftError> {
    let metadata = fs::metadata(path).map_err(|_| SourceDraftError::Unreadable)?;
    if !metadata.is_dir() {
        return Err(SourceDraftError::Unsupported);
    }
    let mut eligible = Vec::new();
    let entries = fs::read_dir(path).map_err(|_| SourceDraftError::Unreadable)?;
    for entry in entries {
        let Ok(entry) = entry else {
            continue;
        };
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() {
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        let file_path = entry.path();
        let Some(basename) = lossless_basename(&file_path) else {
            continue;
        };
        if validate_source_display_name(&basename).is_err() {
            continue;
        }
        let Ok(file_metadata) = entry.metadata() else {
            continue;
        };
        if !file_metadata.is_file() {
            continue;
        }
        if file_metadata.len() > MAX_SOURCE_UTF8 as u64 {
            continue;
        }
        let Ok(bytes) = file_reader.read_regular_file(&file_path) else {
            continue;
        };
        if std::str::from_utf8(&bytes).is_err() {
            continue;
        }
        if bytes.contains(&0) {
            continue;
        }
        // Cap already filled: fail closed without reading remaining siblings.
        if eligible.len() >= MAX_FOLDER_MEMBERS {
            return Err(SourceDraftError::Unsupported);
        }
        eligible.push((basename, bytes));
    }
    Ok(eligible)
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
    let draft_handle = store.replace_with(
        display_name.clone(),
        content.to_owned(),
        SOURCE_ORIGIN_LOCAL_TEXT_FILE.to_owned(),
        1,
        None,
    )?;
    Ok(PickSourceOutcome::Selected {
        draft_handle,
        display_name,
        byte_count,
        origin_kind: SOURCE_ORIGIN_LOCAL_TEXT_FILE.to_owned(),
        member_count: 1,
        canonical_url: None,
    })
}

/// Captures one selected folder into an ephemeral draft without retaining the path.
pub(crate) fn capture_picked_folder<P, R>(
    store: &SourceDraftStore,
    picker: &P,
    reader: &R,
) -> Result<PickSourceOutcome, SourceDraftError>
where
    P: SourceFolderPicker + ?Sized,
    R: SourceFolderReader + ?Sized,
{
    let Some(path) = picker.pick_folder() else {
        return Ok(PickSourceOutcome::Cancelled);
    };
    let display_name = lossless_basename(&path).ok_or(SourceDraftError::Unsupported)?;
    validate_source_display_name(&display_name).map_err(map_validation)?;
    let raw_members = reader.read_shallow_folder(&path)?;
    drop(path);
    if raw_members.is_empty() {
        return Err(SourceDraftError::Unsupported);
    }
    if raw_members.len() > MAX_FOLDER_MEMBERS {
        return Err(SourceDraftError::Unsupported);
    }
    let mut members = Vec::with_capacity(raw_members.len());
    for (basename, bytes) in raw_members {
        let content = std::str::from_utf8(&bytes).map_err(|_| SourceDraftError::Unsupported)?;
        validate_source_content(content).map_err(map_validation)?;
        members.push((basename, content.to_owned()));
    }
    let framed = frame_folder_members(&members).map_err(map_validation)?;
    validate_source_content(&framed).map_err(map_validation)?;
    let member_count = u32::try_from(members.len()).map_err(|_| SourceDraftError::Unsupported)?;
    let byte_count = framed.len() as u64;
    let draft_handle = store.replace_with(
        display_name.clone(),
        framed,
        SOURCE_ORIGIN_LOCAL_TEXT_FOLDER.to_owned(),
        member_count,
        None,
    )?;
    Ok(PickSourceOutcome::Selected {
        draft_handle,
        display_name,
        byte_count,
        origin_kind: SOURCE_ORIGIN_LOCAL_TEXT_FOLDER.to_owned(),
        member_count,
        canonical_url: None,
    })
}

/// Maximum HTTPS redirects followed while capturing a link Source.
pub(crate) const MAX_LINK_REDIRECTS: usize = 5;

/// HTTPS fetch boundary used by production and tests.
pub(crate) trait SourceUrlFetcher: Send + Sync {
    fn fetch_https_text(&self, url: &str) -> Result<FetchedUrlBody, SourceDraftError>;
}

/// Bounded response from one native link fetch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FetchedUrlBody {
    pub(crate) bytes: Vec<u8>,
    pub(crate) content_type: Option<String>,
}

/// Captures one HTTPS URL into an ephemeral draft without retaining live references.
pub(crate) fn capture_link_source<F>(
    store: &SourceDraftStore,
    url: &str,
    fetcher: &F,
) -> Result<PickSourceOutcome, SourceDraftError>
where
    F: SourceUrlFetcher + ?Sized,
{
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return Err(SourceDraftError::Unsupported);
    }
    validate_canonical_https_url(trimmed).map_err(map_validation)?;
    validate_link_destination(trimmed)?;
    let display_name = derive_remote_source_display_name(trimmed).map_err(map_validation)?;
    let fetched = fetcher.fetch_https_text(trimmed)?;
    if !is_allowed_link_content_type(fetched.content_type.as_deref()) {
        return Err(SourceDraftError::Unsupported);
    }
    if fetched.bytes.len() > MAX_SOURCE_UTF8 {
        return Err(SourceDraftError::TooLarge);
    }
    let content = std::str::from_utf8(&fetched.bytes).map_err(|_| SourceDraftError::Unsupported)?;
    validate_source_content(content).map_err(map_validation)?;
    let byte_count = content.len() as u64;
    let draft_handle = store.replace_with(
        display_name.clone(),
        content.to_owned(),
        SOURCE_ORIGIN_REMOTE_TEXT_URL.to_owned(),
        1,
        Some(trimmed.to_owned()),
    )?;
    Ok(PickSourceOutcome::Selected {
        draft_handle,
        display_name,
        byte_count,
        origin_kind: SOURCE_ORIGIN_REMOTE_TEXT_URL.to_owned(),
        member_count: 1,
        canonical_url: Some(trimmed.to_owned()),
    })
}

fn is_allowed_link_content_type(content_type: Option<&str>) -> bool {
    let Some(raw) = content_type else {
        return false;
    };
    let value = raw
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if value.is_empty() {
        return false;
    }
    if value.starts_with("text/") {
        return true;
    }
    matches!(
        value.as_str(),
        "application/json"
            | "application/xml"
            | "application/javascript"
            | "application/ecmascript"
            | "application/x-javascript"
    )
}

fn validate_link_destination(url: &str) -> Result<(), SourceDraftError> {
    let parsed = url::Url::parse(url).map_err(|_| SourceDraftError::Unsupported)?;
    if parsed.scheme() != "https" {
        return Err(SourceDraftError::Unsupported);
    }
    validate_link_host(parsed.host_str().ok_or(SourceDraftError::Unsupported)?)
}

fn validate_link_host(host: &str) -> Result<(), SourceDraftError> {
    let normalized = host.trim_end_matches('.').to_ascii_lowercase();
    if normalized.is_empty() {
        return Err(SourceDraftError::Unsupported);
    }
    if normalized == "localhost" || normalized.ends_with(".localhost") {
        return Err(SourceDraftError::Unsupported);
    }
    if let Ok(ip) = normalized.parse::<std::net::IpAddr>() {
        return if is_blocked_ip(ip) {
            Err(SourceDraftError::Unsupported)
        } else {
            Ok(())
        };
    }
    if normalized.starts_with('[') && normalized.ends_with(']') {
        let inner = &normalized[1..normalized.len() - 1];
        if let Ok(ip) = inner.parse::<std::net::IpAddr>() {
            return if is_blocked_ip(ip) {
                Err(SourceDraftError::Unsupported)
            } else {
                Ok(())
            };
        }
    }
    resolve_and_validate_host(&normalized)
}

fn resolve_and_validate_host(host: &str) -> Result<(), SourceDraftError> {
    use std::net::ToSocketAddrs;
    let authority = format!("{host}:443");
    let mut addresses = authority
        .to_socket_addrs()
        .map_err(|_| SourceDraftError::Unsupported)?;
    let Some(first) = addresses.next() else {
        return Err(SourceDraftError::Unsupported);
    };
    if is_blocked_ip(first.ip()) {
        return Err(SourceDraftError::Unsupported);
    }
    for address in addresses {
        if is_blocked_ip(address.ip()) {
            return Err(SourceDraftError::Unsupported);
        }
    }
    Ok(())
}

fn is_blocked_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            let octets = v4.octets();
            octets[0] == 0
                || octets[0] == 10
                || octets[0] == 127
                || (octets[0] == 100 && (octets[1] & 0xC0) == 64)
                || (octets[0] == 169 && octets[1] == 254)
                || (octets[0] == 172 && (octets[1] >= 16 && octets[1] <= 31))
                || (octets[0] == 192 && octets[1] == 168)
                || octets[0] >= 224
        }
        std::net::IpAddr::V6(v6) => {
            let segments = v6.segments();
            segments[0] == 0
                || (segments[0] & 0xFE00) == 0xFC00
                || (segments[0] & 0xFFC0) == 0xFE80
                || segments == [0, 0, 0, 0, 0, 0, 0, 1]
        }
    }
}

/// Production HTTPS fetcher with redirect validation and a fixed redirect budget.
#[derive(Debug)]
pub(crate) struct NativeSourceUrlFetcher {
    client: reqwest::blocking::Client,
}

impl NativeSourceUrlFetcher {
    pub(crate) fn new() -> Result<Self, SourceDraftError> {
        let client = reqwest::blocking::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|_| SourceDraftError::Unreadable)?;
        Ok(Self { client })
    }

    fn fetch_once(&self, url: &str) -> Result<reqwest::blocking::Response, SourceDraftError> {
        self.client
            .get(url)
            .send()
            .map_err(|_| SourceDraftError::Unreadable)
    }
}

impl SourceUrlFetcher for NativeSourceUrlFetcher {
    fn fetch_https_text(&self, url: &str) -> Result<FetchedUrlBody, SourceDraftError> {
        validate_link_destination(url)?;
        let mut current = url.to_owned();
        for _ in 0..=MAX_LINK_REDIRECTS {
            let response = self.fetch_once(&current)?;
            let status = response.status();
            if status.is_redirection() {
                let location = response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|value| value.to_str().ok())
                    .ok_or(SourceDraftError::Unsupported)?;
                let next = url::Url::parse(location)
                    .or_else(|_| url::Url::parse(&current).and_then(|base| base.join(location)))
                    .map_err(|_| SourceDraftError::Unsupported)?;
                if next.scheme() != "https" {
                    return Err(SourceDraftError::Unsupported);
                }
                let next_url = next.to_string();
                validate_canonical_https_url(&next_url).map_err(map_validation)?;
                validate_link_destination(&next_url)?;
                current = next_url;
                continue;
            }
            if !status.is_success() {
                return Err(SourceDraftError::Unreadable);
            }
            let content_type = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned);
            let mut buffer = Vec::new();
            let mut reader = response;
            let mut chunk = [0_u8; 8192];
            loop {
                let read = reader
                    .read(&mut chunk)
                    .map_err(|_| SourceDraftError::Unreadable)?;
                if read == 0 {
                    break;
                }
                if buffer.len() + read > MAX_SOURCE_UTF8 {
                    return Err(SourceDraftError::TooLarge);
                }
                buffer.extend_from_slice(&chunk[..read]);
            }
            return Ok(FetchedUrlBody {
                bytes: buffer,
                content_type,
            });
        }
        Err(SourceDraftError::Unsupported)
    }
}

fn lossless_basename(path: &Path) -> Option<String> {
    path.file_name()?.to_str().map(str::to_owned)
}

fn map_validation(error: SourceValidationError) -> SourceDraftError {
    match error {
        SourceValidationError::TooLarge { .. } => SourceDraftError::TooLarge,
        SourceValidationError::UnsafeDisplayName
        | SourceValidationError::ContainsNul
        | SourceValidationError::NoEligibleMembers
        | SourceValidationError::TooManyMembers { .. }
        | SourceValidationError::InvalidCanonicalUrl
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

    impl SourceFolderPicker for FakePicker {
        fn pick_folder(&self) -> Option<PathBuf> {
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

    struct FakeFolderReader {
        members: Mutex<FolderReadResult>,
        reads: Mutex<u32>,
    }

    type FolderReadResult = Result<Vec<(String, Vec<u8>)>, SourceDraftError>;

    impl SourceFolderReader for FakeFolderReader {
        fn read_shallow_folder(
            &self,
            _path: &Path,
        ) -> Result<Vec<(String, Vec<u8>)>, SourceDraftError> {
            *self.reads.lock().unwrap() += 1;
            self.members.lock().unwrap().clone()
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
            member_count,
            canonical_url,
        } = outcome
        else {
            panic!("expected selection");
        };
        assert_eq!(display_name, "notes.txt");
        assert_eq!(byte_count, 7);
        assert_eq!(origin_kind, SOURCE_ORIGIN_LOCAL_TEXT_FILE);
        assert_eq!(member_count, 1);
        assert_eq!(canonical_url, None);
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
    fn parse_folder_draft_members_rejects_length_on_non_utf8_boundary() {
        let content = "member: a.txt\ncontent-bytes: 1\né";
        assert!(matches!(
            parse_folder_draft_members(content),
            Err(SourceValidationError::UnsafeDisplayName)
        ));
    }

    #[test]
    fn folder_capture_skips_unreadable_sibling_and_succeeds_with_one_eligible() {
        struct FailLockedReader;

        impl SourceFileReader for FailLockedReader {
            fn read_regular_file(&self, path: &Path) -> Result<Vec<u8>, SourceDraftError> {
                if path.file_name().is_some_and(|name| name == "locked.txt") {
                    return Err(SourceDraftError::Unreadable);
                }
                NativeSourceFileReader.read_regular_file(path)
            }
        }

        let store = SourceDraftStore::new();
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("good.txt"), "alpha").unwrap();
        std::fs::write(root.path().join("locked.txt"), "secret").unwrap();
        let members = collect_shallow_folder_members(root.path(), &FailLockedReader).unwrap();
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].0, "good.txt");

        let picker = FakePicker {
            path: Mutex::new(Some(root.path().to_path_buf())),
        };
        struct FolderReaderWithLockedSibling;
        impl SourceFolderReader for FolderReaderWithLockedSibling {
            fn read_shallow_folder(
                &self,
                path: &Path,
            ) -> Result<Vec<(String, Vec<u8>)>, SourceDraftError> {
                collect_shallow_folder_members(path, &FailLockedReader)
            }
        }
        let outcome =
            capture_picked_folder(&store, &picker, &FolderReaderWithLockedSibling).unwrap();
        let PickSourceOutcome::Selected { member_count, .. } = outcome else {
            panic!("expected selection");
        };
        assert_eq!(member_count, 1);
    }

    #[test]
    fn folder_capture_skips_ineligible_and_excludes_nested_files() {
        let store = SourceDraftStore::new();
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("a.txt"), "alpha").unwrap();
        std::fs::create_dir(root.path().join("nested")).unwrap();
        std::fs::write(root.path().join("nested").join("hidden.txt"), "no").unwrap();
        std::fs::write(root.path().join("binary.bin"), [0xff, 0xfe]).unwrap();
        let picker = FakePicker {
            path: Mutex::new(Some(root.path().to_path_buf())),
        };
        let reader = NativeSourceFolderReader;
        let outcome = capture_picked_folder(&store, &picker, &reader).unwrap();
        let PickSourceOutcome::Selected {
            display_name,
            member_count,
            origin_kind,
            ..
        } = outcome
        else {
            panic!("expected selection");
        };
        assert_eq!(
            display_name,
            root.path().file_name().unwrap().to_str().unwrap()
        );
        assert_eq!(member_count, 1);
        assert_eq!(origin_kind, SOURCE_ORIGIN_LOCAL_TEXT_FOLDER);
    }

    #[test]
    fn folder_capture_fails_on_zero_eligible_over_count_and_over_budget() {
        let store = SourceDraftStore::new();
        let empty = tempfile::tempdir().unwrap();
        let picker = FakePicker {
            path: Mutex::new(Some(empty.path().to_path_buf())),
        };
        let reader = NativeSourceFolderReader;
        assert!(matches!(
            capture_picked_folder(&store, &picker, &reader),
            Err(SourceDraftError::Unsupported)
        ));

        let over_count = tempfile::tempdir().unwrap();
        for index in 0..=MAX_FOLDER_MEMBERS {
            std::fs::write(over_count.path().join(format!("f{index}.txt")), "x").unwrap();
        }
        let picker = FakePicker {
            path: Mutex::new(Some(over_count.path().to_path_buf())),
        };
        assert!(matches!(
            capture_picked_folder(&store, &picker, &reader),
            Err(SourceDraftError::Unsupported)
        ));

        let large = tempfile::tempdir().unwrap();
        let total_files = MAX_FOLDER_MEMBERS + 20;
        for index in 0..total_files {
            std::fs::write(large.path().join(format!("f{index:03}.txt")), "x").unwrap();
        }
        struct CountingReader {
            reads: Mutex<usize>,
        }
        impl SourceFileReader for CountingReader {
            fn read_regular_file(&self, path: &Path) -> Result<Vec<u8>, SourceDraftError> {
                *self.reads.lock().unwrap() += 1;
                NativeSourceFileReader.read_regular_file(path)
            }
        }
        let counting = CountingReader {
            reads: Mutex::new(0),
        };
        assert!(matches!(
            collect_shallow_folder_members(large.path(), &counting),
            Err(SourceDraftError::Unsupported)
        ));
        let reads = *counting.reads.lock().unwrap();
        assert_eq!(reads, MAX_FOLDER_MEMBERS + 1);
        assert!(reads < total_files);

        let members = vec![("big.txt".to_owned(), vec![b'a'; MAX_SOURCE_UTF8])];
        let fake_reader = FakeFolderReader {
            members: Mutex::new(Ok(members)),
            reads: Mutex::new(0),
        };
        let picker = FakePicker {
            path: Mutex::new(Some(PathBuf::from("docs"))),
        };
        assert!(matches!(
            capture_picked_folder(&store, &picker, &fake_reader),
            Err(SourceDraftError::TooLarge)
        ));
    }

    #[test]
    fn folder_snapshot_ignores_post_capture_disk_mutation() {
        let store = SourceDraftStore::new();
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("a.txt");
        std::fs::write(&file, "before").unwrap();
        let picker = FakePicker {
            path: Mutex::new(Some(root.path().to_path_buf())),
        };
        let outcome = capture_picked_folder(&store, &picker, &NativeSourceFolderReader).unwrap();
        let PickSourceOutcome::Selected { draft_handle, .. } = outcome else {
            panic!("expected selection");
        };
        std::fs::write(&file, "after").unwrap();
        std::fs::remove_file(&file).unwrap();
        let draft = store.get(&draft_handle).unwrap();
        assert!(draft.content.contains("before"));
        assert!(!draft.content.contains("after"));
    }

    #[test]
    fn cross_session_handle_cannot_be_substituted() {
        let store = SourceDraftStore::new();
        let session_a = AgentSessionId::generate();
        let session_b = AgentSessionId::generate();
        store.bind_session_scope(session_a);
        let handle = store
            .insert(
                "a.txt".into(),
                "from-a".into(),
                SOURCE_ORIGIN_LOCAL_TEXT_FILE.into(),
                1,
                None,
            )
            .unwrap();
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
        let first = store
            .insert(
                "a.txt".into(),
                "one".into(),
                SOURCE_ORIGIN_LOCAL_TEXT_FILE.into(),
                1,
                None,
            )
            .unwrap();
        store.begin_new_session_scope();
        assert!(store.get(&first).is_none());
        let second = store
            .insert(
                "b.txt".into(),
                "two".into(),
                SOURCE_ORIGIN_LOCAL_TEXT_FILE.into(),
                1,
                None,
            )
            .unwrap();
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
        let handle = store
            .insert(
                "a.txt".into(),
                "body".into(),
                SOURCE_ORIGIN_LOCAL_TEXT_FILE.into(),
                1,
                None,
            )
            .unwrap();
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
        let handle = store
            .insert(
                "a.txt".into(),
                "x".into(),
                SOURCE_ORIGIN_LOCAL_TEXT_FILE.into(),
                1,
                None,
            )
            .unwrap();
        store.clear_handle(&handle);
        assert!(
            store
                .resolve_for_send(&handle, &store.current_scope())
                .is_none()
        );
        let handle = store
            .insert(
                "a.txt".into(),
                "x".into(),
                SOURCE_ORIGIN_LOCAL_TEXT_FILE.into(),
                1,
                None,
            )
            .unwrap();
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

    struct FakeUrlFetcher {
        responses:
            Mutex<std::collections::HashMap<String, Result<FetchedUrlBody, SourceDraftError>>>,
        reads: Mutex<u32>,
    }

    impl SourceUrlFetcher for FakeUrlFetcher {
        fn fetch_https_text(&self, url: &str) -> Result<FetchedUrlBody, SourceDraftError> {
            *self.reads.lock().unwrap() += 1;
            self.responses
                .lock()
                .unwrap()
                .get(url)
                .cloned()
                .unwrap_or(Err(SourceDraftError::Unreadable))
        }
    }

    #[test]
    fn link_capture_reads_once_and_persists_snapshot_without_refetch() {
        let store = SourceDraftStore::new();
        let url = "https://example.com/readme.txt";
        let fetcher = FakeUrlFetcher {
            responses: Mutex::new(std::collections::HashMap::from([(
                url.to_owned(),
                Ok(FetchedUrlBody {
                    bytes: b"hello link".to_vec(),
                    content_type: Some("text/plain".into()),
                }),
            )])),
            reads: Mutex::new(0),
        };
        let outcome = capture_link_source(&store, url, &fetcher).unwrap();
        let PickSourceOutcome::Selected {
            draft_handle,
            display_name,
            byte_count,
            origin_kind,
            canonical_url,
            ..
        } = outcome
        else {
            panic!("expected selection");
        };
        assert_eq!(display_name, "example.com/readme.txt");
        assert_eq!(byte_count, 10);
        assert_eq!(origin_kind, SOURCE_ORIGIN_REMOTE_TEXT_URL);
        assert_eq!(canonical_url.as_deref(), Some(url));
        let draft = store.get(&draft_handle).unwrap();
        assert_eq!(draft.content, "hello link");
        assert_eq!(draft.canonical_url.as_deref(), Some(url));
        let source = draft.into_source().unwrap();
        assert_eq!(source.content(), "hello link");
        assert_eq!(*fetcher.reads.lock().unwrap(), 1);
    }

    #[test]
    fn link_capture_rejects_http_private_loopback_and_bad_redirects() {
        let store = SourceDraftStore::new();
        let fetcher = FakeUrlFetcher {
            responses: Mutex::new(std::collections::HashMap::new()),
            reads: Mutex::new(0),
        };
        for url in [
            "http://example.com/a.txt",
            "https://127.0.0.1/a.txt",
            "https://localhost/a.txt",
            "https://192.168.0.1/a.txt",
            "https://10.0.0.1/a.txt",
            "file:///etc/passwd",
        ] {
            assert!(
                matches!(
                    capture_link_source(&store, url, &fetcher),
                    Err(SourceDraftError::Unsupported)
                ),
                "expected rejection for {url}"
            );
        }
    }

    #[test]
    fn link_capture_rejects_over_budget_unsupported_type_and_invalid_utf8() {
        let store = SourceDraftStore::new();
        let url = "https://example.com/a.txt";
        let fetcher = FakeUrlFetcher {
            responses: Mutex::new(std::collections::HashMap::from([(
                url.to_owned(),
                Ok(FetchedUrlBody {
                    bytes: vec![0xff, 0xfe],
                    content_type: Some("text/plain".into()),
                }),
            )])),
            reads: Mutex::new(0),
        };
        assert!(matches!(
            capture_link_source(&store, url, &fetcher),
            Err(SourceDraftError::Unsupported)
        ));

        *fetcher.responses.lock().unwrap() = std::collections::HashMap::from([(
            url.to_owned(),
            Ok(FetchedUrlBody {
                bytes: vec![b'a'; MAX_SOURCE_UTF8 + 1],
                content_type: Some("text/plain".into()),
            }),
        )]);
        assert!(matches!(
            capture_link_source(&store, url, &fetcher),
            Err(SourceDraftError::TooLarge)
        ));

        *fetcher.responses.lock().unwrap() = std::collections::HashMap::from([(
            url.to_owned(),
            Ok(FetchedUrlBody {
                bytes: b"ok".to_vec(),
                content_type: Some("application/octet-stream".into()),
            }),
        )]);
        assert!(matches!(
            capture_link_source(&store, url, &fetcher),
            Err(SourceDraftError::Unsupported)
        ));
    }
}
