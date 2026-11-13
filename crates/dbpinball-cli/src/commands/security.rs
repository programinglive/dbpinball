//! `dbpinball security` — destination verification and policy evaluation.
//!
//! Every decision comes from `dbpinball_core::security`. This module only
//! resolves the connection, calls the core, and prints the result.

use dbpinball_core::domain::connection::DbConnection;
use dbpinball_core::domain::error::{CoreError, ErrorCode};
use dbpinball_core::security::{
    self, DatabaseOperation, DestinationCategory, FreshnessPolicy, PolicyDecision,
};
use serde_json::json;

use crate::cli::{CategoryArg, Cli, OperationArg, SecurityCommand};
use crate::config;
use crate::credentials;
use crate::output;

pub async fn run(cli: &Cli, command: &SecurityCommand) -> Result<i32, CoreError> {
    match command {
        SecurityCommand::Inspect { id } => inspect(cli, id).await,
        SecurityCommand::Enroll { id, category } => enroll(cli, id, *category).await,
        SecurityCommand::Verify { id } => verify(cli, id).await,
        SecurityCommand::PolicyCheck { id, operation } => policy_check(cli, id, *operation).await,
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn lookup(cli: &Cli, id: &str) -> Result<DbConnection, CoreError> {
    let store = config::store(cli.config_dir.as_deref())?;
    store.get(id)?.ok_or_else(|| {
        CoreError::new(
            ErrorCode::NotFound,
            format!("connection '{id}' was not found"),
        )
    })
}

fn enrollments(cli: &Cli) -> Result<security::EnrollmentStore, CoreError> {
    let directory = config::resolve_config_dir(cli.config_dir.as_deref())?;
    Ok(security::store_for(&directory))
}

async fn inspect(cli: &Cli, id: &str) -> Result<i32, CoreError> {
    let connection = lookup(cli, id)?;
    let provider = credentials::provider(cli.credential_source);
    let store = enrollments(cli)?;

    let status = security::inspect(
        &connection,
        provider.as_ref(),
        &store,
        FreshnessPolicy::default(),
        now_ms(),
    )
    .await?;

    let data = json!({
        "connectionId": status.connection_id,
        "declaredTarget": status.declared_target,
        "identity": status.identity,
        "assessedCategory": status.assessed_category,
        "enrollment": status.enrollment,
        "enrollmentMatch": status.enrollment_match,
        "writeAllowed": status.write_allowed,
        "decisionCode": status.decision_code,
    });

    output::print_success(cli.json, &data, |_| {
        println!("Connection:        {}", status.connection_id);
        println!("Declared target:   {:?}", status.declared_target);
        println!(
            "Identity:          {}",
            status
                .identity
                .identity_key
                .as_deref()
                .unwrap_or("(unidentified)")
        );
        println!("Assessed category: {}", status.assessed_category.as_str());
        println!("Enrollment match:  {:?}", status.enrollment_match);
        println!("Write allowed:     {}", status.write_allowed);
        println!("Decision:          {}", status.decision_code);
    });

    Ok(0)
}

async fn enroll(cli: &Cli, id: &str, category: CategoryArg) -> Result<i32, CoreError> {
    let connection = lookup(cli, id)?;
    let provider = credentials::provider(cli.credential_source);
    let store = enrollments(cli)?;

    let category = match category {
        CategoryArg::Development => DestinationCategory::VerifiedDevelopment,
        CategoryArg::Disposable => DestinationCategory::VerifiedDisposable,
    };

    let fingerprint =
        security::enroll(&connection, provider.as_ref(), &store, category, now_ms()).await?;

    let data = json!({ "enrolled": fingerprint });
    output::print_success(cli.json, &data, |_| {
        println!("Enrolled {} as {}", id, category.as_str());
        println!("Fingerprint: {}", fingerprint.identity_key);
    });

    Ok(0)
}

async fn verify(cli: &Cli, id: &str) -> Result<i32, CoreError> {
    let connection = lookup(cli, id)?;
    let provider = credentials::provider(cli.credential_source);
    let store = enrollments(cli)?;

    let (identity, matched) = security::verify(
        &connection,
        provider.as_ref(),
        &store,
        FreshnessPolicy::default(),
        now_ms(),
    )
    .await?;

    let data = json!({
        "connectionId": id,
        "identity": identity,
        "enrollmentMatch": matched,
    });

    output::print_success(cli.json, &data, |_| {
        println!(
            "Identity: {}",
            identity.identity_key.as_deref().unwrap_or("(unidentified)")
        );
        println!("Enrollment match: {:?}", matched);
    });

    Ok(0)
}

async fn policy_check(cli: &Cli, id: &str, operation: OperationArg) -> Result<i32, CoreError> {
    let connection = lookup(cli, id)?;
    let provider = credentials::provider(cli.credential_source);
    let store = enrollments(cli)?;

    let operation = match operation {
        OperationArg::Inspect => DatabaseOperation::ReadOnlyInspection,
        OperationArg::Query => DatabaseOperation::ReadOnlyQuery,
        OperationArg::Backup => DatabaseOperation::BackupExport,
        OperationArg::Import => DatabaseOperation::LocalDataImport,
        OperationArg::Restore => DatabaseOperation::DatabaseRestore,
        OperationArg::Copy => DatabaseOperation::DatabaseCopy,
        OperationArg::Schema => DatabaseOperation::SchemaMutation,
        OperationArg::Data => DatabaseOperation::DataMutation,
    };

    let decision = security::policy_check(
        &connection,
        provider.as_ref(),
        &store,
        operation,
        FreshnessPolicy::default(),
        now_ms(),
    )
    .await?;

    let allowed = decision.is_allowed();
    let data = json!({
        "connectionId": id,
        "operation": operation.as_str(),
        "decision": decision,
        "allowed": allowed,
    });

    output::print_success(cli.json, &data, |_| match &decision {
        PolicyDecision::Allow { code, evidence } => {
            println!("ALLOW ({})", code.as_str());
            println!("{evidence}");
        }
        PolicyDecision::Deny { code, reason } => {
            println!("DENY ({})", code.as_str());
            println!("{reason}");
        }
        PolicyDecision::RequireVerification { code, reason } => {
            println!("REQUIRE VERIFICATION ({})", code.as_str());
            println!("{reason}");
        }
    });

    if allowed {
        Ok(0)
    } else {
        // A refusal is a policy outcome, not a crash.
        Err(CoreError::new(
            ErrorCode::PolicyRefusal,
            format!("operation '{}' is not authorized", operation.as_str()),
        ))
    }
}
