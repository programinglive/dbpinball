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

    /// Where masking keys are read from.
    #[arg(
        long,
        global = true,
        value_enum,
        default_value_t = MaskKeySource::Keyring
    )]
    pub mask_key_source: MaskKeySource,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum CredentialSource {
    Keyring,
    Env,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaskKeySource {
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
    /// Verify destinations and evaluate the security policy.
    Security {
        #[command(subcommand)]
        command: SecurityCommand,
    },
    /// Bounded, masked PostgreSQL production-to-local sync.
    SafeSync {
        #[command(subcommand)]
        command: SafeSyncCommand,
    },
}

#[derive(Subcommand, Debug)]
pub enum SafeSyncCommand {
    /// Read the identity of a source without writing anything.
    Inspect {
        #[arg(long)]
        source: String,
    },
    /// Build a reviewable, value-free plan from a specification.
    Plan {
        #[arg(long)]
        spec: PathBuf,
        /// Read row counts as an estimate. Never returns values.
        #[arg(long)]
        estimate: bool,
        /// Record the replace conflict policy in the plan.
        #[arg(long)]
        replace: bool,
        /// Also write the plan JSON to this path.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Re-check a plan against live identities, the catalogue and the policy.
    Validate {
        #[arg(long)]
        plan: PathBuf,
        #[arg(long)]
        spec: PathBuf,
    },
    /// Execute a plan. Writes only to the enrolled development destination.
    Execute {
        #[arg(long)]
        plan: PathBuf,
        #[arg(long)]
        spec: PathBuf,
        /// The plan fingerprint the operator reviewed.
        #[arg(long)]
        ack: String,
        /// Replace the selected rows instead of refusing a non-empty destination.
        #[arg(long)]
        replace: bool,
        /// Required when stdin is not a terminal.
        #[arg(long)]
        yes: bool,
    },
    /// Read a recorded run's state from the destination.
    Status {
        #[arg(long)]
        run: String,
        /// The destination profile. Falls back to the local run journal.
        #[arg(long)]
        destination: Option<String>,
    },
    /// Rotate the masking key. Invalidates every existing plan and destination.
    KeyRotate,
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

#[derive(Subcommand, Debug)]
pub enum SecurityCommand {
    /// Read the destination identity and compare it to any enrollment.
    Inspect { id: String },
    /// Verify a destination and enroll its fingerprint.
    Enroll {
        id: String,
        /// The category to enroll. Only verified categories are accepted.
        #[arg(long, value_enum)]
        category: CategoryArg,
    },
    /// Verify a destination without changing the enrollment.
    Verify { id: String },
    /// Evaluate the policy for one operation against one connection.
    PolicyCheck {
        id: String,
        #[arg(long, value_enum)]
        operation: OperationArg,
    },
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum CategoryArg {
    Development,
    Disposable,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationArg {
    Inspect,
    Query,
    Backup,
    Import,
    Restore,
    Copy,
    Schema,
    Data,
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
    fn parses_safe_sync_plan_with_spec() {
        let cli = Cli::try_parse_from([
            "dbpinball",
            "safe-sync",
            "plan",
            "--spec",
            "spec.yaml",
            "--estimate",
            "--out",
            "plan.json",
        ])
        .unwrap();

        match cli.command {
            Command::SafeSync {
                command:
                    SafeSyncCommand::Plan {
                        spec,
                        estimate,
                        replace,
                        out,
                    },
            } => {
                assert_eq!(spec.to_string_lossy(), "spec.yaml");
                assert!(estimate);
                assert!(!replace);
                assert_eq!(out.unwrap().to_string_lossy(), "plan.json");
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_safe_sync_execute_with_ack_and_spec() {
        let cli = Cli::try_parse_from([
            "dbpinball",
            "safe-sync",
            "execute",
            "--plan",
            "plan.json",
            "--spec",
            "spec.yaml",
            "--ack",
            "abc",
            "--yes",
        ])
        .unwrap();

        match cli.command {
            Command::SafeSync {
                command:
                    SafeSyncCommand::Execute {
                        plan,
                        spec,
                        ack,
                        replace,
                        yes,
                    },
            } => {
                assert_eq!(plan.to_string_lossy(), "plan.json");
                assert_eq!(spec.to_string_lossy(), "spec.yaml");
                assert_eq!(ack, "abc");
                assert!(!replace);
                assert!(yes);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn safe_sync_execute_without_spec_is_rejected() {
        let result = Cli::try_parse_from([
            "dbpinball",
            "safe-sync",
            "execute",
            "--plan",
            "p",
            "--ack",
            "a",
        ]);
        assert!(result.is_err(), "execute must require --spec");
    }

    #[test]
    fn safe_sync_has_no_password_option() {
        let result = Cli::try_parse_from([
            "dbpinball",
            "safe-sync",
            "plan",
            "--spec",
            "s",
            "--password",
            "x",
        ]);
        assert!(result.is_err(), "a --password flag must not exist");
    }

    #[test]
    fn the_mask_key_source_defaults_to_keyring() {
        let cli = Cli::try_parse_from(["dbpinball", "safe-sync", "key-rotate"]).unwrap();
        assert_eq!(cli.mask_key_source, MaskKeySource::Keyring);
        assert!(matches!(
            cli.command,
            Command::SafeSync {
                command: SafeSyncCommand::KeyRotate
            }
        ));
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
