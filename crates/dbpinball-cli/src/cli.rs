use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Parser, Debug)]
#[command(
    name = "dbpinball",
    version,
    about = "Inspect MySQL and PostgreSQL databases from the command line",
    long_about = None
)]
pub struct Cli {
    /// Emit exactly one JSON object on stdout.
    #[arg(long, global = true)]
    pub json: bool,

    /// Override the configuration directory.
    #[arg(long, global = true, value_name = "PATH")]
    pub config_dir: Option<PathBuf>,

    /// Where passwords are read from.
    #[arg(
        long,
        global = true,
        value_enum,
        default_value_t = CredentialSource::Keyring
    )]
    pub credential_source: CredentialSource,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum CredentialSource {
    Keyring,
    Env,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Check the local environment.
    Doctor,
    /// Manage stored connection profiles.
    Connections {
        #[command(subcommand)]
        command: ConnectionsCommand,
    },
    /// Inspect databases and tables.
    Schema {
        #[command(subcommand)]
        command: SchemaCommand,
    },
}

#[derive(Subcommand, Debug)]
pub enum ConnectionsCommand {
    /// List stored profiles. Never prints passwords.
    List,
    /// Add a profile. The password is read from stdin.
    Add(AddArgs),
    /// Replace the stored password for a profile. The password is read from stdin.
    SetPassword { id: String },
    /// Test a profile against its database.
    Test { id: String },
    /// Remove a profile and its stored credential.
    Remove {
        id: String,
        /// Required when stdin is not a terminal.
        #[arg(long)]
        yes: bool,
    },
    /// Import profiles from a JSON file.
    Import { file: PathBuf },
}

#[derive(Args, Debug)]
pub struct AddArgs {
    #[arg(long)]
    pub name: String,

    #[arg(long, value_enum)]
    pub engine: EngineArg,

    #[arg(long)]
    pub host: String,

    #[arg(long)]
    pub port: u16,

    #[arg(long)]
    pub database: String,

    #[arg(long)]
    pub username: String,

    #[arg(long, value_enum, default_value_t = TargetArg::Unknown)]
    pub target: TargetArg,

    #[arg(long)]
    pub use_ssl: bool,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum EngineArg {
    Mysql,
    Postgres,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetArg {
    Production,
    Staging,
    Local,
    Unknown,
}

#[derive(Subcommand, Debug)]
pub enum SchemaCommand {
    /// List databases, or tables in one database.
    Inspect {
        #[arg(long)]
        connection: String,
        #[arg(long)]
        database: Option<String>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_doctor_with_json() {
        let cli = Cli::try_parse_from(["dbpinball", "doctor", "--json"]).unwrap();
        assert!(cli.json);
        assert!(matches!(cli.command, Command::Doctor));
    }

    #[test]
    fn parses_connections_add() {
        let cli = Cli::try_parse_from([
            "dbpinball",
            "connections",
            "add",
            "--name",
            "local",
            "--engine",
            "postgres",
            "--host",
            "127.0.0.1",
            "--port",
            "5432",
            "--database",
            "app",
            "--username",
            "postgres",
            "--target",
            "local",
        ])
        .unwrap();

        match cli.command {
            Command::Connections {
                command: ConnectionsCommand::Add(args),
            } => {
                assert_eq!(args.name, "local");
                assert_eq!(args.engine, EngineArg::Postgres);
                assert_eq!(args.target, TargetArg::Local);
                assert!(!args.use_ssl);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn rejects_password_argument() {
        let result = Cli::try_parse_from([
            "dbpinball",
            "connections",
            "add",
            "--name",
            "local",
            "--engine",
            "postgres",
            "--host",
            "127.0.0.1",
            "--port",
            "5432",
            "--database",
            "app",
            "--username",
            "postgres",
            "--password",
            "secret",
        ]);

        assert!(result.is_err(), "a --password flag must not exist");
    }

    #[test]
    fn parses_schema_inspect() {
        let cli = Cli::try_parse_from([
            "dbpinball",
            "schema",
            "inspect",
            "--connection",
            "abc",
            "--database",
            "app",
        ])
        .unwrap();

        match cli.command {
            Command::Schema {
                command:
                    SchemaCommand::Inspect {
                        connection,
                        database,
                    },
            } => {
                assert_eq!(connection, "abc");
                assert_eq!(database.as_deref(), Some("app"));
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_remove_yes() {
        let cli =
            Cli::try_parse_from(["dbpinball", "connections", "remove", "abc", "--yes"]).unwrap();

        match cli.command {
            Command::Connections {
                command: ConnectionsCommand::Remove { id, yes },
            } => {
                assert_eq!(id, "abc");
                assert!(yes);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }
}
