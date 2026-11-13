use dbpinball_core::domain::error::{CoreError, ErrorCode};
use dbpinball_core::services::inspect;
use serde_json::json;

use crate::cli::{Cli, SchemaCommand};
use crate::config;
use crate::credentials;
use crate::output;

pub async fn run(cli: &Cli, command: &SchemaCommand) -> Result<i32, CoreError> {
    match command {
        SchemaCommand::Inspect {
            connection,
            database,
        } => inspect_command(cli, connection, database.as_deref()).await,
    }
}

async fn inspect_command(
    cli: &Cli,
    connection_id: &str,
    database: Option<&str>,
) -> Result<i32, CoreError> {
    let store = config::store(cli.config_dir.as_deref())?;
    let connection = store.get(connection_id)?.ok_or_else(|| {
        CoreError::new(
            ErrorCode::NotFound,
            format!("connection '{connection_id}' was not found"),
        )
    })?;

    let provider = credentials::provider(cli.credential_source);

    match database {
        None => {
            let databases = inspect::list_databases(&connection, provider.as_ref()).await?;
            let data = json!({ "connectionId": connection_id, "databases": databases });

            output::print_success(cli.json, &data, |_| {
                for name in &databases {
                    println!("{name}");
                }
            });
        }
        Some(database) => {
            let tables = inspect::list_tables(&connection, database, provider.as_ref()).await?;
            let data = json!({
                "connectionId": connection_id,
                "database": database,
                "tables": tables
            });

            output::print_success(cli.json, &data, |_| {
                for name in &tables {
                    println!("{name}");
                }
            });
        }
    }

    Ok(0)
}
