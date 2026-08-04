//! Turn-scoped immutable Source snapshots for Agent context.

use std::{error::Error, fmt, str::FromStr};

use sha2::{Digest, Sha256};
use uuid::{Uuid, Variant, Version};

use crate::{AgentTurnId, InvalidAgentId, ProjectTimeError};

/// Origin kind for an explicitly selected local UTF-8 text file snapshot.
pub const SOURCE_ORIGIN_LOCAL_TEXT_FILE: &str = "local_text_file";

/// Maximum accepted Source content size in UTF-8 bytes.
pub const MAX_SOURCE_UTF8: usize = 64 * 1024;

/// Opaque identifier for an immutable Source snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SourceId(Uuid);

impl SourceId {
    /// Generates a new UUID version 7 identifier.
    #[must_use]
    pub fn generate() -> Self {
        Self(Uuid::now_v7())
    }

    /// Parses a persisted UUID version 7 identifier.
    pub fn parse(value: &str) -> Result<Self, InvalidAgentId> {
        value.parse()
    }
}

impl fmt::Display for SourceId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl FromStr for SourceId {
    type Err = InvalidAgentId;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let id =
            Uuid::parse_str(value).map_err(|_| InvalidAgentId::Malformed { kind: "source ID" })?;
        if id.get_variant() != Variant::RFC4122 {
            return Err(InvalidAgentId::InvalidVariant { kind: "source ID" });
        }
        if id.get_version() != Some(Version::SortRand) {
            return Err(InvalidAgentId::NotVersionSeven { kind: "source ID" });
        }
        Ok(Self(id))
    }
}

/// Immutable turn-scoped Source snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    id: SourceId,
    origin_kind: String,
    display_name: String,
    byte_count: u64,
    content_sha256: String,
    content: String,
    created_at_unix_ms: i64,
}

impl Source {
    /// Creates a validated local-text Source from exact captured UTF-8 content.
    pub fn new_local_text(
        display_name: impl Into<String>,
        content: impl Into<String>,
    ) -> Result<Self, SourceValidationError> {
        let display_name = display_name.into();
        let content = content.into();
        validate_source_display_name(&display_name)?;
        validate_source_content(&content)?;
        let byte_count =
            u64::try_from(content.len()).map_err(|_| SourceValidationError::TooLarge {
                byte_count: content.len(),
            })?;
        Ok(Self {
            id: SourceId::generate(),
            origin_kind: SOURCE_ORIGIN_LOCAL_TEXT_FILE.to_owned(),
            display_name,
            byte_count,
            content_sha256: hash_source_bytes(content.as_bytes()),
            content,
            created_at_unix_ms: unix_now_ms()?,
        })
    }

    /// Reconstructs a persisted Source.
    #[allow(clippy::too_many_arguments)]
    pub fn from_stored_parts(
        id: &str,
        origin_kind: impl Into<String>,
        display_name: impl Into<String>,
        byte_count: u64,
        content_sha256: impl Into<String>,
        content: impl Into<String>,
        created_at_unix_ms: i64,
    ) -> Result<Self, SourceReconstructionError> {
        let content_sha256 = content_sha256.into();
        if content_sha256.len() != 64
            || !content_sha256
                .chars()
                .all(|ch| matches!(ch, '0'..='9' | 'a'..='f'))
        {
            return Err(SourceReconstructionError::InvalidHash);
        }
        let origin_kind = origin_kind.into();
        if origin_kind != SOURCE_ORIGIN_LOCAL_TEXT_FILE {
            return Err(SourceReconstructionError::InvalidOrigin);
        }
        Ok(Self {
            id: SourceId::parse(id)?,
            origin_kind,
            display_name: display_name.into(),
            byte_count,
            content_sha256,
            content: content.into(),
            created_at_unix_ms,
        })
    }

    /// Returns the Source identifier.
    #[must_use]
    pub const fn id(&self) -> SourceId {
        self.id
    }

    /// Returns the origin kind label.
    #[must_use]
    pub fn origin_kind(&self) -> &str {
        &self.origin_kind
    }

    /// Returns the safe display name.
    #[must_use]
    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    /// Returns the exact UTF-8 byte count.
    #[must_use]
    pub const fn byte_count(&self) -> u64 {
        self.byte_count
    }

    /// Returns the canonical lowercase SHA-256 hex digest.
    #[must_use]
    pub fn content_sha256(&self) -> &str {
        &self.content_sha256
    }

    /// Returns the exact captured UTF-8 content.
    #[must_use]
    pub fn content(&self) -> &str {
        &self.content
    }

    /// Returns creation time.
    #[must_use]
    pub const fn created_at_unix_ms(&self) -> i64 {
        self.created_at_unix_ms
    }
}

/// Ordered association between a turn and one Source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnSource {
    turn_id: AgentTurnId,
    source: Source,
    attachment_order: u32,
}

impl TurnSource {
    /// Creates an ordered turn association.
    #[must_use]
    pub fn new(turn_id: AgentTurnId, source: Source, attachment_order: u32) -> Self {
        Self {
            turn_id,
            source,
            attachment_order,
        }
    }

    /// Reconstructs a persisted association.
    pub fn from_stored_parts(
        turn_id: &str,
        source: Source,
        attachment_order: u32,
    ) -> Result<Self, SourceReconstructionError> {
        Ok(Self {
            turn_id: AgentTurnId::parse(turn_id)?,
            source,
            attachment_order,
        })
    }

    /// Returns the owning turn identifier.
    #[must_use]
    pub const fn turn_id(&self) -> AgentTurnId {
        self.turn_id
    }

    /// Returns the immutable Source.
    #[must_use]
    pub const fn source(&self) -> &Source {
        &self.source
    }

    /// Returns the deterministic attachment order within the turn.
    #[must_use]
    pub const fn attachment_order(&self) -> u32 {
        self.attachment_order
    }
}

/// Allowlisted Source metadata used when framing provider context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceContext {
    /// Origin kind label.
    pub origin_kind: String,
    /// Safe display name.
    pub display_name: String,
    /// Exact UTF-8 byte count.
    pub byte_count: u64,
    /// Canonical lowercase SHA-256 hex digest.
    pub content_sha256: String,
    /// Exact captured UTF-8 content.
    pub content: String,
}

impl From<&Source> for SourceContext {
    fn from(source: &Source) -> Self {
        Self {
            origin_kind: source.origin_kind().to_owned(),
            display_name: source.display_name().to_owned(),
            byte_count: source.byte_count(),
            content_sha256: source.content_sha256().to_owned(),
            content: source.content().to_owned(),
        }
    }
}

/// Validates a lossless safe basename for Source display.
pub fn validate_source_display_name(display_name: &str) -> Result<(), SourceValidationError> {
    if display_name.is_empty() {
        return Err(SourceValidationError::UnsafeDisplayName);
    }
    for ch in display_name.chars() {
        let code = ch as u32;
        if code <= 0x1F
            || code == 0x7F
            || (0x80..=0x9F).contains(&code)
            || code == 0x061C
            || (0x200E..=0x200F).contains(&code)
            || (0x202A..=0x202E).contains(&code)
            || (0x2028..=0x2029).contains(&code)
            || (0x2066..=0x2069).contains(&code)
        {
            return Err(SourceValidationError::UnsafeDisplayName);
        }
    }
    Ok(())
}

/// Validates exact Source text content bounds and NUL rejection.
pub fn validate_source_content(content: &str) -> Result<(), SourceValidationError> {
    if content.len() > MAX_SOURCE_UTF8 {
        return Err(SourceValidationError::TooLarge {
            byte_count: content.len(),
        });
    }
    if content.as_bytes().contains(&0) {
        return Err(SourceValidationError::ContainsNul);
    }
    Ok(())
}

/// Computes the canonical lowercase SHA-256 hex digest over exact bytes.
#[must_use]
pub fn hash_source_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(64);
    for byte in digest {
        fmt::Write::write_fmt(&mut hex, format_args!("{byte:02x}"))
            .expect("writing to String cannot fail");
    }
    hex
}

/// Formats user text with optional untrusted Source framing for provider input.
#[must_use]
pub fn format_turn_user_content(user_text: &str, source: Option<&SourceContext>) -> String {
    let Some(source) = source else {
        return user_text.to_owned();
    };
    format!(
        "{user_text}\n\nAttached untrusted source snapshot:\norigin: {}\nname: {}\nbytes: {}\nsha256: {}\n\n-----BEGIN ATTACHED SOURCE-----\n{}\n-----END ATTACHED SOURCE-----",
        source.origin_kind,
        source.display_name,
        source.byte_count,
        source.content_sha256,
        source.content
    )
}

fn unix_now_ms() -> Result<i64, ProjectTimeError> {
    use std::time::{SystemTime, UNIX_EPOCH};
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(ProjectTimeError::BeforeUnixEpoch)?;
    i64::try_from(duration.as_millis()).map_err(|_| ProjectTimeError::OutOfRange)
}

/// Source capture or validation failure.
#[derive(Debug)]
pub enum SourceValidationError {
    /// Display name is empty or contains disallowed control characters.
    UnsafeDisplayName,
    /// Content exceeds the UTF-8 byte ceiling.
    TooLarge {
        /// Observed UTF-8 byte count.
        byte_count: usize,
    },
    /// Content contains a NUL byte.
    ContainsNul,
    /// Clock failure while stamping creation time.
    Time(ProjectTimeError),
}

impl From<ProjectTimeError> for SourceValidationError {
    fn from(error: ProjectTimeError) -> Self {
        Self::Time(error)
    }
}

impl fmt::Display for SourceValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsafeDisplayName => formatter.write_str("source display name is unsafe"),
            Self::TooLarge { byte_count } => write!(
                formatter,
                "source content has {byte_count} UTF-8 bytes; the maximum is {MAX_SOURCE_UTF8}"
            ),
            Self::ContainsNul => formatter.write_str("source content contains a NUL character"),
            Self::Time(error) => error.fmt(formatter),
        }
    }
}

impl Error for SourceValidationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Time(error) => Some(error),
            Self::UnsafeDisplayName | Self::TooLarge { .. } | Self::ContainsNul => None,
        }
    }
}

/// Failure reconstructing a persisted Source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceReconstructionError {
    /// Source identifier is invalid.
    InvalidId(InvalidAgentId),
    /// Origin kind is not allowlisted.
    InvalidOrigin,
    /// Hash encoding is not canonical lowercase hex.
    InvalidHash,
}

impl From<InvalidAgentId> for SourceReconstructionError {
    fn from(error: InvalidAgentId) -> Self {
        Self::InvalidId(error)
    }
}

impl fmt::Display for SourceReconstructionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidId(error) => error.fmt(formatter),
            Self::InvalidOrigin => formatter.write_str("source origin kind is invalid"),
            Self::InvalidHash => formatter.write_str("source content hash is invalid"),
        }
    }
}

impl Error for SourceReconstructionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidId(error) => Some(error),
            Self::InvalidOrigin | Self::InvalidHash => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_ids_are_version_seven() {
        let id = SourceId::generate();
        assert_eq!(SourceId::parse(&id.to_string()).unwrap(), id);
    }

    #[test]
    fn hash_is_canonical_lowercase_sha256_over_exact_bytes() {
        let with_bom = "\u{FEFF}line\r\n";
        let digest = hash_source_bytes(with_bom.as_bytes());
        assert_eq!(digest.len(), 64);
        assert!(digest.chars().all(|ch| matches!(ch, '0'..='9' | 'a'..='f')));
        assert_eq!(digest, format!("{:x}", Sha256::digest(with_bom.as_bytes())));
    }

    #[test]
    fn local_text_source_preserves_exact_bytes_and_rejects_unsafe_names() {
        let source = Source::new_local_text("notes.txt", "α\r\n\t ").unwrap();
        assert_eq!(source.origin_kind(), SOURCE_ORIGIN_LOCAL_TEXT_FILE);
        assert_eq!(source.content(), "α\r\n\t ");
        assert_eq!(source.byte_count(), "α\r\n\t ".len() as u64);
        assert_eq!(
            source.content_sha256(),
            hash_source_bytes("α\r\n\t ".as_bytes())
        );
        assert!(matches!(
            validate_source_display_name(""),
            Err(SourceValidationError::UnsafeDisplayName)
        ));
        assert!(matches!(
            validate_source_display_name("bad\nname"),
            Err(SourceValidationError::UnsafeDisplayName)
        ));
        assert!(matches!(
            validate_source_display_name("bad\u{202E}name"),
            Err(SourceValidationError::UnsafeDisplayName)
        ));
        assert!(matches!(
            validate_source_content(&"a".repeat(MAX_SOURCE_UTF8 + 1)),
            Err(SourceValidationError::TooLarge { .. })
        ));
    }

    #[test]
    fn turn_user_content_framing_is_deterministic() {
        let source = SourceContext {
            origin_kind: SOURCE_ORIGIN_LOCAL_TEXT_FILE.to_owned(),
            display_name: "a.txt".to_owned(),
            byte_count: 5,
            content_sha256: "a".repeat(64),
            content: "hello".to_owned(),
        };
        assert_eq!(
            format_turn_user_content("Ask", Some(&source)),
            "Ask\n\nAttached untrusted source snapshot:\norigin: local_text_file\nname: a.txt\nbytes: 5\nsha256: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n\n-----BEGIN ATTACHED SOURCE-----\nhello\n-----END ATTACHED SOURCE-----"
        );
        assert_eq!(format_turn_user_content("Ask", None), "Ask");
    }
}
