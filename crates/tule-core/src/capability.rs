//! Capability grants, scopes, and default-deny evaluation for Harness Runs.

use std::{error::Error, fmt, str::FromStr};

use uuid::{Uuid, Variant, Version};

use crate::{
    ApprovalRecordId, EffectRecordId, ExecutionPlanVersionId, HarnessRunId, InvalidRunId,
    NodeAttemptId, RunGraphVersionId,
};

/// Bootstrap local-read and provider-disclose grants expire after this many milliseconds.
pub const BOOTSTRAP_GRANT_TTL_MS: i64 = 5 * 60 * 1000;

/// Post-approval replacement and native-inspection grants expire after this many milliseconds.
pub const POST_APPROVAL_GRANT_TTL_MS: i64 = 15 * 60 * 1000;

/// Bootstrap and post-approval grants each permit one registered-operation dispatch.
pub const DEFAULT_DISPATCH_BUDGET: u32 = 1;

/// Registered operation identity for typed local read.
pub const OP_LOCAL_READ_V1: &str = "local-read-v1";

/// Registered operation identity for provider disclosure.
pub const OP_PROVIDER_DISCLOSE_V1: &str = "provider-disclose-v1";

/// Registered operation identity for exact-target create-or-replace.
pub const OP_CREATE_OR_REPLACE_V1: &str = "create-or-replace-v1";

/// Registered operation identity for native change inspection.
pub const OP_NATIVE_INSPECT_V1: &str = "native-inspect-v1";

/// Schema version bound into registered operation identity.
pub const REGISTERED_OPERATION_SCHEMA_V1: &str = "tule-registered-op-schema-v1";

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
    /// Opaque identifier for a Capability Grant.
    CapabilityGrantId,
    "capability grant ID"
);

/// Allowlisted capability type for Phase 2 Harness operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CapabilityType {
    /// Metadata-only traversal of the run root.
    List,
    /// Local read of explicitly selected content.
    LocalRead,
    /// Exact-target create-or-replace.
    CreateOrReplace,
    /// Native baseline-to-current inspection.
    NativeInspection,
    /// Provider disclosure of approved context.
    ProviderDisclose,
}

impl CapabilityType {
    /// Stable snake_case product label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::List => "list",
            Self::LocalRead => "local_read",
            Self::CreateOrReplace => "create_or_replace",
            Self::NativeInspection => "native_inspection",
            Self::ProviderDisclose => "provider_disclose",
        }
    }

    /// Parses an allowlisted capability type.
    pub fn parse(value: &str) -> Result<Self, InvalidCapabilityType> {
        match value {
            "list" => Ok(Self::List),
            "local_read" => Ok(Self::LocalRead),
            "create_or_replace" => Ok(Self::CreateOrReplace),
            "native_inspection" => Ok(Self::NativeInspection),
            "provider_disclose" => Ok(Self::ProviderDisclose),
            _ => Err(InvalidCapabilityType),
        }
    }

    /// Default TTL in milliseconds for grants of this type in Work 0022.
    #[must_use]
    pub const fn default_ttl_ms(self) -> i64 {
        match self {
            Self::LocalRead | Self::ProviderDisclose | Self::List => BOOTSTRAP_GRANT_TTL_MS,
            Self::CreateOrReplace | Self::NativeInspection => POST_APPROVAL_GRANT_TTL_MS,
        }
    }

    /// Registered operation identity this capability type authorises.
    #[must_use]
    pub const fn registered_operation(self) -> &'static str {
        match self {
            Self::List | Self::LocalRead => OP_LOCAL_READ_V1,
            Self::ProviderDisclose => OP_PROVIDER_DISCLOSE_V1,
            Self::CreateOrReplace => OP_CREATE_OR_REPLACE_V1,
            Self::NativeInspection => OP_NATIVE_INSPECT_V1,
        }
    }
}

impl fmt::Display for CapabilityType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Unknown capability type label.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidCapabilityType;

impl fmt::Display for InvalidCapabilityType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("capability type is not allowlisted")
    }
}

impl Error for InvalidCapabilityType {}

/// Trusted registered-operation identity and schema version.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RegisteredOperationIdentity {
    operation_id: String,
    schema_version: String,
    /// When true, an unknown settled effect may be retried after confirmed-no-effect.
    repeatable: bool,
}

impl RegisteredOperationIdentity {
    /// Creates a registered-operation identity.
    pub fn new(
        operation_id: impl Into<String>,
        schema_version: impl Into<String>,
        repeatable: bool,
    ) -> Self {
        Self {
            operation_id: operation_id.into(),
            schema_version: schema_version.into(),
            repeatable,
        }
    }

    /// Fixed Work 0022 identities.
    #[must_use]
    pub fn for_capability(capability: CapabilityType) -> Self {
        // Native create-or-replace is non-repeatable by default.
        let repeatable = matches!(
            capability,
            CapabilityType::List | CapabilityType::LocalRead | CapabilityType::NativeInspection
        );
        Self::new(
            capability.registered_operation(),
            REGISTERED_OPERATION_SCHEMA_V1,
            repeatable,
        )
    }

    /// Returns the operation identity string.
    #[must_use]
    pub fn operation_id(&self) -> &str {
        &self.operation_id
    }

    /// Returns the schema version.
    #[must_use]
    pub fn schema_version(&self) -> &str {
        &self.schema_version
    }

    /// Returns whether the trusted definition declares the operation repeatable.
    #[must_use]
    pub const fn repeatable(&self) -> bool {
        self.repeatable
    }
}

/// Resource selector bound into a grant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GrantResourceSelector {
    /// Run-root metadata traversal.
    RunRoot,
    /// Exact relative target path under the run root.
    RelativeTarget(String),
    /// Exact approved context-manifest hash for disclosure.
    ContextManifestHash(String),
    /// Exact preimage/postimage bound replacement target.
    ReplacementTarget {
        /// Run-relative path.
        relative_target: String,
        /// Expected preimage content hash.
        expected_preimage_hash: String,
        /// Expected postimage content hash.
        expected_postimage_hash: String,
    },
}

impl GrantResourceSelector {
    /// Canonical bytes for hashing and comparison.
    #[must_use]
    pub fn canonical_bytes(&self) -> Vec<u8> {
        match self {
            Self::RunRoot => b"run_root".to_vec(),
            Self::RelativeTarget(path) => {
                let mut out = b"relative_target:".to_vec();
                out.extend_from_slice(path.as_bytes());
                out
            }
            Self::ContextManifestHash(hash) => {
                let mut out = b"context_manifest:".to_vec();
                out.extend_from_slice(hash.as_bytes());
                out
            }
            Self::ReplacementTarget {
                relative_target,
                expected_preimage_hash,
                expected_postimage_hash,
            } => {
                let mut out = b"replacement:".to_vec();
                out.extend_from_slice(relative_target.as_bytes());
                out.push(b'|');
                out.extend_from_slice(expected_preimage_hash.as_bytes());
                out.push(b'|');
                out.extend_from_slice(expected_postimage_hash.as_bytes());
                out
            }
        }
    }
}

/// Optional node or effect binding for a grant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GrantActionScope {
    /// Grant applies to the whole run (bootstrap).
    Run,
    /// Grant is bound to a graph node identity string.
    Node(String),
    /// Grant is bound to one effect.
    Effect(EffectRecordId),
    /// Grant is bound to one node attempt.
    Attempt(NodeAttemptId),
}

/// Pair binding for post-approval grants. Bootstrap grants omit the pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PlanGraphPairBinding {
    /// Frozen execution plan version.
    pub plan_version_id: ExecutionPlanVersionId,
    /// Frozen run graph version.
    pub graph_version_id: RunGraphVersionId,
}

/// Immutable Capability Grant record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityGrant {
    id: CapabilityGrantId,
    run_id: HarnessRunId,
    capability: CapabilityType,
    resource: GrantResourceSelector,
    action_scope: GrantActionScope,
    pair: Option<PlanGraphPairBinding>,
    /// Optional approval this grant was issued alongside (never implies authority alone).
    related_approval_id: Option<ApprovalRecordId>,
    issuer: String,
    issued_at_unix_ms: i64,
    expires_at_unix_ms: i64,
    revoked_at_unix_ms: Option<i64>,
    dispatch_budget: u32,
    dispatch_budget_remaining: u32,
    registered_operation: RegisteredOperationIdentity,
}

impl CapabilityGrant {
    /// Issues a new grant with the given budget and expiry.
    #[allow(clippy::too_many_arguments)]
    pub fn issue(
        run_id: HarnessRunId,
        capability: CapabilityType,
        resource: GrantResourceSelector,
        action_scope: GrantActionScope,
        pair: Option<PlanGraphPairBinding>,
        related_approval_id: Option<ApprovalRecordId>,
        issuer: impl Into<String>,
        issued_at_unix_ms: i64,
        expires_at_unix_ms: i64,
        dispatch_budget: u32,
    ) -> Result<Self, CapabilityGrantError> {
        if expires_at_unix_ms <= issued_at_unix_ms {
            return Err(CapabilityGrantError::InvalidExpiry);
        }
        if dispatch_budget == 0 {
            return Err(CapabilityGrantError::ZeroBudget);
        }
        Ok(Self {
            id: CapabilityGrantId::generate(),
            run_id,
            capability,
            resource,
            action_scope,
            pair,
            related_approval_id,
            issuer: issuer.into(),
            issued_at_unix_ms,
            expires_at_unix_ms,
            revoked_at_unix_ms: None,
            dispatch_budget,
            dispatch_budget_remaining: dispatch_budget,
            registered_operation: RegisteredOperationIdentity::for_capability(capability),
        })
    }

    /// Reconstructs a persisted grant.
    #[allow(clippy::too_many_arguments)]
    pub fn from_stored_parts(
        id: &str,
        run_id: HarnessRunId,
        capability: CapabilityType,
        resource: GrantResourceSelector,
        action_scope: GrantActionScope,
        pair: Option<PlanGraphPairBinding>,
        related_approval_id: Option<ApprovalRecordId>,
        issuer: impl Into<String>,
        issued_at_unix_ms: i64,
        expires_at_unix_ms: i64,
        revoked_at_unix_ms: Option<i64>,
        dispatch_budget: u32,
        dispatch_budget_remaining: u32,
        registered_operation: RegisteredOperationIdentity,
    ) -> Result<Self, CapabilityGrantError> {
        let id = CapabilityGrantId::parse(id)?;
        if dispatch_budget_remaining > dispatch_budget {
            return Err(CapabilityGrantError::BudgetInvariant);
        }
        Ok(Self {
            id,
            run_id,
            capability,
            resource,
            action_scope,
            pair,
            related_approval_id,
            issuer: issuer.into(),
            issued_at_unix_ms,
            expires_at_unix_ms,
            revoked_at_unix_ms,
            dispatch_budget,
            dispatch_budget_remaining,
            registered_operation,
        })
    }

    /// Returns the grant identifier.
    #[must_use]
    pub const fn id(&self) -> CapabilityGrantId {
        self.id
    }

    /// Returns the owning run.
    #[must_use]
    pub const fn run_id(&self) -> HarnessRunId {
        self.run_id
    }

    /// Returns the capability type.
    #[must_use]
    pub const fn capability(&self) -> CapabilityType {
        self.capability
    }

    /// Returns the resource selector.
    #[must_use]
    pub const fn resource(&self) -> &GrantResourceSelector {
        &self.resource
    }

    /// Returns the action scope.
    #[must_use]
    pub const fn action_scope(&self) -> &GrantActionScope {
        &self.action_scope
    }

    /// Returns the optional plan/graph pair binding.
    #[must_use]
    pub const fn pair(&self) -> Option<PlanGraphPairBinding> {
        self.pair
    }

    /// Returns the related approval identifier, if any.
    #[must_use]
    pub const fn related_approval_id(&self) -> Option<ApprovalRecordId> {
        self.related_approval_id
    }

    /// Returns the issuer label.
    #[must_use]
    pub fn issuer(&self) -> &str {
        &self.issuer
    }

    /// Returns issuance time.
    #[must_use]
    pub const fn issued_at_unix_ms(&self) -> i64 {
        self.issued_at_unix_ms
    }

    /// Returns expiry time.
    #[must_use]
    pub const fn expires_at_unix_ms(&self) -> i64 {
        self.expires_at_unix_ms
    }

    /// Returns revocation time when revoked.
    #[must_use]
    pub const fn revoked_at_unix_ms(&self) -> Option<i64> {
        self.revoked_at_unix_ms
    }

    /// Returns whether the grant has been revoked.
    #[must_use]
    pub const fn is_revoked(&self) -> bool {
        self.revoked_at_unix_ms.is_some()
    }

    /// Returns the original dispatch budget.
    #[must_use]
    pub const fn dispatch_budget(&self) -> u32 {
        self.dispatch_budget
    }

    /// Returns remaining dispatch budget.
    #[must_use]
    pub const fn dispatch_budget_remaining(&self) -> u32 {
        self.dispatch_budget_remaining
    }

    /// Returns the registered operation identity.
    #[must_use]
    pub const fn registered_operation(&self) -> &RegisteredOperationIdentity {
        &self.registered_operation
    }

    /// Marks the grant revoked at `now_unix_ms`.
    pub fn revoke(&mut self, now_unix_ms: i64) -> Result<(), CapabilityGrantError> {
        if self.revoked_at_unix_ms.is_some() {
            return Err(CapabilityGrantError::AlreadyRevoked);
        }
        self.revoked_at_unix_ms = Some(now_unix_ms);
        Ok(())
    }

    /// Consumes one dispatch from the remaining budget.
    pub fn consume_dispatch(&mut self) -> Result<(), CapabilityGrantError> {
        if self.dispatch_budget_remaining == 0 {
            return Err(CapabilityGrantError::BudgetExhausted);
        }
        self.dispatch_budget_remaining -= 1;
        Ok(())
    }
}

/// Request evaluated against a grant under default-deny rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantEvaluationRequest<'a> {
    /// Run that must match the grant.
    pub run_id: HarnessRunId,
    /// Required capability type.
    pub capability: CapabilityType,
    /// Required registered operation identity.
    pub operation_id: &'a str,
    /// Required resource selector.
    pub resource: &'a GrantResourceSelector,
    /// Required action scope.
    pub action_scope: &'a GrantActionScope,
    /// Required pair binding when the grant is pair-bound.
    pub pair: Option<PlanGraphPairBinding>,
    /// Evaluation clock.
    pub now_unix_ms: i64,
}

/// Outcome of default-deny grant evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GrantEvaluation {
    /// Grant authorises the request.
    Allow,
    /// Grant does not authorise the request.
    Deny(GrantDenialReason),
}

/// Why a grant evaluation denied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantDenialReason {
    /// Wrong run binding.
    WrongRun,
    /// Wrong capability type.
    WrongCapability,
    /// Wrong registered operation.
    WrongOperation,
    /// Resource selector mismatch.
    WrongScope,
    /// Action scope mismatch.
    WrongActionScope,
    /// Pair binding mismatch or missing when required.
    WrongPairBinding,
    /// Grant expired.
    Expired,
    /// Grant revoked.
    Revoked,
    /// Dispatch budget exhausted.
    BudgetExhausted,
}

impl fmt::Display for GrantDenialReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongRun => formatter.write_str("grant run binding does not match"),
            Self::WrongCapability => formatter.write_str("grant capability does not match"),
            Self::WrongOperation => formatter.write_str("grant operation does not match"),
            Self::WrongScope => formatter.write_str("grant resource scope does not match"),
            Self::WrongActionScope => formatter.write_str("grant action scope does not match"),
            Self::WrongPairBinding => formatter.write_str("grant plan/graph pair does not match"),
            Self::Expired => formatter.write_str("grant has expired"),
            Self::Revoked => formatter.write_str("grant has been revoked"),
            Self::BudgetExhausted => formatter.write_str("grant dispatch budget is exhausted"),
        }
    }
}

/// Evaluates a grant under default-deny semantics.
///
/// Absence of a matching grant is a denial at the call site; this function
/// only evaluates one concrete grant against one request.
#[must_use]
pub fn evaluate_grant(
    grant: &CapabilityGrant,
    request: &GrantEvaluationRequest<'_>,
) -> GrantEvaluation {
    if grant.run_id() != request.run_id {
        return GrantEvaluation::Deny(GrantDenialReason::WrongRun);
    }
    if grant.capability() != request.capability {
        return GrantEvaluation::Deny(GrantDenialReason::WrongCapability);
    }
    if grant.registered_operation().operation_id() != request.operation_id {
        return GrantEvaluation::Deny(GrantDenialReason::WrongOperation);
    }
    if grant.resource() != request.resource {
        return GrantEvaluation::Deny(GrantDenialReason::WrongScope);
    }
    if grant.action_scope() != request.action_scope {
        return GrantEvaluation::Deny(GrantDenialReason::WrongActionScope);
    }
    match (grant.pair(), request.pair) {
        (None, None) => {}
        (Some(granted), Some(required)) if granted == required => {}
        _ => return GrantEvaluation::Deny(GrantDenialReason::WrongPairBinding),
    }
    if grant.is_revoked() {
        return GrantEvaluation::Deny(GrantDenialReason::Revoked);
    }
    if request.now_unix_ms >= grant.expires_at_unix_ms() {
        return GrantEvaluation::Deny(GrantDenialReason::Expired);
    }
    if grant.dispatch_budget_remaining() == 0 {
        return GrantEvaluation::Deny(GrantDenialReason::BudgetExhausted);
    }
    GrantEvaluation::Allow
}

/// Capability grant construction or mutation failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapabilityGrantError {
    /// Identifier is invalid.
    InvalidId(InvalidRunId),
    /// Expiry is not after issuance.
    InvalidExpiry,
    /// Dispatch budget must be at least one.
    ZeroBudget,
    /// Remaining budget exceeds original budget.
    BudgetInvariant,
    /// Grant was already revoked.
    AlreadyRevoked,
    /// No remaining dispatch budget.
    BudgetExhausted,
}

impl From<InvalidRunId> for CapabilityGrantError {
    fn from(error: InvalidRunId) -> Self {
        Self::InvalidId(error)
    }
}

impl fmt::Display for CapabilityGrantError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidId(error) => error.fmt(formatter),
            Self::InvalidExpiry => formatter.write_str("grant expiry must be after issuance"),
            Self::ZeroBudget => formatter.write_str("grant dispatch budget must be at least one"),
            Self::BudgetInvariant => {
                formatter.write_str("remaining dispatch budget exceeds original budget")
            }
            Self::AlreadyRevoked => formatter.write_str("grant is already revoked"),
            Self::BudgetExhausted => formatter.write_str("grant dispatch budget is exhausted"),
        }
    }
}

impl Error for CapabilityGrantError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidId(error) => Some(error),
            Self::InvalidExpiry
            | Self::ZeroBudget
            | Self::BudgetInvariant
            | Self::AlreadyRevoked
            | Self::BudgetExhausted => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_grant(now: i64) -> CapabilityGrant {
        CapabilityGrant::issue(
            HarnessRunId::generate(),
            CapabilityType::LocalRead,
            GrantResourceSelector::RelativeTarget("index.html".to_owned()),
            GrantActionScope::Run,
            None,
            None,
            "owner",
            now,
            now + BOOTSTRAP_GRANT_TTL_MS,
            DEFAULT_DISPATCH_BUDGET,
        )
        .unwrap()
    }

    #[test]
    fn default_deny_rejects_wrong_scope_expiry_revocation_and_budget() {
        let now = 1_000_000_i64;
        let mut grant = sample_grant(now);
        let run_id = grant.run_id();
        let resource = GrantResourceSelector::RelativeTarget("index.html".to_owned());
        let scope = GrantActionScope::Run;
        let allow = GrantEvaluationRequest {
            run_id,
            capability: CapabilityType::LocalRead,
            operation_id: OP_LOCAL_READ_V1,
            resource: &resource,
            action_scope: &scope,
            pair: None,
            now_unix_ms: now + 1,
        };
        assert_eq!(evaluate_grant(&grant, &allow), GrantEvaluation::Allow);

        let wrong_resource = GrantResourceSelector::RelativeTarget("other.html".to_owned());
        let mut wrong = allow.clone();
        wrong.resource = &wrong_resource;
        assert!(matches!(
            evaluate_grant(&grant, &wrong),
            GrantEvaluation::Deny(GrantDenialReason::WrongScope)
        ));

        let mut expired = allow.clone();
        expired.now_unix_ms = grant.expires_at_unix_ms();
        assert!(matches!(
            evaluate_grant(&grant, &expired),
            GrantEvaluation::Deny(GrantDenialReason::Expired)
        ));

        grant.revoke(now + 2).unwrap();
        assert!(matches!(
            evaluate_grant(&grant, &allow),
            GrantEvaluation::Deny(GrantDenialReason::Revoked)
        ));

        let mut budgeted = sample_grant(now);
        let budget_resource = GrantResourceSelector::RelativeTarget("index.html".to_owned());
        let budget_scope = GrantActionScope::Run;
        let budget_request = GrantEvaluationRequest {
            run_id: budgeted.run_id(),
            capability: CapabilityType::LocalRead,
            operation_id: OP_LOCAL_READ_V1,
            resource: &budget_resource,
            action_scope: &budget_scope,
            pair: None,
            now_unix_ms: now + 1,
        };
        budgeted.consume_dispatch().unwrap();
        assert!(matches!(
            evaluate_grant(&budgeted, &budget_request),
            GrantEvaluation::Deny(GrantDenialReason::BudgetExhausted)
        ));
    }

    #[test]
    fn pair_binding_is_exact_and_approval_is_not_authority() {
        let now = 50_i64;
        let pair = PlanGraphPairBinding {
            plan_version_id: ExecutionPlanVersionId::generate(),
            graph_version_id: RunGraphVersionId::generate(),
        };
        let approval = ApprovalRecordId::generate();
        let grant = CapabilityGrant::issue(
            HarnessRunId::generate(),
            CapabilityType::CreateOrReplace,
            GrantResourceSelector::ReplacementTarget {
                relative_target: "index.html".to_owned(),
                expected_preimage_hash: "a".repeat(64),
                expected_postimage_hash: "b".repeat(64),
            },
            GrantActionScope::Node("replace-existing-file-v1".to_owned()),
            Some(pair),
            Some(approval),
            "owner",
            now,
            now + POST_APPROVAL_GRANT_TTL_MS,
            DEFAULT_DISPATCH_BUDGET,
        )
        .unwrap();
        assert_eq!(grant.related_approval_id(), Some(approval));
        let resource = grant.resource().clone();
        let scope = grant.action_scope().clone();
        let request = GrantEvaluationRequest {
            run_id: grant.run_id(),
            capability: CapabilityType::CreateOrReplace,
            operation_id: OP_CREATE_OR_REPLACE_V1,
            resource: &resource,
            action_scope: &scope,
            pair: Some(pair),
            now_unix_ms: now + 1,
        };
        assert_eq!(evaluate_grant(&grant, &request), GrantEvaluation::Allow);
        let mut wrong_pair = request.clone();
        wrong_pair.pair = Some(PlanGraphPairBinding {
            plan_version_id: ExecutionPlanVersionId::generate(),
            graph_version_id: pair.graph_version_id,
        });
        assert!(matches!(
            evaluate_grant(&grant, &wrong_pair),
            GrantEvaluation::Deny(GrantDenialReason::WrongPairBinding)
        ));
        // Approval presence on the grant never substitutes for evaluation.
        let mut no_pair = request.clone();
        no_pair.pair = None;
        assert!(matches!(
            evaluate_grant(&grant, &no_pair),
            GrantEvaluation::Deny(GrantDenialReason::WrongPairBinding)
        ));
    }
}
