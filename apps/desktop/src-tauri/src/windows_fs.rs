//! Windows object/path enforcement, exact replacement, and native diff.
#![allow(unsafe_code)]

use std::{
    ffi::OsStr,
    fs, io,
    os::windows::ffi::OsStrExt,
    path::{Component, Path, PathBuf},
    ptr,
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(test)]
use std::cell::Cell;

use tule_core::{MAX_RUN_CONTENT_UTF8, hash_expected_diff, hash_source_bytes, sha256_hex};
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, ERROR_FILE_NOT_FOUND, GetLastError, HANDLE, INVALID_HANDLE_VALUE,
        WAIT_OBJECT_0, WAIT_TIMEOUT,
    },
    Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, CreateFileW, FILE_ATTRIBUTE_NORMAL,
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, GetFileAttributesW,
        GetFileInformationByHandle, GetFinalPathNameByHandleW, MOVEFILE_REPLACE_EXISTING,
        MOVEFILE_WRITE_THROUGH, MoveFileExW, OPEN_EXISTING,
    },
    System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, WaitForSingleObject},
};

/// SYNCHRONIZE access right (Win32). Kept local because the windows-sys feature set
/// used here does not re-export the named constant.
const SYNCHRONIZE: u32 = 0x0010_0000;

/// Versioned native structural diff algorithm identity.
pub(crate) const NATIVE_DIFF_VERSION: &str = "tule-native-diff-v1";

/// Broker-owned temporary file prefix.
const TEMP_PREFIX: &str = ".tule-harness-tmp-";

// Test-only fault: mutate the target between identity check and replace.
// Thread-local so parallel suite runs cannot steal the inject flag.
#[cfg(test)]
thread_local! {
    static INJECT_CHECK_TO_USE_RACE: Cell<bool> = const { Cell::new(false) };
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FilesystemIdentity {
    pub(crate) volume_serial: u32,
    pub(crate) file_index_high: u32,
    pub(crate) file_index_low: u32,
    pub(crate) final_path: String,
}

impl FilesystemIdentity {
    pub(crate) fn fingerprint(&self) -> String {
        sha256_hex(
            format!(
                "{}:{}:{}:{}",
                self.volume_serial, self.file_index_high, self.file_index_low, self.final_path
            )
            .as_bytes(),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WindowsFsError {
    OutsideRoot,
    Traversal,
    UncOrExtendedDenied,
    AlternateDataStream,
    AliasRedirect,
    ReparsePoint,
    StaleIdentity,
    StalePreimage,
    CheckToUseRace,
    UnsupportedOperation(&'static str),
    Io(String),
    ContentTooLarge,
    InvalidUtf8,
}

impl std::fmt::Display for WindowsFsError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OutsideRoot => formatter.write_str("path resolves outside the run root"),
            Self::Traversal => formatter.write_str("path traversal is denied"),
            Self::UncOrExtendedDenied => {
                formatter.write_str("UNC or extended path forms are denied")
            }
            Self::AlternateDataStream => formatter.write_str("alternate data streams are denied"),
            Self::AliasRedirect => formatter.write_str("path alias redirection is denied"),
            Self::ReparsePoint => formatter.write_str("reparse points are denied"),
            Self::StaleIdentity => formatter.write_str("filesystem identity is stale"),
            Self::StalePreimage => formatter.write_str("preimage hash does not match"),
            Self::CheckToUseRace => formatter.write_str("check-to-use substitution was detected"),
            Self::UnsupportedOperation(name) => {
                write!(formatter, "unsupported operation denied: {name}")
            }
            Self::Io(message) => write!(formatter, "windows filesystem I/O failed: {message}"),
            Self::ContentTooLarge => formatter.write_str("content exceeds the 64 KiB ceiling"),
            Self::InvalidUtf8 => formatter.write_str("content is not valid UTF-8"),
        }
    }
}

impl std::error::Error for WindowsFsError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NativeDiff {
    pub(crate) version: &'static str,
    pub(crate) text: String,
    pub(crate) hash: String,
}

/// Fail-closed canonicalization of a run-relative target under an exclusive root.
pub(crate) fn resolve_target_under_root(
    run_root: &Path,
    relative_target: &str,
) -> Result<PathBuf, WindowsFsError> {
    reject_dangerous_path_text(relative_target)?;
    if relative_target.contains(':') {
        // ADS and drive-qualified forms are denied inside relative targets.
        return Err(WindowsFsError::AlternateDataStream);
    }
    let relative = Path::new(relative_target);
    if relative.is_absolute() {
        return Err(WindowsFsError::OutsideRoot);
    }
    for component in relative.components() {
        match component {
            Component::Normal(_) => {}
            Component::CurDir => {}
            Component::ParentDir => return Err(WindowsFsError::Traversal),
            Component::RootDir | Component::Prefix(_) => return Err(WindowsFsError::OutsideRoot),
        }
    }
    let root = canonicalize_strict(run_root)?;
    let candidate = root.join(relative);
    let parent = candidate.parent().ok_or(WindowsFsError::OutsideRoot)?;
    let parent_canon = canonicalize_strict(parent)?;
    if !parent_canon.starts_with(&root) {
        return Err(WindowsFsError::OutsideRoot);
    }
    let file_name = candidate.file_name().ok_or(WindowsFsError::OutsideRoot)?;
    let resolved = parent_canon.join(file_name);
    if resolved.to_string_lossy().contains('\0') {
        return Err(WindowsFsError::Io("NUL in path".to_owned()));
    }
    // Path-prefix attacks: ensure the joined path remains under root with a boundary.
    let resolved_text = resolved.to_string_lossy().to_ascii_lowercase();
    let root_text = root.to_string_lossy().to_ascii_lowercase();
    if resolved_text != root_text
        && !resolved_text.starts_with(&(root_text.clone() + "\\"))
        && !resolved_text.starts_with(&(root_text + "/"))
    {
        return Err(WindowsFsError::OutsideRoot);
    }
    if resolved.exists() {
        enforce_no_alias_redirect(relative_target, &resolved)?;
    }
    Ok(resolved)
}

/// Fail closed when the requested leaf casing/name does not match the on-disk identity.
fn enforce_no_alias_redirect(relative_target: &str, resolved: &Path) -> Result<(), WindowsFsError> {
    let requested_leaf = Path::new(relative_target)
        .file_name()
        .and_then(OsStr::to_str)
        .ok_or(WindowsFsError::OutsideRoot)?;
    let identity = read_identity(resolved)?;
    let final_path = identity.final_path.trim_start_matches(r"\\?\");
    let actual_leaf = Path::new(final_path)
        .file_name()
        .and_then(OsStr::to_str)
        .ok_or_else(|| WindowsFsError::Io("final path leaf missing".to_owned()))?;
    if requested_leaf != actual_leaf {
        return Err(WindowsFsError::AliasRedirect);
    }
    Ok(())
}

fn reject_dangerous_path_text(value: &str) -> Result<(), WindowsFsError> {
    let lower = value.to_ascii_lowercase();
    if lower.starts_with("\\\\")
        || lower.starts_with("//")
        || lower.starts_with(r"\\?\")
        || lower.starts_with("//?/")
        || lower.contains("\\??\\")
    {
        return Err(WindowsFsError::UncOrExtendedDenied);
    }
    if value.contains('\0') {
        return Err(WindowsFsError::Io("NUL in path".to_owned()));
    }
    Ok(())
}

fn canonicalize_strict(path: &Path) -> Result<PathBuf, WindowsFsError> {
    reject_dangerous_path_text(&path.to_string_lossy())?;
    if has_reparse_attribute(path)? {
        return Err(WindowsFsError::ReparsePoint);
    }
    let canonical =
        fs::canonicalize(path).map_err(|error| WindowsFsError::Io(error.to_string()))?;
    let text = canonical.to_string_lossy();
    if text.starts_with(r"\\?\UNC\") || text.starts_with("//?/UNC/") {
        return Err(WindowsFsError::UncOrExtendedDenied);
    }
    Ok(strip_extended_prefix(canonical))
}

fn strip_extended_prefix(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    if let Some(stripped) = text.strip_prefix(r"\\?\") {
        PathBuf::from(stripped)
    } else {
        path
    }
}

fn has_reparse_attribute(path: &Path) -> Result<bool, WindowsFsError> {
    if !path.exists() {
        return Ok(false);
    }
    let wide = wide_path(path);
    let attributes = unsafe { GetFileAttributesW(wide.as_ptr()) };
    if attributes == u32::MAX {
        let error = unsafe { GetLastError() };
        if error == ERROR_FILE_NOT_FOUND {
            return Ok(false);
        }
        return Err(WindowsFsError::Io(format!(
            "GetFileAttributesW failed: {error}"
        )));
    }
    Ok(attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0)
}

fn wide_path(path: &Path) -> Vec<u16> {
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

pub(crate) fn read_identity(path: &Path) -> Result<FilesystemIdentity, WindowsFsError> {
    if has_reparse_attribute(path)? {
        return Err(WindowsFsError::ReparsePoint);
    }
    let handle = open_handle(path, true)?;
    let identity = identity_from_handle(handle)?;
    unsafe {
        CloseHandle(handle);
    }
    Ok(identity)
}

fn open_handle(path: &Path, open_reparse_as_itself: bool) -> Result<HANDLE, WindowsFsError> {
    let wide = wide_path(path);
    let flags = FILE_FLAG_BACKUP_SEMANTICS
        | if open_reparse_as_itself {
            FILE_FLAG_OPEN_REPARSE_POINT
        } else {
            0
        };
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            ptr::null(),
            OPEN_EXISTING,
            flags | FILE_ATTRIBUTE_NORMAL,
            ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(WindowsFsError::Io(format!(
            "CreateFileW failed: {}",
            unsafe { GetLastError() }
        )));
    }
    Ok(handle)
}

fn identity_from_handle(handle: HANDLE) -> Result<FilesystemIdentity, WindowsFsError> {
    let mut info = unsafe { std::mem::zeroed::<BY_HANDLE_FILE_INFORMATION>() };
    let ok = unsafe { GetFileInformationByHandle(handle, &mut info) };
    if ok == 0 {
        return Err(WindowsFsError::Io(format!(
            "GetFileInformationByHandle failed: {}",
            unsafe { GetLastError() }
        )));
    }
    let mut buffer = vec![0u16; 1024];
    let length =
        unsafe { GetFinalPathNameByHandleW(handle, buffer.as_mut_ptr(), buffer.len() as u32, 0) };
    if length == 0 || length as usize >= buffer.len() {
        return Err(WindowsFsError::Io(format!(
            "GetFinalPathNameByHandleW failed: {}",
            unsafe { GetLastError() }
        )));
    }
    buffer.truncate(length as usize);
    let final_path = String::from_utf16_lossy(&buffer);
    if final_path.contains(':')
        && final_path
            .rsplit('\\')
            .next()
            .is_some_and(|name| name.contains(':'))
    {
        return Err(WindowsFsError::AlternateDataStream);
    }
    Ok(FilesystemIdentity {
        volume_serial: info.dwVolumeSerialNumber,
        file_index_high: info.nFileIndexHigh,
        file_index_low: info.nFileIndexLow,
        final_path,
    })
}

pub(crate) fn read_utf8_file(path: &Path) -> Result<String, WindowsFsError> {
    if has_reparse_attribute(path)? {
        return Err(WindowsFsError::ReparsePoint);
    }
    let bytes = fs::read(path).map_err(|error| WindowsFsError::Io(error.to_string()))?;
    if bytes.len() > MAX_RUN_CONTENT_UTF8 {
        return Err(WindowsFsError::ContentTooLarge);
    }
    String::from_utf8(bytes).map_err(|_| WindowsFsError::InvalidUtf8)
}

pub(crate) fn content_hash(content: &str) -> String {
    hash_source_bytes(content.as_bytes())
}

/// Exact create-or-replace using a same-directory private temp and atomic MoveFileEx.
pub(crate) fn exact_create_or_replace(
    run_root: &Path,
    relative_target: &str,
    expected_identity: Option<&FilesystemIdentity>,
    expected_preimage_hash: &str,
    postimage_utf8: &str,
) -> Result<FilesystemIdentity, WindowsFsError> {
    if postimage_utf8.len() > MAX_RUN_CONTENT_UTF8 {
        return Err(WindowsFsError::ContentTooLarge);
    }
    let target = resolve_target_under_root(run_root, relative_target)?;
    let parent = target
        .parent()
        .ok_or(WindowsFsError::OutsideRoot)?
        .to_path_buf();
    if has_reparse_attribute(&parent)? || has_reparse_attribute(&target)? {
        return Err(WindowsFsError::ReparsePoint);
    }
    let before_identity = if target.exists() {
        let identity = read_identity(&target)?;
        if let Some(expected) = expected_identity
            && identity.fingerprint() != expected.fingerprint()
        {
            return Err(WindowsFsError::StaleIdentity);
        }
        let current = read_utf8_file(&target)?;
        if content_hash(&current) != expected_preimage_hash {
            return Err(WindowsFsError::StalePreimage);
        }
        Some(identity)
    } else if expected_identity.is_some() {
        return Err(WindowsFsError::StaleIdentity);
    } else {
        None
    };

    let temp = create_exclusive_temp_file(&parent, postimage_utf8.as_bytes())?;

    #[cfg(test)]
    if INJECT_CHECK_TO_USE_RACE.with(|flag| flag.replace(false)) {
        // Replace the target with a different inode/content between check and use.
        let _ = fs::remove_file(&target);
        fs::write(&target, b"substituted-before-use")
            .map_err(|error| WindowsFsError::Io(error.to_string()))?;
    }

    // Deterministic check-to-use: re-read identity/preimage immediately before replace.
    if target.exists() {
        let identity = read_identity(&target)?;
        if let Some(before) = &before_identity
            && identity.fingerprint() != before.fingerprint()
        {
            let _ = fs::remove_file(&temp);
            return Err(WindowsFsError::CheckToUseRace);
        }
        let current = read_utf8_file(&target)?;
        if content_hash(&current) != expected_preimage_hash {
            let _ = fs::remove_file(&temp);
            return Err(WindowsFsError::StalePreimage);
        }
    }

    let from = wide_path(&temp);
    let to = wide_path(&target);
    let ok = unsafe {
        MoveFileExW(
            from.as_ptr(),
            to.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if ok == 0 {
        let _ = fs::remove_file(&temp);
        return Err(WindowsFsError::Io(format!(
            "MoveFileExW failed: {}",
            unsafe { GetLastError() }
        )));
    }
    read_identity(&target)
}

fn create_exclusive_temp_file(parent: &Path, bytes: &[u8]) -> Result<PathBuf, WindowsFsError> {
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMP_SEQ: AtomicU64 = AtomicU64::new(1);

    for attempt in 0..8 {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| WindowsFsError::Io(error.to_string()))?
            .as_millis();
        let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let name = format!(
            "{TEMP_PREFIX}{millis}-{}-{attempt}-{seq}",
            std::process::id()
        );
        let path = parent.join(name);
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                file.write_all(bytes)
                    .map_err(|error| WindowsFsError::Io(error.to_string()))?;
                file.sync_all()
                    .map_err(|error| WindowsFsError::Io(error.to_string()))?;
                return Ok(path);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(WindowsFsError::Io(error.to_string())),
        }
    }
    Err(WindowsFsError::Io(
        "exhausted exclusive temporary-file create attempts".to_owned(),
    ))
}

pub(crate) fn native_diff(preimage: &str, postimage: &str) -> NativeDiff {
    let mut text = String::new();
    text.push_str(&format!("version:{NATIVE_DIFF_VERSION}\n"));
    let pre_lines: Vec<&str> = preimage.split_inclusive('\n').collect();
    let post_lines: Vec<&str> = postimage.split_inclusive('\n').collect();
    let max = pre_lines.len().max(post_lines.len());
    for index in 0..max {
        let left = pre_lines.get(index).copied().unwrap_or("");
        let right = post_lines.get(index).copied().unwrap_or("");
        if left == right {
            continue;
        }
        if !left.is_empty() {
            text.push('-');
            text.push_str(left);
            if !left.ends_with('\n') {
                text.push('\n');
            }
        }
        if !right.is_empty() {
            text.push('+');
            text.push_str(right);
            if !right.ends_with('\n') {
                text.push('\n');
            }
        }
    }
    let hash = hash_expected_diff(preimage, postimage);
    NativeDiff {
        version: NATIVE_DIFF_VERSION,
        text,
        hash,
    }
}

pub(crate) fn inspect_baseline_to_current(
    run_root: &Path,
    relative_target: &str,
    baseline_utf8: &str,
) -> Result<(String, NativeDiff), WindowsFsError> {
    let target = resolve_target_under_root(run_root, relative_target)?;
    let current = read_utf8_file(&target)?;
    let diff = native_diff(baseline_utf8, &current);
    Ok((content_hash(&current), diff))
}

/// Positive-evidence prior-owner liveness probe for lease takeover.
pub(crate) fn prior_owner_process_gone(
    owner_process_instance: &str,
) -> Result<bool, WindowsFsError> {
    let pid: u32 = owner_process_instance
        .strip_prefix("pid:")
        .unwrap_or(owner_process_instance)
        .parse()
        .map_err(|_| WindowsFsError::Io("owner process instance is not a pid".to_owned()))?;
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE, 0, pid) };
    if handle.is_null() {
        // Fail closed unless we positively observe absence.
        let error = unsafe { GetLastError() };
        // ERROR_INVALID_PARAMETER (87) commonly means the pid does not exist.
        if error == 87 {
            return Ok(true);
        }
        return Err(WindowsFsError::Io(format!(
            "OpenProcess failed without positive absence evidence: {error}"
        )));
    }
    let wait = unsafe { WaitForSingleObject(handle, 0) };
    unsafe {
        CloseHandle(handle);
    }
    match wait {
        WAIT_OBJECT_0 => Ok(true),
        WAIT_TIMEOUT => Ok(false),
        other => Err(WindowsFsError::Io(format!(
            "WaitForSingleObject returned {other}"
        ))),
    }
}

pub(crate) fn deny_unsupported(operation: &'static str) -> WindowsFsError {
    WindowsFsError::UnsupportedOperation(operation)
}

#[allow(dead_code)]
pub(crate) fn cleanup_broker_temp(path: &Path) -> Result<(), WindowsFsError> {
    let name = path
        .file_name()
        .and_then(OsStr::to_str)
        .ok_or(WindowsFsError::Io("temp name missing".to_owned()))?;
    if !name.starts_with(TEMP_PREFIX) {
        return Err(WindowsFsError::Io(
            "refusing to cleanup non-broker temporary object".to_owned(),
        ));
    }
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(WindowsFsError::Io(error.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn fixture_root() -> TempDir {
        let directory = tempfile::tempdir().unwrap();
        fs::write(
            directory.path().join("index.html"),
            "<!doctype html><html><body><h1>Ready</h1></body></html>",
        )
        .unwrap();
        directory
    }

    #[test]
    fn denies_outside_root_absolute_and_traversal() {
        let root = fixture_root();
        assert!(matches!(
            resolve_target_under_root(root.path(), r"C:\Windows\system32\drivers\etc\hosts"),
            Err(WindowsFsError::OutsideRoot
                | WindowsFsError::UncOrExtendedDenied
                | WindowsFsError::Traversal
                | WindowsFsError::AlternateDataStream)
        ));
        assert!(matches!(
            resolve_target_under_root(root.path(), r"..\secret.txt"),
            Err(WindowsFsError::Traversal)
        ));
        assert!(matches!(
            resolve_target_under_root(root.path(), r"folder\..\..\secret.txt"),
            Err(WindowsFsError::Traversal)
        ));
    }

    #[test]
    fn denies_unc_extended_and_ads_forms() {
        let root = fixture_root();
        assert!(matches!(
            resolve_target_under_root(root.path(), r"\\?\C:\temp\index.html"),
            Err(WindowsFsError::UncOrExtendedDenied)
        ));
        assert!(matches!(
            resolve_target_under_root(root.path(), r"\\server\share\index.html"),
            Err(WindowsFsError::UncOrExtendedDenied)
        ));
        assert!(matches!(
            resolve_target_under_root(root.path(), "index.html:zone.identifier"),
            Err(WindowsFsError::AlternateDataStream)
        ));
    }

    #[test]
    fn path_prefix_neighbor_is_outside_root() {
        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("app");
        let neighbor = parent.path().join("app_evil");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&neighbor).unwrap();
        fs::write(root.join("index.html"), "<h1>Ready</h1>").unwrap();
        fs::write(neighbor.join("index.html"), "evil").unwrap();
        // Relative escape via crafted joined path is still blocked by traversal rules.
        assert!(matches!(
            resolve_target_under_root(&root, r"..\app_evil\index.html"),
            Err(WindowsFsError::Traversal)
        ));
    }

    #[test]
    fn exact_replace_and_stale_preimage_matrix() {
        let root = fixture_root();
        let relative = "index.html";
        let target = resolve_target_under_root(root.path(), relative).unwrap();
        let before = read_utf8_file(&target).unwrap();
        let identity = read_identity(&target).unwrap();
        let pre_hash = content_hash(&before);
        let after = before.replace("<h1>Ready</h1>", "<h1>Ready for review</h1>");
        let new_identity =
            exact_create_or_replace(root.path(), relative, Some(&identity), &pre_hash, &after)
                .unwrap();
        assert_ne!(identity.fingerprint(), new_identity.fingerprint());
        assert_eq!(read_utf8_file(&target).unwrap(), after);

        let err = exact_create_or_replace(
            root.path(),
            relative,
            Some(&new_identity),
            &pre_hash,
            &before,
        )
        .unwrap_err();
        assert!(matches!(err, WindowsFsError::StalePreimage));
    }

    #[test]
    fn denies_case_alias_redirect_without_requiring_short_names() {
        let root = fixture_root();
        // Created as index.html; requesting INDEX.HTML is an alias on case-insensitive volumes.
        assert!(matches!(
            resolve_target_under_root(root.path(), "INDEX.HTML"),
            Err(WindowsFsError::AliasRedirect)
        ));
        assert!(resolve_target_under_root(root.path(), "index.html").is_ok());
    }

    #[test]
    fn denies_reparse_junction_redirection() {
        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("root");
        let outside = parent.path().join("outside");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("index.html"), "<h1>Ready</h1>").unwrap();
        let link = root.join("linked");
        let status = std::process::Command::new("cmd")
            .args([
                "/C",
                "mklink",
                "/J",
                &link.to_string_lossy(),
                &outside.to_string_lossy(),
            ])
            .status()
            .expect("mklink junction");
        assert!(
            status.success(),
            "junction creation should succeed without admin on local temp volumes"
        );
        assert!(matches!(
            resolve_target_under_root(&root, r"linked\index.html"),
            Err(WindowsFsError::ReparsePoint)
        ));
    }

    #[test]
    fn denies_stale_identity_and_check_to_use_race() {
        let root = fixture_root();
        let relative = "index.html";
        let target = resolve_target_under_root(root.path(), relative).unwrap();
        let before = read_utf8_file(&target).unwrap();
        let identity = read_identity(&target).unwrap();
        let pre_hash = content_hash(&before);
        let after = before.replace("<h1>Ready</h1>", "<h1>Ready for review</h1>");

        let forged = FilesystemIdentity {
            volume_serial: identity.volume_serial ^ 0xFFFF,
            file_index_high: identity.file_index_high,
            file_index_low: identity.file_index_low,
            final_path: identity.final_path.clone(),
        };
        assert!(matches!(
            exact_create_or_replace(root.path(), relative, Some(&forged), &pre_hash, &after),
            Err(WindowsFsError::StaleIdentity)
        ));

        INJECT_CHECK_TO_USE_RACE.with(|flag| flag.set(true));
        let err =
            exact_create_or_replace(root.path(), relative, Some(&identity), &pre_hash, &after)
                .unwrap_err();
        assert!(
            matches!(
                err,
                WindowsFsError::CheckToUseRace
                    | WindowsFsError::StalePreimage
                    | WindowsFsError::StaleIdentity
            ),
            "unexpected check-to-use error: {err:?}"
        );
        let current = fs::read_to_string(&target).unwrap();
        assert_ne!(current, after);
    }

    #[test]
    fn prior_owner_process_gone_distinguishes_absent_and_live_pids() {
        assert!(prior_owner_process_gone("pid:4294967294").unwrap());
        let live = format!("pid:{}", std::process::id());
        assert!(!prior_owner_process_gone(&live).unwrap());
    }

    #[test]
    fn unsupported_operations_are_denied() {
        assert!(matches!(
            deny_unsupported("process-exec"),
            WindowsFsError::UnsupportedOperation("process-exec")
        ));
        assert!(matches!(
            deny_unsupported("git-write"),
            WindowsFsError::UnsupportedOperation("git-write")
        ));
        assert!(matches!(
            deny_unsupported("publication"),
            WindowsFsError::UnsupportedOperation("publication")
        ));
        assert!(matches!(
            deny_unsupported("arbitrary-network"),
            WindowsFsError::UnsupportedOperation("arbitrary-network")
        ));
    }

    #[test]
    fn native_diff_is_deterministic_and_versioned() {
        let pre = "<h1>Ready</h1>\n";
        let post = "<h1>Ready for review</h1>\n";
        let first = native_diff(pre, post);
        let second = native_diff(pre, post);
        assert_eq!(first, second);
        assert_eq!(first.version, NATIVE_DIFF_VERSION);
        assert!(first.text.contains("-<h1>Ready</h1>"));
        assert!(first.text.contains("+<h1>Ready for review</h1>"));
    }

    #[test]
    fn cleanup_refuses_non_broker_temp_names() {
        let root = fixture_root();
        let path = root.path().join("not-broker.tmp");
        fs::write(&path, "x").unwrap();
        assert!(cleanup_broker_temp(&path).is_err());
    }
}
