//! The policy decision contract.
//!
//! Every dangerous operation asks this module one question: may this operation
//! run against this destination, right now? The answer is typed, deterministic,
//! and fail-closed. It never consults a UI, a label, or a confirmation dialog.

use serde::Serialize;

use crate::domain::connection::TargetClass;
use crate::security::destination::{
    match_enrollment, DestinationCategory, DestinationIdentity, EnrolledFingerprint,
    EnrollmentMatch, FreshnessPolicy,
};
use crate::security::operation::DatabaseOperation;

/// Stable refusal codes. A caller may branch on these; the message is for a
/// human and may change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyCode {
    /// The operation is read-only; no destination authorization is required.
    ReadOnlyAllowed,
    /// The destination was verified and the operation is allowed.
    Allowed,
    /// No destination identity was supplied for a write.
    MissingDestination,
    /// No source identity was supplied for a protected read.
    MissingSource,
    /// The source server did not identify itself.
    UnverifiableSource,
    /// The source session is not database-enforced read-only.
    SourceNotReadOnly,
    /// The source was authorized for a bounded read.
    SourceAuthorized,
    /// The destination could not be identified by the server.
    UnverifiableDestination,
    /// No fingerprint was enrolled for this connection.
    NotEnrolled,
    /// The server identity no longer matches the enrolled fingerprint.
    DestinationChanged,
    /// The verification evidence is older than the freshness policy.
    StaleVerification,
    /// The destination category does not allow writes.
    DestinationNotAllowed,
    /// The source and destination resolve to the same database.
    SameSourceAndDestination,
    /// A protected source was used as a write destination.
    ProtectedSource,
    /// The operation is not implemented in this milestone.
    Unsupported,
}

impl PolicyCode {
    pub fn as_str(self) -> &'static str {
        match self {
            PolicyCode::ReadOnlyAllowed => "read_only_allowed",
            PolicyCode::Allowed => "allowed",
            PolicyCode::MissingDestination => "missing_destination",
            PolicyCode::MissingSource => "missing_source",
            PolicyCode::UnverifiableSource => "unverifiable_source",
            PolicyCode::SourceNotReadOnly => "source_not_read_only",
            PolicyCode::SourceAuthorized => "source_authorized",
            PolicyCode::UnverifiableDestination => "unverifiable_destination",
            PolicyCode::NotEnrolled => "not_enrolled",
            PolicyCode::DestinationChanged => "destination_changed",
            PolicyCode::StaleVerification => "stale_verification",
            PolicyCode::DestinationNotAllowed => "destination_not_allowed",
            PolicyCode::SameSourceAndDestination => "same_source_and_destination",
            PolicyCode::ProtectedSource => "protected_source",
            PolicyCode::Unsupported => "unsupported",
        }
    }
}

/// The decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum PolicyDecision {
    /// The operation may run.
    Allow {
        code: PolicyCode,
        /// Human-readable evidence, never containing a secret.
        evidence: String,
    },
    /// The operation must not run.
    Deny { code: PolicyCode, reason: String },
    /// The operation may run only after the destination is verified or enrolled.
    RequireVerification { code: PolicyCode, reason: String },
}

impl PolicyDecision {
    pub fn is_allowed(&self) -> bool {
        matches!(self, PolicyDecision::Allow { .. })
    }

    pub fn code(&self) -> PolicyCode {
        match self {
            PolicyDecision::Allow { code, .. } => *code,
            PolicyDecision::Deny { code, .. } => *code,
            PolicyDecision::RequireVerification { code, .. } => *code,
        }
    }
}

/// Everything the policy needs to decide.
#[derive(Debug, Clone)]
pub struct OperationContext<'a> {
    pub operation: DatabaseOperation,
    /// The database being read, for source-to-destination workflows.
    pub source: Option<&'a DestinationIdentity>,
    /// The database being written.
    pub destination: Option<&'a DestinationIdentity>,
    /// The user's declared label. Recorded, never trusted.
    pub declared_target: TargetClass,
    /// The fingerprint the user enrolled for this connection, if any.
    pub enrollment: Option<&'a EnrolledFingerprint>,
    pub freshness: FreshnessPolicy,
    /// Unix milliseconds.
    pub now_ms: u64,
}

impl<'a> OperationContext<'a> {
    pub fn new(operation: DatabaseOperation, now_ms: u64) -> Self {
        Self {
            operation,
            source: None,
            destination: None,
            declared_target: TargetClass::Unknown,
            enrollment: None,
            freshness: FreshnessPolicy::default(),
            now_ms,
        }
    }

    pub fn with_destination(mut self, destination: &'a DestinationIdentity) -> Self {
        self.destination = Some(destination);
        self
    }

    pub fn with_source(mut self, source: &'a DestinationIdentity) -> Self {
        self.source = Some(source);
        self
    }

    pub fn with_enrollment(mut self, enrollment: &'a EnrolledFingerprint) -> Self {
        self.enrollment = Some(enrollment);
        self
    }

    /// Set the enrollment from an optional lookup.
    pub fn with_optional_enrollment(mut self, enrollment: Option<&'a EnrolledFingerprint>) -> Self {
        self.enrollment = enrollment;
        self
    }

    pub fn with_declared_target(mut self, target: TargetClass) -> Self {
        self.declared_target = target;
        self
    }
}

/// Decide whether an operation may run.
///
/// The order matters: read-only operations are allowed without a destination,
/// and every write path fails closed.
pub fn evaluate(context: &OperationContext<'_>) -> PolicyDecision {
    // Safe Sync is the only operation that reads a protected source. It is a
    // read of the source, so it must not require destination enrollment, but it
    // must still fail closed on a missing or ambiguous source.
    if context.operation == DatabaseOperation::SafeSyncPrepare {
        let Some(source) = context.source else {
            return PolicyDecision::Deny {
                code: PolicyCode::MissingSource,
                reason: "reading a protected source needs a source identity".to_string(),
            };
        };

        if !source.server.is_identified() {
            return PolicyDecision::Deny {
                code: PolicyCode::UnverifiableSource,
                reason: "the source did not report an identity, so it cannot be trusted"
                    .to_string(),
            };
        }

        if !source.server.read_only.unwrap_or(false) {
            return PolicyDecision::Deny {
                code: PolicyCode::SourceNotReadOnly,
                reason: "the source session is not database-enforced read-only".to_string(),
            };
        }

        if let Some(destination) = context.destination {
            if source.identity_key.is_some() && source.identity_key == destination.identity_key {
                return PolicyDecision::Deny {
                    code: PolicyCode::SameSourceAndDestination,
                    reason: "source and destination resolve to the same database".to_string(),
                };
            }
        }

        return PolicyDecision::Allow {
            code: PolicyCode::SourceAuthorized,
            evidence: format!(
                "source {} is identified and read-only",
                source.identity_key.as_deref().unwrap_or("(unknown)")
            ),
        };
    }

    // Executing a Safe Sync needs the source authorization as well as the
    // destination checks below. The source half is enforced by the session.
    if context.operation == DatabaseOperation::SafeSyncExecute && context.source.is_none() {
        return PolicyDecision::Deny {
            code: PolicyCode::MissingSource,
            reason: "executing a Safe Sync needs an authorized source".to_string(),
        };
    }

    if !context.operation.is_write() {
        return PolicyDecision::Allow {
            code: PolicyCode::ReadOnlyAllowed,
            evidence: format!("{} does not write", context.operation.as_str()),
        };
    }

    let Some(destination) = context.destination else {
        return PolicyDecision::Deny {
            code: PolicyCode::MissingDestination,
            reason: "a write operation needs a destination identity".to_string(),
        };
    };

    if !destination.server.is_identified() {
        return PolicyDecision::Deny {
            code: PolicyCode::UnverifiableDestination,
            reason: "the server did not report an identity, so the destination cannot be verified"
                .to_string(),
        };
    }

    // Source/destination reversal and same-target writes.
    if let Some(source) = context.source {
        if let (Some(source_key), Some(destination_key)) = (
            source.identity_key.as_deref(),
            destination.identity_key.as_deref(),
        ) {
            if source_key == destination_key {
                return PolicyDecision::Deny {
                    code: PolicyCode::SameSourceAndDestination,
                    reason: "source and destination resolve to the same database".to_string(),
                };
            }
        } else {
            return PolicyDecision::Deny {
                code: PolicyCode::UnverifiableDestination,
                reason:
                    "source or destination identity is missing, so reversal cannot be ruled out"
                        .to_string(),
            };
        }
    }

    match match_enrollment(
        destination,
        context.enrollment,
        context.freshness,
        context.now_ms,
    ) {
        EnrollmentMatch::Matches => {}
        EnrollmentMatch::NotEnrolled => {
            return PolicyDecision::RequireVerification {
                code: PolicyCode::NotEnrolled,
                reason: "this destination has no enrolled fingerprint".to_string(),
            }
        }
        EnrollmentMatch::IdentityChanged => {
            return PolicyDecision::Deny {
                code: PolicyCode::DestinationChanged,
                reason: "the server identity no longer matches the enrolled fingerprint"
                    .to_string(),
            }
        }
        EnrollmentMatch::Stale => {
            return PolicyDecision::RequireVerification {
                code: PolicyCode::StaleVerification,
                reason: "the verification evidence is older than the freshness policy".to_string(),
            }
        }
        EnrollmentMatch::Unverifiable => {
            return PolicyDecision::Deny {
                code: PolicyCode::UnverifiableDestination,
                reason: "the destination identity could not be compared".to_string(),
            }
        }
    }

    let category = context
        .enrollment
        .map(|enrollment| enrollment.category)
        .unwrap_or(DestinationCategory::Unknown);

    if !category.allows_write() {
        return PolicyDecision::Deny {
            code: PolicyCode::DestinationNotAllowed,
            reason: format!(
                "destination category '{}' does not allow writes",
                category.as_str()
            ),
        };
    }

    // A declared production label is a warning, not an override: if the user
    // says production, refuse even when the fingerprint matches.
    if matches!(context.declared_target, TargetClass::Production) {
        return PolicyDecision::Deny {
            code: PolicyCode::ProtectedSource,
            reason: "the connection is declared production".to_string(),
        };
    }

    PolicyDecision::Allow {
        code: PolicyCode::Allowed,
        evidence: format!(
            "destination {} matched the enrolled {} fingerprint",
            destination.identity_key.as_deref().unwrap_or("(unknown)"),
            category.as_str()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::connection::DbEngine;
    use crate::security::destination::ServerIdentity;

    fn identity(server_uuid: &str, database: &str, observed: u64) -> DestinationIdentity {
        let server = ServerIdentity {
            engine: Some("postgres".into()),
            version: Some("16".into()),
            server_uuid: Some(server_uuid.into()),
            database: Some(database.into()),
            database_id: Some(database.into()),
            current_user: Some("dev".into()),
            read_only: Some(false),
            can_write: Some(true),
        };
        DestinationIdentity {
            engine: DbEngine::Postgres,
            host: "127.0.0.1".into(),
            port: 5432,
            identity_key: server.identity_key(DbEngine::Postgres, "127.0.0.1", 5432),
            server,
            observed_at_ms: observed,
        }
    }

    fn enrollment(key: &str, category: DestinationCategory) -> EnrolledFingerprint {
        EnrolledFingerprint {
            identity_key: key.into(),
            engine: "postgres".into(),
            database: "dev".into(),
            category,
            enrolled_at_ms: 0,
            note: None,
        }
    }

    #[test]
    fn read_only_operations_need_no_destination() {
        let context = OperationContext::new(DatabaseOperation::ReadOnlyQuery, 1_000);
        assert!(evaluate(&context).is_allowed());
    }

    #[test]
    fn a_write_without_a_destination_is_denied() {
        let context = OperationContext::new(DatabaseOperation::DatabaseRestore, 1_000);
        let decision = evaluate(&context);
        assert_eq!(decision.code(), PolicyCode::MissingDestination);
    }

    #[test]
    fn an_unidentified_server_is_denied() {
        let mut destination = identity("srv-1", "dev", 1_000);
        destination.server.server_uuid = None;
        destination.identity_key = None;
        let context = OperationContext::new(DatabaseOperation::DatabaseRestore, 1_000)
            .with_destination(&destination);
        assert_eq!(
            evaluate(&context).code(),
            PolicyCode::UnverifiableDestination
        );
    }

    #[test]
    fn an_unenrolled_destination_requires_verification() {
        let destination = identity("srv-1", "dev", 1_000);
        let context = OperationContext::new(DatabaseOperation::DatabaseRestore, 1_000)
            .with_destination(&destination);
        assert_eq!(evaluate(&context).code(), PolicyCode::NotEnrolled);
    }

    #[test]
    fn a_changed_destination_is_denied() {
        let destination = identity("srv-2", "dev", 1_000);
        let old = identity("srv-1", "dev", 1_000);
        let enrolled = enrollment(
            old.identity_key.as_deref().unwrap(),
            DestinationCategory::VerifiedDevelopment,
        );
        let context = OperationContext::new(DatabaseOperation::DatabaseRestore, 1_000)
            .with_destination(&destination)
            .with_enrollment(&enrolled);
        assert_eq!(evaluate(&context).code(), PolicyCode::DestinationChanged);
    }

    #[test]
    fn stale_evidence_requires_reverification() {
        let destination = identity("srv-1", "dev", 1_000);
        let enrolled = enrollment(
            destination.identity_key.as_deref().unwrap(),
            DestinationCategory::VerifiedDevelopment,
        );
        let mut context = OperationContext::new(DatabaseOperation::DatabaseRestore, 10_000_000)
            .with_destination(&destination)
            .with_enrollment(&enrolled);
        context.freshness = FreshnessPolicy { max_age_ms: 100 };
        assert_eq!(evaluate(&context).code(), PolicyCode::StaleVerification);
    }

    #[test]
    fn a_verified_development_destination_is_allowed() {
        let destination = identity("srv-1", "dev", 1_000);
        let enrolled = enrollment(
            destination.identity_key.as_deref().unwrap(),
            DestinationCategory::VerifiedDevelopment,
        );
        let context = OperationContext::new(DatabaseOperation::DatabaseRestore, 1_500)
            .with_destination(&destination)
            .with_enrollment(&enrolled);
        assert!(evaluate(&context).is_allowed());
    }

    #[test]
    fn a_production_like_destination_is_denied() {
        let destination = identity("srv-1", "prod", 1_000);
        let enrolled = enrollment(
            destination.identity_key.as_deref().unwrap(),
            DestinationCategory::ProductionLike,
        );
        let context = OperationContext::new(DatabaseOperation::DatabaseRestore, 1_500)
            .with_destination(&destination)
            .with_enrollment(&enrolled);
        assert_eq!(evaluate(&context).code(), PolicyCode::DestinationNotAllowed);
    }

    #[test]
    fn a_declared_production_label_blocks_even_a_matching_fingerprint() {
        let destination = identity("srv-1", "dev", 1_000);
        let enrolled = enrollment(
            destination.identity_key.as_deref().unwrap(),
            DestinationCategory::VerifiedDevelopment,
        );
        let context = OperationContext::new(DatabaseOperation::DatabaseRestore, 1_500)
            .with_destination(&destination)
            .with_enrollment(&enrolled)
            .with_declared_target(TargetClass::Production);
        assert_eq!(evaluate(&context).code(), PolicyCode::ProtectedSource);
    }

    #[test]
    fn a_local_label_alone_does_not_authorize_a_write() {
        let destination = identity("srv-1", "dev", 1_000);
        let context = OperationContext::new(DatabaseOperation::DatabaseRestore, 1_500)
            .with_destination(&destination)
            .with_declared_target(TargetClass::Local);
        assert_eq!(evaluate(&context).code(), PolicyCode::NotEnrolled);
    }

    #[test]
    fn the_same_source_and_destination_is_denied() {
        let destination = identity("srv-1", "dev", 1_000);
        let enrolled = enrollment(
            destination.identity_key.as_deref().unwrap(),
            DestinationCategory::VerifiedDevelopment,
        );
        let context = OperationContext::new(DatabaseOperation::DatabaseCopy, 1_500)
            .with_source(&destination)
            .with_destination(&destination)
            .with_enrollment(&enrolled);
        assert_eq!(
            evaluate(&context).code(),
            PolicyCode::SameSourceAndDestination
        );
    }

    fn read_only_identity(server_uuid: &str, database: &str) -> DestinationIdentity {
        let mut identity = identity(server_uuid, database, 1_000);
        identity.server.read_only = Some(true);
        identity
    }

    #[test]
    fn safe_sync_prepare_needs_an_identified_source() {
        let context = OperationContext::new(DatabaseOperation::SafeSyncPrepare, 1_000);
        assert_eq!(evaluate(&context).code(), PolicyCode::MissingSource);
    }

    #[test]
    fn safe_sync_prepare_allows_a_read_only_identified_source() {
        let source = read_only_identity("srv-1", "prod");
        let context =
            OperationContext::new(DatabaseOperation::SafeSyncPrepare, 1_000).with_source(&source);
        assert!(evaluate(&context).is_allowed());
        assert_eq!(evaluate(&context).code(), PolicyCode::SourceAuthorized);
    }

    #[test]
    fn safe_sync_prepare_refuses_an_unidentified_source() {
        let mut source = read_only_identity("srv-1", "prod");
        source.server.server_uuid = None;
        source.identity_key = None;
        let context =
            OperationContext::new(DatabaseOperation::SafeSyncPrepare, 1_000).with_source(&source);
        assert_eq!(evaluate(&context).code(), PolicyCode::UnverifiableSource);
    }

    #[test]
    fn safe_sync_prepare_refuses_a_writable_source() {
        let source = identity("srv-1", "prod", 1_000);
        let context =
            OperationContext::new(DatabaseOperation::SafeSyncPrepare, 1_000).with_source(&source);
        assert_eq!(evaluate(&context).code(), PolicyCode::SourceNotReadOnly);
    }

    #[test]
    fn safe_sync_prepare_refuses_equal_source_and_destination() {
        let source = read_only_identity("srv-1", "prod");
        let context = OperationContext::new(DatabaseOperation::SafeSyncPrepare, 1_000)
            .with_source(&source)
            .with_destination(&source);
        assert_eq!(
            evaluate(&context).code(),
            PolicyCode::SameSourceAndDestination
        );
    }

    #[test]
    fn safe_sync_execute_needs_both_identities() {
        let destination = identity("srv-9", "dev", 1_000);
        let only_destination = OperationContext::new(DatabaseOperation::SafeSyncExecute, 1_500)
            .with_destination(&destination);
        assert_eq!(
            evaluate(&only_destination).code(),
            PolicyCode::MissingSource
        );
    }

    #[test]
    fn safe_sync_execute_requires_an_enrolled_destination() {
        let source = read_only_identity("srv-1", "prod");
        let destination = identity("srv-9", "dev", 1_000);
        let context = OperationContext::new(DatabaseOperation::SafeSyncExecute, 1_500)
            .with_source(&source)
            .with_destination(&destination);
        assert_eq!(evaluate(&context).code(), PolicyCode::NotEnrolled);
    }

    #[test]
    fn safe_sync_execute_allows_an_enrolled_development_destination() {
        let source = read_only_identity("srv-1", "prod");
        let destination = identity("srv-9", "dev", 1_000);
        let enrolled = enrollment(
            destination.identity_key.as_deref().unwrap(),
            DestinationCategory::VerifiedDevelopment,
        );
        let context = OperationContext::new(DatabaseOperation::SafeSyncExecute, 1_500)
            .with_source(&source)
            .with_destination(&destination)
            .with_enrollment(&enrolled);
        assert!(evaluate(&context).is_allowed());
    }

    #[test]
    fn safe_sync_execute_still_denies_a_production_like_destination() {
        let source = read_only_identity("srv-1", "prod");
        let destination = identity("srv-9", "prod2", 1_000);
        let enrolled = enrollment(
            destination.identity_key.as_deref().unwrap(),
            DestinationCategory::ProductionLike,
        );
        let context = OperationContext::new(DatabaseOperation::SafeSyncExecute, 1_500)
            .with_source(&source)
            .with_destination(&destination)
            .with_enrollment(&enrolled);
        assert_eq!(evaluate(&context).code(), PolicyCode::DestinationNotAllowed);
    }
}
