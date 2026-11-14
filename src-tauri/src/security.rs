//! Thin Tauri adapter over the Core security service.
//!
//! The desktop shows status and surfaces refusals. It never decides: every
//! allow/deny comes from `dbpinball_core::security`.

use dbpinball_core::adapters::keyring::KeyringCredentialProvider;
use dbpinball_core::domain::connection::DbConnection;
use dbpinball_core::domain::error::{CoreError, ErrorCode};
use dbpinball_core::security::{self, FreshnessPolicy, SecurityStatus};
use dbpinball_core::store::default_config_dir;

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn safe_error(error: CoreError) -> String {
    match error.code {
        ErrorCode::PolicyRefusal => error.message,
        ErrorCode::NotFound => "That connection no longer exists.".to_string(),
        ErrorCode::Connection => {
            "The destination could not be reached to verify its identity.".to_string()
        }
        ErrorCode::Unsupported => {
            "This security feature is not supported by this build.".to_string()
        }
        ErrorCode::Usage => error.message,
        ErrorCode::Conflict => {
            "The connection list changed in another window. Reload and try again.".to_string()
        }
        ErrorCode::Busy => "The connection store is busy. Try again in a moment.".to_string(),
        ErrorCode::Operation => "The security check could not be completed.".to_string(),
    }
}

fn enrollment_store() -> Result<security::EnrollmentStore, String> {
    let directory = default_config_dir().map_err(safe_error)?;
    Ok(security::store_for(&directory))
}

/// Read the destination identity and its enrollment state.
pub async fn status(connection: DbConnection) -> Result<SecurityStatus, String> {
    let store = enrollment_store()?;
    security::inspect(
        &connection,
        &KeyringCredentialProvider,
        &store,
        FreshnessPolicy::default(),
        now_ms(),
    )
    .await
    .map_err(safe_error)
}

/// Enroll a destination as a development or disposable write target.
pub async fn enroll(
    connection: DbConnection,
    category: String,
) -> Result<security::EnrolledFingerprint, String> {
    let category = match category.as_str() {
        "development" => security::DestinationCategory::VerifiedDevelopment,
        "disposable" => security::DestinationCategory::VerifiedDisposable,
        _ => return Err("category must be 'development' or 'disposable'".to_string()),
    };

    let store = enrollment_store()?;
    security::enroll(
        &connection,
        &KeyringCredentialProvider,
        &store,
        category,
        now_ms(),
    )
    .await
    .map_err(safe_error)
}

/// Remove an enrollment.
pub fn forget(connection_id: String) -> Result<bool, String> {
    let store = enrollment_store()?;
    store.remove(&connection_id).map_err(safe_error)
}

/// Authorize a write operation before it runs.
///
/// Returns `Ok(())` when the policy allows the operation, and a refusal message
/// otherwise. The desktop calls this before any statement that can write.
pub async fn authorize_write(
    connection: &DbConnection,
    operation: dbpinball_core::security::DatabaseOperation,
) -> Result<(), String> {
    let store = enrollment_store()?;
    let decision = security::policy_check(
        connection,
        &KeyringCredentialProvider,
        &store,
        operation,
        FreshnessPolicy::default(),
        now_ms(),
    )
    .await
    .map_err(safe_error)?;

    refuse_unless_allowed(decision, operation)
}

/// Authorize a copy from `source` into `destination`.
///
/// Both sides are verified, so a reversed or identical pair is refused before
/// any dump starts.
pub async fn authorize_copy(
    source: &DbConnection,
    destination: &DbConnection,
) -> Result<(), String> {
    use dbpinball_core::security::{evaluate, OperationContext};

    let store = enrollment_store()?;

    let source_identity = security::verify_destination(source, &KeyringCredentialProvider)
        .await
        .map_err(safe_error)?;
    let destination_identity =
        security::verify_destination(destination, &KeyringCredentialProvider)
            .await
            .map_err(safe_error)?;

    let registry = store.load().map_err(safe_error)?;
    let enrollment = registry.get(&destination.id);

    let mut context = OperationContext::new(
        dbpinball_core::security::DatabaseOperation::DatabaseCopy,
        now_ms(),
    )
    .with_source(&source_identity)
    .with_destination(&destination_identity)
    .with_optional_enrollment(enrollment)
    .with_declared_target(destination.target);
    context.freshness = FreshnessPolicy::default();

    refuse_unless_allowed(
        evaluate(&context),
        dbpinball_core::security::DatabaseOperation::DatabaseCopy,
    )
}

fn refuse_unless_allowed(
    decision: dbpinball_core::security::PolicyDecision,
    operation: dbpinball_core::security::DatabaseOperation,
) -> Result<(), String> {
    if decision.is_allowed() {
        return Ok(());
    }

    let code = decision.code().as_str();
    let reason = match &decision {
        dbpinball_core::security::PolicyDecision::Deny { reason, .. } => reason.clone(),
        dbpinball_core::security::PolicyDecision::RequireVerification { reason, .. } => {
            reason.clone()
        }
        dbpinball_core::security::PolicyDecision::Allow { .. } => String::new(),
    };

    Err(format!(
        "Refused by the security policy ({code}): {reason}. \
         Enroll the destination with `dbpinball security enroll --connection <id> --category development` \
         before running {}.",
        operation.as_str()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_policy_refusal_keeps_its_message() {
        let error = CoreError::new(ErrorCode::PolicyRefusal, "destination not enrolled");
        assert_eq!(safe_error(error), "destination not enrolled");
    }

    #[test]
    fn a_connection_error_does_not_leak_a_host() {
        let error = CoreError::new(
            ErrorCode::Connection,
            "failed to connect to postgres://user:secret@db.internal:5432/app",
        );
        let message = safe_error(error);
        assert!(!message.contains("secret"));
        assert!(!message.contains("db.internal"));
    }
}
