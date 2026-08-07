//! Immutable Artifact and Artifact Version records created from completed Agent turns.

use std::{error::Error, fmt, str::FromStr};

use uuid::{Uuid, Variant, Version};

use crate::{
    AgentSessionId, AgentTurnId, InvalidAgentId, ProjectId, ProjectTimeError, ProviderRequestId,
    TITLE_MAX_SCALARS, hash_source_bytes,
};

/// Maximum Artifact content size in UTF-8 bytes (matches Agent output ceiling).
pub const MAX_ARTIFACT_CONTENT_UTF8: usize = 1024 * 1024;

/// Allowlisted Artifact kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ArtifactKind {
    /// Short conclusion from an Agent turn.
    Conclusion,
    /// Recommendation captured from an Agent turn.
    Recommendation,
    /// Decision record captured from an Agent turn.
    DecisionRecord,
    /// Requirements captured from an Agent turn.
    Requirements,
    /// Implementation plan captured from an Agent turn.
    ImplementationPlan,
    /// Research brief captured from an Agent turn.
    ResearchBrief,
    /// Critique captured from an Agent turn.
    Critique,
}

impl ArtifactKind {
    /// Default kind when the user does not choose.
    pub const DEFAULT: Self = Self::Conclusion;

    /// Stable wire / storage label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Conclusion => "conclusion",
            Self::Recommendation => "recommendation",
            Self::DecisionRecord => "decision_record",
            Self::Requirements => "requirements",
            Self::ImplementationPlan => "implementation_plan",
            Self::ResearchBrief => "research_brief",
            Self::Critique => "critique",
        }
    }

    /// Parses an allowlisted kind label.
    pub fn parse(value: &str) -> Result<Self, InvalidArtifactKind> {
        match value {
            "conclusion" => Ok(Self::Conclusion),
            "recommendation" => Ok(Self::Recommendation),
            "decision_record" => Ok(Self::DecisionRecord),
            "requirements" => Ok(Self::Requirements),
            "implementation_plan" => Ok(Self::ImplementationPlan),
            "research_brief" => Ok(Self::ResearchBrief),
            "critique" => Ok(Self::Critique),
            _ => Err(InvalidArtifactKind),
        }
    }
}

impl fmt::Display for ArtifactKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Unknown or disallowed Artifact kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidArtifactKind;

impl fmt::Display for InvalidArtifactKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("artifact kind is not allowlisted")
    }
}

impl Error for InvalidArtifactKind {}

macro_rules! define_uuid_v7_id {
    ($(#[$meta:meta])* $name:ident, $label:literal) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(Uuid);

        impl $name {
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

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }

        impl FromStr for $name {
            type Err = InvalidAgentId;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                let id = Uuid::parse_str(value).map_err(|_| InvalidAgentId::Malformed {
                    kind: $label,
                })?;
                if id.get_variant() != Variant::RFC4122 {
                    return Err(InvalidAgentId::InvalidVariant { kind: $label });
                }
                if id.get_version() != Some(Version::SortRand) {
                    return Err(InvalidAgentId::NotVersionSeven { kind: $label });
                }
                Ok(Self(id))
            }
        }
    };
}

define_uuid_v7_id!(
    /// Opaque identifier for an Artifact.
    ArtifactId,
    "artifact ID"
);
define_uuid_v7_id!(
    /// Opaque identifier for an Artifact Version.
    ArtifactVersionId,
    "artifact version ID"
);

/// Durable Artifact record (stable identity and metadata).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Artifact {
    id: ArtifactId,
    title: String,
    kind: ArtifactKind,
    project_id: Option<ProjectId>,
    created_at_unix_ms: i64,
}

impl Artifact {
    /// Creates a new Artifact with a generated identity.
    pub fn new(
        title: impl Into<String>,
        kind: ArtifactKind,
        project_id: Option<ProjectId>,
    ) -> Result<Self, ProjectTimeError> {
        Ok(Self {
            id: ArtifactId::generate(),
            title: title.into(),
            kind,
            project_id,
            created_at_unix_ms: unix_now_ms()?,
        })
    }

    /// Reconstructs a persisted Artifact after validating identifiers and kind.
    pub fn from_stored_parts(
        id: &str,
        title: impl Into<String>,
        kind: &str,
        project_id: Option<&str>,
        created_at_unix_ms: i64,
    ) -> Result<Self, ArtifactReconstructionError> {
        let title = title.into();
        if title.trim().is_empty() {
            return Err(ArtifactReconstructionError::InvalidTitle);
        }
        let project_id = project_id
            .map(ProjectId::parse)
            .transpose()
            .map_err(ArtifactReconstructionError::InvalidProjectId)?;
        Ok(Self {
            id: ArtifactId::parse(id)?,
            title,
            kind: ArtifactKind::parse(kind)?,
            project_id,
            created_at_unix_ms,
        })
    }

    /// Returns the Artifact identifier.
    #[must_use]
    pub const fn id(&self) -> ArtifactId {
        self.id
    }

    /// Returns the short title.
    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Returns the allowlisted kind.
    #[must_use]
    pub const fn kind(&self) -> ArtifactKind {
        self.kind
    }

    /// Returns the optional Project association frozen at save time.
    #[must_use]
    pub const fn project_id(&self) -> Option<ProjectId> {
        self.project_id
    }

    /// Returns creation time in Unix milliseconds.
    #[must_use]
    pub const fn created_at_unix_ms(&self) -> i64 {
        self.created_at_unix_ms
    }
}

/// Frozen provenance copied from the source Agent turn at save time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactVersionProvenance {
    source_session_id: AgentSessionId,
    source_turn_id: AgentTurnId,
    provider_profile_id: String,
    model_id: String,
    prompt_version: String,
    project_id: Option<ProjectId>,
    provider_request_id: ProviderRequestId,
}

impl ArtifactVersionProvenance {
    /// Builds provenance from exact turn-derived values.
    pub fn new(
        source_session_id: AgentSessionId,
        source_turn_id: AgentTurnId,
        provider_profile_id: impl Into<String>,
        model_id: impl Into<String>,
        prompt_version: impl Into<String>,
        project_id: Option<ProjectId>,
        provider_request_id: ProviderRequestId,
    ) -> Self {
        Self {
            source_session_id,
            source_turn_id,
            provider_profile_id: provider_profile_id.into(),
            model_id: model_id.into(),
            prompt_version: prompt_version.into(),
            project_id,
            provider_request_id,
        }
    }

    /// Reconstructs provenance after validating identifiers.
    #[allow(clippy::too_many_arguments)]
    pub fn from_stored_parts(
        source_session_id: &str,
        source_turn_id: &str,
        provider_profile_id: impl Into<String>,
        model_id: impl Into<String>,
        prompt_version: impl Into<String>,
        project_id: Option<&str>,
        provider_request_id: &str,
    ) -> Result<Self, ArtifactReconstructionError> {
        let project_id = project_id
            .map(ProjectId::parse)
            .transpose()
            .map_err(ArtifactReconstructionError::InvalidProjectId)?;
        Ok(Self {
            source_session_id: AgentSessionId::parse(source_session_id)?,
            source_turn_id: AgentTurnId::parse(source_turn_id)?,
            provider_profile_id: provider_profile_id.into(),
            model_id: model_id.into(),
            prompt_version: prompt_version.into(),
            project_id,
            provider_request_id: ProviderRequestId::parse(provider_request_id)?,
        })
    }

    /// Returns the source session identifier.
    #[must_use]
    pub const fn source_session_id(&self) -> AgentSessionId {
        self.source_session_id
    }

    /// Returns the source turn identifier.
    #[must_use]
    pub const fn source_turn_id(&self) -> AgentTurnId {
        self.source_turn_id
    }

    /// Returns the provider-profile identifier.
    #[must_use]
    pub fn provider_profile_id(&self) -> &str {
        &self.provider_profile_id
    }

    /// Returns the model identifier.
    #[must_use]
    pub fn model_id(&self) -> &str {
        &self.model_id
    }

    /// Returns the prompt-version identifier.
    #[must_use]
    pub fn prompt_version(&self) -> &str {
        &self.prompt_version
    }

    /// Returns the optional Project association from the source turn.
    #[must_use]
    pub const fn project_id(&self) -> Option<ProjectId> {
        self.project_id
    }

    /// Returns the provider-request identifier.
    #[must_use]
    pub const fn provider_request_id(&self) -> ProviderRequestId {
        self.provider_request_id
    }
}

/// Immutable Artifact Version (content + hash + provenance).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactVersion {
    id: ArtifactVersionId,
    artifact_id: ArtifactId,
    version_ordinal: u64,
    content: String,
    content_sha256: String,
    provenance: ArtifactVersionProvenance,
    created_at_unix_ms: i64,
}

impl ArtifactVersion {
    /// Creates immutable version 1 for a new Artifact from exact content and provenance.
    pub fn new_first(
        artifact_id: ArtifactId,
        content: impl Into<String>,
        provenance: ArtifactVersionProvenance,
    ) -> Result<Self, ArtifactValidationError> {
        let content = content.into();
        validate_artifact_content(&content)?;
        Ok(Self {
            id: ArtifactVersionId::generate(),
            artifact_id,
            version_ordinal: 1,
            content_sha256: hash_source_bytes(content.as_bytes()),
            content,
            provenance,
            created_at_unix_ms: unix_now_ms()?,
        })
    }

    /// Reconstructs a persisted version after reapplying every canonical invariant.
    #[allow(clippy::too_many_arguments)]
    pub fn from_stored_parts(
        id: &str,
        artifact_id: &str,
        version_ordinal: u64,
        content: impl Into<String>,
        content_sha256: impl Into<String>,
        provenance: ArtifactVersionProvenance,
        created_at_unix_ms: i64,
    ) -> Result<Self, ArtifactReconstructionError> {
        if version_ordinal == 0 {
            return Err(ArtifactReconstructionError::InvalidVersionOrdinal);
        }
        let content = content.into();
        validate_artifact_content(&content).map_err(|error| match error {
            ArtifactValidationError::Empty => ArtifactReconstructionError::EmptyContent,
            ArtifactValidationError::TooLarge { .. } => ArtifactReconstructionError::TooLarge,
            ArtifactValidationError::ContainsNul => ArtifactReconstructionError::ContainsNul,
            ArtifactValidationError::Time(_) => ArtifactReconstructionError::InvalidContent,
        })?;
        let content_sha256 = content_sha256.into();
        if !is_canonical_sha256_hex(&content_sha256) {
            return Err(ArtifactReconstructionError::InvalidHash);
        }
        let recomputed = hash_source_bytes(content.as_bytes());
        if content_sha256 != recomputed {
            return Err(ArtifactReconstructionError::HashMismatch);
        }
        Ok(Self {
            id: ArtifactVersionId::parse(id)?,
            artifact_id: ArtifactId::parse(artifact_id)?,
            version_ordinal,
            content,
            content_sha256,
            provenance,
            created_at_unix_ms,
        })
    }

    /// Returns the version identifier.
    #[must_use]
    pub const fn id(&self) -> ArtifactVersionId {
        self.id
    }

    /// Returns the owning Artifact identifier.
    #[must_use]
    pub const fn artifact_id(&self) -> ArtifactId {
        self.artifact_id
    }

    /// Returns the version ordinal (1-based).
    #[must_use]
    pub const fn version_ordinal(&self) -> u64 {
        self.version_ordinal
    }

    /// Returns the exact frozen content.
    #[must_use]
    pub fn content(&self) -> &str {
        &self.content
    }

    /// Returns the canonical SHA-256 hex digest of the content bytes.
    #[must_use]
    pub fn content_sha256(&self) -> &str {
        &self.content_sha256
    }

    /// Returns frozen turn provenance.
    #[must_use]
    pub const fn provenance(&self) -> &ArtifactVersionProvenance {
        &self.provenance
    }

    /// Returns creation time in Unix milliseconds.
    #[must_use]
    pub const fn created_at_unix_ms(&self) -> i64 {
        self.created_at_unix_ms
    }
}

/// List-row metadata for choosing an Artifact without loading full version bodies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactSummary {
    artifact: Artifact,
    latest_version_id: ArtifactVersionId,
    latest_version_ordinal: u64,
}

impl ArtifactSummary {
    /// Builds a summary from an Artifact and its latest version identity.
    pub fn new(
        artifact: Artifact,
        latest_version_id: ArtifactVersionId,
        latest_version_ordinal: u64,
    ) -> Result<Self, ArtifactReconstructionError> {
        if latest_version_ordinal == 0 {
            return Err(ArtifactReconstructionError::InvalidVersionOrdinal);
        }
        Ok(Self {
            artifact,
            latest_version_id,
            latest_version_ordinal,
        })
    }

    /// Returns the Artifact metadata.
    #[must_use]
    pub const fn artifact(&self) -> &Artifact {
        &self.artifact
    }

    /// Returns the latest version identifier.
    #[must_use]
    pub const fn latest_version_id(&self) -> ArtifactVersionId {
        self.latest_version_id
    }

    /// Returns the latest version ordinal.
    #[must_use]
    pub const fn latest_version_ordinal(&self) -> u64 {
        self.latest_version_ordinal
    }
}

/// Full Artifact with every persisted version (content and provenance).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactDetail {
    artifact: Artifact,
    versions: Vec<ArtifactVersion>,
}

impl ArtifactDetail {
    /// Builds a detail record after checking version ownership.
    pub fn new(
        artifact: Artifact,
        versions: Vec<ArtifactVersion>,
    ) -> Result<Self, ArtifactReconstructionError> {
        if versions.is_empty() {
            return Err(ArtifactReconstructionError::MissingVersions);
        }
        for version in &versions {
            if version.artifact_id() != artifact.id() {
                return Err(ArtifactReconstructionError::VersionArtifactMismatch);
            }
        }
        Ok(Self { artifact, versions })
    }

    /// Returns the Artifact metadata.
    #[must_use]
    pub const fn artifact(&self) -> &Artifact {
        &self.artifact
    }

    /// Returns every version in ascending ordinal order.
    #[must_use]
    pub fn versions(&self) -> &[ArtifactVersion] {
        &self.versions
    }
}

/// Derives a short Artifact title from agent text (same spirit as session titles).
#[must_use]
pub fn derive_artifact_title(agent_text: &str) -> String {
    let line = agent_text
        .lines()
        .map(str::trim)
        .find(|candidate| !candidate.is_empty())
        .unwrap_or("Artifact");
    truncate_scalars(line, TITLE_MAX_SCALARS)
}

/// Resolves the title from an optional user override, falling back to derivation.
#[must_use]
pub fn resolve_artifact_title(title_override: Option<&str>, agent_text: &str) -> String {
    match title_override {
        Some(value) if !value.trim().is_empty() => {
            truncate_scalars(value.trim(), TITLE_MAX_SCALARS)
        }
        _ => derive_artifact_title(agent_text),
    }
}

/// Validates exact Artifact content bounds and NUL rejection.
pub fn validate_artifact_content(content: &str) -> Result<(), ArtifactValidationError> {
    if content.is_empty() {
        return Err(ArtifactValidationError::Empty);
    }
    if content.len() > MAX_ARTIFACT_CONTENT_UTF8 {
        return Err(ArtifactValidationError::TooLarge {
            byte_count: content.len(),
        });
    }
    if content.as_bytes().contains(&0) {
        return Err(ArtifactValidationError::ContainsNul);
    }
    Ok(())
}

fn is_canonical_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.chars().all(|ch| matches!(ch, '0'..='9' | 'a'..='f'))
}

fn truncate_scalars(value: &str, max_scalars: usize) -> String {
    let count = value.chars().count();
    if count <= max_scalars {
        return value.to_owned();
    }
    let mut truncated: String = value.chars().take(max_scalars.saturating_sub(1)).collect();
    truncated.push('…');
    truncated
}

fn unix_now_ms() -> Result<i64, ProjectTimeError> {
    use std::time::{SystemTime, UNIX_EPOCH};
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(ProjectTimeError::BeforeUnixEpoch)?;
    i64::try_from(duration.as_millis()).map_err(|_| ProjectTimeError::OutOfRange)
}

/// Artifact content validation failure.
#[derive(Debug)]
pub enum ArtifactValidationError {
    /// Content is empty.
    Empty,
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

impl From<ProjectTimeError> for ArtifactValidationError {
    fn from(error: ProjectTimeError) -> Self {
        Self::Time(error)
    }
}

impl fmt::Display for ArtifactValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("artifact content is empty"),
            Self::TooLarge { byte_count } => write!(
                formatter,
                "artifact content has {byte_count} UTF-8 bytes; the maximum is {MAX_ARTIFACT_CONTENT_UTF8}"
            ),
            Self::ContainsNul => formatter.write_str("artifact content contains a NUL character"),
            Self::Time(error) => write!(formatter, "could not stamp artifact time: {error}"),
        }
    }
}

impl Error for ArtifactValidationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Time(error) => Some(error),
            Self::Empty | Self::TooLarge { .. } | Self::ContainsNul => None,
        }
    }
}

/// Failure reconstructing a persisted Artifact or version.
#[derive(Debug)]
pub enum ArtifactReconstructionError {
    /// Identifier is invalid.
    InvalidId(InvalidAgentId),
    /// Project identifier is invalid.
    InvalidProjectId(crate::InvalidProjectId),
    /// Kind is not allowlisted.
    InvalidKind(InvalidArtifactKind),
    /// Title is empty after trim.
    InvalidTitle,
    /// Version ordinal is zero.
    InvalidVersionOrdinal,
    /// Content is empty.
    EmptyContent,
    /// Content exceeds bounds.
    TooLarge,
    /// Content contains NUL.
    ContainsNul,
    /// Content failed validation for another reason.
    InvalidContent,
    /// Stored hash is not canonical lowercase hex.
    InvalidHash,
    /// Stored hash does not match recomputed content digest.
    HashMismatch,
    /// Detail has no versions.
    MissingVersions,
    /// Version points at a different Artifact.
    VersionArtifactMismatch,
}

impl From<InvalidAgentId> for ArtifactReconstructionError {
    fn from(error: InvalidAgentId) -> Self {
        Self::InvalidId(error)
    }
}

impl From<InvalidArtifactKind> for ArtifactReconstructionError {
    fn from(error: InvalidArtifactKind) -> Self {
        Self::InvalidKind(error)
    }
}

impl fmt::Display for ArtifactReconstructionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidId(error) => error.fmt(formatter),
            Self::InvalidProjectId(error) => error.fmt(formatter),
            Self::InvalidKind(error) => error.fmt(formatter),
            Self::InvalidTitle => formatter.write_str("stored artifact title is empty"),
            Self::InvalidVersionOrdinal => {
                formatter.write_str("stored artifact version ordinal is invalid")
            }
            Self::EmptyContent => formatter.write_str("stored artifact content is empty"),
            Self::TooLarge => formatter.write_str("stored artifact content exceeds bounds"),
            Self::ContainsNul => formatter.write_str("stored artifact content contains NUL"),
            Self::InvalidContent => formatter.write_str("stored artifact content is invalid"),
            Self::InvalidHash => formatter.write_str("stored artifact content hash is invalid"),
            Self::HashMismatch => {
                formatter.write_str("stored artifact content hash does not match content")
            }
            Self::MissingVersions => formatter.write_str("artifact has no versions"),
            Self::VersionArtifactMismatch => {
                formatter.write_str("artifact version does not belong to the artifact")
            }
        }
    }
}

impl Error for ArtifactReconstructionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidId(error) => Some(error),
            Self::InvalidProjectId(error) => Some(error),
            Self::InvalidKind(error) => Some(error),
            Self::InvalidTitle
            | Self::InvalidVersionOrdinal
            | Self::EmptyContent
            | Self::TooLarge
            | Self::ContainsNul
            | Self::InvalidContent
            | Self::InvalidHash
            | Self::HashMismatch
            | Self::MissingVersions
            | Self::VersionArtifactMismatch => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash_source_bytes;

    #[test]
    fn kind_allowlist_parses_exactly_the_seven_labels() {
        assert_eq!(
            ArtifactKind::parse("conclusion").unwrap(),
            ArtifactKind::Conclusion
        );
        assert_eq!(
            ArtifactKind::parse("recommendation").unwrap(),
            ArtifactKind::Recommendation
        );
        assert_eq!(
            ArtifactKind::parse("decision_record").unwrap(),
            ArtifactKind::DecisionRecord
        );
        assert_eq!(
            ArtifactKind::parse("requirements").unwrap(),
            ArtifactKind::Requirements
        );
        assert_eq!(
            ArtifactKind::parse("implementation_plan").unwrap(),
            ArtifactKind::ImplementationPlan
        );
        assert_eq!(
            ArtifactKind::parse("research_brief").unwrap(),
            ArtifactKind::ResearchBrief
        );
        assert_eq!(
            ArtifactKind::parse("critique").unwrap(),
            ArtifactKind::Critique
        );
        assert!(ArtifactKind::parse("Conclusion").is_err());
        assert!(ArtifactKind::parse("other").is_err());
        assert!(ArtifactKind::parse("").is_err());
    }

    #[test]
    fn derive_title_truncates_like_session_titles() {
        assert_eq!(
            derive_artifact_title("  \nFirst line\nSecond"),
            "First line"
        );
        let long = "字".repeat(TITLE_MAX_SCALARS + 8);
        let title = derive_artifact_title(&long);
        assert_eq!(title.chars().count(), TITLE_MAX_SCALARS);
        assert!(title.ends_with('…'));
        assert_eq!(
            resolve_artifact_title(Some("  Custom  "), "ignored"),
            "Custom"
        );
        assert_eq!(
            resolve_artifact_title(Some("   "), "Body line"),
            "Body line"
        );
        assert_eq!(resolve_artifact_title(None, ""), "Artifact");
    }

    #[test]
    fn version_hashes_content_and_rejects_mismatch_on_reconstruction() {
        let artifact = Artifact::new("Title", ArtifactKind::Conclusion, None).unwrap();
        let provenance = ArtifactVersionProvenance::new(
            AgentSessionId::generate(),
            AgentTurnId::generate(),
            "xai-subscription-oauth",
            "grok-3",
            "tule-direct-agent-v2",
            None,
            ProviderRequestId::generate(),
        );
        let version =
            ArtifactVersion::new_first(artifact.id(), "exact body", provenance.clone()).unwrap();
        assert_eq!(version.content_sha256(), hash_source_bytes(b"exact body"));
        assert_eq!(version.version_ordinal(), 1);

        let restored = ArtifactVersion::from_stored_parts(
            &version.id().to_string(),
            &artifact.id().to_string(),
            1,
            "exact body",
            version.content_sha256(),
            provenance.clone(),
            version.created_at_unix_ms(),
        )
        .unwrap();
        assert_eq!(restored, version);

        assert!(matches!(
            ArtifactVersion::from_stored_parts(
                &version.id().to_string(),
                &artifact.id().to_string(),
                1,
                "tampered",
                version.content_sha256(),
                provenance,
                version.created_at_unix_ms(),
            ),
            Err(ArtifactReconstructionError::HashMismatch)
        ));
    }

    #[test]
    fn empty_content_rejected_without_creating_version() {
        let artifact = Artifact::new("Title", ArtifactKind::Conclusion, None).unwrap();
        let provenance = ArtifactVersionProvenance::new(
            AgentSessionId::generate(),
            AgentTurnId::generate(),
            "profile",
            "model",
            "prompt",
            None,
            ProviderRequestId::generate(),
        );
        assert!(matches!(
            ArtifactVersion::new_first(artifact.id(), "", provenance),
            Err(ArtifactValidationError::Empty)
        ));
    }
}
