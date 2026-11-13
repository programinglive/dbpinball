use dbpinball_core::domain::error::{CoreError, ErrorCode};
use dbpinball_core::ports::credentials::CredentialProvider;
use serde::Serialize;

use crate::cli::Cli;
use crate::config;
use crate::output;

#[derive(Serialize)]
struct DoctorReport {
    version: String,
    checks: Vec<Check>,
}

#[derive(Serialize)]
struct Check {
    name: &'static str,
    status: &'static str,
    detail: Option<String>,
}

pub async fn run(cli: &Cli) -> Result<i32, CoreError> {
    let mut checks = vec![Check {
        name: "version",
        status: "ok",
        detail: Some(env!("CARGO_PKG_VERSION").to_string()),
    }];

    let config_dir = config::resolve_config_dir(cli.config_dir.as_deref())?;
    let writable = std::fs::create_dir_all(&config_dir).is_ok();
    checks.push(Check {
        name: "config_dir",
        status: if writable { "ok" } else { "fail" },
        detail: Some(config_dir.display().to_string()),
    });

    let store = config::store(cli.config_dir.as_deref())?;
    match store.load() {
        Ok(connections) => checks.push(Check {
            name: "connection_store",
            status: "ok",
            detail: Some(format!("{} profile(s)", connections.len())),
        }),
        Err(error) => checks.push(Check {
            name: "connection_store",
            status: "fail",
            detail: Some(error.message),
        }),
    }

    let keyring = dbpinball_core::adapters::keyring::KeyringCredentialProvider;
    let keyring_status = match keyring.get("dbpinball-doctor-probe") {
        Ok(_) => "ok",
        // A missing probe entry means the backend answered, which is healthy.
        // An unavailable backend surfaces the same way, so report a warning
        // instead of a hard failure; `--credential-source env` remains usable.
        Err(_) => "warn",
    };
    checks.push(Check {
        name: "keyring",
        status: keyring_status,
        detail: Some("probe completed".to_string()),
    });

    for binary in ["mysqldump", "pg_dump"] {
        let found = which::which(binary).is_ok();
        checks.push(Check {
            name: binary,
            status: if found { "ok" } else { "warn" },
            detail: Some(if found {
                "found in PATH".to_string()
            } else {
                "not found in PATH".to_string()
            }),
        });
    }

    let failed = checks.iter().any(|check| check.status == "fail");
    let report = DoctorReport {
        version: env!("CARGO_PKG_VERSION").to_string(),
        checks,
    };

    output::print_success(cli.json, &report, |report| {
        println!("dbpinball {}", report.version);
        for check in &report.checks {
            println!(
                "{:<18} {:<6} {}",
                check.name,
                check.status,
                check.detail.as_deref().unwrap_or("")
            );
        }
    });

    if failed {
        return Ok(crate::exit::exit_code(&CoreError::new(
            ErrorCode::Operation,
            "environment checks failed",
        )));
    }

    Ok(0)
}
