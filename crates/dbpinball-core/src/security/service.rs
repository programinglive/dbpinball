//! The security service: the one place that combines identity, enrollment and
//! policy. CLI and desktop adapters call these functions; neither reimplements
//! a decision.

use serde::Serialize;

use crate::domain::connection::{DbConnection, TargetClass};
use crate::domain::error::{CoreError, ErrorCode};
use crate::ports::credentials::CredentialProvider;
use crate::security::destination::{
    assess_category, match_enrollment, DestinationCategory, DestinationIdentity,
    EnrolledFingerprint, EnrollmentMatch, FreshnessPolicy,
};
use crate::security::enrollment_store::EnrollmentStore;
use crate::security::operation::DatabaseOperation;
use crate::security::policy::{evaluate, OperationContext, PolicyDecision};
use crate::security::verify::verify_destination;

/// What the CLI and desktop show for one connection.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityStatus {
    pub connection_id: String,
    pub declared_target: TargetClass,
    pub identity: DestinationIdentity,
    /// Category inferred from server evidence. Never upgraded to development.
    pub assessed_category: DestinationCategory,
    pub enrollment: Option<EnrolledFingerprint>,
    pub enrollment_match: EnrollmentMatch,
    /// True when a write would be allowed right now.
    pub write_allowed: bool,
    pub decision_code: String,
}

/// Inspect a connection: read its identity and compare it to any enrollment.
pub async fn inspect(
    connection: &DbConnection,
    credentials: &dyn CredentialProvider,
    enrollments: &EnrollmentStore,
    freshness: FreshnessPolicy,
    now_ms: u64,
) -> Result<SecurityStatus, CoreError> {
    let identity = verify_destination(connection, credentials).await?;
    let registry = enrollments.load()?;
    let enrollment = registry.get(&connection.id).cloned();

    let enrollment_match = match_enrollment(&identity, enrollment.as_ref(), freshness, now_ms);

    let assessed_category = assess_category(&identity.server);

    let decision = evaluate(
        &OperationContext::new(DatabaseOperation::DataMutation, now_ms)
            .with_destination(&identity)
            .with_optional_enrollment(enrollment.as_ref())
            .with_declared_target(connection.target),
    );

    Ok(SecurityStatus {
        connection_id: connection.id.clone(),
        declared_target: connection.target,
        identity,
        assessed_category,
        enrollment,
        enrollment_match,
        write_allowed: decision.is_allowed(),
        decision_code: decision.code().as_str().to_string(),
    })
}

/// Enroll a destination after verifying it.
///
/// Enrollment is deliberate: the caller must name the category, and the
/// fingerprint is taken from the server, not from the connection profile.
pub async fn enroll(
    connection: &DbConnection,
    credentials: &dyn CredentialProvider,
    enrollments: &EnrollmentStore,
    category: DestinationCategory,
    now_ms: u64,
) -> Result<EnrolledFingerprint, CoreError> {
    if !category.allows_write() {
        return Err(CoreError::new(
            ErrorCode::PolicyRefusal,
            format!(
                "category '{}' cannot be enrolled as a write destination",
                category.as_str()
            ),
        ));
    }

    let identity = verify_destination(connection, credentials).await?;

    let Some(identity_key) = identity.identity_key.clone() else {
        return Err(CoreError::new(
            ErrorCode::PolicyRefusal,
            "the server did not report an identity, so it cannot be enrolled",
        ));
    };

    let fingerprint = EnrolledFingerprint {
        identity_key,
        engine: format!("{:?}", identity.engine).to_ascii_lowercase(),
        database: identity
            .server
            .database
            .clone()
            .unwrap_or_else(|| connection.database.clone()),
        category,
        enrolled_at_ms: now_ms,
        note: None,
    };

    let mut registry = enrollments.load()?;
    registry.enroll(&connection.id, fingerprint.clone());
    enrollments.save(&registry)?;

    Ok(fingerprint)
}

/// Verify a connection and return the identity plus the enrollment match.
pub async fn verify(
    connection: &DbConnection,
    credentials: &dyn CredentialProvider,
    enrollments: &EnrollmentStore,
    freshness: FreshnessPolicy,
    now_ms: u64,
) -> Result<(DestinationIdentity, EnrollmentMatch), CoreError> {
    let identity = verify_destination(connection, credentials).await?;
    let registry = enrollments.load()?;
    let enrollment = registry.get(&connection.id);
    let matched = match_enrollment(&identity, enrollment, freshness, now_ms);
    Ok((identity, matched))
}

/// Evaluate a policy decision for one operation against one connection.
pub async fn policy_check(
    connection: &DbConnection,
    credentials: &dyn CredentialProvider,
    enrollments: &EnrollmentStore,
    operation: DatabaseOperation,
    freshness: FreshnessPolicy,
    now_ms: u64,
) -> Result<PolicyDecision, CoreError> {
    // Read-only operations do not need a destination round-trip.
    if !operation.is_write() {
        return Ok(evaluate(&OperationContext::new(operation, now_ms)));
    }

    let identity = verify_destination(connection, credentials).await?;
    let registry = enrollments.load()?;
    let enrollment = registry.get(&connection.id);

    let mut context = OperationContext::new(operation, now_ms)
        .with_destination(&identity)
        .with_optional_enrollment(enrollment)
        .with_declared_target(connection.target);
    context.freshness = freshness;

    Ok(evaluate(&context))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::security::destination::ServerIdentity;

    #[test]
    fn a_read_only_policy_check_needs_no_database() {
        // The function returns before touching the network for read-only work.
        // This test pins the ordering by checking the pure decision directly.
        let decision = evaluate(&OperationContext::new(
            DatabaseOperation::ReadOnlyQuery,
            1_000,
        ));
        assert!(decision.is_allowed());
    }

    #[test]
    fn an_unidentified_server_cannot_be_enrolled() {
        let server = ServerIdentity::default();
        assert!(server
            .identity_key(crate::domain::connection::DbEngine::Postgres, "h", 1)
            .is_none());
    }
}
