CREATE TABLE harness_runs (
    id TEXT PRIMARY KEY NOT NULL,
    run_root_display_name TEXT NOT NULL,
    created_at_unix_ms INTEGER NOT NULL,
    cohort_json TEXT
) STRICT;

CREATE TABLE harness_run_events (
    id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL REFERENCES harness_runs(id),
    sequence INTEGER NOT NULL,
    kind_tag TEXT NOT NULL,
    kind_payload_json TEXT NOT NULL,
    recorded_at_unix_ms INTEGER NOT NULL,
    UNIQUE (run_id, sequence)
) STRICT;

CREATE TABLE harness_replacements (
    id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL REFERENCES harness_runs(id),
    relative_target TEXT NOT NULL,
    preimage_hash TEXT NOT NULL,
    postimage_hash TEXT NOT NULL,
    expected_diff_hash TEXT NOT NULL,
    postimage_utf8 TEXT NOT NULL,
    provider_request_id TEXT,
    provider_response_id TEXT,
    created_at_unix_ms INTEGER NOT NULL
) STRICT;

CREATE TABLE harness_run_graphs (
    id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL REFERENCES harness_runs(id),
    nodes_json TEXT NOT NULL,
    edges_json TEXT NOT NULL,
    retry_rule TEXT NOT NULL,
    validation_rule TEXT NOT NULL,
    content_hash TEXT NOT NULL
) STRICT;

CREATE TABLE harness_execution_plans (
    id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL REFERENCES harness_runs(id),
    graph_version_id TEXT NOT NULL REFERENCES harness_run_graphs(id),
    replacement_id TEXT NOT NULL REFERENCES harness_replacements(id),
    instructions TEXT NOT NULL,
    provider_profile_id TEXT NOT NULL,
    model_id TEXT NOT NULL,
    disclosure_policy_id TEXT NOT NULL,
    disclosure_allowed TEXT NOT NULL,
    capability_envelope_json TEXT NOT NULL,
    context_manifest_id TEXT NOT NULL,
    context_content_hash TEXT NOT NULL,
    context_request_semantic_hash TEXT NOT NULL,
    context_disclosed_byte_count INTEGER NOT NULL,
    context_summary TEXT NOT NULL,
    preimage_filesystem_identity TEXT NOT NULL,
    execution_policy_revision TEXT NOT NULL,
    approval_hash TEXT NOT NULL,
    created_at_unix_ms INTEGER NOT NULL
) STRICT;

CREATE TABLE harness_approvals (
    id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL REFERENCES harness_runs(id),
    plan_version_id TEXT NOT NULL REFERENCES harness_execution_plans(id),
    graph_version_id TEXT NOT NULL REFERENCES harness_run_graphs(id),
    approval_hash TEXT NOT NULL,
    approver TEXT NOT NULL,
    approved_at_unix_ms INTEGER NOT NULL
) STRICT;

CREATE TABLE harness_grants (
    id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL REFERENCES harness_runs(id),
    capability TEXT NOT NULL,
    resource_json TEXT NOT NULL,
    action_scope_json TEXT NOT NULL,
    pair_plan_version_id TEXT,
    pair_graph_version_id TEXT,
    related_approval_id TEXT,
    issuer TEXT NOT NULL,
    issued_at_unix_ms INTEGER NOT NULL,
    expires_at_unix_ms INTEGER NOT NULL,
    revoked_at_unix_ms INTEGER,
    dispatch_budget INTEGER NOT NULL,
    dispatch_budget_remaining INTEGER NOT NULL,
    registered_operation_id TEXT NOT NULL,
    registered_operation_schema TEXT NOT NULL,
    registered_operation_repeatable INTEGER NOT NULL
) STRICT;

CREATE TABLE harness_effects (
    id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL REFERENCES harness_runs(id),
    attempt_id TEXT,
    plan_version_id TEXT,
    graph_version_id TEXT,
    operation_id TEXT NOT NULL,
    operation_schema_version TEXT NOT NULL,
    target_hash TEXT NOT NULL,
    grant_id TEXT NOT NULL REFERENCES harness_grants(id),
    phase TEXT NOT NULL,
    claimant TEXT,
    operation_result TEXT,
    certainty TEXT,
    prepared_at_unix_ms INTEGER NOT NULL,
    claimed_at_unix_ms INTEGER,
    dispatched_at_unix_ms INTEGER,
    settled_at_unix_ms INTEGER,
    expected_preimage_hash TEXT,
    expected_postimage_hash TEXT
) STRICT;

CREATE TABLE harness_checkpoints (
    id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL REFERENCES harness_runs(id),
    last_event_sequence INTEGER NOT NULL,
    event_chain_hash TEXT NOT NULL,
    plan_version_id TEXT NOT NULL,
    graph_version_id TEXT NOT NULL,
    execution_policy_revision TEXT NOT NULL,
    expected_postimage_hash TEXT NOT NULL,
    created_at_unix_ms INTEGER NOT NULL
) STRICT;

CREATE TABLE harness_validations (
    id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL REFERENCES harness_runs(id),
    plan_version_id TEXT NOT NULL,
    graph_version_id TEXT NOT NULL,
    label TEXT NOT NULL,
    approved_postimage_hash TEXT NOT NULL,
    observed_postimage_hash TEXT NOT NULL,
    native_diff_hash TEXT NOT NULL,
    passed INTEGER NOT NULL,
    validated_at_unix_ms INTEGER NOT NULL
) STRICT;

CREATE TABLE harness_denials (
    id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL REFERENCES harness_runs(id),
    reason TEXT NOT NULL,
    grant_id TEXT,
    resource_json TEXT,
    recorded_at_unix_ms INTEGER NOT NULL
) STRICT;

CREATE TABLE harness_leases (
    run_id TEXT PRIMARY KEY NOT NULL REFERENCES harness_runs(id),
    id TEXT NOT NULL,
    owner_process_instance TEXT NOT NULL,
    acquired_at_unix_ms INTEGER NOT NULL,
    expires_at_unix_ms INTEGER NOT NULL,
    renew_interval_ms INTEGER NOT NULL
) STRICT;

CREATE TABLE harness_final_results (
    run_id TEXT PRIMARY KEY NOT NULL REFERENCES harness_runs(id),
    plan_version_id TEXT NOT NULL,
    graph_version_id TEXT NOT NULL,
    validation_label TEXT NOT NULL,
    publication_stopped INTEGER NOT NULL,
    instrumentation_json TEXT NOT NULL,
    fingerprint_algorithm TEXT NOT NULL,
    fingerprint_value TEXT NOT NULL,
    cohort_json TEXT,
    completed_at_unix_ms INTEGER NOT NULL
) STRICT;

CREATE INDEX harness_run_events_run_sequence_index
ON harness_run_events (run_id, sequence ASC);

CREATE INDEX harness_grants_run_id_index
ON harness_grants (run_id);

CREATE INDEX harness_effects_run_id_index
ON harness_effects (run_id);
