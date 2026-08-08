//! Provider-neutral Harness Run records, canonical hashing, and lifecycle policy.

use std::{
    error::Error,
    fmt::{self, Write as _},
    str::FromStr,
};

use sha2::{Digest, Sha256};
use uuid::{Uuid, Variant, Version};

use crate::{
    CapabilityGrantId, CapabilityType, GrantResourceSelector, REGISTERED_OPERATION_SCHEMA_V1,
    hash_source_bytes,
};

/// Versioned canonical field encoding used for approval and pair hashes.
pub const CANONICAL_ENCODING_VERSION: &str = "tule-canonical-v1";

/// Versioned graph-shape fingerprint algorithm.
pub const GRAPH_SHAPE_FINGERPRINT_VERSION: &str = "tule-graph-shape-v1";

/// Maximum preimage and postimage size in UTF-8 bytes.
pub const MAX_RUN_CONTENT_UTF8: usize = 64 * 1024;

/// Exclusive run-root lease duration in milliseconds.
pub const ROOT_LEASE_TTL_MS: i64 = 30_000;

/// Exclusive run-root lease renewal interval in milliseconds.
pub const ROOT_LEASE_RENEW_INTERVAL_MS: i64 = 10_000;

/// Fixed task node registered operation / graph node kind.
pub const NODE_REPLACE_EXISTING_FILE_V1: &str = "replace-existing-file-v1";

/// Fixed protected-validation node kind.
pub const NODE_VERIFY_APPROVED_POSTIMAGE_V1: &str = "verify-approved-postimage-v1";

/// Protected validation product label.
pub const NATIVE_STRUCTURAL_VALIDATION_LABEL: &str = "native structural validation";

/// Exact bootstrap heading before replacement.
pub const BOOTSTRAP_HEADING_BEFORE: &str = "<h1>Ready</h1>";

/// Exact bootstrap heading after replacement.
pub const BOOTSTRAP_HEADING_AFTER: &str = "<h1>Ready for review</h1>";

/// Relative target required by the controlled fixture.
pub const CONTROLLED_RELATIVE_TARGET: &str = "index.html";

/// Execution-policy revision frozen into the first Work 0022 pair.
pub const EXECUTION_POLICY_REVISION_V1: &str = "tule-execution-policy-v1";

/// Retry rule frozen into the first graph: no automatic retry.
pub const RETRY_RULE_NO_AUTOMATIC: &str = "no-automatic-retry-v1";

/// Validation rule identity frozen into the first graph.
pub const VALIDATION_RULE_NATIVE_POSTIMAGE_V1: &str = "native-approved-postimage-v1";

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
            pub fn parse(value: &str) -> Result<Self, InvalidRunId> {
                value.parse()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }

        impl FromStr for $name {
            type Err = InvalidRunId;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                let id = Uuid::parse_str(value).map_err(|_| InvalidRunId::Malformed {
                    kind: $label,
                })?;
                if id.get_variant() != Variant::RFC4122 {
                    return Err(InvalidRunId::InvalidVariant { kind: $label });
                }
                if id.get_version() != Some(Version::SortRand) {
                    return Err(InvalidRunId::NotVersionSeven { kind: $label });
                }
                Ok(Self(id))
            }
        }
    };
}

define_uuid_v7_id!(
    /// Opaque identifier for a Harness Run.
    HarnessRunId,
    "harness run ID"
);
define_uuid_v7_id!(
    /// Opaque identifier for an Execution Plan Version.
    ExecutionPlanVersionId,
    "execution plan version ID"
);
define_uuid_v7_id!(
    /// Opaque identifier for a Run Graph Version.
    RunGraphVersionId,
    "run graph version ID"
);
define_uuid_v7_id!(
    /// Opaque identifier for a Context Manifest.
    ContextManifestId,
    "context manifest ID"
);
define_uuid_v7_id!(
    /// Opaque identifier for an Approval Record.
    ApprovalRecordId,
    "approval record ID"
);
define_uuid_v7_id!(
    /// Opaque identifier for a Node Attempt.
    NodeAttemptId,
    "node attempt ID"
);
define_uuid_v7_id!(
    /// Opaque identifier for a Run Event.
    RunEventId,
    "run event ID"
);
define_uuid_v7_id!(
    /// Opaque identifier for an Effect Record.
    EffectRecordId,
    "effect record ID"
);
define_uuid_v7_id!(
    /// Opaque identifier for a Checkpoint.
    CheckpointId,
    "checkpoint ID"
);
define_uuid_v7_id!(
    /// Opaque identifier for a Validation Result.
    ValidationResultId,
    "validation result ID"
);
define_uuid_v7_id!(
    /// Opaque identifier for immutable replacement content input.
    ReplacementContentId,
    "replacement content ID"
);
define_uuid_v7_id!(
    /// Opaque identifier for a root lease.
    RootLeaseId,
    "root lease ID"
);
define_uuid_v7_id!(
    /// Opaque identifier for denial evidence.
    DenialEvidenceId,
    "denial evidence ID"
);

/// The reason a persisted Harness identifier is invalid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvalidRunId {
    /// The value is not a UUID.
    Malformed {
        /// Human-readable identifier kind.
        kind: &'static str,
    },
    /// The UUID does not use the RFC 4122 variant.
    InvalidVariant {
        /// Human-readable identifier kind.
        kind: &'static str,
    },
    /// The value is a UUID, but not UUID version 7.
    NotVersionSeven {
        /// Human-readable identifier kind.
        kind: &'static str,
    },
}

impl fmt::Display for InvalidRunId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed { kind } => write!(formatter, "{kind} is not a valid UUID"),
            Self::InvalidVariant { kind } => {
                write!(formatter, "{kind} does not use the RFC 4122 UUID variant")
            }
            Self::NotVersionSeven { kind } => write!(formatter, "{kind} is not UUID version 7"),
        }
    }
}

impl Error for InvalidRunId {}

/// Injected clock for grant/lease expiry tests.
pub trait Clock: Send + Sync {
    /// Returns the current Unix epoch milliseconds.
    fn unix_ms(&self) -> i64;
}

/// System clock using wall time.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn unix_ms(&self) -> i64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        let duration = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time before Unix epoch");
        i64::try_from(duration.as_millis()).expect("system time out of i64 range")
    }
}

/// Mutable test clock.
#[derive(Debug)]
pub struct FakeClock {
    now_unix_ms: std::sync::Mutex<i64>,
}

impl FakeClock {
    /// Creates a clock fixed at `now_unix_ms`.
    #[must_use]
    pub fn new(now_unix_ms: i64) -> Self {
        Self {
            now_unix_ms: std::sync::Mutex::new(now_unix_ms),
        }
    }

    /// Advances the clock by `delta_ms`.
    pub fn advance(&self, delta_ms: i64) {
        let mut guard = self.now_unix_ms.lock().expect("clock lock");
        *guard = guard.saturating_add(delta_ms);
    }

    /// Sets an absolute time.
    pub fn set(&self, now_unix_ms: i64) {
        *self.now_unix_ms.lock().expect("clock lock") = now_unix_ms;
    }
}

impl Clock for FakeClock {
    fn unix_ms(&self) -> i64 {
        *self.now_unix_ms.lock().expect("clock lock")
    }
}

/// Appends one length-prefixed tagged field to a canonical buffer.
pub fn append_canonical_field(buffer: &mut Vec<u8>, name: &str, value: &[u8]) {
    let name_bytes = name.as_bytes();
    buffer.extend_from_slice(&(name_bytes.len() as u32).to_be_bytes());
    buffer.extend_from_slice(name_bytes);
    buffer.extend_from_slice(&(value.len() as u32).to_be_bytes());
    buffer.extend_from_slice(value);
}

/// Hashes a sequence of named fields under `tule-canonical-v1`.
///
/// Fields are encoded in the caller-supplied order. Callers must use a fixed
/// schema order; never depend on incidental map/JSON member order.
#[must_use]
pub fn hash_canonical_fields(fields: &[(&str, &[u8])]) -> String {
    let mut buffer = Vec::new();
    append_canonical_field(
        &mut buffer,
        "encoding",
        CANONICAL_ENCODING_VERSION.as_bytes(),
    );
    for (name, value) in fields {
        append_canonical_field(&mut buffer, name, value);
    }
    hash_source_bytes(&buffer)
}

fn is_canonical_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.chars().all(|ch| matches!(ch, '0'..='9' | 'a'..='f'))
}

/// Validates bounded UTF-8 run content (preimage/postimage).
pub fn validate_run_content_bytes(content: &str) -> Result<(), RunContentError> {
    if content.len() > MAX_RUN_CONTENT_UTF8 {
        return Err(RunContentError::TooLarge {
            byte_count: content.len(),
        });
    }
    if content.as_bytes().contains(&0) {
        return Err(RunContentError::ContainsNul);
    }
    Ok(())
}

/// Content validation failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunContentError {
    /// Content exceeds the 64 KiB ceiling.
    TooLarge {
        /// Observed UTF-8 byte count.
        byte_count: usize,
    },
    /// Content contains a NUL byte.
    ContainsNul,
}

impl fmt::Display for RunContentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge { byte_count } => write!(
                formatter,
                "run content has {byte_count} UTF-8 bytes; the maximum is {MAX_RUN_CONTENT_UTF8}"
            ),
            Self::ContainsNul => formatter.write_str("run content contains a NUL character"),
        }
    }
}

impl Error for RunContentError {}

/// Bootstrap plan-proposal validation failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BootstrapProposalError {
    /// Content failed size/NUL checks.
    Content(RunContentError),
    /// Proposal contained unknown or disallowed fields.
    UnknownFields,
    /// Proposal included framing text or non-postimage payload.
    FramingOrNonPostimage,
    /// Proposal included path/target instructions.
    PathInstructions,
    /// Proposal included patch or shell content.
    PatchOrShell,
    /// Preimage does not contain exactly one required heading.
    PreimageHeadingCount {
        /// Observed occurrence count.
        count: usize,
    },
    /// Diff is broader than the single heading change.
    BroaderDiff,
    /// Multiple heading matches would be changed.
    MultipleMatches,
    /// Relative target is not the controlled fixture path.
    WrongTarget,
}

impl fmt::Display for BootstrapProposalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Content(error) => error.fmt(formatter),
            Self::UnknownFields => formatter.write_str("bootstrap proposal has unknown fields"),
            Self::FramingOrNonPostimage => {
                formatter.write_str("bootstrap proposal is not one complete UTF-8 postimage")
            }
            Self::PathInstructions => {
                formatter.write_str("bootstrap proposal includes path instructions")
            }
            Self::PatchOrShell => {
                formatter.write_str("bootstrap proposal includes patch or shell content")
            }
            Self::PreimageHeadingCount { count } => write!(
                formatter,
                "preimage must contain exactly one `{BOOTSTRAP_HEADING_BEFORE}`; found {count}"
            ),
            Self::BroaderDiff => {
                formatter.write_str("bootstrap proposal changes bytes beyond the approved heading")
            }
            Self::MultipleMatches => {
                formatter.write_str("bootstrap proposal would change multiple heading matches")
            }
            Self::WrongTarget => formatter.write_str("bootstrap proposal target is not index.html"),
        }
    }
}

impl Error for BootstrapProposalError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Content(error) => Some(error),
            _ => None,
        }
    }
}

impl From<RunContentError> for BootstrapProposalError {
    fn from(error: RunContentError) -> Self {
        Self::Content(error)
    }
}

/// Counts non-overlapping occurrences of `needle` in `haystack`.
fn count_occurrences(haystack: &str, needle: &str) -> usize {
    if needle.is_empty() {
        return 0;
    }
    let mut count = 0;
    let mut rest = haystack;
    while let Some(index) = rest.find(needle) {
        count += 1;
        rest = &rest[index + needle.len()..];
    }
    count
}

/// Validates a controlled-fixture bootstrap proposal against its preimage.
///
/// `postimage` must be one complete bounded UTF-8 document that changes exactly
/// one `<h1>Ready</h1>` occurrence to `<h1>Ready for review</h1>` and leaves
/// every other byte unchanged.
pub fn validate_bootstrap_proposal(
    preimage: &str,
    postimage: &str,
    relative_target: &str,
) -> Result<(), BootstrapProposalError> {
    if relative_target != CONTROLLED_RELATIVE_TARGET {
        return Err(BootstrapProposalError::WrongTarget);
    }
    validate_run_content_bytes(preimage)?;
    validate_run_content_bytes(postimage)?;

    // Reject payloads that look like framing, patches, shell, or path instructions
    // rather than a complete HTML postimage for the selected target.
    let lowered = postimage.to_ascii_lowercase();
    if lowered.contains("diff --git")
        || lowered.contains("*** begin patch")
        || lowered.contains("#!/bin/")
        || lowered.contains("powershell")
        || lowered.contains("cmd.exe")
    {
        return Err(BootstrapProposalError::PatchOrShell);
    }
    if lowered.contains("path:")
        || lowered.contains("target path")
        || lowered.contains("write to ")
        || lowered.contains("save as ")
    {
        return Err(BootstrapProposalError::PathInstructions);
    }
    if postimage.contains("-----BEGIN") || postimage.contains("```") {
        return Err(BootstrapProposalError::FramingOrNonPostimage);
    }

    let before_count = count_occurrences(preimage, BOOTSTRAP_HEADING_BEFORE);
    if before_count != 1 {
        return Err(BootstrapProposalError::PreimageHeadingCount {
            count: before_count,
        });
    }
    if count_occurrences(preimage, BOOTSTRAP_HEADING_AFTER) != 0 {
        return Err(BootstrapProposalError::BroaderDiff);
    }

    let expected = preimage.replacen(BOOTSTRAP_HEADING_BEFORE, BOOTSTRAP_HEADING_AFTER, 1);
    if postimage != expected {
        // Distinguish multiple heading replacements from broader edits.
        if count_occurrences(postimage, BOOTSTRAP_HEADING_AFTER) > 1
            || count_occurrences(preimage, BOOTSTRAP_HEADING_BEFORE) > 1
        {
            return Err(BootstrapProposalError::MultipleMatches);
        }
        return Err(BootstrapProposalError::BroaderDiff);
    }
    Ok(())
}

/// Rejects structured proposal maps that carry unknown fields.
pub fn reject_unknown_proposal_fields(
    allowed: &[&str],
    present: &[&str],
) -> Result<(), BootstrapProposalError> {
    for field in present {
        if !allowed.iter().any(|allowed_field| allowed_field == field) {
            return Err(BootstrapProposalError::UnknownFields);
        }
    }
    Ok(())
}

/// Computes the expected unified-style content hash for the exact byte diff.
#[must_use]
pub fn hash_expected_diff(preimage: &str, postimage: &str) -> String {
    let mut buffer = Vec::new();
    append_canonical_field(
        &mut buffer,
        "encoding",
        CANONICAL_ENCODING_VERSION.as_bytes(),
    );
    append_canonical_field(&mut buffer, "preimage", preimage.as_bytes());
    append_canonical_field(&mut buffer, "postimage", postimage.as_bytes());
    hash_source_bytes(&buffer)
}

/// Lifecycle state derived from append-only Run Events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HarnessRunLifecycle {
    /// Run created; bootstrap may proceed.
    Created,
    /// Plan/graph pair frozen awaiting approval.
    AwaitingApproval,
    /// Approved; grants may be issued and execution may proceed.
    Approved,
    /// Task node executing.
    Executing,
    /// Blocked because an effect certainty is unknown or partial.
    BlockedReconciliationRequired,
    /// Quiescent checkpoint recorded; validation may proceed.
    Checkpointed,
    /// Protected validation completed.
    Validated,
    /// Terminal success with Final Work Result.
    Completed,
    /// Explicitly paused.
    Paused,
    /// Explicitly cancelled.
    Cancelled,
    /// Explicitly abandoned.
    Abandoned,
}

impl HarnessRunLifecycle {
    /// Stable snake_case label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::AwaitingApproval => "awaiting_approval",
            Self::Approved => "approved",
            Self::Executing => "executing",
            Self::BlockedReconciliationRequired => "blocked_reconciliation_required",
            Self::Checkpointed => "checkpointed",
            Self::Validated => "validated",
            Self::Completed => "completed",
            Self::Paused => "paused",
            Self::Cancelled => "cancelled",
            Self::Abandoned => "abandoned",
        }
    }

    /// Returns whether the lifecycle is terminal.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled | Self::Abandoned)
    }
}

impl fmt::Display for HarnessRunLifecycle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Effect journal phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EffectJournalPhase {
    /// Immutable prepared record before claim.
    Prepared,
    /// Single claimant bound.
    Claimed,
    /// Durable dispatch recorded before external boundary.
    Dispatched,
    /// Settlement appended.
    Settled,
}

impl EffectJournalPhase {
    /// Stable label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Prepared => "prepared",
            Self::Claimed => "claimed",
            Self::Dispatched => "dispatched",
            Self::Settled => "settled",
        }
    }

    /// Parses a stable label.
    pub fn parse(value: &str) -> Result<Self, InvalidRunId> {
        match value {
            "prepared" => Ok(Self::Prepared),
            "claimed" => Ok(Self::Claimed),
            "dispatched" => Ok(Self::Dispatched),
            "settled" => Ok(Self::Settled),
            _ => Err(InvalidRunId::Malformed {
                kind: "effect journal phase",
            }),
        }
    }
}

impl fmt::Display for EffectJournalPhase {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Operation result once known.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EffectOperationResult {
    /// Operation reported success.
    Success,
    /// Operation reported error.
    Error,
    /// Operation reported cancellation.
    Cancelled,
}

impl EffectOperationResult {
    /// Stable label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Error => "error",
            Self::Cancelled => "cancelled",
        }
    }

    /// Parses a stable label.
    pub fn parse(value: &str) -> Result<Self, InvalidRunId> {
        match value {
            "success" => Ok(Self::Success),
            "error" => Ok(Self::Error),
            "cancelled" => Ok(Self::Cancelled),
            _ => Err(InvalidRunId::Malformed {
                kind: "effect operation result",
            }),
        }
    }
}

/// Effect certainty independent of operation result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EffectCertainty {
    /// Positive evidence the effect committed.
    ConfirmedCommitted,
    /// Positive evidence no effect occurred.
    ConfirmedNoEffect,
    /// Unknown or partial; blocks the whole Run.
    UnknownOrPartial,
}

impl EffectCertainty {
    /// Stable label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ConfirmedCommitted => "confirmed_committed",
            Self::ConfirmedNoEffect => "confirmed_no_effect",
            Self::UnknownOrPartial => "unknown_or_partial",
        }
    }

    /// Parses a stable label.
    pub fn parse(value: &str) -> Result<Self, InvalidRunId> {
        match value {
            "confirmed_committed" => Ok(Self::ConfirmedCommitted),
            "confirmed_no_effect" => Ok(Self::ConfirmedNoEffect),
            "unknown_or_partial" => Ok(Self::UnknownOrPartial),
            _ => Err(InvalidRunId::Malformed {
                kind: "effect certainty",
            }),
        }
    }

    /// Returns whether this certainty blocks the whole Run.
    #[must_use]
    pub const fn blocks_run(self) -> bool {
        matches!(self, Self::UnknownOrPartial)
    }
}

impl fmt::Display for EffectCertainty {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Positive-evidence reconciliation inputs for a replacement effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReconciliationProbe {
    /// Current bytes match the recorded preimage hash.
    MatchesPreimage,
    /// Current bytes match the recorded postimage hash.
    MatchesPostimage,
    /// Current bytes match neither expected hash.
    MatchesNeither,
}

/// Reconciles unknown replacement certainty from positive evidence only.
#[must_use]
pub fn reconcile_replacement_certainty(probe: ReconciliationProbe) -> EffectCertainty {
    match probe {
        ReconciliationProbe::MatchesPreimage => EffectCertainty::ConfirmedNoEffect,
        ReconciliationProbe::MatchesPostimage => EffectCertainty::ConfirmedCommitted,
        ReconciliationProbe::MatchesNeither => EffectCertainty::UnknownOrPartial,
    }
}

/// Kind of append-only Run Event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunEventKind {
    /// Run created.
    RunCreated,
    /// Pair compiled and frozen.
    PairFrozen {
        /// Plan version id.
        plan_version_id: ExecutionPlanVersionId,
        /// Graph version id.
        graph_version_id: RunGraphVersionId,
        /// Approval identity hash.
        approval_hash: String,
    },
    /// Approval recorded.
    Approved {
        /// Approval record id.
        approval_id: ApprovalRecordId,
    },
    /// Grant issued.
    GrantIssued {
        /// Grant id.
        grant_id: CapabilityGrantId,
    },
    /// Grant revoked.
    GrantRevoked {
        /// Grant id.
        grant_id: CapabilityGrantId,
    },
    /// Denial recorded.
    Denied {
        /// Denial evidence id.
        denial_id: DenialEvidenceId,
    },
    /// Effect prepared.
    EffectPrepared {
        /// Effect id.
        effect_id: EffectRecordId,
    },
    /// Effect claimed.
    EffectClaimed {
        /// Effect id.
        effect_id: EffectRecordId,
        /// Claimant identity.
        claimant: String,
    },
    /// Effect dispatched.
    EffectDispatched {
        /// Effect id.
        effect_id: EffectRecordId,
    },
    /// Effect settled.
    EffectSettled {
        /// Effect id.
        effect_id: EffectRecordId,
        /// Certainty.
        certainty: EffectCertainty,
    },
    /// Quiescent checkpoint persisted.
    Checkpointed {
        /// Checkpoint id.
        checkpoint_id: CheckpointId,
    },
    /// Validation recorded.
    Validated {
        /// Validation result id.
        validation_id: ValidationResultId,
    },
    /// Final work result recorded.
    Completed,
    /// Run paused.
    Paused,
    /// Run cancelled.
    Cancelled,
    /// Run abandoned.
    Abandoned,
    /// Lease acquired.
    LeaseAcquired {
        /// Lease id.
        lease_id: RootLeaseId,
    },
    /// Lease released.
    LeaseReleased {
        /// Lease id.
        lease_id: RootLeaseId,
    },
    /// Lease takeover recorded.
    LeaseTakeover {
        /// Lease id.
        lease_id: RootLeaseId,
    },
    /// Resume revalidation succeeded.
    Resumed,
}

impl RunEventKind {
    /// Stable snake_case tag for canonical event-chain hashing.
    #[must_use]
    pub const fn canonical_tag(&self) -> &'static str {
        match self {
            Self::RunCreated => "run_created",
            Self::PairFrozen { .. } => "pair_frozen",
            Self::Approved { .. } => "approved",
            Self::GrantIssued { .. } => "grant_issued",
            Self::GrantRevoked { .. } => "grant_revoked",
            Self::Denied { .. } => "denied",
            Self::EffectPrepared { .. } => "effect_prepared",
            Self::EffectClaimed { .. } => "effect_claimed",
            Self::EffectDispatched { .. } => "effect_dispatched",
            Self::EffectSettled { .. } => "effect_settled",
            Self::Checkpointed { .. } => "checkpointed",
            Self::Validated { .. } => "validated",
            Self::Completed => "completed",
            Self::Paused => "paused",
            Self::Cancelled => "cancelled",
            Self::Abandoned => "abandoned",
            Self::LeaseAcquired { .. } => "lease_acquired",
            Self::LeaseReleased { .. } => "lease_released",
            Self::LeaseTakeover { .. } => "lease_takeover",
            Self::Resumed => "resumed",
        }
    }
}

/// Append-only ordered Run Event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunEvent {
    id: RunEventId,
    run_id: HarnessRunId,
    sequence: u64,
    kind: RunEventKind,
    recorded_at_unix_ms: i64,
}

impl RunEvent {
    /// Creates an event with the given sequence.
    #[must_use]
    pub fn new(
        run_id: HarnessRunId,
        sequence: u64,
        kind: RunEventKind,
        recorded_at_unix_ms: i64,
    ) -> Self {
        Self {
            id: RunEventId::generate(),
            run_id,
            sequence,
            kind,
            recorded_at_unix_ms,
        }
    }

    /// Reconstructs a persisted event.
    pub fn from_stored_parts(
        id: &str,
        run_id: HarnessRunId,
        sequence: u64,
        kind: RunEventKind,
        recorded_at_unix_ms: i64,
    ) -> Result<Self, InvalidRunId> {
        Ok(Self {
            id: RunEventId::parse(id)?,
            run_id,
            sequence,
            kind,
            recorded_at_unix_ms,
        })
    }

    /// Returns the event id.
    #[must_use]
    pub const fn id(&self) -> RunEventId {
        self.id
    }

    /// Returns the run id.
    #[must_use]
    pub const fn run_id(&self) -> HarnessRunId {
        self.run_id
    }

    /// Returns the per-run sequence number.
    #[must_use]
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    /// Returns the event kind.
    #[must_use]
    pub const fn kind(&self) -> &RunEventKind {
        &self.kind
    }

    /// Returns the recorded time.
    #[must_use]
    pub const fn recorded_at_unix_ms(&self) -> i64 {
        self.recorded_at_unix_ms
    }
}

/// Derives lifecycle from ordered events. Unknown certainty blocks the whole Run.
#[must_use]
pub fn derive_lifecycle(events: &[RunEvent], effects: &[EffectRecord]) -> HarnessRunLifecycle {
    if effects.iter().any(|effect| {
        matches!(effect.certainty(), Some(EffectCertainty::UnknownOrPartial))
            || matches!(
                effect.phase(),
                EffectJournalPhase::Claimed | EffectJournalPhase::Dispatched
            ) && effect.certainty().is_none()
    }) {
        // Unsettled claimed/dispatched after restart is treated as unknown.
        if effects.iter().any(|effect| {
            matches!(effect.certainty(), Some(EffectCertainty::UnknownOrPartial))
                || (matches!(
                    effect.phase(),
                    EffectJournalPhase::Claimed | EffectJournalPhase::Dispatched
                ) && effect.settled_at_unix_ms().is_none())
        }) {
            // Prefer explicit terminal events if present after abandon.
            if events
                .iter()
                .any(|event| matches!(event.kind(), RunEventKind::Abandoned))
            {
                return HarnessRunLifecycle::Abandoned;
            }
            if events
                .iter()
                .any(|event| matches!(event.kind(), RunEventKind::Cancelled))
            {
                return HarnessRunLifecycle::Cancelled;
            }
            return HarnessRunLifecycle::BlockedReconciliationRequired;
        }
    }

    let mut lifecycle = HarnessRunLifecycle::Created;
    for event in events {
        lifecycle = match event.kind() {
            RunEventKind::RunCreated => HarnessRunLifecycle::Created,
            RunEventKind::PairFrozen { .. } => HarnessRunLifecycle::AwaitingApproval,
            RunEventKind::Approved { .. } => HarnessRunLifecycle::Approved,
            RunEventKind::EffectPrepared { .. }
            | RunEventKind::EffectClaimed { .. }
            | RunEventKind::EffectDispatched { .. }
            | RunEventKind::GrantIssued { .. } => {
                if lifecycle == HarnessRunLifecycle::BlockedReconciliationRequired {
                    lifecycle
                } else if matches!(
                    lifecycle,
                    HarnessRunLifecycle::Approved
                        | HarnessRunLifecycle::Executing
                        | HarnessRunLifecycle::AwaitingApproval
                        | HarnessRunLifecycle::Created
                ) {
                    HarnessRunLifecycle::Executing
                } else {
                    lifecycle
                }
            }
            RunEventKind::EffectSettled { certainty, .. } => {
                if certainty.blocks_run() {
                    HarnessRunLifecycle::BlockedReconciliationRequired
                } else if lifecycle == HarnessRunLifecycle::BlockedReconciliationRequired {
                    // Stay blocked until no unknown remains; recomputed above.
                    HarnessRunLifecycle::Executing
                } else {
                    HarnessRunLifecycle::Executing
                }
            }
            RunEventKind::Checkpointed { .. } => HarnessRunLifecycle::Checkpointed,
            RunEventKind::Validated { .. } => HarnessRunLifecycle::Validated,
            RunEventKind::Completed => HarnessRunLifecycle::Completed,
            RunEventKind::Paused => HarnessRunLifecycle::Paused,
            RunEventKind::Cancelled => HarnessRunLifecycle::Cancelled,
            RunEventKind::Abandoned => HarnessRunLifecycle::Abandoned,
            RunEventKind::Denied { .. }
            | RunEventKind::GrantRevoked { .. }
            | RunEventKind::LeaseAcquired { .. }
            | RunEventKind::LeaseReleased { .. }
            | RunEventKind::LeaseTakeover { .. }
            | RunEventKind::Resumed => lifecycle,
        };
    }

    // Re-check unknown after event fold.
    if effects
        .iter()
        .any(|effect| matches!(effect.certainty(), Some(EffectCertainty::UnknownOrPartial)))
        && !lifecycle.is_terminal()
    {
        return HarnessRunLifecycle::BlockedReconciliationRequired;
    }
    lifecycle
}

/// Returns whether a checkpoint may be taken (no unsettled claimed/dispatched effects).
#[must_use]
pub fn is_quiescent_for_checkpoint(effects: &[EffectRecord]) -> bool {
    !effects.iter().any(|effect| {
        matches!(
            effect.phase(),
            EffectJournalPhase::Claimed | EffectJournalPhase::Dispatched
        )
    })
}

/// Context disclosure policy frozen into the pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisclosurePolicy {
    /// Policy identity.
    policy_id: String,
    /// Exact allowed disclosure description.
    allowed_disclosure: String,
}

impl DisclosurePolicy {
    /// Creates a disclosure policy.
    pub fn new(policy_id: impl Into<String>, allowed_disclosure: impl Into<String>) -> Self {
        Self {
            policy_id: policy_id.into(),
            allowed_disclosure: allowed_disclosure.into(),
        }
    }

    /// Returns the policy id.
    #[must_use]
    pub fn policy_id(&self) -> &str {
        &self.policy_id
    }

    /// Returns the allowed disclosure description.
    #[must_use]
    pub fn allowed_disclosure(&self) -> &str {
        &self.allowed_disclosure
    }
}

/// Capability envelope requested by a frozen plan (not a grant).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityEnvelope {
    /// Ordered requested capability types.
    requested: Vec<CapabilityType>,
    /// Human-readable envelope summary.
    summary: String,
}

impl CapabilityEnvelope {
    /// Creates an envelope.
    pub fn new(requested: Vec<CapabilityType>, summary: impl Into<String>) -> Self {
        Self {
            requested,
            summary: summary.into(),
        }
    }

    /// Returns requested capabilities.
    #[must_use]
    pub fn requested(&self) -> &[CapabilityType] {
        &self.requested
    }

    /// Returns the summary.
    #[must_use]
    pub fn summary(&self) -> &str {
        &self.summary
    }

    /// Canonical bytes for hashing.
    #[must_use]
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut buffer = Vec::new();
        append_canonical_field(&mut buffer, "summary", self.summary.as_bytes());
        for capability in &self.requested {
            append_canonical_field(&mut buffer, "capability", capability.as_str().as_bytes());
        }
        buffer
    }
}

/// Exact content-addressed Context Manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextManifest {
    id: ContextManifestId,
    content_hash: String,
    request_semantic_hash: String,
    disclosed_byte_count: u64,
    summary: String,
}

impl ContextManifest {
    /// Creates a manifest from exact disclosed content and request semantics.
    pub fn new(
        disclosed_content: &str,
        request_semantics: &str,
        summary: impl Into<String>,
    ) -> Result<Self, RunContentError> {
        validate_run_content_bytes(disclosed_content)?;
        let content_hash = hash_source_bytes(disclosed_content.as_bytes());
        let request_semantic_hash = hash_canonical_fields(&[
            ("content_hash", content_hash.as_bytes()),
            ("request_semantics", request_semantics.as_bytes()),
        ]);
        Ok(Self {
            id: ContextManifestId::generate(),
            content_hash,
            request_semantic_hash,
            disclosed_byte_count: disclosed_content.len() as u64,
            summary: summary.into(),
        })
    }

    /// Reconstructs a persisted manifest.
    pub fn from_stored_parts(
        id: &str,
        content_hash: impl Into<String>,
        request_semantic_hash: impl Into<String>,
        disclosed_byte_count: u64,
        summary: impl Into<String>,
    ) -> Result<Self, InvalidRunId> {
        Ok(Self {
            id: ContextManifestId::parse(id)?,
            content_hash: content_hash.into(),
            request_semantic_hash: request_semantic_hash.into(),
            disclosed_byte_count,
            summary: summary.into(),
        })
    }

    /// Returns the manifest id.
    #[must_use]
    pub const fn id(&self) -> ContextManifestId {
        self.id
    }

    /// Returns the content hash.
    #[must_use]
    pub fn content_hash(&self) -> &str {
        &self.content_hash
    }

    /// Returns the request semantic hash.
    #[must_use]
    pub fn request_semantic_hash(&self) -> &str {
        &self.request_semantic_hash
    }

    /// Returns disclosed byte count.
    #[must_use]
    pub const fn disclosed_byte_count(&self) -> u64 {
        self.disclosed_byte_count
    }

    /// Returns the summary.
    #[must_use]
    pub fn summary(&self) -> &str {
        &self.summary
    }
}

/// Dedicated immutable replacement-content Run input (not an Artifact).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplacementContentInput {
    id: ReplacementContentId,
    relative_target: String,
    preimage_hash: String,
    postimage_hash: String,
    expected_diff_hash: String,
    postimage_utf8: String,
    provider_request_id: Option<String>,
    provider_response_id: Option<String>,
    created_at_unix_ms: i64,
}

impl ReplacementContentInput {
    /// Creates replacement content after bootstrap validation.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        relative_target: impl Into<String>,
        preimage: &str,
        postimage: impl Into<String>,
        provider_request_id: Option<String>,
        provider_response_id: Option<String>,
        created_at_unix_ms: i64,
    ) -> Result<Self, BootstrapProposalError> {
        let relative_target = relative_target.into();
        let postimage = postimage.into();
        validate_bootstrap_proposal(preimage, &postimage, &relative_target)?;
        let preimage_hash = hash_source_bytes(preimage.as_bytes());
        let postimage_hash = hash_source_bytes(postimage.as_bytes());
        let expected_diff_hash = hash_expected_diff(preimage, &postimage);
        Ok(Self {
            id: ReplacementContentId::generate(),
            relative_target,
            preimage_hash,
            postimage_hash,
            expected_diff_hash,
            postimage_utf8: postimage,
            provider_request_id,
            provider_response_id,
            created_at_unix_ms,
        })
    }

    /// Reconstructs persisted replacement content.
    #[allow(clippy::too_many_arguments)]
    pub fn from_stored_parts(
        id: &str,
        relative_target: impl Into<String>,
        preimage_hash: impl Into<String>,
        postimage_hash: impl Into<String>,
        expected_diff_hash: impl Into<String>,
        postimage_utf8: impl Into<String>,
        provider_request_id: Option<String>,
        provider_response_id: Option<String>,
        created_at_unix_ms: i64,
    ) -> Result<Self, InvalidRunId> {
        Ok(Self {
            id: ReplacementContentId::parse(id)?,
            relative_target: relative_target.into(),
            preimage_hash: preimage_hash.into(),
            postimage_hash: postimage_hash.into(),
            expected_diff_hash: expected_diff_hash.into(),
            postimage_utf8: postimage_utf8.into(),
            provider_request_id,
            provider_response_id,
            created_at_unix_ms,
        })
    }

    /// Returns the id.
    #[must_use]
    pub const fn id(&self) -> ReplacementContentId {
        self.id
    }

    /// Returns the relative target.
    #[must_use]
    pub fn relative_target(&self) -> &str {
        &self.relative_target
    }

    /// Returns the preimage hash.
    #[must_use]
    pub fn preimage_hash(&self) -> &str {
        &self.preimage_hash
    }

    /// Returns the postimage hash.
    #[must_use]
    pub fn postimage_hash(&self) -> &str {
        &self.postimage_hash
    }

    /// Returns the expected diff hash.
    #[must_use]
    pub fn expected_diff_hash(&self) -> &str {
        &self.expected_diff_hash
    }

    /// Returns the exact postimage UTF-8.
    #[must_use]
    pub fn postimage_utf8(&self) -> &str {
        &self.postimage_utf8
    }

    /// Returns provider request provenance when present.
    #[must_use]
    pub fn provider_request_id(&self) -> Option<&str> {
        self.provider_request_id.as_deref()
    }

    /// Returns provider response provenance when present.
    #[must_use]
    pub fn provider_response_id(&self) -> Option<&str> {
        self.provider_response_id.as_deref()
    }

    /// Returns creation time.
    #[must_use]
    pub const fn created_at_unix_ms(&self) -> i64 {
        self.created_at_unix_ms
    }
}

/// Graph node in the frozen Run Graph Version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphNode {
    /// Stable node kind / registered operation identity.
    kind: String,
    /// Responsibility label.
    responsibility: String,
    /// Optional model assignment label.
    model_assignment: Option<String>,
    /// Whether this node is protected validation.
    protected_validation: bool,
}

impl GraphNode {
    /// Creates a graph node.
    pub fn new(
        kind: impl Into<String>,
        responsibility: impl Into<String>,
        model_assignment: Option<String>,
        protected_validation: bool,
    ) -> Self {
        Self {
            kind: kind.into(),
            responsibility: responsibility.into(),
            model_assignment,
            protected_validation,
        }
    }

    /// Returns the node kind.
    #[must_use]
    pub fn kind(&self) -> &str {
        &self.kind
    }

    /// Returns the responsibility.
    #[must_use]
    pub fn responsibility(&self) -> &str {
        &self.responsibility
    }

    /// Returns the model assignment.
    #[must_use]
    pub fn model_assignment(&self) -> Option<&str> {
        self.model_assignment.as_deref()
    }

    /// Returns whether this is protected validation.
    #[must_use]
    pub const fn is_protected_validation(&self) -> bool {
        self.protected_validation
    }
}

/// Directed edge between graph nodes by kind identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphEdge {
    from_kind: String,
    to_kind: String,
}

impl GraphEdge {
    /// Creates an edge.
    pub fn new(from_kind: impl Into<String>, to_kind: impl Into<String>) -> Self {
        Self {
            from_kind: from_kind.into(),
            to_kind: to_kind.into(),
        }
    }

    /// Returns the source kind.
    #[must_use]
    pub fn from_kind(&self) -> &str {
        &self.from_kind
    }

    /// Returns the destination kind.
    #[must_use]
    pub fn to_kind(&self) -> &str {
        &self.to_kind
    }
}

/// Immutable Run Graph Version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunGraphVersion {
    id: RunGraphVersionId,
    nodes: Vec<GraphNode>,
    edges: Vec<GraphEdge>,
    retry_rule: String,
    validation_rule: String,
    content_hash: String,
}

impl RunGraphVersion {
    /// Compiles the fixed first linear graph.
    pub fn compile_fixed_first_graph() -> Self {
        let nodes = vec![
            GraphNode::new(NODE_REPLACE_EXISTING_FILE_V1, "builder", None, false),
            GraphNode::new(NODE_VERIFY_APPROVED_POSTIMAGE_V1, "reviewer", None, true),
        ];
        let edges = vec![GraphEdge::new(
            NODE_REPLACE_EXISTING_FILE_V1,
            NODE_VERIFY_APPROVED_POSTIMAGE_V1,
        )];
        let content_hash = hash_graph_content(
            &nodes,
            &edges,
            RETRY_RULE_NO_AUTOMATIC,
            VALIDATION_RULE_NATIVE_POSTIMAGE_V1,
        );
        Self {
            id: RunGraphVersionId::generate(),
            nodes,
            edges,
            retry_rule: RETRY_RULE_NO_AUTOMATIC.to_owned(),
            validation_rule: VALIDATION_RULE_NATIVE_POSTIMAGE_V1.to_owned(),
            content_hash,
        }
    }

    /// Reconstructs a persisted graph version.
    pub fn from_stored_parts(
        id: &str,
        nodes: Vec<GraphNode>,
        edges: Vec<GraphEdge>,
        retry_rule: impl Into<String>,
        validation_rule: impl Into<String>,
        content_hash: impl Into<String>,
    ) -> Result<Self, InvalidRunId> {
        Ok(Self {
            id: RunGraphVersionId::parse(id)?,
            nodes,
            edges,
            retry_rule: retry_rule.into(),
            validation_rule: validation_rule.into(),
            content_hash: content_hash.into(),
        })
    }

    /// Returns the id.
    #[must_use]
    pub const fn id(&self) -> RunGraphVersionId {
        self.id
    }

    /// Returns nodes.
    #[must_use]
    pub fn nodes(&self) -> &[GraphNode] {
        &self.nodes
    }

    /// Returns edges.
    #[must_use]
    pub fn edges(&self) -> &[GraphEdge] {
        &self.edges
    }

    /// Returns the retry rule.
    #[must_use]
    pub fn retry_rule(&self) -> &str {
        &self.retry_rule
    }

    /// Returns the validation rule.
    #[must_use]
    pub fn validation_rule(&self) -> &str {
        &self.validation_rule
    }

    /// Returns the content hash.
    #[must_use]
    pub fn content_hash(&self) -> &str {
        &self.content_hash
    }
}

fn hash_graph_content(
    nodes: &[GraphNode],
    edges: &[GraphEdge],
    retry_rule: &str,
    validation_rule: &str,
) -> String {
    let mut fields: Vec<(String, Vec<u8>)> = Vec::new();
    for (index, node) in nodes.iter().enumerate() {
        let mut node_buf = Vec::new();
        append_canonical_field(&mut node_buf, "kind", node.kind().as_bytes());
        append_canonical_field(
            &mut node_buf,
            "responsibility",
            node.responsibility().as_bytes(),
        );
        if let Some(model) = node.model_assignment() {
            append_canonical_field(&mut node_buf, "model", model.as_bytes());
        }
        append_canonical_field(
            &mut node_buf,
            "protected_validation",
            if node.is_protected_validation() {
                b"1"
            } else {
                b"0"
            },
        );
        fields.push((format!("node:{index}"), node_buf));
    }
    for (index, edge) in edges.iter().enumerate() {
        let mut edge_buf = Vec::new();
        append_canonical_field(&mut edge_buf, "from", edge.from_kind().as_bytes());
        append_canonical_field(&mut edge_buf, "to", edge.to_kind().as_bytes());
        fields.push((format!("edge:{index}"), edge_buf));
    }
    fields.push(("retry_rule".to_owned(), retry_rule.as_bytes().to_vec()));
    fields.push((
        "validation_rule".to_owned(),
        validation_rule.as_bytes().to_vec(),
    ));
    let borrowed: Vec<(&str, &[u8])> = fields
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_slice()))
        .collect();
    hash_canonical_fields(&borrowed)
}

/// Schema-versioned graph-shape fingerprint (excludes run ids, timestamps, paths, instance inputs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphShapeFingerprint {
    algorithm_version: String,
    value: String,
}

impl GraphShapeFingerprint {
    /// Derives a fingerprint from a frozen graph and execution-policy revision.
    #[must_use]
    pub fn derive(graph: &RunGraphVersion, execution_policy_revision: &str) -> Self {
        let mut buffer = Vec::new();
        append_canonical_field(
            &mut buffer,
            "algorithm",
            GRAPH_SHAPE_FINGERPRINT_VERSION.as_bytes(),
        );
        append_canonical_field(&mut buffer, "graph_hash", graph.content_hash().as_bytes());
        append_canonical_field(&mut buffer, "retry_rule", graph.retry_rule().as_bytes());
        append_canonical_field(
            &mut buffer,
            "validation_rule",
            graph.validation_rule().as_bytes(),
        );
        append_canonical_field(
            &mut buffer,
            "execution_policy_revision",
            execution_policy_revision.as_bytes(),
        );
        for node in graph.nodes() {
            append_canonical_field(&mut buffer, "node_kind", node.kind().as_bytes());
            append_canonical_field(
                &mut buffer,
                "responsibility",
                node.responsibility().as_bytes(),
            );
            append_canonical_field(
                &mut buffer,
                "protected_validation",
                if node.is_protected_validation() {
                    b"1"
                } else {
                    b"0"
                },
            );
        }
        for edge in graph.edges() {
            append_canonical_field(&mut buffer, "edge_from", edge.from_kind().as_bytes());
            append_canonical_field(&mut buffer, "edge_to", edge.to_kind().as_bytes());
        }
        Self {
            algorithm_version: GRAPH_SHAPE_FINGERPRINT_VERSION.to_owned(),
            value: hash_source_bytes(&buffer),
        }
    }

    /// Reconstructs a persisted fingerprint.
    pub fn from_stored_parts(
        algorithm_version: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        Self {
            algorithm_version: algorithm_version.into(),
            value: value.into(),
        }
    }

    /// Returns the algorithm version.
    #[must_use]
    pub fn algorithm_version(&self) -> &str {
        &self.algorithm_version
    }

    /// Returns the fingerprint value.
    #[must_use]
    pub fn value(&self) -> &str {
        &self.value
    }
}

/// Owner-governed fixture cohort assignment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskCohortAssignment {
    taxonomy_version: String,
    cohort_id: String,
    assigning_authority: String,
    rationale: String,
    assigned_at_unix_ms: i64,
}

impl TaskCohortAssignment {
    /// Creates a cohort assignment (never model-assigned).
    pub fn new(
        taxonomy_version: impl Into<String>,
        cohort_id: impl Into<String>,
        assigning_authority: impl Into<String>,
        rationale: impl Into<String>,
        assigned_at_unix_ms: i64,
    ) -> Self {
        Self {
            taxonomy_version: taxonomy_version.into(),
            cohort_id: cohort_id.into(),
            assigning_authority: assigning_authority.into(),
            rationale: rationale.into(),
            assigned_at_unix_ms,
        }
    }

    /// Returns the taxonomy version.
    #[must_use]
    pub fn taxonomy_version(&self) -> &str {
        &self.taxonomy_version
    }

    /// Returns the cohort id.
    #[must_use]
    pub fn cohort_id(&self) -> &str {
        &self.cohort_id
    }

    /// Returns the assigning authority.
    #[must_use]
    pub fn assigning_authority(&self) -> &str {
        &self.assigning_authority
    }

    /// Returns the rationale.
    #[must_use]
    pub fn rationale(&self) -> &str {
        &self.rationale
    }

    /// Returns assignment time.
    #[must_use]
    pub const fn assigned_at_unix_ms(&self) -> i64 {
        self.assigned_at_unix_ms
    }
}

/// Comparison-ready instrumentation measures (nullable when unavailable).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ComparisonInstrumentation {
    /// Time to first provider output in milliseconds.
    pub time_to_first_provider_output_ms: Option<u64>,
    /// Total time to structural result in milliseconds.
    pub total_time_to_structural_result_ms: Option<u64>,
    /// Provider-reported input tokens.
    pub provider_input_tokens: Option<u64>,
    /// Provider-reported output tokens.
    pub provider_output_tokens: Option<u64>,
    /// Provider-reported cached tokens.
    pub provider_cached_tokens: Option<u64>,
    /// Context bytes resent.
    pub context_bytes_resent: Option<u64>,
    /// Model turn count.
    pub model_turns: Option<u32>,
    /// Registered-operation call count.
    pub registered_operation_calls: Option<u32>,
    /// Validation time in milliseconds.
    pub validation_time_ms: Option<u64>,
    /// Retry count (always zero for the fixed first graph policy).
    pub retries: Option<u32>,
    /// Task success flag.
    pub task_success: Option<bool>,
}

/// Immutable Execution Plan Version paired one-to-one with a Run Graph Version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionPlanVersion {
    id: ExecutionPlanVersionId,
    graph_version_id: RunGraphVersionId,
    instructions: String,
    provider_profile_id: String,
    model_id: String,
    disclosure_policy: DisclosurePolicy,
    capability_envelope: CapabilityEnvelope,
    context_manifest: ContextManifest,
    replacement: ReplacementContentInput,
    preimage_filesystem_identity: String,
    execution_policy_revision: String,
    approval_hash: String,
    created_at_unix_ms: i64,
}

impl ExecutionPlanVersion {
    /// Freezes a plan/graph pair and computes the approval hash.
    #[allow(clippy::too_many_arguments)]
    pub fn freeze(
        graph: &RunGraphVersion,
        instructions: impl Into<String>,
        provider_profile_id: impl Into<String>,
        model_id: impl Into<String>,
        disclosure_policy: DisclosurePolicy,
        capability_envelope: CapabilityEnvelope,
        context_manifest: ContextManifest,
        replacement: ReplacementContentInput,
        preimage_filesystem_identity: impl Into<String>,
        execution_policy_revision: impl Into<String>,
        created_at_unix_ms: i64,
    ) -> Self {
        let id = ExecutionPlanVersionId::generate();
        let instructions = instructions.into();
        let provider_profile_id = provider_profile_id.into();
        let model_id = model_id.into();
        let preimage_filesystem_identity = preimage_filesystem_identity.into();
        let execution_policy_revision = execution_policy_revision.into();
        let envelope_bytes = capability_envelope.canonical_bytes();
        let approval_hash = hash_execution_plan_approval(
            id,
            graph,
            &instructions,
            &provider_profile_id,
            &model_id,
            &disclosure_policy,
            &envelope_bytes,
            &context_manifest,
            &replacement,
            &preimage_filesystem_identity,
            &execution_policy_revision,
            REGISTERED_OPERATION_SCHEMA_V1,
        );
        Self {
            id,
            graph_version_id: graph.id(),
            instructions,
            provider_profile_id,
            model_id,
            disclosure_policy,
            capability_envelope,
            context_manifest,
            replacement,
            preimage_filesystem_identity,
            execution_policy_revision,
            approval_hash,
            created_at_unix_ms,
        }
    }

    /// Reconstructs a persisted plan version.
    #[allow(clippy::too_many_arguments)]
    pub fn from_stored_parts(
        id: &str,
        graph_version_id: RunGraphVersionId,
        instructions: impl Into<String>,
        provider_profile_id: impl Into<String>,
        model_id: impl Into<String>,
        disclosure_policy: DisclosurePolicy,
        capability_envelope: CapabilityEnvelope,
        context_manifest: ContextManifest,
        replacement: ReplacementContentInput,
        preimage_filesystem_identity: impl Into<String>,
        execution_policy_revision: impl Into<String>,
        approval_hash: impl Into<String>,
        created_at_unix_ms: i64,
    ) -> Result<Self, InvalidRunId> {
        Ok(Self {
            id: ExecutionPlanVersionId::parse(id)?,
            graph_version_id,
            instructions: instructions.into(),
            provider_profile_id: provider_profile_id.into(),
            model_id: model_id.into(),
            disclosure_policy,
            capability_envelope,
            context_manifest,
            replacement,
            preimage_filesystem_identity: preimage_filesystem_identity.into(),
            execution_policy_revision: execution_policy_revision.into(),
            approval_hash: approval_hash.into(),
            created_at_unix_ms,
        })
    }

    /// Returns the plan version id.
    #[must_use]
    pub const fn id(&self) -> ExecutionPlanVersionId {
        self.id
    }

    /// Returns the paired graph version id.
    #[must_use]
    pub const fn graph_version_id(&self) -> RunGraphVersionId {
        self.graph_version_id
    }

    /// Returns instructions.
    #[must_use]
    pub fn instructions(&self) -> &str {
        &self.instructions
    }

    /// Returns the provider profile id.
    #[must_use]
    pub fn provider_profile_id(&self) -> &str {
        &self.provider_profile_id
    }

    /// Returns the model id.
    #[must_use]
    pub fn model_id(&self) -> &str {
        &self.model_id
    }

    /// Returns the disclosure policy.
    #[must_use]
    pub const fn disclosure_policy(&self) -> &DisclosurePolicy {
        &self.disclosure_policy
    }

    /// Returns the capability envelope.
    #[must_use]
    pub const fn capability_envelope(&self) -> &CapabilityEnvelope {
        &self.capability_envelope
    }

    /// Returns the context manifest.
    #[must_use]
    pub const fn context_manifest(&self) -> &ContextManifest {
        &self.context_manifest
    }

    /// Returns the replacement content input.
    #[must_use]
    pub const fn replacement(&self) -> &ReplacementContentInput {
        &self.replacement
    }

    /// Returns the preimage filesystem identity.
    #[must_use]
    pub fn preimage_filesystem_identity(&self) -> &str {
        &self.preimage_filesystem_identity
    }

    /// Returns the execution-policy revision.
    #[must_use]
    pub fn execution_policy_revision(&self) -> &str {
        &self.execution_policy_revision
    }

    /// Returns the approval hash covering executable meaning.
    #[must_use]
    pub fn approval_hash(&self) -> &str {
        &self.approval_hash
    }

    /// Returns creation time.
    #[must_use]
    pub const fn created_at_unix_ms(&self) -> i64 {
        self.created_at_unix_ms
    }
}

/// Approval Record bound to a frozen plan/graph pair hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalRecord {
    id: ApprovalRecordId,
    run_id: HarnessRunId,
    plan_version_id: ExecutionPlanVersionId,
    graph_version_id: RunGraphVersionId,
    approval_hash: String,
    approver: String,
    approved_at_unix_ms: i64,
}

impl ApprovalRecord {
    /// Creates an approval for an exact approval hash.
    pub fn new(
        run_id: HarnessRunId,
        plan: &ExecutionPlanVersion,
        graph: &RunGraphVersion,
        approver: impl Into<String>,
        approved_at_unix_ms: i64,
    ) -> Result<Self, ApprovalError> {
        if plan.graph_version_id() != graph.id() {
            return Err(ApprovalError::PairMismatch);
        }
        if plan.approval_hash().is_empty() || !is_canonical_sha256_hex(plan.approval_hash()) {
            return Err(ApprovalError::InvalidApprovalHash);
        }
        Ok(Self {
            id: ApprovalRecordId::generate(),
            run_id,
            plan_version_id: plan.id(),
            graph_version_id: graph.id(),
            approval_hash: plan.approval_hash().to_owned(),
            approver: approver.into(),
            approved_at_unix_ms,
        })
    }

    /// Reconstructs a persisted approval.
    #[allow(clippy::too_many_arguments)]
    pub fn from_stored_parts(
        id: &str,
        run_id: HarnessRunId,
        plan_version_id: ExecutionPlanVersionId,
        graph_version_id: RunGraphVersionId,
        approval_hash: impl Into<String>,
        approver: impl Into<String>,
        approved_at_unix_ms: i64,
    ) -> Result<Self, InvalidRunId> {
        Ok(Self {
            id: ApprovalRecordId::parse(id)?,
            run_id,
            plan_version_id,
            graph_version_id,
            approval_hash: approval_hash.into(),
            approver: approver.into(),
            approved_at_unix_ms,
        })
    }

    /// Returns whether this approval still matches a plan's current hash.
    #[must_use]
    pub fn matches_plan(&self, plan: &ExecutionPlanVersion) -> bool {
        self.plan_version_id == plan.id()
            && self.graph_version_id == plan.graph_version_id()
            && self.approval_hash == plan.approval_hash()
    }

    /// Returns the approval id.
    #[must_use]
    pub const fn id(&self) -> ApprovalRecordId {
        self.id
    }

    /// Returns the run id.
    #[must_use]
    pub const fn run_id(&self) -> HarnessRunId {
        self.run_id
    }

    /// Returns the plan version id.
    #[must_use]
    pub const fn plan_version_id(&self) -> ExecutionPlanVersionId {
        self.plan_version_id
    }

    /// Returns the graph version id.
    #[must_use]
    pub const fn graph_version_id(&self) -> RunGraphVersionId {
        self.graph_version_id
    }

    /// Returns the bound approval hash.
    #[must_use]
    pub fn approval_hash(&self) -> &str {
        &self.approval_hash
    }

    /// Returns the approver.
    #[must_use]
    pub fn approver(&self) -> &str {
        &self.approver
    }

    /// Returns approval time.
    #[must_use]
    pub const fn approved_at_unix_ms(&self) -> i64 {
        self.approved_at_unix_ms
    }
}

/// Approval construction failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalError {
    /// Plan and graph are not a pair.
    PairMismatch,
    /// Approval hash is missing or malformed.
    InvalidApprovalHash,
}

impl fmt::Display for ApprovalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PairMismatch => formatter.write_str("plan/graph pair mismatch"),
            Self::InvalidApprovalHash => formatter.write_str("approval hash is invalid"),
        }
    }
}

impl Error for ApprovalError {}

/// Node Attempt record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeAttempt {
    id: NodeAttemptId,
    run_id: HarnessRunId,
    plan_version_id: ExecutionPlanVersionId,
    graph_version_id: RunGraphVersionId,
    node_kind: String,
    grant_id: CapabilityGrantId,
    started_at_unix_ms: i64,
    finished_at_unix_ms: Option<i64>,
}

impl NodeAttempt {
    /// Creates a node attempt.
    pub fn new(
        run_id: HarnessRunId,
        plan_version_id: ExecutionPlanVersionId,
        graph_version_id: RunGraphVersionId,
        node_kind: impl Into<String>,
        grant_id: CapabilityGrantId,
        started_at_unix_ms: i64,
    ) -> Self {
        Self {
            id: NodeAttemptId::generate(),
            run_id,
            plan_version_id,
            graph_version_id,
            node_kind: node_kind.into(),
            grant_id,
            started_at_unix_ms,
            finished_at_unix_ms: None,
        }
    }

    /// Returns the attempt id.
    #[must_use]
    pub const fn id(&self) -> NodeAttemptId {
        self.id
    }

    /// Returns the run id.
    #[must_use]
    pub const fn run_id(&self) -> HarnessRunId {
        self.run_id
    }

    /// Returns the plan version id.
    #[must_use]
    pub const fn plan_version_id(&self) -> ExecutionPlanVersionId {
        self.plan_version_id
    }

    /// Returns the graph version id.
    #[must_use]
    pub const fn graph_version_id(&self) -> RunGraphVersionId {
        self.graph_version_id
    }

    /// Returns the node kind.
    #[must_use]
    pub fn node_kind(&self) -> &str {
        &self.node_kind
    }

    /// Returns the grant id.
    #[must_use]
    pub const fn grant_id(&self) -> CapabilityGrantId {
        self.grant_id
    }

    /// Returns start time.
    #[must_use]
    pub const fn started_at_unix_ms(&self) -> i64 {
        self.started_at_unix_ms
    }

    /// Returns finish time when set.
    #[must_use]
    pub const fn finished_at_unix_ms(&self) -> Option<i64> {
        self.finished_at_unix_ms
    }

    /// Marks the attempt finished.
    pub fn finish(&mut self, finished_at_unix_ms: i64) {
        self.finished_at_unix_ms = Some(finished_at_unix_ms);
    }
}

/// Effect Record with four-phase handshake and independent certainty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectRecord {
    id: EffectRecordId,
    run_id: HarnessRunId,
    attempt_id: Option<NodeAttemptId>,
    plan_version_id: Option<ExecutionPlanVersionId>,
    graph_version_id: Option<RunGraphVersionId>,
    operation_id: String,
    operation_schema_version: String,
    target_hash: String,
    grant_id: CapabilityGrantId,
    phase: EffectJournalPhase,
    claimant: Option<String>,
    operation_result: Option<EffectOperationResult>,
    certainty: Option<EffectCertainty>,
    prepared_at_unix_ms: i64,
    claimed_at_unix_ms: Option<i64>,
    dispatched_at_unix_ms: Option<i64>,
    settled_at_unix_ms: Option<i64>,
    expected_preimage_hash: Option<String>,
    expected_postimage_hash: Option<String>,
}

impl EffectRecord {
    /// Prepares an immutable effect record.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare(
        run_id: HarnessRunId,
        attempt_id: Option<NodeAttemptId>,
        plan_version_id: Option<ExecutionPlanVersionId>,
        graph_version_id: Option<RunGraphVersionId>,
        operation_id: impl Into<String>,
        operation_schema_version: impl Into<String>,
        target_hash: impl Into<String>,
        grant_id: CapabilityGrantId,
        prepared_at_unix_ms: i64,
        expected_preimage_hash: Option<String>,
        expected_postimage_hash: Option<String>,
    ) -> Self {
        Self {
            id: EffectRecordId::generate(),
            run_id,
            attempt_id,
            plan_version_id,
            graph_version_id,
            operation_id: operation_id.into(),
            operation_schema_version: operation_schema_version.into(),
            target_hash: target_hash.into(),
            grant_id,
            phase: EffectJournalPhase::Prepared,
            claimant: None,
            operation_result: None,
            certainty: None,
            prepared_at_unix_ms,
            claimed_at_unix_ms: None,
            dispatched_at_unix_ms: None,
            settled_at_unix_ms: None,
            expected_preimage_hash,
            expected_postimage_hash,
        }
    }

    /// Reconstructs a persisted effect record.
    #[allow(clippy::too_many_arguments)]
    pub fn from_stored_parts(
        id: &str,
        run_id: HarnessRunId,
        attempt_id: Option<NodeAttemptId>,
        plan_version_id: Option<ExecutionPlanVersionId>,
        graph_version_id: Option<RunGraphVersionId>,
        operation_id: impl Into<String>,
        operation_schema_version: impl Into<String>,
        target_hash: impl Into<String>,
        grant_id: CapabilityGrantId,
        phase: EffectJournalPhase,
        claimant: Option<String>,
        operation_result: Option<EffectOperationResult>,
        certainty: Option<EffectCertainty>,
        prepared_at_unix_ms: i64,
        claimed_at_unix_ms: Option<i64>,
        dispatched_at_unix_ms: Option<i64>,
        settled_at_unix_ms: Option<i64>,
        expected_preimage_hash: Option<String>,
        expected_postimage_hash: Option<String>,
    ) -> Result<Self, InvalidRunId> {
        Ok(Self {
            id: EffectRecordId::parse(id)?,
            run_id,
            attempt_id,
            plan_version_id,
            graph_version_id,
            operation_id: operation_id.into(),
            operation_schema_version: operation_schema_version.into(),
            target_hash: target_hash.into(),
            grant_id,
            phase,
            claimant,
            operation_result,
            certainty,
            prepared_at_unix_ms,
            claimed_at_unix_ms,
            dispatched_at_unix_ms,
            settled_at_unix_ms,
            expected_preimage_hash,
            expected_postimage_hash,
        })
    }

    /// Atomically claims a prepared effect for one claimant.
    pub fn claim(
        &mut self,
        claimant: impl Into<String>,
        now_unix_ms: i64,
    ) -> Result<(), EffectError> {
        if self.phase != EffectJournalPhase::Prepared {
            return Err(EffectError::NotPrepared);
        }
        if self.claimant.is_some() {
            return Err(EffectError::AlreadyClaimed);
        }
        self.claimant = Some(claimant.into());
        self.phase = EffectJournalPhase::Claimed;
        self.claimed_at_unix_ms = Some(now_unix_ms);
        Ok(())
    }

    /// Records durable dispatch for the successful claimant only.
    pub fn mark_dispatched(&mut self, claimant: &str, now_unix_ms: i64) -> Result<(), EffectError> {
        if self.phase != EffectJournalPhase::Claimed {
            return Err(EffectError::NotClaimed);
        }
        if self.claimant.as_deref() != Some(claimant) {
            return Err(EffectError::WrongClaimant);
        }
        self.phase = EffectJournalPhase::Dispatched;
        self.dispatched_at_unix_ms = Some(now_unix_ms);
        Ok(())
    }

    /// Settles the effect with independent result and certainty.
    pub fn settle(
        &mut self,
        claimant: &str,
        operation_result: EffectOperationResult,
        certainty: EffectCertainty,
        now_unix_ms: i64,
    ) -> Result<(), EffectError> {
        if self.phase != EffectJournalPhase::Dispatched {
            return Err(EffectError::NotDispatched);
        }
        if self.claimant.as_deref() != Some(claimant) {
            return Err(EffectError::WrongClaimant);
        }
        // Timeout/error alone never proves no effect.
        if matches!(
            operation_result,
            EffectOperationResult::Error | EffectOperationResult::Cancelled
        ) && matches!(certainty, EffectCertainty::ConfirmedNoEffect)
        {
            return Err(EffectError::InvalidCertainty);
        }
        self.operation_result = Some(operation_result);
        self.certainty = Some(certainty);
        self.phase = EffectJournalPhase::Settled;
        self.settled_at_unix_ms = Some(now_unix_ms);
        Ok(())
    }

    /// Applies positive-evidence reconciliation onto an unknown settled effect.
    pub fn reconcile(
        &mut self,
        probe: ReconciliationProbe,
        now_unix_ms: i64,
    ) -> Result<(), EffectError> {
        if self.phase != EffectJournalPhase::Settled {
            return Err(EffectError::NotSettled);
        }
        if !matches!(self.certainty, Some(EffectCertainty::UnknownOrPartial)) {
            return Err(EffectError::NotUnknown);
        }
        self.certainty = Some(reconcile_replacement_certainty(probe));
        self.settled_at_unix_ms = Some(now_unix_ms);
        Ok(())
    }

    /// Returns the effect id.
    #[must_use]
    pub const fn id(&self) -> EffectRecordId {
        self.id
    }

    /// Returns the run id.
    #[must_use]
    pub const fn run_id(&self) -> HarnessRunId {
        self.run_id
    }

    /// Returns the attempt id.
    #[must_use]
    pub const fn attempt_id(&self) -> Option<NodeAttemptId> {
        self.attempt_id
    }

    /// Returns the plan version id.
    #[must_use]
    pub const fn plan_version_id(&self) -> Option<ExecutionPlanVersionId> {
        self.plan_version_id
    }

    /// Returns the graph version id.
    #[must_use]
    pub const fn graph_version_id(&self) -> Option<RunGraphVersionId> {
        self.graph_version_id
    }

    /// Returns the operation id.
    #[must_use]
    pub fn operation_id(&self) -> &str {
        &self.operation_id
    }

    /// Returns the operation schema version.
    #[must_use]
    pub fn operation_schema_version(&self) -> &str {
        &self.operation_schema_version
    }

    /// Returns the target hash.
    #[must_use]
    pub fn target_hash(&self) -> &str {
        &self.target_hash
    }

    /// Returns the grant id.
    #[must_use]
    pub const fn grant_id(&self) -> CapabilityGrantId {
        self.grant_id
    }

    /// Returns the journal phase.
    #[must_use]
    pub const fn phase(&self) -> EffectJournalPhase {
        self.phase
    }

    /// Returns the claimant.
    #[must_use]
    pub fn claimant(&self) -> Option<&str> {
        self.claimant.as_deref()
    }

    /// Returns the operation result.
    #[must_use]
    pub const fn operation_result(&self) -> Option<EffectOperationResult> {
        self.operation_result
    }

    /// Returns the certainty.
    #[must_use]
    pub const fn certainty(&self) -> Option<EffectCertainty> {
        self.certainty
    }

    /// Returns prepared time.
    #[must_use]
    pub const fn prepared_at_unix_ms(&self) -> i64 {
        self.prepared_at_unix_ms
    }

    /// Returns claimed time.
    #[must_use]
    pub const fn claimed_at_unix_ms(&self) -> Option<i64> {
        self.claimed_at_unix_ms
    }

    /// Returns dispatched time.
    #[must_use]
    pub const fn dispatched_at_unix_ms(&self) -> Option<i64> {
        self.dispatched_at_unix_ms
    }

    /// Returns settled time.
    #[must_use]
    pub const fn settled_at_unix_ms(&self) -> Option<i64> {
        self.settled_at_unix_ms
    }

    /// Returns expected preimage hash.
    #[must_use]
    pub fn expected_preimage_hash(&self) -> Option<&str> {
        self.expected_preimage_hash.as_deref()
    }

    /// Returns expected postimage hash.
    #[must_use]
    pub fn expected_postimage_hash(&self) -> Option<&str> {
        self.expected_postimage_hash.as_deref()
    }
}

/// Effect lifecycle failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EffectError {
    /// Effect is not in prepared phase.
    NotPrepared,
    /// Effect already has a claimant.
    AlreadyClaimed,
    /// Effect is not claimed.
    NotClaimed,
    /// Claimant identity does not match.
    WrongClaimant,
    /// Effect is not dispatched.
    NotDispatched,
    /// Effect is not settled.
    NotSettled,
    /// Effect certainty is not unknown.
    NotUnknown,
    /// Certainty conflicts with operation result rules.
    InvalidCertainty,
}

impl fmt::Display for EffectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotPrepared => formatter.write_str("effect is not prepared"),
            Self::AlreadyClaimed => formatter.write_str("effect is already claimed"),
            Self::NotClaimed => formatter.write_str("effect is not claimed"),
            Self::WrongClaimant => formatter.write_str("effect claimant does not match"),
            Self::NotDispatched => formatter.write_str("effect is not dispatched"),
            Self::NotSettled => formatter.write_str("effect is not settled"),
            Self::NotUnknown => formatter.write_str("effect certainty is not unknown"),
            Self::InvalidCertainty => {
                formatter.write_str("timeout or error alone cannot confirm no effect")
            }
        }
    }
}

impl Error for EffectError {}

/// Checkpoint projection of append-only events at a quiescent boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checkpoint {
    id: CheckpointId,
    run_id: HarnessRunId,
    last_event_sequence: u64,
    event_chain_hash: String,
    plan_version_id: ExecutionPlanVersionId,
    graph_version_id: RunGraphVersionId,
    execution_policy_revision: String,
    expected_postimage_hash: String,
    created_at_unix_ms: i64,
}

impl Checkpoint {
    /// Creates a checkpoint projection.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        run_id: HarnessRunId,
        last_event_sequence: u64,
        event_chain_hash: impl Into<String>,
        plan_version_id: ExecutionPlanVersionId,
        graph_version_id: RunGraphVersionId,
        execution_policy_revision: impl Into<String>,
        expected_postimage_hash: impl Into<String>,
        created_at_unix_ms: i64,
    ) -> Self {
        Self {
            id: CheckpointId::generate(),
            run_id,
            last_event_sequence,
            event_chain_hash: event_chain_hash.into(),
            plan_version_id,
            graph_version_id,
            execution_policy_revision: execution_policy_revision.into(),
            expected_postimage_hash: expected_postimage_hash.into(),
            created_at_unix_ms,
        }
    }

    /// Reconstructs a persisted checkpoint.
    #[allow(clippy::too_many_arguments)]
    pub fn from_stored_parts(
        id: &str,
        run_id: HarnessRunId,
        last_event_sequence: u64,
        event_chain_hash: impl Into<String>,
        plan_version_id: ExecutionPlanVersionId,
        graph_version_id: RunGraphVersionId,
        execution_policy_revision: impl Into<String>,
        expected_postimage_hash: impl Into<String>,
        created_at_unix_ms: i64,
    ) -> Result<Self, InvalidRunId> {
        Ok(Self {
            id: CheckpointId::parse(id)?,
            run_id,
            last_event_sequence,
            event_chain_hash: event_chain_hash.into(),
            plan_version_id,
            graph_version_id,
            execution_policy_revision: execution_policy_revision.into(),
            expected_postimage_hash: expected_postimage_hash.into(),
            created_at_unix_ms,
        })
    }

    /// Returns the checkpoint id.
    #[must_use]
    pub const fn id(&self) -> CheckpointId {
        self.id
    }

    /// Returns the run id.
    #[must_use]
    pub const fn run_id(&self) -> HarnessRunId {
        self.run_id
    }

    /// Returns the last event sequence.
    #[must_use]
    pub const fn last_event_sequence(&self) -> u64 {
        self.last_event_sequence
    }

    /// Returns the event-chain hash.
    #[must_use]
    pub fn event_chain_hash(&self) -> &str {
        &self.event_chain_hash
    }

    /// Returns the plan version id.
    #[must_use]
    pub const fn plan_version_id(&self) -> ExecutionPlanVersionId {
        self.plan_version_id
    }

    /// Returns the graph version id.
    #[must_use]
    pub const fn graph_version_id(&self) -> RunGraphVersionId {
        self.graph_version_id
    }

    /// Returns the execution-policy revision.
    #[must_use]
    pub fn execution_policy_revision(&self) -> &str {
        &self.execution_policy_revision
    }

    /// Returns the expected postimage hash.
    #[must_use]
    pub fn expected_postimage_hash(&self) -> &str {
        &self.expected_postimage_hash
    }

    /// Returns creation time.
    #[must_use]
    pub const fn created_at_unix_ms(&self) -> i64 {
        self.created_at_unix_ms
    }
}

/// Hashes the executable meaning covered by plan/graph approval.
#[allow(clippy::too_many_arguments)]
fn hash_execution_plan_approval(
    plan_id: ExecutionPlanVersionId,
    graph: &RunGraphVersion,
    instructions: &str,
    provider_profile_id: &str,
    model_id: &str,
    disclosure_policy: &DisclosurePolicy,
    envelope_bytes: &[u8],
    context_manifest: &ContextManifest,
    replacement: &ReplacementContentInput,
    preimage_filesystem_identity: &str,
    execution_policy_revision: &str,
    registered_operation_schema: &str,
) -> String {
    hash_canonical_fields(&[
        ("plan_id", plan_id.to_string().as_bytes()),
        ("graph_id", graph.id().to_string().as_bytes()),
        ("graph_hash", graph.content_hash().as_bytes()),
        ("instructions", instructions.as_bytes()),
        ("provider_profile_id", provider_profile_id.as_bytes()),
        ("model_id", model_id.as_bytes()),
        (
            "disclosure_policy_id",
            disclosure_policy.policy_id().as_bytes(),
        ),
        (
            "allowed_disclosure",
            disclosure_policy.allowed_disclosure().as_bytes(),
        ),
        ("capability_envelope", envelope_bytes),
        (
            "context_manifest_hash",
            context_manifest.content_hash().as_bytes(),
        ),
        (
            "request_semantic_hash",
            context_manifest.request_semantic_hash().as_bytes(),
        ),
        ("relative_target", replacement.relative_target().as_bytes()),
        ("preimage_hash", replacement.preimage_hash().as_bytes()),
        ("postimage_hash", replacement.postimage_hash().as_bytes()),
        (
            "expected_diff_hash",
            replacement.expected_diff_hash().as_bytes(),
        ),
        (
            "preimage_filesystem_identity",
            preimage_filesystem_identity.as_bytes(),
        ),
        ("retry_rule", graph.retry_rule().as_bytes()),
        ("validation_rule", graph.validation_rule().as_bytes()),
        ("op_replace", NODE_REPLACE_EXISTING_FILE_V1.as_bytes()),
        ("op_verify", NODE_VERIFY_APPROVED_POSTIMAGE_V1.as_bytes()),
        (
            "registered_operation_schema",
            registered_operation_schema.as_bytes(),
        ),
        (
            "execution_policy_revision",
            execution_policy_revision.as_bytes(),
        ),
    ])
}

/// Hashes an ordered event chain for checkpoint integrity.
#[must_use]
pub fn hash_event_chain(events: &[RunEvent]) -> String {
    let mut buffer = Vec::new();
    append_canonical_field(
        &mut buffer,
        "encoding",
        CANONICAL_ENCODING_VERSION.as_bytes(),
    );
    for event in events {
        append_canonical_field(&mut buffer, "sequence", &event.sequence().to_be_bytes());
        append_canonical_field(&mut buffer, "id", event.id().to_string().as_bytes());
        append_canonical_field(&mut buffer, "kind", event.kind().canonical_tag().as_bytes());
    }
    hash_source_bytes(&buffer)
}

/// Protected validation result labelled native structural validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationResult {
    id: ValidationResultId,
    run_id: HarnessRunId,
    plan_version_id: ExecutionPlanVersionId,
    graph_version_id: RunGraphVersionId,
    label: String,
    approved_postimage_hash: String,
    observed_postimage_hash: String,
    native_diff_hash: String,
    passed: bool,
    validated_at_unix_ms: i64,
}

impl ValidationResult {
    /// Creates a native structural validation result.
    #[allow(clippy::too_many_arguments)]
    pub fn native_structural(
        run_id: HarnessRunId,
        plan_version_id: ExecutionPlanVersionId,
        graph_version_id: RunGraphVersionId,
        approved_postimage_hash: impl Into<String>,
        observed_postimage_hash: impl Into<String>,
        native_diff_hash: impl Into<String>,
        validated_at_unix_ms: i64,
    ) -> Self {
        let approved_postimage_hash = approved_postimage_hash.into();
        let observed_postimage_hash = observed_postimage_hash.into();
        let native_diff_hash = native_diff_hash.into();
        let passed = approved_postimage_hash == observed_postimage_hash;
        Self {
            id: ValidationResultId::generate(),
            run_id,
            plan_version_id,
            graph_version_id,
            label: NATIVE_STRUCTURAL_VALIDATION_LABEL.to_owned(),
            approved_postimage_hash,
            observed_postimage_hash,
            native_diff_hash,
            passed,
            validated_at_unix_ms,
        }
    }

    /// Reconstructs a persisted validation result.
    #[allow(clippy::too_many_arguments)]
    pub fn from_stored_parts(
        id: &str,
        run_id: HarnessRunId,
        plan_version_id: ExecutionPlanVersionId,
        graph_version_id: RunGraphVersionId,
        label: impl Into<String>,
        approved_postimage_hash: impl Into<String>,
        observed_postimage_hash: impl Into<String>,
        native_diff_hash: impl Into<String>,
        passed: bool,
        validated_at_unix_ms: i64,
    ) -> Result<Self, InvalidRunId> {
        Ok(Self {
            id: ValidationResultId::parse(id)?,
            run_id,
            plan_version_id,
            graph_version_id,
            label: label.into(),
            approved_postimage_hash: approved_postimage_hash.into(),
            observed_postimage_hash: observed_postimage_hash.into(),
            native_diff_hash: native_diff_hash.into(),
            passed,
            validated_at_unix_ms,
        })
    }

    /// Returns the validation id.
    #[must_use]
    pub const fn id(&self) -> ValidationResultId {
        self.id
    }

    /// Returns the run id.
    #[must_use]
    pub const fn run_id(&self) -> HarnessRunId {
        self.run_id
    }

    /// Returns the plan version id.
    #[must_use]
    pub const fn plan_version_id(&self) -> ExecutionPlanVersionId {
        self.plan_version_id
    }

    /// Returns the graph version id.
    #[must_use]
    pub const fn graph_version_id(&self) -> RunGraphVersionId {
        self.graph_version_id
    }

    /// Returns the validation label.
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Returns the approved postimage hash.
    #[must_use]
    pub fn approved_postimage_hash(&self) -> &str {
        &self.approved_postimage_hash
    }

    /// Returns the observed postimage hash.
    #[must_use]
    pub fn observed_postimage_hash(&self) -> &str {
        &self.observed_postimage_hash
    }

    /// Returns the native diff hash.
    #[must_use]
    pub fn native_diff_hash(&self) -> &str {
        &self.native_diff_hash
    }

    /// Returns whether validation passed.
    #[must_use]
    pub const fn passed(&self) -> bool {
        self.passed
    }

    /// Returns validation time.
    #[must_use]
    pub const fn validated_at_unix_ms(&self) -> i64 {
        self.validated_at_unix_ms
    }
}

/// Final Work Result for a completed run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinalWorkResult {
    run_id: HarnessRunId,
    plan_version_id: ExecutionPlanVersionId,
    graph_version_id: RunGraphVersionId,
    validation_label: String,
    publication_stopped: bool,
    instrumentation: ComparisonInstrumentation,
    fingerprint: GraphShapeFingerprint,
    cohort: Option<TaskCohortAssignment>,
    completed_at_unix_ms: i64,
}

impl FinalWorkResult {
    /// Creates a final work result that explicitly stops before publication.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        run_id: HarnessRunId,
        plan_version_id: ExecutionPlanVersionId,
        graph_version_id: RunGraphVersionId,
        instrumentation: ComparisonInstrumentation,
        fingerprint: GraphShapeFingerprint,
        cohort: Option<TaskCohortAssignment>,
        completed_at_unix_ms: i64,
    ) -> Self {
        Self {
            run_id,
            plan_version_id,
            graph_version_id,
            validation_label: NATIVE_STRUCTURAL_VALIDATION_LABEL.to_owned(),
            publication_stopped: true,
            instrumentation,
            fingerprint,
            cohort,
            completed_at_unix_ms,
        }
    }

    /// Reconstructs a persisted final work result.
    #[allow(clippy::too_many_arguments)]
    pub fn from_stored_parts(
        run_id: HarnessRunId,
        plan_version_id: ExecutionPlanVersionId,
        graph_version_id: RunGraphVersionId,
        validation_label: impl Into<String>,
        publication_stopped: bool,
        instrumentation: ComparisonInstrumentation,
        fingerprint: GraphShapeFingerprint,
        cohort: Option<TaskCohortAssignment>,
        completed_at_unix_ms: i64,
    ) -> Self {
        Self {
            run_id,
            plan_version_id,
            graph_version_id,
            validation_label: validation_label.into(),
            publication_stopped,
            instrumentation,
            fingerprint,
            cohort,
            completed_at_unix_ms,
        }
    }

    /// Returns the run id.
    #[must_use]
    pub const fn run_id(&self) -> HarnessRunId {
        self.run_id
    }

    /// Returns the plan version id.
    #[must_use]
    pub const fn plan_version_id(&self) -> ExecutionPlanVersionId {
        self.plan_version_id
    }

    /// Returns the graph version id.
    #[must_use]
    pub const fn graph_version_id(&self) -> RunGraphVersionId {
        self.graph_version_id
    }

    /// Returns the validation label.
    #[must_use]
    pub fn validation_label(&self) -> &str {
        &self.validation_label
    }

    /// Returns whether publication was explicitly stopped.
    #[must_use]
    pub const fn publication_stopped(&self) -> bool {
        self.publication_stopped
    }

    /// Returns instrumentation.
    #[must_use]
    pub const fn instrumentation(&self) -> &ComparisonInstrumentation {
        &self.instrumentation
    }

    /// Returns the fingerprint.
    #[must_use]
    pub const fn fingerprint(&self) -> &GraphShapeFingerprint {
        &self.fingerprint
    }

    /// Returns the cohort assignment.
    #[must_use]
    pub const fn cohort(&self) -> Option<&TaskCohortAssignment> {
        self.cohort.as_ref()
    }

    /// Returns completion time.
    #[must_use]
    pub const fn completed_at_unix_ms(&self) -> i64 {
        self.completed_at_unix_ms
    }
}

/// Exclusive durable run-root lease.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootLease {
    id: RootLeaseId,
    run_id: HarnessRunId,
    owner_process_instance: String,
    acquired_at_unix_ms: i64,
    expires_at_unix_ms: i64,
    renew_interval_ms: i64,
}

impl RootLease {
    /// Acquires a lease with Work 0022 TTL and renew interval.
    pub fn acquire(
        run_id: HarnessRunId,
        owner_process_instance: impl Into<String>,
        now_unix_ms: i64,
    ) -> Self {
        Self {
            id: RootLeaseId::generate(),
            run_id,
            owner_process_instance: owner_process_instance.into(),
            acquired_at_unix_ms: now_unix_ms,
            expires_at_unix_ms: now_unix_ms + ROOT_LEASE_TTL_MS,
            renew_interval_ms: ROOT_LEASE_RENEW_INTERVAL_MS,
        }
    }

    /// Reconstructs a persisted lease.
    #[allow(clippy::too_many_arguments)]
    pub fn from_stored_parts(
        id: &str,
        run_id: HarnessRunId,
        owner_process_instance: impl Into<String>,
        acquired_at_unix_ms: i64,
        expires_at_unix_ms: i64,
        renew_interval_ms: i64,
    ) -> Result<Self, InvalidRunId> {
        Ok(Self {
            id: RootLeaseId::parse(id)?,
            run_id,
            owner_process_instance: owner_process_instance.into(),
            acquired_at_unix_ms,
            expires_at_unix_ms,
            renew_interval_ms,
        })
    }

    /// Renews the lease for the owning process.
    pub fn renew(
        &mut self,
        owner_process_instance: &str,
        now_unix_ms: i64,
    ) -> Result<(), LeaseError> {
        if self.owner_process_instance != owner_process_instance {
            return Err(LeaseError::WrongOwner);
        }
        if now_unix_ms >= self.expires_at_unix_ms {
            return Err(LeaseError::Expired);
        }
        self.expires_at_unix_ms = now_unix_ms + ROOT_LEASE_TTL_MS;
        Ok(())
    }

    /// Returns whether the lease is expired at `now_unix_ms`.
    #[must_use]
    pub const fn is_expired(&self, now_unix_ms: i64) -> bool {
        now_unix_ms >= self.expires_at_unix_ms
    }

    /// Takeover is never authorised by expiry alone.
    pub fn authorize_takeover(
        &self,
        prior_owner_confirmed_gone: bool,
        unsettled_effects_reconciled: bool,
    ) -> Result<(), LeaseError> {
        if !prior_owner_confirmed_gone {
            return Err(LeaseError::PriorOwnerAlive);
        }
        if !unsettled_effects_reconciled {
            return Err(LeaseError::UnsettledEffects);
        }
        Ok(())
    }

    /// Returns the lease id.
    #[must_use]
    pub const fn id(&self) -> RootLeaseId {
        self.id
    }

    /// Returns the run id.
    #[must_use]
    pub const fn run_id(&self) -> HarnessRunId {
        self.run_id
    }

    /// Returns the owner process instance.
    #[must_use]
    pub fn owner_process_instance(&self) -> &str {
        &self.owner_process_instance
    }

    /// Returns acquisition time.
    #[must_use]
    pub const fn acquired_at_unix_ms(&self) -> i64 {
        self.acquired_at_unix_ms
    }

    /// Returns expiry time.
    #[must_use]
    pub const fn expires_at_unix_ms(&self) -> i64 {
        self.expires_at_unix_ms
    }

    /// Returns the renew interval.
    #[must_use]
    pub const fn renew_interval_ms(&self) -> i64 {
        self.renew_interval_ms
    }
}

/// Lease failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeaseError {
    /// Caller is not the lease owner.
    WrongOwner,
    /// Lease has expired.
    Expired,
    /// Prior owner process is not confirmed gone.
    PriorOwnerAlive,
    /// Unsettled effects remain.
    UnsettledEffects,
}

impl fmt::Display for LeaseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongOwner => formatter.write_str("lease owner does not match"),
            Self::Expired => formatter.write_str("lease has expired"),
            Self::PriorOwnerAlive => {
                formatter.write_str("lease takeover requires prior owner confirmed gone")
            }
            Self::UnsettledEffects => {
                formatter.write_str("lease takeover requires unsettled effects reconciled")
            }
        }
    }
}

impl Error for LeaseError {}

/// Denial evidence appended when authority checks fail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DenialEvidence {
    id: DenialEvidenceId,
    run_id: HarnessRunId,
    reason: String,
    grant_id: Option<CapabilityGrantId>,
    resource: Option<GrantResourceSelector>,
    recorded_at_unix_ms: i64,
}

impl DenialEvidence {
    /// Creates denial evidence.
    pub fn new(
        run_id: HarnessRunId,
        reason: impl Into<String>,
        grant_id: Option<CapabilityGrantId>,
        resource: Option<GrantResourceSelector>,
        recorded_at_unix_ms: i64,
    ) -> Self {
        Self {
            id: DenialEvidenceId::generate(),
            run_id,
            reason: reason.into(),
            grant_id,
            resource,
            recorded_at_unix_ms,
        }
    }

    /// Reconstructs persisted denial evidence.
    pub fn from_stored_parts(
        id: &str,
        run_id: HarnessRunId,
        reason: impl Into<String>,
        grant_id: Option<CapabilityGrantId>,
        resource: Option<GrantResourceSelector>,
        recorded_at_unix_ms: i64,
    ) -> Result<Self, InvalidRunId> {
        Ok(Self {
            id: DenialEvidenceId::parse(id)?,
            run_id,
            reason: reason.into(),
            grant_id,
            resource,
            recorded_at_unix_ms,
        })
    }

    /// Returns the denial id.
    #[must_use]
    pub const fn id(&self) -> DenialEvidenceId {
        self.id
    }

    /// Returns the run id.
    #[must_use]
    pub const fn run_id(&self) -> HarnessRunId {
        self.run_id
    }

    /// Returns the reason.
    #[must_use]
    pub fn reason(&self) -> &str {
        &self.reason
    }

    /// Returns the related grant id.
    #[must_use]
    pub const fn grant_id(&self) -> Option<CapabilityGrantId> {
        self.grant_id
    }

    /// Returns the resource selector.
    #[must_use]
    pub const fn resource(&self) -> Option<&GrantResourceSelector> {
        self.resource.as_ref()
    }

    /// Returns recorded time.
    #[must_use]
    pub const fn recorded_at_unix_ms(&self) -> i64 {
        self.recorded_at_unix_ms
    }
}

/// Harness Run aggregate header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarnessRun {
    id: HarnessRunId,
    run_root_display_name: String,
    created_at_unix_ms: i64,
    cohort: Option<TaskCohortAssignment>,
}

impl HarnessRun {
    /// Creates a new Harness Run.
    pub fn new(
        run_root_display_name: impl Into<String>,
        created_at_unix_ms: i64,
        cohort: Option<TaskCohortAssignment>,
    ) -> Self {
        Self {
            id: HarnessRunId::generate(),
            run_root_display_name: run_root_display_name.into(),
            created_at_unix_ms,
            cohort,
        }
    }

    /// Reconstructs a persisted run header.
    pub fn from_stored_parts(
        id: &str,
        run_root_display_name: impl Into<String>,
        created_at_unix_ms: i64,
        cohort: Option<TaskCohortAssignment>,
    ) -> Result<Self, InvalidRunId> {
        Ok(Self {
            id: HarnessRunId::parse(id)?,
            run_root_display_name: run_root_display_name.into(),
            created_at_unix_ms,
            cohort,
        })
    }

    /// Returns the run id.
    #[must_use]
    pub const fn id(&self) -> HarnessRunId {
        self.id
    }

    /// Returns the run-root display name.
    #[must_use]
    pub fn run_root_display_name(&self) -> &str {
        &self.run_root_display_name
    }

    /// Returns creation time.
    #[must_use]
    pub const fn created_at_unix_ms(&self) -> i64 {
        self.created_at_unix_ms
    }

    /// Returns the cohort assignment.
    #[must_use]
    pub const fn cohort(&self) -> Option<&TaskCohortAssignment> {
        self.cohort.as_ref()
    }
}

/// Resume revalidation inputs and outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeRevalidation {
    /// Event-chain hash must match the checkpoint.
    pub event_chain_matches: bool,
    /// Expected filesystem postimage hash must match.
    pub filesystem_matches_expected: bool,
    /// Execution-policy revision must match the frozen pair.
    pub execution_policy_matches: bool,
    /// Operation/schema versions must match.
    pub operation_versions_match: bool,
}

/// Resume policy outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResumeDecision {
    /// Resume may continue with remaining unexpired grants only.
    Continue,
    /// Material mismatch; requires new pair or abandonment.
    RequireReapprovalOrAbandon,
    /// Confirmed replacement must not be redispatched.
    SkipConfirmedReplacement {
        /// Effect that already committed.
        effect_id: EffectRecordId,
    },
    /// Expired grant must not be revived; request a distinct new grant.
    RequireFreshGrant {
        /// Expired grant id.
        expired_grant_id: CapabilityGrantId,
    },
}

/// Evaluates resume rules without reviving expired grants or redispatched writes.
#[must_use]
pub fn evaluate_resume(
    revalidation: &ResumeRevalidation,
    expired_grant_ids: &[CapabilityGrantId],
    confirmed_replacement_effect: Option<EffectRecordId>,
) -> ResumeDecision {
    if !revalidation.event_chain_matches
        || !revalidation.filesystem_matches_expected
        || !revalidation.execution_policy_matches
        || !revalidation.operation_versions_match
    {
        return ResumeDecision::RequireReapprovalOrAbandon;
    }
    if let Some(effect_id) = confirmed_replacement_effect {
        return ResumeDecision::SkipConfirmedReplacement { effect_id };
    }
    if let Some(expired_grant_id) = expired_grant_ids.first().copied() {
        return ResumeDecision::RequireFreshGrant { expired_grant_id };
    }
    ResumeDecision::Continue
}

/// Resource selector helper used when hashing grant-bound targets in tests.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(64);
    for byte in digest {
        write!(&mut hex, "{byte:02x}").expect("writing to String cannot fail");
    }
    hex
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        BOOTSTRAP_GRANT_TTL_MS, CapabilityGrant, DEFAULT_DISPATCH_BUDGET, GrantActionScope,
        GrantEvaluation, GrantEvaluationRequest, OP_LOCAL_READ_V1, PlanGraphPairBinding,
        REGISTERED_OPERATION_SCHEMA_V1, evaluate_grant,
    };

    fn sample_preimage() -> String {
        "<!doctype html><html><body><h1>Ready</h1><p>ok</p></body></html>".to_owned()
    }

    fn sample_postimage() -> String {
        "<!doctype html><html><body><h1>Ready for review</h1><p>ok</p></body></html>".to_owned()
    }

    fn freeze_pair(model_id: &str) -> (ExecutionPlanVersion, RunGraphVersion) {
        let graph = RunGraphVersion::compile_fixed_first_graph();
        let preimage = sample_preimage();
        let postimage = sample_postimage();
        let replacement = ReplacementContentInput::new(
            CONTROLLED_RELATIVE_TARGET,
            &preimage,
            postimage,
            Some("req-1".to_owned()),
            Some("resp-1".to_owned()),
            10,
        )
        .unwrap();
        let manifest = ContextManifest::new(&preimage, "change heading", "fixture").unwrap();
        let plan = ExecutionPlanVersion::freeze(
            &graph,
            "replace heading",
            "profile-1",
            model_id,
            DisclosurePolicy::new("disclose-v1", "index.html only"),
            CapabilityEnvelope::new(
                vec![
                    CapabilityType::CreateOrReplace,
                    CapabilityType::NativeInspection,
                ],
                "exact replace + inspect",
            ),
            manifest,
            replacement,
            "fs-id-1",
            EXECUTION_POLICY_REVISION_V1,
            10,
        );
        (plan, graph)
    }

    #[test]
    fn canonical_hash_is_stable_and_order_defined() {
        let a = hash_canonical_fields(&[("a", b"1"), ("b", b"2")]);
        let b = hash_canonical_fields(&[("a", b"1"), ("b", b"2")]);
        let c = hash_canonical_fields(&[("b", b"2"), ("a", b"1")]);
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert!(is_canonical_sha256_hex(&a));
    }

    #[test]
    fn changing_hash_covered_field_invalidates_prior_approval_and_pair_grants() {
        let (plan_a, graph_a) = freeze_pair("model-a");
        let (plan_b, _graph_b) = freeze_pair("model-b");
        assert_ne!(plan_a.approval_hash(), plan_b.approval_hash());
        let approval =
            ApprovalRecord::new(HarnessRunId::generate(), &plan_a, &graph_a, "owner", 20).unwrap();
        assert!(approval.matches_plan(&plan_a));
        assert!(!approval.matches_plan(&plan_b));

        let pair = PlanGraphPairBinding {
            plan_version_id: plan_a.id(),
            graph_version_id: graph_a.id(),
        };
        let grant = CapabilityGrant::issue(
            approval.run_id(),
            CapabilityType::CreateOrReplace,
            GrantResourceSelector::ReplacementTarget {
                relative_target: CONTROLLED_RELATIVE_TARGET.to_owned(),
                expected_preimage_hash: plan_a.replacement().preimage_hash().to_owned(),
                expected_postimage_hash: plan_a.replacement().postimage_hash().to_owned(),
            },
            GrantActionScope::Node(NODE_REPLACE_EXISTING_FILE_V1.to_owned()),
            Some(pair),
            Some(approval.id()),
            "owner",
            20,
            20 + crate::POST_APPROVAL_GRANT_TTL_MS,
            DEFAULT_DISPATCH_BUDGET,
        )
        .unwrap();
        let resource = grant.resource().clone();
        let scope = grant.action_scope().clone();
        let ok = GrantEvaluationRequest {
            run_id: grant.run_id(),
            capability: CapabilityType::CreateOrReplace,
            operation_id: crate::OP_CREATE_OR_REPLACE_V1,
            resource: &resource,
            action_scope: &scope,
            pair: Some(pair),
            now_unix_ms: 21,
        };
        assert_eq!(evaluate_grant(&grant, &ok), GrantEvaluation::Allow);
        let mut other_pair = ok.clone();
        other_pair.pair = Some(PlanGraphPairBinding {
            plan_version_id: plan_b.id(),
            graph_version_id: plan_b.graph_version_id(),
        });
        assert!(matches!(
            evaluate_grant(&grant, &other_pair),
            GrantEvaluation::Deny(_)
        ));
    }

    #[test]
    fn changing_registered_operation_schema_version_requires_reapproval() {
        let (plan, graph) = freeze_pair("model-a");
        let envelope_bytes = plan.capability_envelope().canonical_bytes();
        let with_current = hash_execution_plan_approval(
            plan.id(),
            &graph,
            plan.instructions(),
            plan.provider_profile_id(),
            plan.model_id(),
            plan.disclosure_policy(),
            &envelope_bytes,
            plan.context_manifest(),
            plan.replacement(),
            plan.preimage_filesystem_identity(),
            plan.execution_policy_revision(),
            REGISTERED_OPERATION_SCHEMA_V1,
        );
        let with_other = hash_execution_plan_approval(
            plan.id(),
            &graph,
            plan.instructions(),
            plan.provider_profile_id(),
            plan.model_id(),
            plan.disclosure_policy(),
            &envelope_bytes,
            plan.context_manifest(),
            plan.replacement(),
            plan.preimage_filesystem_identity(),
            plan.execution_policy_revision(),
            "tule-registered-op-schema-v2",
        );
        assert_eq!(plan.approval_hash(), with_current);
        assert_ne!(plan.approval_hash(), with_other);

        let approval =
            ApprovalRecord::new(HarnessRunId::generate(), &plan, &graph, "owner", 20).unwrap();
        assert!(approval.matches_plan(&plan));
        assert_ne!(approval.approval_hash(), with_other);

        let pair = PlanGraphPairBinding {
            plan_version_id: plan.id(),
            graph_version_id: graph.id(),
        };
        let grant = CapabilityGrant::issue(
            approval.run_id(),
            CapabilityType::CreateOrReplace,
            GrantResourceSelector::ReplacementTarget {
                relative_target: CONTROLLED_RELATIVE_TARGET.to_owned(),
                expected_preimage_hash: plan.replacement().preimage_hash().to_owned(),
                expected_postimage_hash: plan.replacement().postimage_hash().to_owned(),
            },
            GrantActionScope::Node(NODE_REPLACE_EXISTING_FILE_V1.to_owned()),
            Some(pair),
            Some(approval.id()),
            "owner",
            20,
            20 + crate::POST_APPROVAL_GRANT_TTL_MS,
            DEFAULT_DISPATCH_BUDGET,
        )
        .unwrap();
        let resource = grant.resource().clone();
        let scope = grant.action_scope().clone();
        let ok = GrantEvaluationRequest {
            run_id: grant.run_id(),
            capability: CapabilityType::CreateOrReplace,
            operation_id: crate::OP_CREATE_OR_REPLACE_V1,
            resource: &resource,
            action_scope: &scope,
            pair: Some(pair),
            now_unix_ms: 21,
        };
        assert_eq!(evaluate_grant(&grant, &ok), GrantEvaluation::Allow);
        // Pair-bound grants stay tied to the approved plan/graph ids; a schema-driven
        // re-freeze yields a new plan id and therefore fails the pair binding gate.
        let mut other_pair = ok.clone();
        other_pair.pair = Some(PlanGraphPairBinding {
            plan_version_id: ExecutionPlanVersionId::generate(),
            graph_version_id: RunGraphVersionId::generate(),
        });
        assert!(matches!(
            evaluate_grant(&grant, &other_pair),
            GrantEvaluation::Deny(_)
        ));
    }

    #[test]
    fn bootstrap_proposal_rejects_broader_and_unknown_payloads() {
        let preimage = sample_preimage();
        let postimage = sample_postimage();
        assert!(validate_bootstrap_proposal(&preimage, &postimage, "index.html").is_ok());
        assert!(matches!(
            validate_bootstrap_proposal(&preimage, &postimage, "other.html"),
            Err(BootstrapProposalError::WrongTarget)
        ));
        let broader = preimage.replace("ok", "changed");
        assert!(matches!(
            validate_bootstrap_proposal(&preimage, &broader, "index.html"),
            Err(BootstrapProposalError::BroaderDiff)
        ));
        assert!(matches!(
            validate_bootstrap_proposal(&preimage, "```html\n", "index.html"),
            Err(BootstrapProposalError::FramingOrNonPostimage)
        ));
        assert!(matches!(
            reject_unknown_proposal_fields(&["postimage"], &["postimage", "patch"]),
            Err(BootstrapProposalError::UnknownFields)
        ));
        let with_nul = format!("{postimage}\0");
        assert!(matches!(
            validate_bootstrap_proposal(&preimage, &with_nul, "index.html"),
            Err(BootstrapProposalError::Content(
                RunContentError::ContainsNul
            ))
        ));
    }

    #[test]
    fn fixed_graph_is_linear_task_then_validation_without_retry() {
        let graph = RunGraphVersion::compile_fixed_first_graph();
        assert_eq!(graph.nodes().len(), 2);
        assert_eq!(graph.nodes()[0].kind(), NODE_REPLACE_EXISTING_FILE_V1);
        assert!(graph.nodes()[1].is_protected_validation());
        assert_eq!(graph.retry_rule(), RETRY_RULE_NO_AUTOMATIC);
        assert_eq!(graph.edges().len(), 1);
    }

    #[test]
    fn single_claim_effects_and_unknown_blocks_run() {
        let run_id = HarnessRunId::generate();
        let grant_id = CapabilityGrantId::generate();
        let mut effect = EffectRecord::prepare(
            run_id,
            None,
            None,
            None,
            crate::OP_CREATE_OR_REPLACE_V1,
            crate::REGISTERED_OPERATION_SCHEMA_V1,
            "target",
            grant_id,
            1,
            Some("pre".to_owned()),
            Some("post".to_owned()),
        );
        effect.claim("broker-a", 2).unwrap();
        assert!(matches!(
            effect.claim("broker-b", 3),
            Err(EffectError::AlreadyClaimed) | Err(EffectError::NotPrepared)
        ));
        effect.mark_dispatched("broker-a", 4).unwrap();
        assert!(matches!(
            effect.settle(
                "broker-a",
                EffectOperationResult::Error,
                EffectCertainty::ConfirmedNoEffect,
                5
            ),
            Err(EffectError::InvalidCertainty)
        ));
        effect
            .settle(
                "broker-a",
                EffectOperationResult::Error,
                EffectCertainty::UnknownOrPartial,
                5,
            )
            .unwrap();
        let events = vec![
            RunEvent::new(run_id, 1, RunEventKind::RunCreated, 1),
            RunEvent::new(
                run_id,
                2,
                RunEventKind::EffectSettled {
                    effect_id: effect.id(),
                    certainty: EffectCertainty::UnknownOrPartial,
                },
                5,
            ),
        ];
        assert_eq!(
            derive_lifecycle(&events, &[effect.clone()]),
            HarnessRunLifecycle::BlockedReconciliationRequired
        );
        assert_eq!(
            reconcile_replacement_certainty(ReconciliationProbe::MatchesPreimage),
            EffectCertainty::ConfirmedNoEffect
        );
        assert_eq!(
            reconcile_replacement_certainty(ReconciliationProbe::MatchesPostimage),
            EffectCertainty::ConfirmedCommitted
        );
        assert_eq!(
            reconcile_replacement_certainty(ReconciliationProbe::MatchesNeither),
            EffectCertainty::UnknownOrPartial
        );
    }

    #[test]
    fn quiescent_checkpoint_rejects_unsettled_claimed_or_dispatched() {
        let run_id = HarnessRunId::generate();
        let grant_id = CapabilityGrantId::generate();
        let mut effect = EffectRecord::prepare(
            run_id,
            None,
            None,
            None,
            crate::OP_CREATE_OR_REPLACE_V1,
            crate::REGISTERED_OPERATION_SCHEMA_V1,
            "target",
            grant_id,
            1,
            None,
            None,
        );
        assert!(is_quiescent_for_checkpoint(&[effect.clone()]));
        effect.claim("broker", 2).unwrap();
        assert!(!is_quiescent_for_checkpoint(&[effect.clone()]));
        effect.mark_dispatched("broker", 3).unwrap();
        assert!(!is_quiescent_for_checkpoint(&[effect.clone()]));
        effect
            .settle(
                "broker",
                EffectOperationResult::Success,
                EffectCertainty::ConfirmedCommitted,
                4,
            )
            .unwrap();
        assert!(is_quiescent_for_checkpoint(&[effect]));
    }

    #[test]
    fn resume_never_revives_expired_grant_or_redispatches_confirmed_replacement() {
        let grant_id = CapabilityGrantId::generate();
        let effect_id = EffectRecordId::generate();
        let revalidation = ResumeRevalidation {
            event_chain_matches: true,
            filesystem_matches_expected: true,
            execution_policy_matches: true,
            operation_versions_match: true,
        };
        assert!(matches!(
            evaluate_resume(&revalidation, &[], Some(effect_id)),
            ResumeDecision::SkipConfirmedReplacement { .. }
        ));
        assert!(matches!(
            evaluate_resume(&revalidation, &[grant_id], None),
            ResumeDecision::RequireFreshGrant { .. }
        ));
        let bad = ResumeRevalidation {
            filesystem_matches_expected: false,
            ..revalidation.clone()
        };
        assert!(matches!(
            evaluate_resume(&bad, &[], None),
            ResumeDecision::RequireReapprovalOrAbandon
        ));
    }

    #[test]
    fn lease_expiry_alone_never_authorises_takeover() {
        let clock = FakeClock::new(1_000);
        let mut lease = RootLease::acquire(HarnessRunId::generate(), "proc-1", clock.unix_ms());
        assert_eq!(lease.renew_interval_ms(), ROOT_LEASE_RENEW_INTERVAL_MS);
        clock.advance(ROOT_LEASE_RENEW_INTERVAL_MS);
        lease.renew("proc-1", clock.unix_ms()).unwrap();
        clock.advance(ROOT_LEASE_TTL_MS);
        assert!(lease.is_expired(clock.unix_ms()));
        assert!(matches!(
            lease.authorize_takeover(false, true),
            Err(LeaseError::PriorOwnerAlive)
        ));
        assert!(matches!(
            lease.authorize_takeover(true, false),
            Err(LeaseError::UnsettledEffects)
        ));
        assert!(lease.authorize_takeover(true, true).is_ok());
    }

    #[test]
    fn fingerprint_and_cohort_are_recorded() {
        let graph = RunGraphVersion::compile_fixed_first_graph();
        let fingerprint = GraphShapeFingerprint::derive(&graph, EXECUTION_POLICY_REVISION_V1);
        assert_eq!(
            fingerprint.algorithm_version(),
            GRAPH_SHAPE_FINGERPRINT_VERSION
        );
        assert!(is_canonical_sha256_hex(fingerprint.value()));
        let cohort = TaskCohortAssignment::new(
            "cohort-tax-v1",
            "static-heading-fixture",
            "owner-work-0022",
            "controlled acceptance fixture",
            42,
        );
        let result = FinalWorkResult::new(
            HarnessRunId::generate(),
            ExecutionPlanVersionId::generate(),
            graph.id(),
            ComparisonInstrumentation {
                retries: Some(0),
                task_success: Some(true),
                ..ComparisonInstrumentation::default()
            },
            fingerprint,
            Some(cohort.clone()),
            100,
        );
        assert_eq!(
            result.validation_label(),
            NATIVE_STRUCTURAL_VALIDATION_LABEL
        );
        assert!(result.publication_stopped());
        assert_eq!(result.cohort().unwrap().cohort_id(), cohort.cohort_id());
    }

    #[test]
    fn approval_does_not_imply_grant() {
        let (plan, graph) = freeze_pair("model-a");
        let run_id = HarnessRunId::generate();
        let approval = ApprovalRecord::new(run_id, &plan, &graph, "owner", 5).unwrap();
        let resource = GrantResourceSelector::RelativeTarget("index.html".to_owned());
        let scope = GrantActionScope::Run;
        // No grant object exists; evaluation requires an explicit grant record.
        let orphan_grant = CapabilityGrant::issue(
            run_id,
            CapabilityType::LocalRead,
            resource.clone(),
            scope.clone(),
            None,
            Some(approval.id()),
            "owner",
            5,
            5 + BOOTSTRAP_GRANT_TTL_MS,
            DEFAULT_DISPATCH_BUDGET,
        )
        .unwrap();
        let request = GrantEvaluationRequest {
            run_id,
            capability: CapabilityType::LocalRead,
            operation_id: OP_LOCAL_READ_V1,
            resource: &resource,
            action_scope: &scope,
            pair: None,
            now_unix_ms: 6,
        };
        assert_eq!(
            evaluate_grant(&orphan_grant, &request),
            GrantEvaluation::Allow
        );
        // Approval alone has no evaluate_grant path; related_approval_id is metadata only.
        assert_eq!(orphan_grant.related_approval_id(), Some(approval.id()));
    }
}
