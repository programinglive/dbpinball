//! The Safe Sync service: the one place that combines sessions, planning and the
//! security policy. Adapters call these functions and never reimplement a
//! decision.

use serde::Serialize;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::domain::connection::DbConnection;
use crate::domain::error::{CoreError, ErrorCode};
use crate::ports::credentials::CredentialProvider;
use crate::safesync::audit::{AuditRecord, TableAudit};
use crate::safesync::graph::TableInfo;
use crate::safesync::import::{self, ConflictPolicy, TableImport};
use crate::safesync::key::MaskKeyStore;
use crate::safesync::mask::{MaskStrategy, Masker, MaskingPolicy};
use crate::safesync::plan::{assert_graph_matches, check_schema_drift, DryRunPlan, PlanIdentity};
use crate::safesync::planner::{self};
use crate::safesync::session::{BoundSession, SessionIdentity, SessionRole};
use crate::safesync::spec::{Limits, QualifiedName, SafeSyncSpec};
use crate::security::{
    evaluate, EnrollmentStore, FreshnessPolicy, OperationContext, PolicyDecision,
};

/// The outcome of a re-validation.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationReport {
    pub plan_fingerprint: String,
    pub source_identity_key: Option<String>,
    pub destination_identity_key: Option<String>,
    pub tables_checked: usize,
    pub valid: bool,
}

/// Read the identity of one connection through a bound session.
pub async fn inspect(
    connection: &DbConnection,
    credentials: &dyn CredentialProvider,
    limits: &Limits,
) -> Result<SessionIdentity, CoreError> {
    let mut session =
        BoundSession::open(SessionRole::Source, connection, credentials, limits).await?;
    let identity = session.identity().clone();
    let _ = session.rollback().await;
    Ok(identity)
}

/// Resolve the installed masking policy named by the spec.
pub fn resolve_policy(name: &str) -> Result<MaskingPolicy, CoreError> {
    match name {
        "development-default@1" | "development-default" => Ok(MaskingPolicy::builtin_default()),
        "development-default@2" => Ok(MaskingPolicy::builtin_v2()),
        other => Err(CoreError::with_detail(
            ErrorCode::Usage,
            format!("masking policy '{other}' is not installed"),
            "mask_policy_invalid",
        )),
    }
}

/// Build a masker from the spec and the key store.
pub fn masker_for(spec: &SafeSyncSpec, keys: &dyn MaskKeyStore) -> Result<Masker, CoreError> {
    let policy = resolve_policy(&spec.masking.policy)?;
    let key = keys.load()?;
    Masker::new(policy, key)
}

/// Produce a reviewable plan. No destination write happens here.
#[allow(clippy::too_many_arguments)]
pub async fn plan(
    spec: &SafeSyncSpec,
    source: &DbConnection,
    destination: &DbConnection,
    credentials: &dyn CredentialProvider,
    enrollments: &EnrollmentStore,
    keys: &dyn MaskKeyStore,
    now_ms: u64,
    estimate: bool,
    replace: bool,
) -> Result<DryRunPlan, CoreError> {
    spec.validate()?;
    let masker = masker_for(spec, keys)?;

    let root_tables = root_tables(spec)?;

    let mut source_session =
        BoundSession::open(SessionRole::Source, source, credentials, &spec.limits).await?;
    source_session.assert_source_role(&root_tables).await?;

    // Metadata discovery and (optionally) row counts run on the source session.
    let selection = planner::build(&mut source_session, spec, &masker, estimate).await?;

    let estimates: Vec<(QualifiedName, u64)> = Vec::new();

    let _ = source_session.rollback().await;

    let destination_session = BoundSession::open(
        SessionRole::Destination,
        destination,
        credentials,
        &spec.limits,
    )
    .await?;

    let registry = enrollments.load()?;
    let enrollment = registry.get(&destination.id);
    let category = enrollment
        .map(|entry| entry.category)
        .unwrap_or(crate::security::DestinationCategory::Unknown);

    authorize(
        &source_session,
        &destination_session,
        destination,
        enrollment,
        now_ms,
    )?;

    let mut plan = DryRunPlan::build(
        spec,
        source_session.identity(),
        destination_session.identity(),
        category,
        &selection,
        &masker,
        &estimates,
        now_ms,
    )?;

    // The conflict policy is the reviewed contract, so record it in the plan and
    // bind it into the fingerprint.
    if replace {
        plan.import.conflict_policy = "replace".to_string();
        plan.plan_fingerprint = plan.fingerprint();
    }

    // Authenticate the final document, including its issuance time and expiry.
    plan.seal(&masker);

    Ok(plan)
}

/// Re-check a plan against live identities, the catalogue and the policy.
#[allow(clippy::too_many_arguments)]
pub async fn validate(
    plan: &DryRunPlan,
    spec: &SafeSyncSpec,
    source: &DbConnection,
    destination: &DbConnection,
    credentials: &dyn CredentialProvider,
    enrollments: &EnrollmentStore,
    keys: &dyn MaskKeyStore,
    now_ms: u64,
) -> Result<ValidationReport, CoreError> {
    plan.check_version()?;
    plan.check_expiry_contract(now_ms)?;

    if plan.fingerprint() != plan.plan_fingerprint {
        return Err(CoreError::with_detail(
            ErrorCode::Conflict,
            "the plan file was modified after it was produced",
            "plan_tampered",
        ));
    }

    let masker = masker_for(spec, keys)?;
    plan.verify_authorization(&masker)?;

    if plan.is_expired(now_ms) {
        return Err(CoreError::with_detail(
            ErrorCode::Conflict,
            "the plan expired; regenerate it",
            "plan_expired",
        ));
    }

    if masker.policy().hash != plan.masking.policy_hash {
        return Err(CoreError::with_detail(
            ErrorCode::Conflict,
            "the masking policy changed since the plan was made",
            "mask_policy_changed",
        ));
    }
    let root_tables = root_tables(spec)?;

    let mut source_session =
        BoundSession::open(SessionRole::Source, source, credentials, &spec.limits).await?;
    source_session.assert_source_role(&root_tables).await?;

    let destination_session = BoundSession::open(
        SessionRole::Destination,
        destination,
        credentials,
        &spec.limits,
    )
    .await?;

    if source_session.identity().identity_key != plan.source.identity_key {
        return Err(CoreError::with_detail(
            ErrorCode::Conflict,
            "the source identity changed since the plan was made",
            "source_changed",
        ));
    }
    if destination_session.identity().identity_key != plan.destination.identity_key {
        return Err(CoreError::with_detail(
            ErrorCode::Conflict,
            "the destination identity changed since the plan was made",
            "destination_changed",
        ));
    }

    let registry = enrollments.load()?;
    let enrollment = registry.get(&destination.id);
    authorize(
        &source_session,
        &destination_session,
        destination,
        enrollment,
        now_ms,
    )?;

    let selection = planner::build(&mut source_session, spec, &masker, true).await?;
    check_schema_drift(plan, &selection)?;
    assert_graph_matches(plan, &selection)?;
    if !selection.complete {
        return Err(CoreError::with_detail(
            ErrorCode::Operation,
            "the selection is incomplete and cannot be imported",
            "limit_exceeded",
        ));
    }
    let _ = source_session.rollback().await;

    check_filter_digests(plan, spec, &masker)?;

    Ok(ValidationReport {
        plan_fingerprint: plan.plan_fingerprint.clone(),
        source_identity_key: plan.source.identity_key.clone(),
        destination_identity_key: plan.destination.identity_key.clone(),
        tables_checked: selection.rows.len(),
        valid: true,
    })
}

/// Compare the plan's keyed filter digests with the specification's.
pub fn check_filter_digests(
    plan: &DryRunPlan,
    spec: &SafeSyncSpec,
    masker: &Masker,
) -> Result<(), CoreError> {
    if plan.selection.roots.len() != spec.selection.roots.len() {
        return Err(filter_mismatch("the plan does not match the specification"));
    }

    for (planned, root) in plan.selection.roots.iter().zip(&spec.selection.roots) {
        let canonical = crate::safesync::spec::canonical_filter(&root.filter);
        let digest = masker.filter_digest(&planned.schema, &planned.table, &canonical);
        if digest != planned.filter_digest {
            return Err(filter_mismatch(
                "the specification does not match the plan's filter digest",
            ));
        }
    }

    Ok(())
}

fn authorize(
    source: &BoundSession,
    destination: &BoundSession,
    destination_profile: &DbConnection,
    enrollment: Option<&crate::security::EnrolledFingerprint>,
    now_ms: u64,
) -> Result<(), CoreError> {
    let source_identity = source.identity().as_destination_identity(now_ms);
    let destination_identity = destination.identity().as_destination_identity(now_ms);

    let mut context =
        OperationContext::new(crate::security::DatabaseOperation::SafeSyncExecute, now_ms)
            .with_source(&source_identity)
            .with_destination(&destination_identity)
            .with_optional_enrollment(enrollment)
            .with_declared_target(destination_profile.target);
    context.freshness = FreshnessPolicy::default();

    match evaluate(&context) {
        PolicyDecision::Allow { .. } => Ok(()),
        decision => Err(refusal(decision)),
    }
}

fn refusal(decision: PolicyDecision) -> CoreError {
    let code = decision.code().as_str();
    let reason = match &decision {
        PolicyDecision::Deny { reason, .. } => reason.clone(),
        PolicyDecision::RequireVerification { reason, .. } => reason.clone(),
        PolicyDecision::Allow { .. } => String::new(),
    };
    CoreError::with_detail(
        ErrorCode::PolicyRefusal,
        format!("Safe Sync refused: {reason}"),
        code,
    )
}

fn root_tables(spec: &SafeSyncSpec) -> Result<Vec<QualifiedName>, CoreError> {
    spec.selection
        .roots
        .iter()
        .map(|root| QualifiedName::parse(&root.table))
        .collect()
}

fn filter_mismatch(message: &str) -> CoreError {
    CoreError::with_detail(ErrorCode::Usage, message, "spec_plan_mismatch")
}

/// Everything `execute` needs.
pub struct ExecuteRequest<'a> {
    pub plan: &'a DryRunPlan,
    pub spec: &'a SafeSyncSpec,
    pub acknowledgement: &'a str,
    pub replace: bool,
    pub source: &'a DbConnection,
    pub destination: &'a DbConnection,
}

/// The result of a successful execution.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunReport {
    pub run_id: String,
    pub plan_fingerprint: String,
    pub already_completed: bool,
    pub rows_written: u64,
    pub destination_digest: String,
    pub tables: Vec<TableImport>,
    /// completed | not_required | pending (post-commit sequence reconciliation).
    pub sequence_status: String,
    pub status: String,
}

/// The state of a recorded run.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunStatus {
    pub run_id: String,
    pub plan_fingerprint: String,
    pub status: String,
    pub rows_written: u64,
    pub destination_digest: String,
    pub sequence_status: String,
    pub finished_at: String,
}

/// Execute a plan. This is the only function that writes to a destination.
#[allow(clippy::too_many_arguments)]
pub async fn execute(
    request: ExecuteRequest<'_>,
    credentials: &dyn CredentialProvider,
    enrollments: &EnrollmentStore,
    keys: &dyn MaskKeyStore,
    audit_dir: Option<&Path>,
    now_ms: u64,
    cancel: Arc<AtomicBool>,
) -> Result<RunReport, CoreError> {
    request.spec.validate()?;
    request.plan.check_version()?;
    request.plan.check_expiry_contract(now_ms)?;

    // The plan file is the reviewed contract. Recompute its fingerprint so a
    // hand-edited plan cannot silently change the schema hash, key id or
    // conflict policy under the fingerprint the operator acknowledged.
    if request.plan.fingerprint() != request.plan.plan_fingerprint {
        return Err(CoreError::with_detail(
            ErrorCode::Conflict,
            "the plan file was modified after it was produced",
            "plan_tampered",
        ));
    }

    let masker = masker_for(request.spec, keys)?;

    // A plan is bound to one mask key. Running it with a rotated key would
    // produce different pseudonyms for the same rows, so refuse instead. This
    // is checked before the authorization MAC so a rotation is reported as a
    // key change, not as tampering.
    if masker.key_id() != request.plan.masking.key_id {
        return Err(CoreError::with_detail(
            ErrorCode::Conflict,
            "the masking key changed since the plan was made",
            "mask_key_changed",
        ));
    }

    // The authorization envelope binds the content, the issuance time, the
    // expiry, the run id and the key id. A re-dated plan fails here even when
    // its public fingerprint was recomputed.
    request.plan.verify_authorization(&masker)?;

    if request.plan.is_expired(now_ms) {
        return Err(CoreError::with_detail(
            ErrorCode::Conflict,
            "the plan expired; regenerate it",
            "plan_expired",
        ));
    }

    if !request.plan.complete {
        return Err(CoreError::with_detail(
            ErrorCode::Operation,
            "the plan is incomplete and cannot be executed",
            "limit_exceeded",
        ));
    }

    if request.acknowledgement != request.plan.plan_fingerprint {
        return Err(CoreError::with_detail(
            ErrorCode::Usage,
            "the acknowledgement does not match the plan fingerprint",
            "acknowledgement_mismatch",
        ));
    }

    if masker.policy().hash != request.plan.masking.policy_hash {
        return Err(CoreError::with_detail(
            ErrorCode::Conflict,
            "the masking policy changed since the plan was made",
            "mask_policy_changed",
        ));
    }

    // The conflict policy is part of the reviewed plan. A plan reviewed as
    // non-destructive cannot be run with `--replace`.
    let plan_replaces = request.plan.import.conflict_policy == "replace";
    if request.replace != plan_replaces {
        return Err(CoreError::with_detail(
            ErrorCode::Usage,
            "the conflict policy does not match the plan",
            "conflict_policy_mismatch",
        ));
    }

    let root_tables = root_tables(request.spec)?;

    let mut source_session = BoundSession::open(
        SessionRole::Source,
        request.source,
        credentials,
        &request.plan.limits,
    )
    .await?;
    assert_identity(
        &source_session.identity().identity_key,
        &request.plan.source,
        "source_changed",
    )?;
    source_session.assert_source_role(&root_tables).await?;

    // The specification is not trusted on its own. `planner::build` refuses a
    // non-zero depth or child traversal, and the resulting contract must match
    // the reviewed plan byte for byte.
    let selection = planner::build(&mut source_session, request.spec, &masker, true).await?;
    if crate::safesync::plan::spec_hash(request.spec, &selection) != request.plan.spec_hash {
        return Err(filter_mismatch("the specification does not match the plan"));
    }
    check_filter_digests(request.plan, request.spec, &masker)?;
    assert_graph_matches(request.plan, &selection)?;

    if !selection.complete {
        return Err(CoreError::with_detail(
            ErrorCode::Operation,
            "the selection is incomplete and cannot be imported",
            "limit_exceeded",
        ));
    }

    // The selection is already masked and ordered. Compare its schema hashes to
    // the reviewed plan before any destination work.
    for planned in &request.plan.tables {
        let Some(live) = selection
            .rows
            .iter()
            .find(|rows| rows.table.schema == planned.schema && rows.table.table == planned.table)
        else {
            return Err(filter_mismatch(
                "the selection changed since the plan was made",
            ));
        };
        if live.schema_hash != planned.schema_hash {
            return Err(CoreError::with_detail(
                ErrorCode::Conflict,
                format!(
                    "the schema of {}.{} changed since the plan was made",
                    planned.schema, planned.table
                ),
                "schema_drift",
            ));
        }
    }

    if cancel.load(Ordering::Relaxed) {
        return Err(CoreError::with_detail(
            ErrorCode::Operation,
            "the run was cancelled",
            "cancelled",
        ));
    }

    let extracted = selection.rows.clone();
    let total_rows: u64 = extracted.iter().map(|rows| rows.row_count()).sum();
    let total_bytes: u64 = extracted.iter().map(|rows| rows.total_bytes).sum();
    if total_rows > request.plan.limits.max_total_rows
        || total_bytes > request.plan.limits.max_bytes
    {
        return Err(CoreError::with_detail(
            ErrorCode::Operation,
            "the selection exceeded a bound",
            "limit_exceeded",
        ));
    }

    let _ = source_session.rollback().await;

    let mut destination_session = BoundSession::open(
        SessionRole::Destination,
        request.destination,
        credentials,
        &request.plan.limits,
    )
    .await?;
    assert_identity(
        &destination_session.identity().identity_key,
        &request.plan.destination,
        "destination_changed",
    )?;

    let registry = enrollments.load()?;
    let enrollment = registry.get(&request.destination.id);
    authorize(
        &source_session,
        &destination_session,
        request.destination,
        enrollment,
        now_ms,
    )?;

    let policy = if plan_replaces {
        ConflictPolicy::Replace
    } else {
        ConflictPolicy::Refuse
    };

    let report = import::run(
        &mut destination_session,
        request.plan,
        &extracted,
        policy,
        cancel,
    )
    .await?;

    // Sequence adjustment is not part of the row transaction. It runs here,
    // after the commit, and is idempotent. A failure leaves the marker pending.
    let sequence_status =
        import::reconcile_sequences(&mut destination_session, &extracted, &report.run_id)
            .await
            .unwrap_or_else(|_| "pending".to_string());

    if let Some(directory) = audit_dir {
        let record = AuditRecord {
            run_id: report.run_id.clone(),
            plan_fingerprint: request.plan.plan_fingerprint.clone(),
            destination_connection_id: request.destination.id.clone(),
            source_identity_key: request.plan.source.identity_key.clone(),
            destination_identity_key: request.plan.destination.identity_key.clone(),
            tables: extracted
                .iter()
                .map(|rows| TableAudit {
                    schema: rows.table.schema.clone(),
                    table: rows.table.table.clone(),
                    rows: rows.row_count(),
                    masked_columns: rows
                        .columns
                        .iter()
                        .filter(|column| {
                            masker
                                .policy()
                                .rule_for(&rows.table.schema, &rows.table.table, column)
                                .map(|rule| rule.strategy != MaskStrategy::Keep)
                                .unwrap_or(true)
                        })
                        .count() as u64,
                })
                .collect(),
            policy: masker.policy().identifier(),
            key_id: masker.key_id().to_string(),
            outcome: if report.already_completed {
                "already_completed".to_string()
            } else {
                "completed".to_string()
            },
            started_at_ms: now_ms,
            finished_at_ms: now_ms,
        };
        let _ = crate::safesync::audit::write_record(directory, &record);
    }

    Ok(RunReport {
        run_id: report.run_id,
        plan_fingerprint: request.plan.plan_fingerprint.clone(),
        already_completed: report.already_completed,
        rows_written: report.rows_written,
        destination_digest: report.destination_digest,
        tables: report.tables,
        status: if report.already_completed {
            "already_completed".to_string()
        } else {
            "completed".to_string()
        },
        sequence_status,
    })
}

/// Read a recorded run's state from the destination.
pub async fn status(
    run_id: &str,
    destination: &DbConnection,
    credentials: &dyn CredentialProvider,
    limits: &Limits,
) -> Result<RunStatus, CoreError> {
    let mut session =
        BoundSession::open(SessionRole::Destination, destination, credentials, limits).await?;

    let statement = format!(
        "SELECT run_id::text, plan_fingerprint, status, rows_written, destination_digest, sequence_status, finished_at::text
         FROM {}
         WHERE run_id = $1::uuid",
        crate::safesync::import::RUN_TABLE
    );

    let row = sqlx::query(&statement)
        .bind(run_id)
        .fetch_optional(&mut *session.connection())
        .await
        .map_err(|error| {
            crate::safesync::database_error(
                ErrorCode::Connection,
                "the run state read",
                "run_state_unreadable",
                error,
            )
        })?;

    let Some(row) = row else {
        return Err(CoreError::with_detail(
            ErrorCode::NotFound,
            format!("run '{run_id}' was not found on the destination"),
            "run_not_found",
        ));
    };

    use sqlx::Row;
    Ok(RunStatus {
        run_id: row.try_get(0).unwrap_or_default(),
        plan_fingerprint: row.try_get(1).unwrap_or_default(),
        status: row.try_get(2).unwrap_or_default(),
        rows_written: row.try_get::<i64, _>(3).unwrap_or(0) as u64,
        destination_digest: row.try_get(4).unwrap_or_default(),
        sequence_status: row.try_get(5).unwrap_or_else(|_| "pending".to_string()),
        finished_at: row.try_get(6).unwrap_or_default(),
    })
}

fn assert_identity(
    session_key: &Option<String>,
    planned: &PlanIdentity,
    code: &'static str,
) -> Result<(), CoreError> {
    if *session_key != planned.identity_key {
        return Err(CoreError::with_detail(
            ErrorCode::Conflict,
            "the database identity changed since the plan was made",
            code,
        ));
    }
    Ok(())
}

/// Re-exported so a later slice can import one table without reaching into the
/// planner module.
pub async fn inspect_table(
    session: &mut BoundSession,
    table: &QualifiedName,
) -> Result<TableInfo, CoreError> {
    crate::safesync::graph::inspect_table(session, table).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_builtin_policy_resolves() {
        assert!(resolve_policy("development-default@1").is_ok());
        assert!(resolve_policy("development-default").is_ok());
    }

    #[test]
    fn an_unknown_policy_is_refused() {
        let error = resolve_policy("production-safe@7").unwrap_err();
        assert_eq!(error.detail.as_deref(), Some("mask_policy_invalid"));
    }
}
