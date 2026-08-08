//! SQLite persistence for Harness Runs, effects, grants, leases, and reconstruction.

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde_json::{Value, json};
use tule_core::{
    AcquireLeaseIntent, ApprovalRecord, CapabilityGrant, CapabilityGrantId, CapabilityType,
    Checkpoint, ClaimEffectIntent, ComparisonInstrumentation, ConsumeDispatchBudgetIntent,
    ContextManifest, DenialEvidence, DisclosurePolicy, EffectCertainty, EffectJournalPhase,
    EffectOperationResult, EffectRecord, ExecutionPlanVersion, FinalWorkResult, GrantActionScope,
    GrantResourceSelector, GraphEdge, GraphNode, GraphShapeFingerprint, HarnessRun, HarnessRunId,
    PersistCheckpointIntent, PlanGraphPairBinding, ReconstructedRun, RegisteredOperationIdentity,
    ReleaseLeaseIntent, ReplacementContentInput, RootLease, RunEvent, RunEventKind,
    RunGraphVersion, RunRepository, TakeoverLeaseIntent, TaskCohortAssignment, ValidationResult,
    is_quiescent_for_checkpoint,
};

use super::{SqliteStore, SqliteStoreError};

impl RunRepository for SqliteStore {
    type Error = SqliteStoreError;

    fn create_run(&self, run: &HarnessRun, event: &RunEvent) -> Result<(), Self::Error> {
        let mut connection = self.connection()?;
        let tx = connection
            .transaction()
            .map_err(SqliteStoreError::Database)?;
        tx.execute(
            "INSERT INTO harness_runs (id, run_root_display_name, created_at_unix_ms, cohort_json)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                run.id().to_string(),
                run.run_root_display_name(),
                run.created_at_unix_ms(),
                cohort_to_json(run.cohort())?
            ],
        )
        .map_err(SqliteStoreError::Database)?;
        insert_event(&tx, event)?;
        tx.commit().map_err(SqliteStoreError::Database)
    }

    fn append_event(&self, event: &RunEvent) -> Result<(), Self::Error> {
        let mut connection = self.connection()?;
        let tx = connection
            .transaction()
            .map_err(SqliteStoreError::Database)?;
        assert_next_sequence(&tx, event)?;
        insert_event(&tx, event)?;
        tx.commit().map_err(SqliteStoreError::Database)
    }

    fn persist_frozen_pair(
        &self,
        plan: &ExecutionPlanVersion,
        graph: &RunGraphVersion,
        event: &RunEvent,
    ) -> Result<(), Self::Error> {
        let mut connection = self.connection()?;
        let tx = connection
            .transaction()
            .map_err(SqliteStoreError::Database)?;
        assert_next_sequence(&tx, event)?;
        let replacement = plan.replacement();
        tx.execute(
            "INSERT INTO harness_replacements (
                id, run_id, relative_target, preimage_hash, postimage_hash, expected_diff_hash,
                postimage_utf8, provider_request_id, provider_response_id, created_at_unix_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                replacement.id().to_string(),
                event.run_id().to_string(),
                replacement.relative_target(),
                replacement.preimage_hash(),
                replacement.postimage_hash(),
                replacement.expected_diff_hash(),
                replacement.postimage_utf8(),
                replacement.provider_request_id(),
                replacement.provider_response_id(),
                replacement.created_at_unix_ms()
            ],
        )
        .map_err(SqliteStoreError::Database)?;
        tx.execute(
            "INSERT INTO harness_run_graphs (
                id, run_id, nodes_json, edges_json, retry_rule, validation_rule, content_hash
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                graph.id().to_string(),
                event.run_id().to_string(),
                nodes_to_json(graph.nodes())?,
                edges_to_json(graph.edges())?,
                graph.retry_rule(),
                graph.validation_rule(),
                graph.content_hash()
            ],
        )
        .map_err(SqliteStoreError::Database)?;
        let manifest = plan.context_manifest();
        let disclosure = plan.disclosure_policy();
        tx.execute(
            "INSERT INTO harness_execution_plans (
                id, run_id, graph_version_id, replacement_id, instructions, provider_profile_id,
                model_id, disclosure_policy_id, disclosure_allowed, capability_envelope_json,
                context_manifest_id, context_content_hash, context_request_semantic_hash,
                context_disclosed_byte_count, context_summary, preimage_filesystem_identity,
                execution_policy_revision, approval_hash, created_at_unix_ms
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19
             )",
            params![
                plan.id().to_string(),
                event.run_id().to_string(),
                graph.id().to_string(),
                replacement.id().to_string(),
                plan.instructions(),
                plan.provider_profile_id(),
                plan.model_id(),
                disclosure.policy_id(),
                disclosure.allowed_disclosure(),
                envelope_to_json(plan.capability_envelope())?,
                manifest.id().to_string(),
                manifest.content_hash(),
                manifest.request_semantic_hash(),
                i64::try_from(manifest.disclosed_byte_count())
                    .map_err(|_| SqliteStoreError::Numeric)?,
                manifest.summary(),
                plan.preimage_filesystem_identity(),
                plan.execution_policy_revision(),
                plan.approval_hash(),
                plan.created_at_unix_ms()
            ],
        )
        .map_err(SqliteStoreError::Database)?;
        insert_event(&tx, event)?;
        tx.commit().map_err(SqliteStoreError::Database)
    }

    fn persist_approval(
        &self,
        approval: &ApprovalRecord,
        event: &RunEvent,
    ) -> Result<(), Self::Error> {
        let mut connection = self.connection()?;
        let tx = connection
            .transaction()
            .map_err(SqliteStoreError::Database)?;
        assert_next_sequence(&tx, event)?;
        tx.execute(
            "INSERT INTO harness_approvals (
                id, run_id, plan_version_id, graph_version_id, approval_hash, approver, approved_at_unix_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                approval.id().to_string(),
                approval.run_id().to_string(),
                approval.plan_version_id().to_string(),
                approval.graph_version_id().to_string(),
                approval.approval_hash(),
                approval.approver(),
                approval.approved_at_unix_ms()
            ],
        )
        .map_err(SqliteStoreError::Database)?;
        insert_event(&tx, event)?;
        tx.commit().map_err(SqliteStoreError::Database)
    }

    fn persist_grant(&self, grant: &CapabilityGrant, event: &RunEvent) -> Result<(), Self::Error> {
        let mut connection = self.connection()?;
        let tx = connection
            .transaction()
            .map_err(SqliteStoreError::Database)?;
        assert_next_sequence(&tx, event)?;
        insert_grant(&tx, grant)?;
        insert_event(&tx, event)?;
        tx.commit().map_err(SqliteStoreError::Database)
    }

    fn persist_grant_revocation(
        &self,
        grant: &CapabilityGrant,
        event: &RunEvent,
    ) -> Result<(), Self::Error> {
        let mut connection = self.connection()?;
        let tx = connection
            .transaction()
            .map_err(SqliteStoreError::Database)?;
        assert_next_sequence(&tx, event)?;
        tx.execute(
            "UPDATE harness_grants
             SET revoked_at_unix_ms = ?1, dispatch_budget_remaining = ?2
             WHERE id = ?3 AND run_id = ?4",
            params![
                grant.revoked_at_unix_ms(),
                i64::from(grant.dispatch_budget_remaining()),
                grant.id().to_string(),
                grant.run_id().to_string()
            ],
        )
        .map_err(SqliteStoreError::Database)?;
        insert_event(&tx, event)?;
        tx.commit().map_err(SqliteStoreError::Database)
    }

    fn persist_denial(&self, denial: &DenialEvidence, event: &RunEvent) -> Result<(), Self::Error> {
        let mut connection = self.connection()?;
        let tx = connection
            .transaction()
            .map_err(SqliteStoreError::Database)?;
        assert_next_sequence(&tx, event)?;
        tx.execute(
            "INSERT INTO harness_denials (id, run_id, reason, grant_id, resource_json, recorded_at_unix_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                denial.id().to_string(),
                denial.run_id().to_string(),
                denial.reason(),
                denial.grant_id().map(|id| id.to_string()),
                denial
                    .resource()
                    .map(resource_to_json)
                    .transpose()?
                    .flatten(),
                denial.recorded_at_unix_ms()
            ],
        )
        .map_err(SqliteStoreError::Database)?;
        insert_event(&tx, event)?;
        tx.commit().map_err(SqliteStoreError::Database)
    }

    fn persist_prepared_effect(
        &self,
        effect: &EffectRecord,
        event: &RunEvent,
    ) -> Result<(), Self::Error> {
        let mut connection = self.connection()?;
        let tx = connection
            .transaction()
            .map_err(SqliteStoreError::Database)?;
        assert_next_sequence(&tx, event)?;
        insert_effect(&tx, effect)?;
        insert_event(&tx, event)?;
        tx.commit().map_err(SqliteStoreError::Database)
    }

    fn claim_effect(&self, intent: &ClaimEffectIntent) -> Result<EffectRecord, Self::Error> {
        let mut connection = self.connection()?;
        let tx = connection
            .transaction()
            .map_err(SqliteStoreError::Database)?;
        assert_next_sequence(&tx, &intent.event)?;
        let updated = tx
            .execute(
                "UPDATE harness_effects
                 SET phase = 'claimed', claimant = ?1, claimed_at_unix_ms = ?2
                 WHERE id = ?3 AND run_id = ?4 AND phase = 'prepared' AND claimant IS NULL",
                params![
                    intent.claimant,
                    intent.now_unix_ms,
                    intent.effect_id.to_string(),
                    intent.run_id.to_string()
                ],
            )
            .map_err(SqliteStoreError::Database)?;
        if updated != 1 {
            return Err(SqliteStoreError::HarnessClaimLost);
        }
        insert_event(&tx, &intent.event)?;
        let effect = load_effect(&tx, &intent.effect_id.to_string())?
            .ok_or(SqliteStoreError::HarnessNotFound)?;
        tx.commit().map_err(SqliteStoreError::Database)?;
        Ok(effect)
    }

    fn persist_effect_dispatched(
        &self,
        effect: &EffectRecord,
        event: &RunEvent,
    ) -> Result<(), Self::Error> {
        let mut connection = self.connection()?;
        let tx = connection
            .transaction()
            .map_err(SqliteStoreError::Database)?;
        assert_next_sequence(&tx, event)?;
        update_effect(&tx, effect)?;
        insert_event(&tx, event)?;
        tx.commit().map_err(SqliteStoreError::Database)
    }

    fn persist_effect_settled(
        &self,
        effect: &EffectRecord,
        event: &RunEvent,
    ) -> Result<(), Self::Error> {
        let mut connection = self.connection()?;
        let tx = connection
            .transaction()
            .map_err(SqliteStoreError::Database)?;
        assert_next_sequence(&tx, event)?;
        update_effect(&tx, effect)?;
        insert_event(&tx, event)?;
        tx.commit().map_err(SqliteStoreError::Database)
    }

    fn consume_dispatch_budget(
        &self,
        intent: &ConsumeDispatchBudgetIntent,
    ) -> Result<CapabilityGrant, Self::Error> {
        let mut connection = self.connection()?;
        let tx = connection
            .transaction()
            .map_err(SqliteStoreError::Database)?;
        let updated = tx
            .execute(
                "UPDATE harness_grants
                 SET dispatch_budget_remaining = dispatch_budget_remaining - 1
                 WHERE id = ?1 AND run_id = ?2
                   AND revoked_at_unix_ms IS NULL
                   AND expires_at_unix_ms > ?3
                   AND dispatch_budget_remaining > 0",
                params![
                    intent.grant_id.to_string(),
                    intent.run_id.to_string(),
                    intent.now_unix_ms
                ],
            )
            .map_err(SqliteStoreError::Database)?;
        if updated != 1 {
            return Err(SqliteStoreError::HarnessGrantDenied);
        }
        let grant = load_grant(&tx, &intent.grant_id.to_string())?
            .ok_or(SqliteStoreError::HarnessNotFound)?;
        tx.commit().map_err(SqliteStoreError::Database)?;
        Ok(grant)
    }

    fn persist_quiescent_checkpoint(
        &self,
        intent: &PersistCheckpointIntent,
    ) -> Result<(), Self::Error> {
        let mut connection = self.connection()?;
        let tx = connection
            .transaction()
            .map_err(SqliteStoreError::Database)?;
        let effects = load_effects_for_run(&tx, &intent.checkpoint.run_id().to_string())?;
        if !is_quiescent_for_checkpoint(&effects) {
            return Err(SqliteStoreError::HarnessNotQuiescent);
        }
        assert_next_sequence(&tx, &intent.event)?;
        tx.execute(
            "INSERT INTO harness_checkpoints (
                id, run_id, last_event_sequence, event_chain_hash, plan_version_id,
                graph_version_id, execution_policy_revision, expected_postimage_hash, created_at_unix_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                intent.checkpoint.id().to_string(),
                intent.checkpoint.run_id().to_string(),
                i64::try_from(intent.checkpoint.last_event_sequence())
                    .map_err(|_| SqliteStoreError::Numeric)?,
                intent.checkpoint.event_chain_hash(),
                intent.checkpoint.plan_version_id().to_string(),
                intent.checkpoint.graph_version_id().to_string(),
                intent.checkpoint.execution_policy_revision(),
                intent.checkpoint.expected_postimage_hash(),
                intent.checkpoint.created_at_unix_ms()
            ],
        )
        .map_err(SqliteStoreError::Database)?;
        insert_event(&tx, &intent.event)?;
        tx.commit().map_err(SqliteStoreError::Database)
    }

    fn persist_validation(
        &self,
        validation: &ValidationResult,
        event: &RunEvent,
    ) -> Result<(), Self::Error> {
        let mut connection = self.connection()?;
        let tx = connection
            .transaction()
            .map_err(SqliteStoreError::Database)?;
        assert_next_sequence(&tx, event)?;
        tx.execute(
            "INSERT INTO harness_validations (
                id, run_id, plan_version_id, graph_version_id, label, approved_postimage_hash,
                observed_postimage_hash, native_diff_hash, passed, validated_at_unix_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                validation.id().to_string(),
                validation.run_id().to_string(),
                validation.plan_version_id().to_string(),
                validation.graph_version_id().to_string(),
                validation.label(),
                validation.approved_postimage_hash(),
                validation.observed_postimage_hash(),
                validation.native_diff_hash(),
                i64::from(validation.passed()),
                validation.validated_at_unix_ms()
            ],
        )
        .map_err(SqliteStoreError::Database)?;
        insert_event(&tx, event)?;
        tx.commit().map_err(SqliteStoreError::Database)
    }

    fn persist_final_result(
        &self,
        result: &FinalWorkResult,
        event: &RunEvent,
    ) -> Result<(), Self::Error> {
        let mut connection = self.connection()?;
        let tx = connection
            .transaction()
            .map_err(SqliteStoreError::Database)?;
        assert_next_sequence(&tx, event)?;
        tx.execute(
            "INSERT INTO harness_final_results (
                run_id, plan_version_id, graph_version_id, validation_label, publication_stopped,
                instrumentation_json, fingerprint_algorithm, fingerprint_value, cohort_json,
                completed_at_unix_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                result.run_id().to_string(),
                result.plan_version_id().to_string(),
                result.graph_version_id().to_string(),
                result.validation_label(),
                i64::from(result.publication_stopped()),
                instrumentation_to_json(result.instrumentation())?,
                result.fingerprint().algorithm_version(),
                result.fingerprint().value(),
                cohort_to_json(result.cohort())?,
                result.completed_at_unix_ms()
            ],
        )
        .map_err(SqliteStoreError::Database)?;
        insert_event(&tx, event)?;
        tx.commit().map_err(SqliteStoreError::Database)
    }

    fn acquire_lease(&self, intent: &AcquireLeaseIntent) -> Result<(), Self::Error> {
        let mut connection = self.connection()?;
        let tx = connection
            .transaction()
            .map_err(SqliteStoreError::Database)?;
        if let Some(existing) = load_lease(&tx, &intent.lease.run_id().to_string())?
            && !existing.is_expired(intent.lease.acquired_at_unix_ms())
        {
            return Err(SqliteStoreError::HarnessLeaseConflict);
        }
        assert_next_sequence(&tx, &intent.event)?;
        upsert_lease(&tx, &intent.lease)?;
        insert_event(&tx, &intent.event)?;
        tx.commit().map_err(SqliteStoreError::Database)
    }

    fn release_lease(&self, intent: &ReleaseLeaseIntent) -> Result<(), Self::Error> {
        let mut connection = self.connection()?;
        let tx = connection
            .transaction()
            .map_err(SqliteStoreError::Database)?;
        assert_next_sequence(&tx, &intent.event)?;
        tx.execute(
            "DELETE FROM harness_leases WHERE run_id = ?1 AND id = ?2",
            params![intent.run_id.to_string(), intent.lease_id.to_string()],
        )
        .map_err(SqliteStoreError::Database)?;
        insert_event(&tx, &intent.event)?;
        tx.commit().map_err(SqliteStoreError::Database)
    }

    fn takeover_lease(&self, intent: &TakeoverLeaseIntent) -> Result<(), Self::Error> {
        let mut connection = self.connection()?;
        let tx = connection
            .transaction()
            .map_err(SqliteStoreError::Database)?;
        assert_next_sequence(&tx, &intent.event)?;
        upsert_lease(&tx, &intent.lease)?;
        insert_event(&tx, &intent.event)?;
        tx.commit().map_err(SqliteStoreError::Database)
    }

    fn reconstruct_run(
        &self,
        run_id: &HarnessRunId,
    ) -> Result<Option<ReconstructedRun>, Self::Error> {
        let connection = self.connection()?;
        let run_id_text = run_id.to_string();
        let Some(run) = load_run(&connection, &run_id_text)? else {
            return Ok(None);
        };
        Ok(Some(ReconstructedRun {
            run,
            events: load_events(&connection, &run_id_text)?,
            plans: load_plans(&connection, &run_id_text)?,
            graphs: load_graphs(&connection, &run_id_text)?,
            replacements: load_replacements(&connection, &run_id_text)?,
            approvals: load_approvals(&connection, &run_id_text)?,
            grants: load_grants(&connection, &run_id_text)?,
            effects: load_effects_for_run(&connection, &run_id_text)?,
            checkpoints: load_checkpoints(&connection, &run_id_text)?,
            validations: load_validations(&connection, &run_id_text)?,
            denials: load_denials(&connection, &run_id_text)?,
            lease: load_lease(&connection, &run_id_text)?,
            final_result: load_final_result(&connection, &run_id_text)?,
        }))
    }
}

fn assert_next_sequence(tx: &Transaction<'_>, event: &RunEvent) -> Result<(), SqliteStoreError> {
    let max: Option<i64> = tx
        .query_row(
            "SELECT MAX(sequence) FROM harness_run_events WHERE run_id = ?1",
            [event.run_id().to_string()],
            |row| row.get(0),
        )
        .optional()
        .map_err(SqliteStoreError::Database)?
        .flatten();
    let expected = max.map_or(1, |value| value + 1);
    let actual = i64::try_from(event.sequence()).map_err(|_| SqliteStoreError::Numeric)?;
    if actual != expected {
        return Err(SqliteStoreError::HarnessSequenceGap { expected, actual });
    }
    Ok(())
}

fn insert_event(tx: &Transaction<'_>, event: &RunEvent) -> Result<(), SqliteStoreError> {
    let (kind_tag, payload) = event_kind_to_parts(event.kind())?;
    tx.execute(
        "INSERT INTO harness_run_events (id, run_id, sequence, kind_tag, kind_payload_json, recorded_at_unix_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            event.id().to_string(),
            event.run_id().to_string(),
            i64::try_from(event.sequence()).map_err(|_| SqliteStoreError::Numeric)?,
            kind_tag,
            payload.to_string(),
            event.recorded_at_unix_ms()
        ],
    )
    .map_err(SqliteStoreError::Database)?;
    Ok(())
}

fn insert_grant(tx: &Transaction<'_>, grant: &CapabilityGrant) -> Result<(), SqliteStoreError> {
    let op = grant.registered_operation();
    tx.execute(
        "INSERT INTO harness_grants (
            id, run_id, capability, resource_json, action_scope_json, pair_plan_version_id,
            pair_graph_version_id, related_approval_id, issuer, issued_at_unix_ms, expires_at_unix_ms,
            revoked_at_unix_ms, dispatch_budget, dispatch_budget_remaining, registered_operation_id,
            registered_operation_schema, registered_operation_repeatable
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
        params![
            grant.id().to_string(),
            grant.run_id().to_string(),
            grant.capability().as_str(),
            resource_to_json(grant.resource())?.unwrap_or_else(|| "null".to_owned()),
            action_scope_to_json(grant.action_scope())?,
            grant.pair().map(|pair| pair.plan_version_id.to_string()),
            grant.pair().map(|pair| pair.graph_version_id.to_string()),
            grant.related_approval_id().map(|id| id.to_string()),
            grant.issuer(),
            grant.issued_at_unix_ms(),
            grant.expires_at_unix_ms(),
            grant.revoked_at_unix_ms(),
            i64::from(grant.dispatch_budget()),
            i64::from(grant.dispatch_budget_remaining()),
            op.operation_id(),
            op.schema_version(),
            i64::from(op.repeatable())
        ],
    )
    .map_err(SqliteStoreError::Database)?;
    Ok(())
}

fn insert_effect(tx: &Transaction<'_>, effect: &EffectRecord) -> Result<(), SqliteStoreError> {
    tx.execute(
        "INSERT INTO harness_effects (
            id, run_id, attempt_id, plan_version_id, graph_version_id, operation_id,
            operation_schema_version, target_hash, grant_id, phase, claimant, operation_result,
            certainty, prepared_at_unix_ms, claimed_at_unix_ms, dispatched_at_unix_ms,
            settled_at_unix_ms, expected_preimage_hash, expected_postimage_hash
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19
         )",
        params![
            effect.id().to_string(),
            effect.run_id().to_string(),
            effect.attempt_id().map(|id| id.to_string()),
            effect.plan_version_id().map(|id| id.to_string()),
            effect.graph_version_id().map(|id| id.to_string()),
            effect.operation_id(),
            effect.operation_schema_version(),
            effect.target_hash(),
            effect.grant_id().to_string(),
            effect.phase().as_str(),
            effect.claimant(),
            effect.operation_result().map(|value| value.as_str()),
            effect.certainty().map(|value| value.as_str()),
            effect.prepared_at_unix_ms(),
            effect.claimed_at_unix_ms(),
            effect.dispatched_at_unix_ms(),
            effect.settled_at_unix_ms(),
            effect.expected_preimage_hash(),
            effect.expected_postimage_hash()
        ],
    )
    .map_err(SqliteStoreError::Database)?;
    Ok(())
}

fn update_effect(tx: &Transaction<'_>, effect: &EffectRecord) -> Result<(), SqliteStoreError> {
    tx.execute(
        "UPDATE harness_effects SET
            phase = ?1, claimant = ?2, operation_result = ?3, certainty = ?4,
            claimed_at_unix_ms = ?5, dispatched_at_unix_ms = ?6, settled_at_unix_ms = ?7
         WHERE id = ?8 AND run_id = ?9",
        params![
            effect.phase().as_str(),
            effect.claimant(),
            effect.operation_result().map(|value| value.as_str()),
            effect.certainty().map(|value| value.as_str()),
            effect.claimed_at_unix_ms(),
            effect.dispatched_at_unix_ms(),
            effect.settled_at_unix_ms(),
            effect.id().to_string(),
            effect.run_id().to_string()
        ],
    )
    .map_err(SqliteStoreError::Database)?;
    Ok(())
}

fn upsert_lease(tx: &Transaction<'_>, lease: &RootLease) -> Result<(), SqliteStoreError> {
    tx.execute(
        "INSERT INTO harness_leases (
            run_id, id, owner_process_instance, acquired_at_unix_ms, expires_at_unix_ms, renew_interval_ms
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(run_id) DO UPDATE SET
            id = excluded.id,
            owner_process_instance = excluded.owner_process_instance,
            acquired_at_unix_ms = excluded.acquired_at_unix_ms,
            expires_at_unix_ms = excluded.expires_at_unix_ms,
            renew_interval_ms = excluded.renew_interval_ms",
        params![
            lease.run_id().to_string(),
            lease.id().to_string(),
            lease.owner_process_instance(),
            lease.acquired_at_unix_ms(),
            lease.expires_at_unix_ms(),
            lease.renew_interval_ms()
        ],
    )
    .map_err(SqliteStoreError::Database)?;
    Ok(())
}

fn load_run(connection: &Connection, run_id: &str) -> Result<Option<HarnessRun>, SqliteStoreError> {
    let row = connection
        .query_row(
            "SELECT id, run_root_display_name, created_at_unix_ms, cohort_json
             FROM harness_runs WHERE id = ?1",
            [run_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            },
        )
        .optional()
        .map_err(SqliteStoreError::Database)?;
    row.map(|(id, name, created, cohort)| {
        HarnessRun::from_stored_parts(&id, name, created, cohort_from_json(cohort.as_deref())?)
            .map_err(SqliteStoreError::MalformedHarness)
    })
    .transpose()
}

fn load_events(connection: &Connection, run_id: &str) -> Result<Vec<RunEvent>, SqliteStoreError> {
    let mut statement = connection
        .prepare(
            "SELECT id, run_id, sequence, kind_tag, kind_payload_json, recorded_at_unix_ms
             FROM harness_run_events WHERE run_id = ?1 ORDER BY sequence ASC",
        )
        .map_err(SqliteStoreError::Database)?;
    let rows = statement
        .query_map([run_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, i64>(5)?,
            ))
        })
        .map_err(SqliteStoreError::Database)?;
    let mut events = Vec::new();
    for row in rows {
        let (id, stored_run_id, sequence, kind_tag, payload, recorded) =
            row.map_err(SqliteStoreError::Database)?;
        let run_id =
            HarnessRunId::parse(&stored_run_id).map_err(SqliteStoreError::MalformedHarness)?;
        let kind = event_kind_from_parts(&kind_tag, &payload)?;
        events.push(
            RunEvent::from_stored_parts(
                &id,
                run_id,
                u64::try_from(sequence).map_err(|_| SqliteStoreError::Numeric)?,
                kind,
                recorded,
            )
            .map_err(SqliteStoreError::MalformedHarness)?,
        );
    }
    Ok(events)
}

fn load_replacements(
    connection: &Connection,
    run_id: &str,
) -> Result<Vec<ReplacementContentInput>, SqliteStoreError> {
    let mut statement = connection
        .prepare(
            "SELECT id, relative_target, preimage_hash, postimage_hash, expected_diff_hash,
                    postimage_utf8, provider_request_id, provider_response_id, created_at_unix_ms
             FROM harness_replacements WHERE run_id = ?1 ORDER BY created_at_unix_ms ASC, id ASC",
        )
        .map_err(SqliteStoreError::Database)?;
    let rows = statement
        .query_map([run_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, i64>(8)?,
            ))
        })
        .map_err(SqliteStoreError::Database)?;
    let mut out = Vec::new();
    for row in rows {
        let (id, target, pre, post, diff, utf8, req, resp, created) =
            row.map_err(SqliteStoreError::Database)?;
        out.push(
            ReplacementContentInput::from_stored_parts(
                &id, target, pre, post, diff, utf8, req, resp, created,
            )
            .map_err(SqliteStoreError::MalformedHarness)?,
        );
    }
    Ok(out)
}

fn load_graphs(
    connection: &Connection,
    run_id: &str,
) -> Result<Vec<RunGraphVersion>, SqliteStoreError> {
    let mut statement = connection
        .prepare(
            "SELECT id, nodes_json, edges_json, retry_rule, validation_rule, content_hash
             FROM harness_run_graphs WHERE run_id = ?1 ORDER BY id ASC",
        )
        .map_err(SqliteStoreError::Database)?;
    let rows = statement
        .query_map([run_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
            ))
        })
        .map_err(SqliteStoreError::Database)?;
    let mut out = Vec::new();
    for row in rows {
        let (id, nodes, edges, retry, validation, hash) =
            row.map_err(SqliteStoreError::Database)?;
        out.push(
            RunGraphVersion::from_stored_parts(
                &id,
                nodes_from_json(&nodes)?,
                edges_from_json(&edges)?,
                retry,
                validation,
                hash,
            )
            .map_err(SqliteStoreError::MalformedHarness)?,
        );
    }
    Ok(out)
}

fn load_plans(
    connection: &Connection,
    run_id: &str,
) -> Result<Vec<ExecutionPlanVersion>, SqliteStoreError> {
    let mut statement = connection
        .prepare(
            "SELECT p.id, p.graph_version_id, p.instructions, p.provider_profile_id, p.model_id,
                    p.disclosure_policy_id, p.disclosure_allowed, p.capability_envelope_json,
                    p.context_manifest_id, p.context_content_hash, p.context_request_semantic_hash,
                    p.context_disclosed_byte_count, p.context_summary, p.preimage_filesystem_identity,
                    p.execution_policy_revision, p.approval_hash, p.created_at_unix_ms,
                    r.id, r.relative_target, r.preimage_hash, r.postimage_hash, r.expected_diff_hash,
                    r.postimage_utf8, r.provider_request_id, r.provider_response_id, r.created_at_unix_ms
             FROM harness_execution_plans p
             INNER JOIN harness_replacements r ON r.id = p.replacement_id
             WHERE p.run_id = ?1
             ORDER BY p.created_at_unix_ms ASC, p.id ASC",
        )
        .map_err(SqliteStoreError::Database)?;
    let rows = statement
        .query_map([run_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, String>(9)?,
                row.get::<_, String>(10)?,
                row.get::<_, i64>(11)?,
                row.get::<_, String>(12)?,
                row.get::<_, String>(13)?,
                row.get::<_, String>(14)?,
                row.get::<_, String>(15)?,
                row.get::<_, i64>(16)?,
                row.get::<_, String>(17)?,
                row.get::<_, String>(18)?,
                row.get::<_, String>(19)?,
                row.get::<_, String>(20)?,
                row.get::<_, String>(21)?,
                row.get::<_, String>(22)?,
                row.get::<_, Option<String>>(23)?,
                row.get::<_, Option<String>>(24)?,
                row.get::<_, i64>(25)?,
            ))
        })
        .map_err(SqliteStoreError::Database)?;
    let mut out = Vec::new();
    for row in rows {
        let (
            id,
            graph_id,
            instructions,
            profile,
            model,
            disclosure_id,
            disclosure_allowed,
            envelope_json,
            manifest_id,
            content_hash,
            request_hash,
            disclosed_bytes,
            summary,
            fs_identity,
            policy_rev,
            approval_hash,
            created,
            replacement_id,
            relative_target,
            preimage_hash,
            postimage_hash,
            expected_diff_hash,
            postimage_utf8,
            provider_request_id,
            provider_response_id,
            replacement_created,
        ) = row.map_err(SqliteStoreError::Database)?;
        let graph_version_id = tule_core::RunGraphVersionId::parse(&graph_id)
            .map_err(SqliteStoreError::MalformedHarness)?;
        let replacement = ReplacementContentInput::from_stored_parts(
            &replacement_id,
            relative_target,
            preimage_hash,
            postimage_hash,
            expected_diff_hash,
            postimage_utf8,
            provider_request_id,
            provider_response_id,
            replacement_created,
        )
        .map_err(SqliteStoreError::MalformedHarness)?;
        let manifest = ContextManifest::from_stored_parts(
            &manifest_id,
            content_hash,
            request_hash,
            u64::try_from(disclosed_bytes).map_err(|_| SqliteStoreError::Numeric)?,
            summary,
        )
        .map_err(SqliteStoreError::MalformedHarness)?;
        out.push(
            ExecutionPlanVersion::from_stored_parts(
                &id,
                graph_version_id,
                instructions,
                profile,
                model,
                DisclosurePolicy::new(disclosure_id, disclosure_allowed),
                envelope_from_json(&envelope_json)?,
                manifest,
                replacement,
                fs_identity,
                policy_rev,
                approval_hash,
                created,
            )
            .map_err(SqliteStoreError::MalformedHarness)?,
        );
    }
    Ok(out)
}

fn load_approvals(
    connection: &Connection,
    run_id: &str,
) -> Result<Vec<ApprovalRecord>, SqliteStoreError> {
    let mut statement = connection
        .prepare(
            "SELECT id, run_id, plan_version_id, graph_version_id, approval_hash, approver, approved_at_unix_ms
             FROM harness_approvals WHERE run_id = ?1 ORDER BY approved_at_unix_ms ASC, id ASC",
        )
        .map_err(SqliteStoreError::Database)?;
    let rows = statement
        .query_map([run_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, i64>(6)?,
            ))
        })
        .map_err(SqliteStoreError::Database)?;
    let mut out = Vec::new();
    for row in rows {
        let (id, stored_run, plan, graph, hash, approver, approved) =
            row.map_err(SqliteStoreError::Database)?;
        out.push(
            ApprovalRecord::from_stored_parts(
                &id,
                HarnessRunId::parse(&stored_run).map_err(SqliteStoreError::MalformedHarness)?,
                tule_core::ExecutionPlanVersionId::parse(&plan)
                    .map_err(SqliteStoreError::MalformedHarness)?,
                tule_core::RunGraphVersionId::parse(&graph)
                    .map_err(SqliteStoreError::MalformedHarness)?,
                hash,
                approver,
                approved,
            )
            .map_err(SqliteStoreError::MalformedHarness)?,
        );
    }
    Ok(out)
}

fn load_grants(
    connection: &Connection,
    run_id: &str,
) -> Result<Vec<CapabilityGrant>, SqliteStoreError> {
    let mut statement = connection
        .prepare(
            "SELECT id FROM harness_grants WHERE run_id = ?1 ORDER BY issued_at_unix_ms ASC, id ASC",
        )
        .map_err(SqliteStoreError::Database)?;
    let rows = statement
        .query_map([run_id], |row| row.get::<_, String>(0))
        .map_err(SqliteStoreError::Database)?;
    let mut out = Vec::new();
    for row in rows {
        let id = row.map_err(SqliteStoreError::Database)?;
        out.push(load_grant(connection, &id)?.ok_or(SqliteStoreError::HarnessNotFound)?);
    }
    Ok(out)
}

fn load_grant(
    connection: &Connection,
    id: &str,
) -> Result<Option<CapabilityGrant>, SqliteStoreError> {
    let row = connection
        .query_row(
            "SELECT id, run_id, capability, resource_json, action_scope_json, pair_plan_version_id,
                    pair_graph_version_id, related_approval_id, issuer, issued_at_unix_ms,
                    expires_at_unix_ms, revoked_at_unix_ms, dispatch_budget, dispatch_budget_remaining,
                    registered_operation_id, registered_operation_schema, registered_operation_repeatable
             FROM harness_grants WHERE id = ?1",
            [id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, i64>(9)?,
                    row.get::<_, i64>(10)?,
                    row.get::<_, Option<i64>>(11)?,
                    row.get::<_, i64>(12)?,
                    row.get::<_, i64>(13)?,
                    row.get::<_, String>(14)?,
                    row.get::<_, String>(15)?,
                    row.get::<_, i64>(16)?,
                ))
            },
        )
        .optional()
        .map_err(SqliteStoreError::Database)?;
    row.map(
        |(
            id,
            run_id,
            capability,
            resource_json,
            action_scope_json,
            pair_plan,
            pair_graph,
            related_approval,
            issuer,
            issued,
            expires,
            revoked,
            budget,
            remaining,
            op_id,
            op_schema,
            op_repeatable,
        )| {
            let pair = match (pair_plan, pair_graph) {
                (Some(plan), Some(graph)) => Some(PlanGraphPairBinding {
                    plan_version_id: tule_core::ExecutionPlanVersionId::parse(&plan)
                        .map_err(SqliteStoreError::MalformedHarness)?,
                    graph_version_id: tule_core::RunGraphVersionId::parse(&graph)
                        .map_err(SqliteStoreError::MalformedHarness)?,
                }),
                (None, None) => None,
                _ => return Err(SqliteStoreError::MalformedHarnessPayload),
            };
            CapabilityGrant::from_stored_parts(
                &id,
                HarnessRunId::parse(&run_id).map_err(SqliteStoreError::MalformedHarness)?,
                CapabilityType::parse(&capability)
                    .map_err(|_| SqliteStoreError::MalformedHarnessPayload)?,
                resource_from_json(&resource_json)?,
                action_scope_from_json(&action_scope_json)?,
                pair,
                related_approval
                    .as_deref()
                    .map(tule_core::ApprovalRecordId::parse)
                    .transpose()
                    .map_err(SqliteStoreError::MalformedHarness)?,
                issuer,
                issued,
                expires,
                revoked,
                u32::try_from(budget).map_err(|_| SqliteStoreError::Numeric)?,
                u32::try_from(remaining).map_err(|_| SqliteStoreError::Numeric)?,
                RegisteredOperationIdentity::new(op_id, op_schema, op_repeatable != 0),
            )
            .map_err(|error| SqliteStoreError::MalformedHarnessGrant(error.to_string()))
        },
    )
    .transpose()
}

fn load_effects_for_run(
    connection: &Connection,
    run_id: &str,
) -> Result<Vec<EffectRecord>, SqliteStoreError> {
    let mut statement = connection
        .prepare(
            "SELECT id FROM harness_effects WHERE run_id = ?1 ORDER BY prepared_at_unix_ms ASC, id ASC",
        )
        .map_err(SqliteStoreError::Database)?;
    let rows = statement
        .query_map([run_id], |row| row.get::<_, String>(0))
        .map_err(SqliteStoreError::Database)?;
    let mut out = Vec::new();
    for row in rows {
        let id = row.map_err(SqliteStoreError::Database)?;
        out.push(load_effect(connection, &id)?.ok_or(SqliteStoreError::HarnessNotFound)?);
    }
    Ok(out)
}

fn load_effect(
    connection: &Connection,
    id: &str,
) -> Result<Option<EffectRecord>, SqliteStoreError> {
    let row = connection
        .query_row(
            "SELECT id, run_id, attempt_id, plan_version_id, graph_version_id, operation_id,
                    operation_schema_version, target_hash, grant_id, phase, claimant, operation_result,
                    certainty, prepared_at_unix_ms, claimed_at_unix_ms, dispatched_at_unix_ms,
                    settled_at_unix_ms, expected_preimage_hash, expected_postimage_hash
             FROM harness_effects WHERE id = ?1",
            [id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, String>(9)?,
                    row.get::<_, Option<String>>(10)?,
                    row.get::<_, Option<String>>(11)?,
                    row.get::<_, Option<String>>(12)?,
                    row.get::<_, i64>(13)?,
                    row.get::<_, Option<i64>>(14)?,
                    row.get::<_, Option<i64>>(15)?,
                    row.get::<_, Option<i64>>(16)?,
                    row.get::<_, Option<String>>(17)?,
                    row.get::<_, Option<String>>(18)?,
                ))
            },
        )
        .optional()
        .map_err(SqliteStoreError::Database)?;
    row.map(
        |(
            id,
            run_id,
            attempt_id,
            plan_version_id,
            graph_version_id,
            operation_id,
            operation_schema_version,
            target_hash,
            grant_id,
            phase,
            claimant,
            operation_result,
            certainty,
            prepared,
            claimed,
            dispatched,
            settled,
            expected_preimage,
            expected_postimage,
        )| {
            EffectRecord::from_stored_parts(
                &id,
                HarnessRunId::parse(&run_id).map_err(SqliteStoreError::MalformedHarness)?,
                attempt_id
                    .as_deref()
                    .map(tule_core::NodeAttemptId::parse)
                    .transpose()
                    .map_err(SqliteStoreError::MalformedHarness)?,
                plan_version_id
                    .as_deref()
                    .map(tule_core::ExecutionPlanVersionId::parse)
                    .transpose()
                    .map_err(SqliteStoreError::MalformedHarness)?,
                graph_version_id
                    .as_deref()
                    .map(tule_core::RunGraphVersionId::parse)
                    .transpose()
                    .map_err(SqliteStoreError::MalformedHarness)?,
                operation_id,
                operation_schema_version,
                target_hash,
                CapabilityGrantId::parse(&grant_id).map_err(SqliteStoreError::MalformedHarness)?,
                EffectJournalPhase::parse(&phase).map_err(SqliteStoreError::MalformedHarness)?,
                claimant,
                operation_result
                    .as_deref()
                    .map(EffectOperationResult::parse)
                    .transpose()
                    .map_err(SqliteStoreError::MalformedHarness)?,
                certainty
                    .as_deref()
                    .map(EffectCertainty::parse)
                    .transpose()
                    .map_err(SqliteStoreError::MalformedHarness)?,
                prepared,
                claimed,
                dispatched,
                settled,
                expected_preimage,
                expected_postimage,
            )
            .map_err(SqliteStoreError::MalformedHarness)
        },
    )
    .transpose()
}

fn load_checkpoints(
    connection: &Connection,
    run_id: &str,
) -> Result<Vec<Checkpoint>, SqliteStoreError> {
    let mut statement = connection
        .prepare(
            "SELECT id, run_id, last_event_sequence, event_chain_hash, plan_version_id,
                    graph_version_id, execution_policy_revision, expected_postimage_hash, created_at_unix_ms
             FROM harness_checkpoints WHERE run_id = ?1 ORDER BY created_at_unix_ms ASC, id ASC",
        )
        .map_err(SqliteStoreError::Database)?;
    let rows = statement
        .query_map([run_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, i64>(8)?,
            ))
        })
        .map_err(SqliteStoreError::Database)?;
    let mut out = Vec::new();
    for row in rows {
        let (id, stored_run, seq, chain, plan, graph, policy, post, created) =
            row.map_err(SqliteStoreError::Database)?;
        out.push(
            Checkpoint::from_stored_parts(
                &id,
                HarnessRunId::parse(&stored_run).map_err(SqliteStoreError::MalformedHarness)?,
                u64::try_from(seq).map_err(|_| SqliteStoreError::Numeric)?,
                chain,
                tule_core::ExecutionPlanVersionId::parse(&plan)
                    .map_err(SqliteStoreError::MalformedHarness)?,
                tule_core::RunGraphVersionId::parse(&graph)
                    .map_err(SqliteStoreError::MalformedHarness)?,
                policy,
                post,
                created,
            )
            .map_err(SqliteStoreError::MalformedHarness)?,
        );
    }
    Ok(out)
}

fn load_validations(
    connection: &Connection,
    run_id: &str,
) -> Result<Vec<ValidationResult>, SqliteStoreError> {
    let mut statement = connection
        .prepare(
            "SELECT id, run_id, plan_version_id, graph_version_id, label, approved_postimage_hash,
                    observed_postimage_hash, native_diff_hash, passed, validated_at_unix_ms
             FROM harness_validations WHERE run_id = ?1 ORDER BY validated_at_unix_ms ASC, id ASC",
        )
        .map_err(SqliteStoreError::Database)?;
    let rows = statement
        .query_map([run_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, i64>(8)?,
                row.get::<_, i64>(9)?,
            ))
        })
        .map_err(SqliteStoreError::Database)?;
    let mut out = Vec::new();
    for row in rows {
        let (id, stored_run, plan, graph, label, approved, observed, diff, passed, validated) =
            row.map_err(SqliteStoreError::Database)?;
        out.push(
            ValidationResult::from_stored_parts(
                &id,
                HarnessRunId::parse(&stored_run).map_err(SqliteStoreError::MalformedHarness)?,
                tule_core::ExecutionPlanVersionId::parse(&plan)
                    .map_err(SqliteStoreError::MalformedHarness)?,
                tule_core::RunGraphVersionId::parse(&graph)
                    .map_err(SqliteStoreError::MalformedHarness)?,
                label,
                approved,
                observed,
                diff,
                passed != 0,
                validated,
            )
            .map_err(SqliteStoreError::MalformedHarness)?,
        );
    }
    Ok(out)
}

fn load_denials(
    connection: &Connection,
    run_id: &str,
) -> Result<Vec<DenialEvidence>, SqliteStoreError> {
    let mut statement = connection
        .prepare(
            "SELECT id, run_id, reason, grant_id, resource_json, recorded_at_unix_ms
             FROM harness_denials WHERE run_id = ?1 ORDER BY recorded_at_unix_ms ASC, id ASC",
        )
        .map_err(SqliteStoreError::Database)?;
    let rows = statement
        .query_map([run_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, i64>(5)?,
            ))
        })
        .map_err(SqliteStoreError::Database)?;
    let mut out = Vec::new();
    for row in rows {
        let (id, stored_run, reason, grant_id, resource_json, recorded) =
            row.map_err(SqliteStoreError::Database)?;
        out.push(
            DenialEvidence::from_stored_parts(
                &id,
                HarnessRunId::parse(&stored_run).map_err(SqliteStoreError::MalformedHarness)?,
                reason,
                grant_id
                    .as_deref()
                    .map(CapabilityGrantId::parse)
                    .transpose()
                    .map_err(SqliteStoreError::MalformedHarness)?,
                resource_json
                    .as_deref()
                    .map(resource_from_json)
                    .transpose()?,
                recorded,
            )
            .map_err(SqliteStoreError::MalformedHarness)?,
        );
    }
    Ok(out)
}

fn load_lease(
    connection: &Connection,
    run_id: &str,
) -> Result<Option<RootLease>, SqliteStoreError> {
    let row = connection
        .query_row(
            "SELECT id, run_id, owner_process_instance, acquired_at_unix_ms, expires_at_unix_ms, renew_interval_ms
             FROM harness_leases WHERE run_id = ?1",
            [run_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                ))
            },
        )
        .optional()
        .map_err(SqliteStoreError::Database)?;
    row.map(|(id, stored_run, owner, acquired, expires, renew)| {
        RootLease::from_stored_parts(
            &id,
            HarnessRunId::parse(&stored_run).map_err(SqliteStoreError::MalformedHarness)?,
            owner,
            acquired,
            expires,
            renew,
        )
        .map_err(SqliteStoreError::MalformedHarness)
    })
    .transpose()
}

fn load_final_result(
    connection: &Connection,
    run_id: &str,
) -> Result<Option<FinalWorkResult>, SqliteStoreError> {
    let row = connection
        .query_row(
            "SELECT run_id, plan_version_id, graph_version_id, validation_label, publication_stopped,
                    instrumentation_json, fingerprint_algorithm, fingerprint_value, cohort_json,
                    completed_at_unix_ms
             FROM harness_final_results WHERE run_id = ?1",
            [run_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, i64>(9)?,
                ))
            },
        )
        .optional()
        .map_err(SqliteStoreError::Database)?;
    row.map(
        |(
            stored_run,
            plan,
            graph,
            label,
            publication_stopped,
            instrumentation_json,
            fingerprint_algorithm,
            fingerprint_value,
            cohort_json,
            completed,
        )| {
            Ok(FinalWorkResult::from_stored_parts(
                HarnessRunId::parse(&stored_run).map_err(SqliteStoreError::MalformedHarness)?,
                tule_core::ExecutionPlanVersionId::parse(&plan)
                    .map_err(SqliteStoreError::MalformedHarness)?,
                tule_core::RunGraphVersionId::parse(&graph)
                    .map_err(SqliteStoreError::MalformedHarness)?,
                label,
                publication_stopped != 0,
                instrumentation_from_json(&instrumentation_json)?,
                GraphShapeFingerprint::from_stored_parts(fingerprint_algorithm, fingerprint_value),
                cohort_from_json(cohort_json.as_deref())?,
                completed,
            ))
        },
    )
    .transpose()
}

fn event_kind_to_parts(kind: &RunEventKind) -> Result<(&'static str, Value), SqliteStoreError> {
    Ok(match kind {
        RunEventKind::RunCreated => ("run_created", json!({})),
        RunEventKind::PairFrozen {
            plan_version_id,
            graph_version_id,
            approval_hash,
        } => (
            "pair_frozen",
            json!({
                "plan_version_id": plan_version_id.to_string(),
                "graph_version_id": graph_version_id.to_string(),
                "approval_hash": approval_hash,
            }),
        ),
        RunEventKind::Approved { approval_id } => (
            "approved",
            json!({ "approval_id": approval_id.to_string() }),
        ),
        RunEventKind::GrantIssued { grant_id } => {
            ("grant_issued", json!({ "grant_id": grant_id.to_string() }))
        }
        RunEventKind::GrantRevoked { grant_id } => {
            ("grant_revoked", json!({ "grant_id": grant_id.to_string() }))
        }
        RunEventKind::Denied { denial_id } => {
            ("denied", json!({ "denial_id": denial_id.to_string() }))
        }
        RunEventKind::EffectPrepared { effect_id } => (
            "effect_prepared",
            json!({ "effect_id": effect_id.to_string() }),
        ),
        RunEventKind::EffectClaimed {
            effect_id,
            claimant,
        } => (
            "effect_claimed",
            json!({ "effect_id": effect_id.to_string(), "claimant": claimant }),
        ),
        RunEventKind::EffectDispatched { effect_id } => (
            "effect_dispatched",
            json!({ "effect_id": effect_id.to_string() }),
        ),
        RunEventKind::EffectSettled {
            effect_id,
            certainty,
        } => (
            "effect_settled",
            json!({
                "effect_id": effect_id.to_string(),
                "certainty": certainty.as_str(),
            }),
        ),
        RunEventKind::Checkpointed { checkpoint_id } => (
            "checkpointed",
            json!({ "checkpoint_id": checkpoint_id.to_string() }),
        ),
        RunEventKind::Validated { validation_id } => (
            "validated",
            json!({ "validation_id": validation_id.to_string() }),
        ),
        RunEventKind::Completed => ("completed", json!({})),
        RunEventKind::Paused => ("paused", json!({})),
        RunEventKind::Cancelled => ("cancelled", json!({})),
        RunEventKind::Abandoned => ("abandoned", json!({})),
        RunEventKind::LeaseAcquired { lease_id } => (
            "lease_acquired",
            json!({ "lease_id": lease_id.to_string() }),
        ),
        RunEventKind::LeaseReleased { lease_id } => (
            "lease_released",
            json!({ "lease_id": lease_id.to_string() }),
        ),
        RunEventKind::LeaseTakeover { lease_id } => (
            "lease_takeover",
            json!({ "lease_id": lease_id.to_string() }),
        ),
        RunEventKind::Resumed => ("resumed", json!({})),
    })
}

fn event_kind_from_parts(tag: &str, payload: &str) -> Result<RunEventKind, SqliteStoreError> {
    let value: Value =
        serde_json::from_str(payload).map_err(|_| SqliteStoreError::MalformedHarnessPayload)?;
    let text = |key: &str| -> Result<String, SqliteStoreError> {
        value
            .get(key)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or(SqliteStoreError::MalformedHarnessPayload)
    };
    Ok(match tag {
        "run_created" => RunEventKind::RunCreated,
        "pair_frozen" => RunEventKind::PairFrozen {
            plan_version_id: tule_core::ExecutionPlanVersionId::parse(&text("plan_version_id")?)
                .map_err(SqliteStoreError::MalformedHarness)?,
            graph_version_id: tule_core::RunGraphVersionId::parse(&text("graph_version_id")?)
                .map_err(SqliteStoreError::MalformedHarness)?,
            approval_hash: text("approval_hash")?,
        },
        "approved" => RunEventKind::Approved {
            approval_id: tule_core::ApprovalRecordId::parse(&text("approval_id")?)
                .map_err(SqliteStoreError::MalformedHarness)?,
        },
        "grant_issued" => RunEventKind::GrantIssued {
            grant_id: CapabilityGrantId::parse(&text("grant_id")?)
                .map_err(SqliteStoreError::MalformedHarness)?,
        },
        "grant_revoked" => RunEventKind::GrantRevoked {
            grant_id: CapabilityGrantId::parse(&text("grant_id")?)
                .map_err(SqliteStoreError::MalformedHarness)?,
        },
        "denied" => RunEventKind::Denied {
            denial_id: tule_core::DenialEvidenceId::parse(&text("denial_id")?)
                .map_err(SqliteStoreError::MalformedHarness)?,
        },
        "effect_prepared" => RunEventKind::EffectPrepared {
            effect_id: tule_core::EffectRecordId::parse(&text("effect_id")?)
                .map_err(SqliteStoreError::MalformedHarness)?,
        },
        "effect_claimed" => RunEventKind::EffectClaimed {
            effect_id: tule_core::EffectRecordId::parse(&text("effect_id")?)
                .map_err(SqliteStoreError::MalformedHarness)?,
            claimant: text("claimant")?,
        },
        "effect_dispatched" => RunEventKind::EffectDispatched {
            effect_id: tule_core::EffectRecordId::parse(&text("effect_id")?)
                .map_err(SqliteStoreError::MalformedHarness)?,
        },
        "effect_settled" => RunEventKind::EffectSettled {
            effect_id: tule_core::EffectRecordId::parse(&text("effect_id")?)
                .map_err(SqliteStoreError::MalformedHarness)?,
            certainty: EffectCertainty::parse(&text("certainty")?)
                .map_err(SqliteStoreError::MalformedHarness)?,
        },
        "checkpointed" => RunEventKind::Checkpointed {
            checkpoint_id: tule_core::CheckpointId::parse(&text("checkpoint_id")?)
                .map_err(SqliteStoreError::MalformedHarness)?,
        },
        "validated" => RunEventKind::Validated {
            validation_id: tule_core::ValidationResultId::parse(&text("validation_id")?)
                .map_err(SqliteStoreError::MalformedHarness)?,
        },
        "completed" => RunEventKind::Completed,
        "paused" => RunEventKind::Paused,
        "cancelled" => RunEventKind::Cancelled,
        "abandoned" => RunEventKind::Abandoned,
        "lease_acquired" => RunEventKind::LeaseAcquired {
            lease_id: tule_core::RootLeaseId::parse(&text("lease_id")?)
                .map_err(SqliteStoreError::MalformedHarness)?,
        },
        "lease_released" => RunEventKind::LeaseReleased {
            lease_id: tule_core::RootLeaseId::parse(&text("lease_id")?)
                .map_err(SqliteStoreError::MalformedHarness)?,
        },
        "lease_takeover" => RunEventKind::LeaseTakeover {
            lease_id: tule_core::RootLeaseId::parse(&text("lease_id")?)
                .map_err(SqliteStoreError::MalformedHarness)?,
        },
        "resumed" => RunEventKind::Resumed,
        _ => return Err(SqliteStoreError::MalformedHarnessPayload),
    })
}

fn resource_to_json(resource: &GrantResourceSelector) -> Result<Option<String>, SqliteStoreError> {
    let value = match resource {
        GrantResourceSelector::RunRoot => json!({ "kind": "run_root" }),
        GrantResourceSelector::RelativeTarget(path) => {
            json!({ "kind": "relative_target", "path": path })
        }
        GrantResourceSelector::ContextManifestHash(hash) => {
            json!({ "kind": "context_manifest", "hash": hash })
        }
        GrantResourceSelector::ReplacementTarget {
            relative_target,
            expected_preimage_hash,
            expected_postimage_hash,
        } => json!({
            "kind": "replacement",
            "relative_target": relative_target,
            "expected_preimage_hash": expected_preimage_hash,
            "expected_postimage_hash": expected_postimage_hash,
        }),
    };
    Ok(Some(value.to_string()))
}

fn resource_from_json(raw: &str) -> Result<GrantResourceSelector, SqliteStoreError> {
    let value: Value =
        serde_json::from_str(raw).map_err(|_| SqliteStoreError::MalformedHarnessPayload)?;
    match value.get("kind").and_then(Value::as_str) {
        Some("run_root") => Ok(GrantResourceSelector::RunRoot),
        Some("relative_target") => Ok(GrantResourceSelector::RelativeTarget(
            value
                .get("path")
                .and_then(Value::as_str)
                .ok_or(SqliteStoreError::MalformedHarnessPayload)?
                .to_owned(),
        )),
        Some("context_manifest") => Ok(GrantResourceSelector::ContextManifestHash(
            value
                .get("hash")
                .and_then(Value::as_str)
                .ok_or(SqliteStoreError::MalformedHarnessPayload)?
                .to_owned(),
        )),
        Some("replacement") => Ok(GrantResourceSelector::ReplacementTarget {
            relative_target: value
                .get("relative_target")
                .and_then(Value::as_str)
                .ok_or(SqliteStoreError::MalformedHarnessPayload)?
                .to_owned(),
            expected_preimage_hash: value
                .get("expected_preimage_hash")
                .and_then(Value::as_str)
                .ok_or(SqliteStoreError::MalformedHarnessPayload)?
                .to_owned(),
            expected_postimage_hash: value
                .get("expected_postimage_hash")
                .and_then(Value::as_str)
                .ok_or(SqliteStoreError::MalformedHarnessPayload)?
                .to_owned(),
        }),
        _ => Err(SqliteStoreError::MalformedHarnessPayload),
    }
}

fn action_scope_to_json(scope: &GrantActionScope) -> Result<String, SqliteStoreError> {
    let value = match scope {
        GrantActionScope::Run => json!({ "kind": "run" }),
        GrantActionScope::Node(node) => json!({ "kind": "node", "node": node }),
        GrantActionScope::Effect(id) => json!({ "kind": "effect", "id": id.to_string() }),
        GrantActionScope::Attempt(id) => json!({ "kind": "attempt", "id": id.to_string() }),
    };
    Ok(value.to_string())
}

fn action_scope_from_json(raw: &str) -> Result<GrantActionScope, SqliteStoreError> {
    let value: Value =
        serde_json::from_str(raw).map_err(|_| SqliteStoreError::MalformedHarnessPayload)?;
    match value.get("kind").and_then(Value::as_str) {
        Some("run") => Ok(GrantActionScope::Run),
        Some("node") => Ok(GrantActionScope::Node(
            value
                .get("node")
                .and_then(Value::as_str)
                .ok_or(SqliteStoreError::MalformedHarnessPayload)?
                .to_owned(),
        )),
        Some("effect") => Ok(GrantActionScope::Effect(
            tule_core::EffectRecordId::parse(
                value
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or(SqliteStoreError::MalformedHarnessPayload)?,
            )
            .map_err(SqliteStoreError::MalformedHarness)?,
        )),
        Some("attempt") => Ok(GrantActionScope::Attempt(
            tule_core::NodeAttemptId::parse(
                value
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or(SqliteStoreError::MalformedHarnessPayload)?,
            )
            .map_err(SqliteStoreError::MalformedHarness)?,
        )),
        _ => Err(SqliteStoreError::MalformedHarnessPayload),
    }
}

fn nodes_to_json(nodes: &[GraphNode]) -> Result<String, SqliteStoreError> {
    let values: Vec<Value> = nodes
        .iter()
        .map(|node| {
            json!({
                "kind": node.kind(),
                "responsibility": node.responsibility(),
                "model_assignment": node.model_assignment(),
                "protected_validation": node.is_protected_validation(),
            })
        })
        .collect();
    Ok(Value::Array(values).to_string())
}

fn nodes_from_json(raw: &str) -> Result<Vec<GraphNode>, SqliteStoreError> {
    let value: Value =
        serde_json::from_str(raw).map_err(|_| SqliteStoreError::MalformedHarnessPayload)?;
    let array = value
        .as_array()
        .ok_or(SqliteStoreError::MalformedHarnessPayload)?;
    array
        .iter()
        .map(|item| {
            Ok(GraphNode::new(
                item.get("kind")
                    .and_then(Value::as_str)
                    .ok_or(SqliteStoreError::MalformedHarnessPayload)?,
                item.get("responsibility")
                    .and_then(Value::as_str)
                    .ok_or(SqliteStoreError::MalformedHarnessPayload)?,
                item.get("model_assignment")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                item.get("protected_validation")
                    .and_then(Value::as_bool)
                    .ok_or(SqliteStoreError::MalformedHarnessPayload)?,
            ))
        })
        .collect()
}

fn edges_to_json(edges: &[GraphEdge]) -> Result<String, SqliteStoreError> {
    let values: Vec<Value> = edges
        .iter()
        .map(|edge| json!({ "from": edge.from_kind(), "to": edge.to_kind() }))
        .collect();
    Ok(Value::Array(values).to_string())
}

fn edges_from_json(raw: &str) -> Result<Vec<GraphEdge>, SqliteStoreError> {
    let value: Value =
        serde_json::from_str(raw).map_err(|_| SqliteStoreError::MalformedHarnessPayload)?;
    let array = value
        .as_array()
        .ok_or(SqliteStoreError::MalformedHarnessPayload)?;
    array
        .iter()
        .map(|item| {
            Ok(GraphEdge::new(
                item.get("from")
                    .and_then(Value::as_str)
                    .ok_or(SqliteStoreError::MalformedHarnessPayload)?,
                item.get("to")
                    .and_then(Value::as_str)
                    .ok_or(SqliteStoreError::MalformedHarnessPayload)?,
            ))
        })
        .collect()
}

fn envelope_to_json(envelope: &tule_core::CapabilityEnvelope) -> Result<String, SqliteStoreError> {
    Ok(json!({
        "summary": envelope.summary(),
        "requested": envelope
            .requested()
            .iter()
            .map(|capability| capability.as_str())
            .collect::<Vec<_>>(),
    })
    .to_string())
}

fn envelope_from_json(raw: &str) -> Result<tule_core::CapabilityEnvelope, SqliteStoreError> {
    let value: Value =
        serde_json::from_str(raw).map_err(|_| SqliteStoreError::MalformedHarnessPayload)?;
    let summary = value
        .get("summary")
        .and_then(Value::as_str)
        .ok_or(SqliteStoreError::MalformedHarnessPayload)?;
    let requested = value
        .get("requested")
        .and_then(Value::as_array)
        .ok_or(SqliteStoreError::MalformedHarnessPayload)?
        .iter()
        .map(|item| {
            CapabilityType::parse(
                item.as_str()
                    .ok_or(SqliteStoreError::MalformedHarnessPayload)?,
            )
            .map_err(|_| SqliteStoreError::MalformedHarnessPayload)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(tule_core::CapabilityEnvelope::new(requested, summary))
}

fn cohort_to_json(
    cohort: Option<&TaskCohortAssignment>,
) -> Result<Option<String>, SqliteStoreError> {
    Ok(cohort.map(|value| {
        json!({
            "taxonomy_version": value.taxonomy_version(),
            "cohort_id": value.cohort_id(),
            "assigning_authority": value.assigning_authority(),
            "rationale": value.rationale(),
            "assigned_at_unix_ms": value.assigned_at_unix_ms(),
        })
        .to_string()
    }))
}

fn cohort_from_json(raw: Option<&str>) -> Result<Option<TaskCohortAssignment>, SqliteStoreError> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let value: Value =
        serde_json::from_str(raw).map_err(|_| SqliteStoreError::MalformedHarnessPayload)?;
    Ok(Some(TaskCohortAssignment::new(
        value
            .get("taxonomy_version")
            .and_then(Value::as_str)
            .ok_or(SqliteStoreError::MalformedHarnessPayload)?,
        value
            .get("cohort_id")
            .and_then(Value::as_str)
            .ok_or(SqliteStoreError::MalformedHarnessPayload)?,
        value
            .get("assigning_authority")
            .and_then(Value::as_str)
            .ok_or(SqliteStoreError::MalformedHarnessPayload)?,
        value
            .get("rationale")
            .and_then(Value::as_str)
            .ok_or(SqliteStoreError::MalformedHarnessPayload)?,
        value
            .get("assigned_at_unix_ms")
            .and_then(Value::as_i64)
            .ok_or(SqliteStoreError::MalformedHarnessPayload)?,
    )))
}

fn instrumentation_to_json(
    instrumentation: &ComparisonInstrumentation,
) -> Result<String, SqliteStoreError> {
    Ok(json!({
        "time_to_first_provider_output_ms": instrumentation.time_to_first_provider_output_ms,
        "total_time_to_structural_result_ms": instrumentation.total_time_to_structural_result_ms,
        "provider_input_tokens": instrumentation.provider_input_tokens,
        "provider_output_tokens": instrumentation.provider_output_tokens,
        "provider_cached_tokens": instrumentation.provider_cached_tokens,
        "context_bytes_resent": instrumentation.context_bytes_resent,
        "model_turns": instrumentation.model_turns,
        "registered_operation_calls": instrumentation.registered_operation_calls,
        "validation_time_ms": instrumentation.validation_time_ms,
        "retries": instrumentation.retries,
        "task_success": instrumentation.task_success,
    })
    .to_string())
}

fn instrumentation_from_json(raw: &str) -> Result<ComparisonInstrumentation, SqliteStoreError> {
    let value: Value =
        serde_json::from_str(raw).map_err(|_| SqliteStoreError::MalformedHarnessPayload)?;
    Ok(ComparisonInstrumentation {
        time_to_first_provider_output_ms: value
            .get("time_to_first_provider_output_ms")
            .and_then(Value::as_u64),
        total_time_to_structural_result_ms: value
            .get("total_time_to_structural_result_ms")
            .and_then(Value::as_u64),
        provider_input_tokens: value.get("provider_input_tokens").and_then(Value::as_u64),
        provider_output_tokens: value.get("provider_output_tokens").and_then(Value::as_u64),
        provider_cached_tokens: value.get("provider_cached_tokens").and_then(Value::as_u64),
        context_bytes_resent: value.get("context_bytes_resent").and_then(Value::as_u64),
        model_turns: value
            .get("model_turns")
            .and_then(Value::as_u64)
            .map(|value| value as u32),
        registered_operation_calls: value
            .get("registered_operation_calls")
            .and_then(Value::as_u64)
            .map(|value| value as u32),
        validation_time_ms: value.get("validation_time_ms").and_then(Value::as_u64),
        retries: value
            .get("retries")
            .and_then(Value::as_u64)
            .map(|value| value as u32),
        task_success: value.get("task_success").and_then(Value::as_bool),
    })
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Barrier, Mutex};
    use std::thread;
    use tempfile::TempDir;
    use tule_core::{
        BOOTSTRAP_HEADING_AFTER, BOOTSTRAP_HEADING_BEFORE, CONTROLLED_RELATIVE_TARGET,
        CapabilityEnvelope, CapabilityType, Clock, DisclosurePolicy, EffectCertainty,
        EffectJournalPhase, EffectOperationResult, FakeClock, GrantActionScope,
        GrantResourceSelector, NODE_REPLACE_EXISTING_FILE_V1, OP_CREATE_OR_REPLACE_V1,
        PlanGraphPairBinding, RunRepository, TaskCohortAssignment, acquire_root_lease,
        approve_pair, checkpoint_run, claim_effect, compile_and_freeze_pair, create_run,
        dispatch_effect, issue_grant, prepare_effect, settle_effect,
    };

    use super::*;
    use crate::sqlite::DATABASE_FILENAME;

    fn preimage() -> String {
        format!("<!doctype html><html><body>{BOOTSTRAP_HEADING_BEFORE}<p>ok</p></body></html>")
    }

    fn postimage() -> String {
        format!("<!doctype html><html><body>{BOOTSTRAP_HEADING_AFTER}<p>ok</p></body></html>")
    }

    fn open_store() -> (TempDir, Arc<SqliteStore>) {
        let directory = tempfile::tempdir().unwrap();
        let store = Arc::new(SqliteStore::open(directory.path().join(DATABASE_FILENAME)).unwrap());
        (directory, store)
    }

    #[test]
    fn sqlite_run_round_trips_and_rejects_duplicate_claim() {
        let (_dir, store) = open_store();
        let clock = FakeClock::new(5_000);
        let cohort = TaskCohortAssignment::new(
            "tax-v1",
            "static-heading-fixture",
            "owner",
            "work 0022",
            clock.unix_ms(),
        );
        let run = create_run(
            store.as_ref(),
            "fixture-root",
            Some(cohort),
            clock.unix_ms(),
        )
        .unwrap();
        let manifest = tule_core::ContextManifest::new(&preimage(), "heading", "preview").unwrap();
        let (plan, graph) = compile_and_freeze_pair(
            store.as_ref(),
            run.id(),
            "change heading",
            "profile",
            "model",
            DisclosurePolicy::new("d1", "index.html"),
            CapabilityEnvelope::new(
                vec![
                    CapabilityType::CreateOrReplace,
                    CapabilityType::NativeInspection,
                ],
                "replace+inspect",
            ),
            manifest,
            &preimage(),
            postimage(),
            CONTROLLED_RELATIVE_TARGET,
            "fs-1",
            Some("req".to_owned()),
            Some("resp".to_owned()),
            clock.unix_ms(),
        )
        .unwrap();
        let approval = approve_pair(
            store.as_ref(),
            run.id(),
            &plan,
            &graph,
            "owner",
            clock.unix_ms(),
        )
        .unwrap();
        let grant = issue_grant(
            store.as_ref(),
            run.id(),
            CapabilityType::CreateOrReplace,
            GrantResourceSelector::ReplacementTarget {
                relative_target: CONTROLLED_RELATIVE_TARGET.to_owned(),
                expected_preimage_hash: plan.replacement().preimage_hash().to_owned(),
                expected_postimage_hash: plan.replacement().postimage_hash().to_owned(),
            },
            GrantActionScope::Node(NODE_REPLACE_EXISTING_FILE_V1.to_owned()),
            Some(PlanGraphPairBinding {
                plan_version_id: plan.id(),
                graph_version_id: graph.id(),
            }),
            Some(approval.id()),
            "owner",
            clock.unix_ms(),
        )
        .unwrap();
        acquire_root_lease(store.as_ref(), run.id(), "proc-1", clock.unix_ms()).unwrap();
        let effect = prepare_effect(
            store.as_ref(),
            run.id(),
            None,
            Some(plan.id()),
            Some(graph.id()),
            OP_CREATE_OR_REPLACE_V1,
            plan.replacement().postimage_hash(),
            grant.id(),
            clock.unix_ms(),
            Some(plan.replacement().preimage_hash().to_owned()),
            Some(plan.replacement().postimage_hash().to_owned()),
        )
        .unwrap();
        claim_effect(
            store.as_ref(),
            run.id(),
            effect.id(),
            "broker-a",
            clock.unix_ms(),
        )
        .unwrap();
        assert!(
            claim_effect(
                store.as_ref(),
                run.id(),
                effect.id(),
                "broker-b",
                clock.unix_ms()
            )
            .is_err()
        );
        dispatch_effect(
            store.as_ref(),
            run.id(),
            effect.id(),
            grant.id(),
            "broker-a",
            clock.unix_ms(),
        )
        .unwrap();
        settle_effect(
            store.as_ref(),
            run.id(),
            effect.id(),
            "broker-a",
            EffectOperationResult::Success,
            EffectCertainty::ConfirmedCommitted,
            clock.unix_ms(),
        )
        .unwrap();
        let checkpoint =
            checkpoint_run(store.as_ref(), run.id(), &plan, &graph, clock.unix_ms()).unwrap();
        let reconstructed = store.reconstruct_run(&run.id()).unwrap().unwrap();
        assert_eq!(reconstructed.approvals.len(), 1);
        assert_eq!(reconstructed.grants.len(), 1);
        assert_eq!(reconstructed.effects.len(), 1);
        assert_eq!(
            reconstructed.effects[0].phase(),
            EffectJournalPhase::Settled
        );
        assert_eq!(reconstructed.checkpoints[0].id(), checkpoint.id());
        assert!(
            reconstructed
                .events
                .windows(2)
                .all(|window| { window[0].sequence() + 1 == window[1].sequence() })
        );
    }

    #[test]
    fn lease_expiry_alone_does_not_take_over_without_positive_evidence() {
        let (_dir, store) = open_store();
        let clock = FakeClock::new(1_000);
        let run = create_run(store.as_ref(), "root", None, clock.unix_ms()).unwrap();
        acquire_root_lease(store.as_ref(), run.id(), "proc-1", clock.unix_ms()).unwrap();
        clock.advance(tule_core::ROOT_LEASE_TTL_MS + 1);
        let err = tule_core::takeover_root_lease(
            store.as_ref(),
            run.id(),
            "proc-2",
            false,
            clock.unix_ms(),
        )
        .unwrap_err();
        assert!(matches!(err, tule_core::LeaseUseCaseError::Lease(_)));
    }

    #[test]
    fn barrier_race_allows_only_one_effect_claimant() {
        let (_dir, store) = open_store();
        let clock = FakeClock::new(9_000);
        let run = create_run(store.as_ref(), "root", None, clock.unix_ms()).unwrap();
        let manifest = tule_core::ContextManifest::new(&preimage(), "heading", "preview").unwrap();
        let (plan, graph) = compile_and_freeze_pair(
            store.as_ref(),
            run.id(),
            "change heading",
            "profile",
            "model",
            DisclosurePolicy::new("d1", "index.html"),
            CapabilityEnvelope::new(vec![CapabilityType::CreateOrReplace], "replace"),
            manifest,
            &preimage(),
            postimage(),
            CONTROLLED_RELATIVE_TARGET,
            "fs-1",
            None,
            None,
            clock.unix_ms(),
        )
        .unwrap();
        let approval = approve_pair(
            store.as_ref(),
            run.id(),
            &plan,
            &graph,
            "owner",
            clock.unix_ms(),
        )
        .unwrap();
        let grant = issue_grant(
            store.as_ref(),
            run.id(),
            CapabilityType::CreateOrReplace,
            GrantResourceSelector::ReplacementTarget {
                relative_target: CONTROLLED_RELATIVE_TARGET.to_owned(),
                expected_preimage_hash: plan.replacement().preimage_hash().to_owned(),
                expected_postimage_hash: plan.replacement().postimage_hash().to_owned(),
            },
            GrantActionScope::Node(NODE_REPLACE_EXISTING_FILE_V1.to_owned()),
            Some(PlanGraphPairBinding {
                plan_version_id: plan.id(),
                graph_version_id: graph.id(),
            }),
            Some(approval.id()),
            "owner",
            clock.unix_ms(),
        )
        .unwrap();
        let effect = prepare_effect(
            store.as_ref(),
            run.id(),
            None,
            Some(plan.id()),
            Some(graph.id()),
            OP_CREATE_OR_REPLACE_V1,
            plan.replacement().postimage_hash(),
            grant.id(),
            clock.unix_ms(),
            Some(plan.replacement().preimage_hash().to_owned()),
            Some(plan.replacement().postimage_hash().to_owned()),
        )
        .unwrap();
        let barrier = Arc::new(Barrier::new(2));
        let wins = Arc::new(Mutex::new(0_u32));
        let mut handles = Vec::new();
        for name in ["a", "b"] {
            let store = Arc::clone(&store);
            let barrier = Arc::clone(&barrier);
            let wins = Arc::clone(&wins);
            let run_id = run.id();
            let effect_id = effect.id();
            let now = clock.unix_ms();
            handles.push(thread::spawn(move || {
                barrier.wait();
                if claim_effect(store.as_ref(), run_id, effect_id, name, now).is_ok() {
                    *wins.lock().unwrap() += 1;
                }
            }));
        }
        for handle in handles {
            handle.join().unwrap();
        }
        assert_eq!(*wins.lock().unwrap(), 1);
    }
}
