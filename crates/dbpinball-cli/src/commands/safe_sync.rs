//! `dbpinball safe-sync` — bounded, masked production-to-local PostgreSQL sync.
//!
//! Every decision comes from `dbpinball_core::safesync`. This module resolves
//! profiles, reads the spec and plan files, calls the core and prints the result.

use std::io::IsTerminal;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use dbpinball_core::domain::connection::DbConnection;
use dbpinball_core::domain::error::{CoreError, ErrorCode};
use dbpinball_core::safesync::{
    self, DryRunPlan, ExecuteRequest, InMemoryMaskKeyStore, KeyringMaskKeyStore, Limits, MaskKey,
    MaskKeyStore, SafeSyncSpec,
};
use serde_json::json;

use crate::cli::{Cli, MaskKeySource, SafeSyncCommand};
use crate::config;
use crate::credentials;
use crate::output;

pub async fn run(cli: &Cli, command: &SafeSyncCommand) -> Result<i32, CoreError> {
    match command {
        SafeSyncCommand::Inspect { source } => inspect(cli, source).await,
        SafeSyncCommand::Plan {
            spec,
            estimate,
            replace,
            out,
        } => plan(cli, spec, *estimate, *replace, out.as_deref()).await,
        SafeSyncCommand::Validate { plan, spec } => validate(cli, plan, spec).await,
        SafeSyncCommand::Execute {
            plan,
            spec,
            ack,
            replace,
            yes,
        } => execute(cli, plan, spec, ack, *replace, *yes).await,
        SafeSyncCommand::Status { run, destination } => {
            status(cli, run, destination.as_deref()).await
        }
        SafeSyncCommand::KeyRotate => key_rotate(cli),
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

fn enrollments(cli: &Cli) -> Result<dbpinball_core::security::EnrollmentStore, CoreError> {
    let directory = config::resolve_config_dir(cli.config_dir.as_deref())?;
    Ok(dbpinball_core::security::store_for(&directory))
}

/// The mask key store selected on the command line. A key is never an argument.
fn mask_keys(cli: &Cli) -> Result<Box<dyn MaskKeyStore>, CoreError> {
    match cli.mask_key_source {
        MaskKeySource::Keyring => Ok(Box::new(KeyringMaskKeyStore)),
        MaskKeySource::Env => {
            let value = std::env::var("DBPINBALL_SAFE_SYNC_KEY").map_err(|_| {
                CoreError::with_detail(
                    ErrorCode::Operation,
                    "DBPINBALL_SAFE_SYNC_KEY is not set for the environment key source",
                    "mask_key_missing",
                )
            })?;
            let key = MaskKey::from_hex(&value)?;
            Ok(Box::new(InMemoryMaskKeyStore::with_key(key)))
        }
    }
}

async fn inspect(cli: &Cli, source: &str) -> Result<i32, CoreError> {
    let connection = lookup(cli, source)?;
    let provider = credentials::provider(cli.credential_source);

    let identity = safesync::inspect(&connection, provider.as_ref(), &Limits::default()).await?;

    output::print_success(cli.json, &identity, |identity| {
        println!("Source:            {}", connection.id);
        println!(
            "Identity:          {}",
            identity.identity_key.as_deref().unwrap_or("(unidentified)")
        );
        println!("Current user:      {}", identity.current_user);
        println!("Read-only session: {}", identity.transaction_read_only);
    });

    Ok(0)
}

async fn plan(
    cli: &Cli,
    spec_path: &Path,
    estimate: bool,
    replace: bool,
    out: Option<&Path>,
) -> Result<i32, CoreError> {
    let spec = read_spec(spec_path)?;
    let source = lookup(cli, &spec.source)?;
    let destination = lookup(cli, &spec.destination)?;
    let provider = credentials::provider(cli.credential_source);
    let store = enrollments(cli)?;
    let keys = mask_keys(cli)?;

    let plan = safesync::build_plan(
        &spec,
        &source,
        &destination,
        provider.as_ref(),
        &store,
        keys.as_ref(),
        now_ms(),
        estimate,
        replace,
    )
    .await?;

    if let Some(path) = out {
        write_private(
            path,
            &serde_json::to_string_pretty(&plan).unwrap_or_default(),
        )?;
    }

    output::print_success(cli.json, &plan, |plan| {
        println!("Plan:            {}", plan.plan_fingerprint);
        println!("Expires at (ms): {}", plan.expires_at_ms);
        println!(
            "Source:          {}",
            identity_label(&plan.source.identity_key)
        );
        println!(
            "Destination:     {}",
            identity_label(&plan.destination.identity_key)
        );
        println!("Masking policy:  {}", plan.masking.policy);
        println!("Complete:        {}", plan.complete);
        for table in &plan.tables {
            println!(
                "Table:           {}.{} ({} bytes / {} rows)",
                table.schema,
                table.table,
                table.schema_hash,
                table
                    .rows_estimated
                    .map(|rows| rows.to_string())
                    .unwrap_or_else(|| "not estimated".to_string())
            );
        }
        println!();
        println!("Review it, then run:");
        println!(
            "  dbpinball safe-sync execute --plan <file> --spec <file> --ack {}",
            plan.plan_fingerprint
        );
    });

    Ok(0)
}

async fn validate(cli: &Cli, plan_path: &Path, spec_path: &Path) -> Result<i32, CoreError> {
    let plan = read_plan(plan_path)?;
    let spec = read_spec(spec_path)?;
    let source = lookup(cli, &spec.source)?;
    let destination = lookup(cli, &spec.destination)?;
    let provider = credentials::provider(cli.credential_source);
    let store = enrollments(cli)?;
    let keys = mask_keys(cli)?;

    let report = safesync::validate(
        &plan,
        &spec,
        &source,
        &destination,
        provider.as_ref(),
        &store,
        keys.as_ref(),
        now_ms(),
    )
    .await?;

    output::print_success(cli.json, &report, |report| {
        println!("Plan:            {}", report.plan_fingerprint);
        println!("Valid:           {}", report.valid);
        println!("Tables checked:  {}", report.tables_checked);
    });

    Ok(0)
}

async fn execute(
    cli: &Cli,
    plan_path: &Path,
    spec_path: &Path,
    acknowledgement: &str,
    replace: bool,
    yes: bool,
) -> Result<i32, CoreError> {
    let plan = read_plan(plan_path)?;
    let spec = read_spec(spec_path)?;

    if !yes {
        if !std::io::stdin().is_terminal() {
            return Err(CoreError::with_detail(
                ErrorCode::Usage,
                "execute needs --yes when stdin is not a terminal",
                "acknowledgement_required",
            ));
        }
        if !confirm(acknowledgement)? {
            return Err(CoreError::with_detail(
                ErrorCode::Usage,
                "the acknowledgement did not match the plan fingerprint",
                "acknowledgement_mismatch",
            ));
        }
    }

    let source = lookup(cli, &spec.source)?;
    let destination = lookup(cli, &spec.destination)?;
    let provider = credentials::provider(cli.credential_source);
    let store = enrollments(cli)?;
    let keys = mask_keys(cli)?;
    let directory = config::resolve_config_dir(cli.config_dir.as_deref())?;

    let report = safesync::execute(
        ExecuteRequest {
            plan: &plan,
            spec: &spec,
            acknowledgement,
            replace,
            source: &source,
            destination: &destination,
        },
        provider.as_ref(),
        &store,
        keys.as_ref(),
        Some(&directory),
        now_ms(),
        Arc::new(AtomicBool::new(false)),
    )
    .await?;

    output::print_success(cli.json, &report, |report| {
        println!("Run:             {}", report.run_id);
        println!("Status:          {}", report.status);
        println!("Sequence:        {}", report.sequence_status);
        println!("Rows written:    {}", report.rows_written);
        println!("Destination:     {}", report.destination_digest);
        if report.already_completed {
            println!("This plan already completed; nothing was written.");
        }
        if report.sequence_status == "reconciliation_required"
            || report.sequence_status == "pending"
        {
            println!(
                "Sequence reconciliation is incomplete; re-run execute with the same plan to retry."
            );
        }
    });

    // A committed row import with an unreconciled sequence is not full success.
    if report.sequence_status == "reconciliation_required" || report.sequence_status == "pending" {
        return Ok(9);
    }

    Ok(0)
}

async fn status(cli: &Cli, run_id: &str, destination: Option<&str>) -> Result<i32, CoreError> {
    let destination_id = match destination {
        Some(id) => id.to_string(),
        None => destination_from_journal(cli, run_id)?,
    };

    let connection = lookup(cli, &destination_id)?;
    let provider = credentials::provider(cli.credential_source);
    let report =
        safesync::status(run_id, &connection, provider.as_ref(), &Limits::default()).await?;

    output::print_success(cli.json, &report, |report| {
        println!("Run:             {}", report.run_id);
        println!("Status:          {}", report.status);
        println!("Sequence:        {}", report.sequence_status);
        println!("Rows written:    {}", report.rows_written);
        println!("Finished at:     {}", report.finished_at);
    });

    if report.sequence_status == "reconciliation_required" || report.sequence_status == "pending" {
        return Ok(9);
    }

    Ok(0)
}

fn key_rotate(cli: &Cli) -> Result<i32, CoreError> {
    match cli.mask_key_source {
        MaskKeySource::Env => Err(CoreError::with_detail(
            ErrorCode::Unsupported,
            "the environment mask key source cannot store a rotated key",
            "mask_key_read_only",
        )),
        MaskKeySource::Keyring => {
            let key = MaskKey::generate()?;
            KeyringMaskKeyStore.store(&key)?;
            let data = json!({ "keyId": key.id() });
            output::print_success(cli.json, &data, |data| {
                println!("New mask key: {}", data["keyId"].as_str().unwrap_or(""));
                println!("Every existing plan and destination is now invalid.");
            });
            Ok(0)
        }
    }
}

fn confirm(expected: &str) -> Result<bool, CoreError> {
    use std::io::Write;

    print!("Type the plan fingerprint to continue: ");
    std::io::stdout().flush().ok();
    let mut input = String::new();
    std::io::stdin().read_line(&mut input).map_err(|error| {
        CoreError::new(
            ErrorCode::Usage,
            format!("the confirmation could not be read: {error}"),
        )
    })?;
    Ok(input.trim() == expected)
}

fn read_spec(path: &Path) -> Result<SafeSyncSpec, CoreError> {
    let text = std::fs::read_to_string(path).map_err(|error| {
        CoreError::new(
            ErrorCode::Usage,
            format!("the spec could not be read: {error}"),
        )
    })?;
    SafeSyncSpec::parse_yaml(&text)
}

fn read_plan(path: &Path) -> Result<DryRunPlan, CoreError> {
    let text = std::fs::read_to_string(path).map_err(|error| {
        CoreError::new(
            ErrorCode::Usage,
            format!("the plan could not be read: {error}"),
        )
    })?;
    serde_json::from_str(&text).map_err(|error| {
        CoreError::new(
            ErrorCode::Usage,
            format!("the plan could not be parsed: {error}"),
        )
    })
}

fn destination_from_journal(cli: &Cli, run_id: &str) -> Result<String, CoreError> {
    let directory = config::resolve_config_dir(cli.config_dir.as_deref())?;
    let path = directory
        .join("safe-sync")
        .join("runs")
        .join(format!("{run_id}.json"));

    let text = std::fs::read_to_string(&path).map_err(|_| {
        CoreError::with_detail(
            ErrorCode::Usage,
            "pass --destination: the local run journal has no record of this run",
            "run_not_found",
        )
    })?;

    let value: serde_json::Value = serde_json::from_str(&text).map_err(|error| {
        CoreError::new(
            ErrorCode::Usage,
            format!("the run journal could not be parsed: {error}"),
        )
    })?;

    value["destinationConnectionId"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| {
            CoreError::new(
                ErrorCode::Usage,
                "the run journal does not name a destination",
            )
        })
}

fn write_private(path: &Path, body: &str) -> Result<(), CoreError> {
    std::fs::write(path, body).map_err(|error| {
        CoreError::new(
            ErrorCode::Usage,
            format!("the plan could not be written: {error}"),
        )
    })?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }

    Ok(())
}

fn identity_label(identity: &Option<String>) -> &str {
    identity.as_deref().unwrap_or("(unidentified)")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_private_plan_file_is_written() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("plan.json");
        write_private(&path, "{}").unwrap();
        assert!(path.exists());
    }
}
