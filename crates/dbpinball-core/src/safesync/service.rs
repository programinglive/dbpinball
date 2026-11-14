//! The Safe Sync service: the one place that combines sessions, planning and the
//! security policy. Adapters call these functions and never reimplement a
//! decision.

use serde::Serialize;

use crate::domain::connection::DbConnection;
use crate::domain::error::{CoreError, ErrorCode};
use crate::ports::credentials::CredentialProvider;
use crate::safesync::graph::TableInfo;
use crate::safesync::key::MaskKeyStore;
use crate::safesync::mask::{Masker, MaskingPolicy};
use crate::safesync::plan::{check_schema_drift, DryRunPlan};
use crate::safesync::planner::{self, SelectionPlan};
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
) -> Result<DryRunPlan, CoreError> {
    spec.validate()?;
    let masker = masker_for(spec, keys)?;

    let root_tables = root_tables(spec)?;

    let mut source_session =
        BoundSession::open(SessionRole::Source, source, credentials, &spec.limits).await?;
    source_session.assert_source_role(&root_tables).await?;

    // Metadata discovery and (optionally) row counts run on the source session.
    let selection = planner::build(&mut source_session, spec, &masker).await?;

    let estimates = if estimate {
        estimate_rows(&mut source_session, spec, &selection, &masker).await?
    } else {
        Vec::new()
    };

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

    DryRunPlan::build(
        spec,
        source_session.identity(),
        destination_session.identity(),
        category,
        &selection,
        &masker,
        &estimates,
        now_ms,
    )
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
    if plan.is_expired(now_ms) {
        return Err(CoreError::with_detail(
            ErrorCode::Conflict,
            "the plan expired; regenerate it",
            "plan_expired",
        ));
    }

    let masker = masker_for(spec, keys)?;
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

    let selection = planner::build(&mut source_session, spec, &masker).await?;
    check_schema_drift(plan, &selection)?;
    let _ = source_session.rollback().await;

    check_filter_digests(plan, spec, &masker)?;

    Ok(ValidationReport {
        plan_fingerprint: plan.plan_fingerprint.clone(),
        source_identity_key: plan.source.identity_key.clone(),
        destination_identity_key: plan.destination.identity_key.clone(),
        tables_checked: selection.tables.len(),
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
        let canonical = root
            .filter
            .iter()
            .map(crate::safesync::spec::FilterPredicate::canonical)
            .collect::<Vec<_>>()
            .join("|");
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

async fn estimate_rows(
    session: &mut BoundSession,
    spec: &SafeSyncSpec,
    selection: &SelectionPlan,
    masker: &Masker,
) -> Result<Vec<(QualifiedName, u64)>, CoreError> {
    let mut estimates = Vec::new();
    for root in &spec.selection.roots {
        let name = QualifiedName::parse(&root.table)?;
        let Some(table) = selection.tables.iter().find(|table| table.name() == name) else {
            continue;
        };
        let rows = planner::extract_table(session, table, root, masker, &spec.limits).await?;
        estimates.push((name, rows.row_count()));
    }
    Ok(estimates)
}

fn filter_mismatch(message: &str) -> CoreError {
    CoreError::with_detail(ErrorCode::Usage, message, "spec_plan_mismatch")
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
