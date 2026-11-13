use std::io::{IsTerminal, Write};
use std::path::Path;

use dbpinball_core::adapters::keyring::KeyringCredentialProvider;
use dbpinball_core::domain::connection::{DbConnection, DbEngine, TargetClass};
use dbpinball_core::domain::error::{CoreError, ErrorCode};
use dbpinball_core::ports::credentials::{CredentialProvider, Secret};
use dbpinball_core::services::inspect;
use dbpinball_core::store::{ImportReport, StoreFile};
use serde::Deserialize;
use serde_json::json;

use crate::cli::{AddArgs, Cli, ConnectionsCommand, CredentialSource, EngineArg, TargetArg};
use crate::config;
use crate::credentials;
use crate::output;

pub async fn run(cli: &Cli, command: &ConnectionsCommand) -> Result<i32, CoreError> {
    match command {
        ConnectionsCommand::List => list(cli),
        ConnectionsCommand::Add(args) => add(cli, args),
        ConnectionsCommand::SetPassword { id } => set_password(cli, id),
        ConnectionsCommand::Test { id } => test(cli, id).await,
        ConnectionsCommand::Remove { id, yes } => remove(cli, id, *yes),
        ConnectionsCommand::Import { file } => import(cli, file),
    }
}

fn list(cli: &Cli) -> Result<i32, CoreError> {
    let store = config::store(cli.config_dir.as_deref())?;
    let connections = store.load()?;

    let data = json!({ "connections": connections });

    output::print_success(cli.json, &data, |_| {
        if connections.is_empty() {
            println!("No connection profiles.");
            return;
        }

        println!(
            "{:<38} {:<14} {:<9} {:<28} {:<6} {:<10}",
            "ID", "NAME", "ENGINE", "HOST", "PORT", "TARGET"
        );

        for connection in &connections {
            println!(
                "{:<38} {:<14} {:<9} {:<28} {:<6} {:<10}",
                connection.id,
                connection.name,
                engine_label(&connection.engine),
                connection.host,
                connection.port,
                target_label(connection.target),
            );
        }
    });

    Ok(0)
}

fn add(cli: &Cli, args: &AddArgs) -> Result<i32, CoreError> {
    if cli.credential_source == CredentialSource::Env {
        return Err(CoreError::new(
            ErrorCode::Usage,
            "the environment credential source cannot store passwords; use the keyring source",
        ));
    }

    let password = read_password()?;
    let id = dbpinball_core::store::ConnectionStore::new_id();

    let connection = DbConnection {
        id: id.clone(),
        name: args.name.clone(),
        engine: match args.engine {
            EngineArg::Mysql => DbEngine::Mysql,
            EngineArg::Postgres => DbEngine::Postgres,
        },
        host: args.host.clone(),
        port: args.port,
        database: args.database.clone(),
        username: args.username.clone(),
        use_ssl: args.use_ssl,
        target: match args.target {
            TargetArg::Production => TargetClass::Production,
            TargetArg::Staging => TargetClass::Staging,
            TargetArg::Local => TargetClass::Local,
            TargetArg::Unknown => TargetClass::Unknown,
        },
    };

    let keyring = KeyringCredentialProvider;
    keyring.set(&id, &Secret::new(password))?;

    let store = config::store(cli.config_dir.as_deref())?;

    if let Err(error) = store.add(connection.clone()) {
        // Keep metadata and credential state consistent: if the profile cannot
        // be written, remove the credential we just stored.
        let _ = keyring.delete(&id);
        return Err(error);
    }

    let data = json!({ "connection": connection });
    output::print_success(cli.json, &data, |_| {
        println!("Created connection {} ({})", connection.name, connection.id);
    });

    Ok(0)
}

fn set_password(cli: &Cli, id: &str) -> Result<i32, CoreError> {
    if cli.credential_source == CredentialSource::Env {
        return Err(CoreError::new(
            ErrorCode::Usage,
            "the environment credential source cannot store passwords; use the keyring source",
        ));
    }

    let store = config::store(cli.config_dir.as_deref())?;
    lookup(&store, id)?;

    let password = read_password()?;
    KeyringCredentialProvider.set(id, &Secret::new(password))?;

    let data = json!({ "id": id, "updated": true });
    output::print_success(cli.json, &data, |_| {
        println!("Password updated for connection {id}.");
    });

    Ok(0)
}

async fn test(cli: &Cli, id: &str) -> Result<i32, CoreError> {
    let store = config::store(cli.config_dir.as_deref())?;
    let connection = lookup(&store, id)?;

    let provider = credentials::provider(cli.credential_source);
    inspect::test_connection(&connection, provider.as_ref()).await?;

    let data = json!({ "id": id, "reachable": true });
    output::print_success(cli.json, &data, |_| {
        println!("Connection {id} is reachable.");
    });

    Ok(0)
}

fn remove(cli: &Cli, id: &str, yes: bool) -> Result<i32, CoreError> {
    let store = config::store(cli.config_dir.as_deref())?;
    let connection = lookup(&store, id)?;

    if !yes {
        if !std::io::stdin().is_terminal() {
            return Err(CoreError::new(
                ErrorCode::PolicyRefusal,
                "refusing to remove a connection without --yes in non-interactive mode",
            ));
        }

        print!(
            "Remove connection '{}' ({})? [y/N] ",
            connection.name, connection.id
        );
        std::io::stdout().flush().map_err(|error| {
            CoreError::new(
                ErrorCode::Operation,
                format!("confirmation prompt could not be written: {error}"),
            )
        })?;

        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer).map_err(|error| {
            CoreError::new(
                ErrorCode::Operation,
                format!("confirmation could not be read: {error}"),
            )
        })?;

        if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
            return Err(CoreError::new(
                ErrorCode::PolicyRefusal,
                "removal was not confirmed",
            ));
        }
    }

    // Delete the credential first. A failure here keeps the metadata so the
    // profile is never left looking removed while a credential is still stored.
    if cli.credential_source == CredentialSource::Keyring {
        KeyringCredentialProvider.delete(id)?;
    }

    if !store.remove(id)? {
        return Err(CoreError::new(
            ErrorCode::NotFound,
            format!("connection '{id}' was not found"),
        ));
    }

    let data = json!({ "id": id, "removed": true });
    output::print_success(cli.json, &data, |_| {
        println!("Removed connection {id}.");
    });

    Ok(0)
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ImportPayload {
    Array(Vec<DbConnection>),
    File(StoreFile),
}

fn import(cli: &Cli, file: &Path) -> Result<i32, CoreError> {
    let contents = std::fs::read_to_string(file).map_err(|error| {
        CoreError::new(
            ErrorCode::Operation,
            format!("import file could not be read: {error}"),
        )
    })?;

    let connections = match serde_json::from_str::<ImportPayload>(&contents) {
        Ok(ImportPayload::Array(items)) => items,
        Ok(ImportPayload::File(file)) => file.connections,
        Err(error) => {
            return Err(CoreError::new(
                ErrorCode::Usage,
                format!("import file is not valid JSON: {error}"),
            ))
        }
    };

    let store = config::store(cli.config_dir.as_deref())?;
    let report: ImportReport = store.import(connections)?;

    output::print_success(cli.json, &report, |report| {
        println!(
            "Imported {}. Skipped {} invalid and {} duplicate record(s).",
            report.imported,
            report.skipped_invalid.len(),
            report.skipped_duplicates
        );
    });

    Ok(0)
}

fn lookup(
    store: &dbpinball_core::store::ConnectionStore,
    id: &str,
) -> Result<DbConnection, CoreError> {
    store.get(id)?.ok_or_else(|| {
        CoreError::new(
            ErrorCode::NotFound,
            format!("connection '{id}' was not found"),
        )
    })
}

fn read_password() -> Result<String, CoreError> {
    if std::io::stdin().is_terminal() {
        let password = rpassword::prompt_password("Password: ").map_err(|error| {
            CoreError::new(
                ErrorCode::Operation,
                format!("password could not be read: {error}"),
            )
        })?;

        if password.is_empty() {
            return Err(CoreError::new(ErrorCode::Usage, "a password is required"));
        }

        return Ok(password);
    }

    // Non-interactive: read exactly one line. `connections add` and
    // `set-password` define stdin as the password, so nothing else is consumed.
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).map_err(|error| {
        CoreError::new(
            ErrorCode::Operation,
            format!("password could not be read from stdin: {error}"),
        )
    })?;

    let password = line.trim_end_matches(['\r', '\n']).to_string();

    if password.is_empty() {
        return Err(CoreError::new(
            ErrorCode::Usage,
            "a password is required on stdin",
        ));
    }

    Ok(password)
}

fn engine_label(engine: &DbEngine) -> &'static str {
    match engine {
        DbEngine::Mysql => "mysql",
        DbEngine::Postgres => "postgres",
    }
}

fn target_label(target: TargetClass) -> &'static str {
    match target {
        TargetClass::Production => "production",
        TargetClass::Staging => "staging",
        TargetClass::Local => "local",
        TargetClass::Unknown => "unknown",
    }
}
