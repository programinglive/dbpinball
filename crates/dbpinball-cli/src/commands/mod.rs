pub mod connections;
pub mod doctor;
pub mod schema;

use dbpinball_core::domain::error::CoreError;

use crate::cli::{Cli, Command};
use crate::output;

/// Run the selected command and return the process exit code.
pub async fn dispatch(cli: Cli) -> i32 {
    let json = cli.json;

    match run(&cli).await {
        Ok(code) => code,
        Err(error) => output::print_error(&error, json),
    }
}

async fn run(cli: &Cli) -> Result<i32, CoreError> {
    match &cli.command {
        Command::Doctor => doctor::run(cli).await,
        Command::Connections { command } => connections::run(cli, command).await,
        Command::Schema { command } => schema::run(cli, command).await,
    }
}
