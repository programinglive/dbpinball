//! A database session that owns exactly one connection and its identity.
//!
//! The identity is read on the same connection that later runs every statement,
//! so there is no pool that can hand out a different server between the check and
//! the work. A reconnect builds a new session and therefore a new authorization.

use serde::Serialize;
use sqlx::mysql::{MySqlConnectOptions, MySqlConnection};
use sqlx::postgres::{PgConnectOptions, PgConnection};
use sqlx::{Connection, Row};

use crate::domain::connection::{DbConnection, DbEngine};
use crate::domain::error::{CoreError, ErrorCode};
use crate::ports::credentials::CredentialProvider;
use crate::safesync::spec::{Limits, QualifiedName};
use crate::security::{DestinationIdentity, ServerIdentity};

/// Which side of a Safe Sync this session serves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionRole {
    Source,
    Destination,
}

impl SessionRole {
    fn changed_code(self) -> &'static str {
        match self {
            SessionRole::Source => "source_changed",
            SessionRole::Destination => "destination_changed",
        }
    }
}

/// The identity the server reported for this session.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionIdentity {
    pub engine: DbEngine,
    /// `postgres://host:port/db_oid#system_identifier`, or `None` when the server
    /// did not report a usable identity.
    pub identity_key: Option<String>,
    pub system_identifier: Option<String>,
    pub database_oid: String,
    pub database_name: String,
    pub current_user: String,
    pub backend_pid: i32,
    pub backend_start: String,
    pub server_addr: String,
    pub server_port: u16,
    pub transaction_read_only: bool,
    pub host: String,
    pub port: u16,
    pub server: ServerIdentity,
}

impl SessionIdentity {
    /// The M3 identity view of this session, so the policy can decide.
    pub fn as_destination_identity(&self, observed_at_ms: u64) -> DestinationIdentity {
        DestinationIdentity {
            engine: self.engine,
            host: self.host.clone(),
            port: self.port,
            identity_key: self.identity_key.clone(),
            server: self.server.clone(),
            observed_at_ms,
        }
    }
}

enum SessionConnection {
    Postgres(PgConnection),
    MariaDb(MySqlConnection),
}

/// One owned connection plus the identity read from it.
pub struct BoundSession {
    role: SessionRole,
    connection: SessionConnection,
    identity: SessionIdentity,
}

impl std::fmt::Debug for BoundSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The connection is never rendered; it can carry connection options.
        formatter
            .debug_struct("BoundSession")
            .field("role", &self.role)
            .field("identity", &self.identity)
            .finish_non_exhaustive()
    }
}

impl BoundSession {
    /// Open a single connection and read its identity.
    ///
    /// A source session is put into database-enforced read-only mode before any
    /// other statement. A destination session is left read-write.
    pub async fn open(
        role: SessionRole,
        profile: &DbConnection,
        credentials: &dyn CredentialProvider,
        limits: &Limits,
    ) -> Result<Self, CoreError> {
        let secret = credentials.get(&profile.id)?;
        match profile.engine {
            DbEngine::Postgres => {
                let options = PgConnectOptions::new()
                    .host(&profile.host)
                    .port(profile.port)
                    .username(&profile.username)
                    .password(secret.expose())
                    .database(&profile.database);

                let mut connection =
                    PgConnection::connect_with(&options)
                        .await
                        .map_err(|error| {
                            crate::safesync::database_error(
                                ErrorCode::Connection,
                                &format!("the {} connection", role_label(role)),
                                "connect_failed",
                                error,
                            )
                        })?;

                if role == SessionRole::Source {
                    apply_source_guards(&mut connection, limits).await?;
                }

                let identity = read_identity(&mut connection, role, profile).await?;

                Ok(Self {
                    role,
                    connection: SessionConnection::Postgres(connection),
                    identity,
                })
            }
            DbEngine::Mysql => {
                if profile.database.trim().is_empty() {
                    return Err(CoreError::with_detail(
                        ErrorCode::Usage,
                        "a database name is required for MariaDB Safe Sync",
                        "missing_database",
                    ));
                }

                let options = MySqlConnectOptions::new()
                    .host(&profile.host)
                    .port(profile.port)
                    .username(&profile.username)
                    .password(secret.expose())
                    .database(&profile.database);

                let mut connection =
                    MySqlConnection::connect_with(&options)
                        .await
                        .map_err(|error| {
                            crate::safesync::database_error(
                                ErrorCode::Connection,
                                &format!("the {} connection", role_label(role)),
                                "connect_failed",
                                error,
                            )
                        })?;

                match role {
                    SessionRole::Source => {
                        apply_mariadb_source_guards(&mut connection, limits).await?;
                    }
                    SessionRole::Destination => {
                        apply_mariadb_destination_guards(&mut connection, limits).await?;
                    }
                }

                let identity = read_mariadb_identity(&mut connection, role, profile).await?;

                Ok(Self {
                    role,
                    connection: SessionConnection::MariaDb(connection),
                    identity,
                })
            }
        }
    }

    pub fn role(&self) -> SessionRole {
        self.role
    }

    pub fn engine(&self) -> DbEngine {
        self.identity.engine
    }

    pub fn identity(&self) -> &SessionIdentity {
        &self.identity
    }

    pub fn connection(&mut self) -> &mut PgConnection {
        match &mut self.connection {
            SessionConnection::Postgres(conn) => conn,
            SessionConnection::MariaDb(_) => {
                panic!("pg connection requested on a MariaDB BoundSession")
            }
        }
    }

    pub fn pg_connection(&mut self) -> Result<&mut PgConnection, CoreError> {
        match &mut self.connection {
            SessionConnection::Postgres(conn) => Ok(conn),
            SessionConnection::MariaDb(_) => Err(CoreError::with_detail(
                ErrorCode::Operation,
                "expected a PostgreSQL session",
                "engine_mismatch",
            )),
        }
    }

    pub fn mysql_connection(&mut self) -> Result<&mut MySqlConnection, CoreError> {
        match &mut self.connection {
            SessionConnection::MariaDb(conn) => Ok(conn),
            SessionConnection::Postgres(_) => Err(CoreError::with_detail(
                ErrorCode::Operation,
                "expected a MariaDB session",
                "engine_mismatch",
            )),
        }
    }

    /// Fail when the session is no longer the one the plan recorded.
    pub fn assert_binding(&self, expected: &SessionIdentity) -> Result<(), CoreError> {
        bind(self.role, &self.identity, expected)
    }

    /// Refuse a source role that could write, or that could bypass a row policy.
    pub async fn assert_source_role(&mut self, tables: &[QualifiedName]) -> Result<(), CoreError> {
        match &mut self.connection {
            SessionConnection::Postgres(conn) => {
                let (is_super, bypass_rls): (bool, bool) = sqlx::query_as(
                    "SELECT rolsuper, rolbypassrls FROM pg_roles WHERE rolname = current_user",
                )
                .fetch_one(&mut *conn)
                .await
                .map_err(connection_error)?;

                if is_super || bypass_rls {
                    return Err(CoreError::with_detail(
                        ErrorCode::Operation,
                        "the source role is privileged; use a SELECT-only role",
                        "source_privileged",
                    ));
                }

                for table in tables {
                    if table_writes(conn, table).await? {
                        return Err(CoreError::with_detail(
                            ErrorCode::Operation,
                            format!("the source role can write to {table}"),
                            "source_privileged",
                        ));
                    }
                    if owns_table(conn, table).await? {
                        return Err(CoreError::with_detail(
                            ErrorCode::Operation,
                            format!("the source role owns {table}"),
                            "source_privileged",
                        ));
                    }
                }

                Ok(())
            }
            SessionConnection::MariaDb(conn) => {
                assert_mariadb_source_role(conn, &self.identity.database_name, tables).await
            }
        }
    }

    /// Open a read-only transaction. A write on it fails at the engine.
    pub async fn begin_read_only(&mut self) -> Result<(), CoreError> {
        match &mut self.connection {
            SessionConnection::Postgres(conn) => {
                sqlx::query("BEGIN READ ONLY")
                    .execute(&mut *conn)
                    .await
                    .map_err(|error| {
                        CoreError::with_detail(
                            ErrorCode::Operation,
                            format!("the source transaction could not start: {error}"),
                            "source_not_read_only",
                        )
                    })?;
                Ok(())
            }
            SessionConnection::MariaDb(conn) => {
                sqlx::query("START TRANSACTION READ ONLY")
                    .execute(&mut *conn)
                    .await
                    .map_err(|error| {
                        CoreError::with_detail(
                            ErrorCode::Operation,
                            format!("the MariaDB source transaction could not start: {error}"),
                            "source_not_read_only",
                        )
                    })?;
                Ok(())
            }
        }
    }

    /// Open the destination transaction.
    ///
    /// `READ COMMITTED` is deliberate. The destination is protected by a
    /// transaction-scoped advisory lock per plan (PostgreSQL) or session-scoped
    /// `GET_LOCK` (MariaDB) and table locks, so every statement sees the state
    /// left by whoever held the lock before. `REPEATABLE READ` would take its
    /// snapshot before the lock is granted.
    pub async fn begin_destination_transaction(&mut self) -> Result<(), CoreError> {
        match &mut self.connection {
            SessionConnection::Postgres(conn) => {
                sqlx::query("BEGIN ISOLATION LEVEL READ COMMITTED")
                    .execute(&mut *conn)
                    .await
                    .map_err(connection_error)?;
                Ok(())
            }
            SessionConnection::MariaDb(conn) => {
                sqlx::query("SET TRANSACTION ISOLATION LEVEL READ COMMITTED")
                    .execute(&mut *conn)
                    .await
                    .map_err(connection_error)?;
                sqlx::query("START TRANSACTION")
                    .execute(&mut *conn)
                    .await
                    .map_err(connection_error)?;
                Ok(())
            }
        }
    }

    pub async fn rollback(&mut self) -> Result<(), CoreError> {
        match &mut self.connection {
            SessionConnection::Postgres(conn) => {
                sqlx::query("ROLLBACK")
                    .execute(&mut *conn)
                    .await
                    .map_err(connection_error)?;
            }
            SessionConnection::MariaDb(conn) => {
                sqlx::query("ROLLBACK")
                    .execute(&mut *conn)
                    .await
                    .map_err(connection_error)?;
            }
        }
        Ok(())
    }
}

async fn apply_source_guards(
    connection: &mut PgConnection,
    limits: &Limits,
) -> Result<(), CoreError> {
    sqlx::query("SET default_transaction_read_only = on")
        .execute(&mut *connection)
        .await
        .map_err(connection_error)?;
    // `SET` does not accept bind parameters, so `set_config` carries them.
    sqlx::query("SELECT set_config('statement_timeout', $1, false)")
        .bind(limits.timeout_ms.to_string())
        .execute(&mut *connection)
        .await
        .map_err(connection_error)?;
    sqlx::query("SELECT set_config('lock_timeout', $1, false)")
        .bind(limits.timeout_ms.min(5_000).to_string())
        .execute(&mut *connection)
        .await
        .map_err(connection_error)?;
    sqlx::query("SELECT set_config('idle_in_transaction_session_timeout', $1, false)")
        .bind(limits.timeout_ms.to_string())
        .execute(&mut *connection)
        .await
        .map_err(connection_error)?;
    sqlx::query("SET client_encoding = 'UTF8'")
        .execute(&mut *connection)
        .await
        .map_err(connection_error)?;

    let default_ro: String =
        sqlx::query_scalar("SELECT current_setting('default_transaction_read_only')")
            .fetch_one(&mut *connection)
            .await
            .map_err(connection_error)?;

    if default_ro != "on" {
        return Err(CoreError::with_detail(
            ErrorCode::Operation,
            "the source session is not read-only",
            "source_not_read_only",
        ));
    }

    Ok(())
}

async fn read_identity(
    connection: &mut PgConnection,
    role: SessionRole,
    profile: &DbConnection,
) -> Result<SessionIdentity, CoreError> {
    let row = sqlx::query(
        "SELECT current_database()::text,
                current_user::text,
                COALESCE(inet_server_addr()::text, ''),
                COALESCE(inet_server_port(), 0)::int,
                (SELECT oid::text FROM pg_database WHERE datname = current_database()),
                pg_backend_pid(),
                COALESCE((SELECT backend_start::text FROM pg_stat_activity WHERE pid = pg_backend_pid()), ''),
                current_setting('transaction_read_only'),
                current_setting('default_transaction_read_only')",
    )
    .fetch_one(&mut *connection)
    .await
    .map_err(connection_error)?;

    let database_name: String = row.try_get(0).unwrap_or_default();
    let current_user: String = row.try_get(1).unwrap_or_default();
    let server_addr: String = row.try_get(2).unwrap_or_default();
    let server_port: i32 = row.try_get(3).unwrap_or(0);
    let database_oid: String = row.try_get(4).unwrap_or_default();
    let backend_pid: i32 = row.try_get(5).unwrap_or(0);
    let backend_start: String = row.try_get(6).unwrap_or_default();
    let transaction_read_only: String = row.try_get(7).unwrap_or_default();
    let default_transaction_read_only: String = row.try_get(8).unwrap_or_default();

    // A source session is read-only because its default transaction mode is on;
    // outside a transaction the session-level setting still reports `off`. A
    // destination session is not read-only.
    let is_read_only = match role {
        SessionRole::Source => default_transaction_read_only == "on",
        SessionRole::Destination => transaction_read_only == "on",
    };

    // `pg_control_system()` is the strongest evidence but can be denied. A
    // denial leaves the identity missing, and the policy refuses.
    let system_identifier: Option<String> =
        sqlx::query_scalar("SELECT system_identifier::text FROM pg_control_system()")
            .fetch_one(&mut *connection)
            .await
            .ok();

    let server = ServerIdentity {
        engine: Some("postgres".into()),
        version: None,
        server_uuid: system_identifier.clone(),
        database: Some(database_name.clone()),
        database_id: Some(database_oid.clone()),
        current_user: Some(current_user.clone()),
        read_only: Some(is_read_only),
        can_write: None,
    };

    let host = if server_addr.is_empty() {
        profile.host.clone()
    } else {
        server_addr.clone()
    };
    let port = if server_port > 0 {
        server_port as u16
    } else {
        profile.port
    };

    let identity_key = server.identity_key(DbEngine::Postgres, &host, port);

    Ok(SessionIdentity {
        engine: DbEngine::Postgres,
        identity_key,
        system_identifier,
        database_oid,
        database_name,
        current_user,
        backend_pid,
        backend_start,
        server_addr,
        server_port: port,
        transaction_read_only: is_read_only,
        host,
        port,
        server,
    })
}

async fn table_writes(
    connection: &mut PgConnection,
    table: &QualifiedName,
) -> Result<bool, CoreError> {
    let row = sqlx::query(
        "SELECT has_table_privilege(current_user, format('%I.%I', $1, $2), 'INSERT')
             OR has_table_privilege(current_user, format('%I.%I', $1, $2), 'UPDATE')
             OR has_table_privilege(current_user, format('%I.%I', $1, $2), 'DELETE')
             OR has_table_privilege(current_user, format('%I.%I', $1, $2), 'TRUNCATE')",
    )
    .bind(&table.schema)
    .bind(&table.table)
    .fetch_one(&mut *connection)
    .await
    .map_err(connection_error)?;

    Ok(row.try_get::<bool, _>(0).unwrap_or(false))
}

async fn owns_table(
    connection: &mut PgConnection,
    table: &QualifiedName,
) -> Result<bool, CoreError> {
    let owned: Option<bool> = sqlx::query_scalar(
        "SELECT c.relowner = (SELECT oid FROM pg_roles WHERE rolname = current_user)
         FROM pg_class c
         JOIN pg_namespace n ON n.oid = c.relnamespace
         WHERE n.nspname = $1 AND c.relname = $2",
    )
    .bind(&table.schema)
    .bind(&table.table)
    .fetch_optional(&mut *connection)
    .await
    .map_err(connection_error)?;

    match owned {
        Some(value) => Ok(value),
        None => Err(CoreError::with_detail(
            ErrorCode::Usage,
            format!("{table} does not exist"),
            "invalid_identifier",
        )),
    }
}

fn role_label(role: SessionRole) -> &'static str {
    match role {
        SessionRole::Source => "source",
        SessionRole::Destination => "destination",
    }
}

/// The one rule behind [`BoundSession::assert_binding`], extracted so it can be
/// tested without a live connection.
fn bind(
    role: SessionRole,
    actual: &SessionIdentity,
    expected: &SessionIdentity,
) -> Result<(), CoreError> {
    if actual.identity_key != expected.identity_key {
        return Err(CoreError::with_detail(
            ErrorCode::Conflict,
            "the database identity changed since the plan was made",
            role.changed_code(),
        ));
    }
    Ok(())
}

fn connection_error(error: sqlx::Error) -> CoreError {
    crate::safesync::database_error(
        ErrorCode::Connection,
        "database operation",
        "session_error",
        error,
    )
}

async fn apply_mariadb_source_guards(
    connection: &mut MySqlConnection,
    limits: &Limits,
) -> Result<(), CoreError> {
    sqlx::query("SET NAMES utf8mb4")
        .execute(&mut *connection)
        .await
        .map_err(connection_error)?;
    sqlx::query("SET SESSION time_zone = '+00:00'")
        .execute(&mut *connection)
        .await
        .map_err(connection_error)?;
    sqlx::query("SET SESSION TRANSACTION READ ONLY")
        .execute(&mut *connection)
        .await
        .map_err(connection_error)?;

    let stmt_secs = (limits.timeout_ms as f64 / 1000.0).max(1.0);
    sqlx::query("SET SESSION max_statement_time = ?")
        .bind(stmt_secs)
        .execute(&mut *connection)
        .await
        .map_err(connection_error)?;

    let lock_secs = (limits.timeout_ms.min(5_000) / 1000).max(1);
    sqlx::query("SET SESSION lock_wait_timeout = ?")
        .bind(lock_secs)
        .execute(&mut *connection)
        .await
        .map_err(connection_error)?;

    let tx_ro: i8 = sqlx::query_scalar("SELECT @@SESSION.tx_read_only")
        .fetch_one(&mut *connection)
        .await
        .map_err(connection_error)?;
    if tx_ro == 0 {
        return Err(CoreError::with_detail(
            ErrorCode::Operation,
            "the MariaDB source session is not read-only",
            "source_not_read_only",
        ));
    }

    Ok(())
}

async fn apply_mariadb_destination_guards(
    connection: &mut MySqlConnection,
    limits: &Limits,
) -> Result<(), CoreError> {
    sqlx::query("SET NAMES utf8mb4")
        .execute(&mut *connection)
        .await
        .map_err(connection_error)?;
    sqlx::query("SET SESSION time_zone = '+00:00'")
        .execute(&mut *connection)
        .await
        .map_err(connection_error)?;
    let lock_secs = (limits.timeout_ms.min(5_000) / 1000).max(1);
    sqlx::query("SET SESSION lock_wait_timeout = ?")
        .bind(lock_secs)
        .execute(&mut *connection)
        .await
        .map_err(connection_error)?;
    Ok(())
}

fn decode_mysql_string(row: &sqlx::mysql::MySqlRow, idx: usize) -> String {
    row.try_get::<String, _>(idx)
        .or_else(|_| {
            row.try_get::<Vec<u8>, _>(idx)
                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        })
        .unwrap_or_default()
}

async fn read_mariadb_identity(
    connection: &mut MySqlConnection,
    role: SessionRole,
    profile: &DbConnection,
) -> Result<SessionIdentity, CoreError> {
    let row = sqlx::query(
        "SELECT COALESCE(DATABASE(), ''), \
                CURRENT_USER(), \
                COALESCE(@@hostname, ''), \
                @@port, \
                VERSION(), \
                @@SESSION.tx_read_only, \
                CONNECTION_ID(), \
                COALESCE(CAST(@@datadir AS CHAR CHARACTER SET utf8mb4), ''), \
                COALESCE(CAST(@@server_id AS CHAR), '')",
    )
    .fetch_one(&mut *connection)
    .await
    .map_err(connection_error)?;

    let database_name = decode_mysql_string(&row, 0);
    let current_user = decode_mysql_string(&row, 1);
    let hostname = decode_mysql_string(&row, 2);
    let server_port_u32: u32 = row
        .try_get::<u32, _>(3)
        .or_else(|_| row.try_get::<u64, _>(3).map(|v| v as u32))
        .or_else(|_| row.try_get::<i64, _>(3).map(|v| v.max(0) as u32))
        .unwrap_or(0);
    let version = decode_mysql_string(&row, 4);
    let tx_read_only: i8 = row.try_get(5).unwrap_or(0);
    let conn_id: u64 = row
        .try_get::<u64, _>(6)
        .or_else(|_| row.try_get::<u32, _>(6).map(u64::from))
        .or_else(|_| row.try_get::<i64, _>(6).map(|v| v.max(0) as u64))
        .unwrap_or(0);
    let datadir = decode_mysql_string(&row, 7);
    let server_id = decode_mysql_string(&row, 8);

    if !crate::security::verify::is_mariadb_version(&version) {
        return Err(CoreError::with_detail(
            ErrorCode::Unsupported,
            "Safe Sync supports PostgreSQL and MariaDB only",
            "unsupported_engine",
        ));
    }

    let fingerprint = crate::security::verify::mariadb_server_fingerprint(
        &hostname,
        server_port_u32,
        &datadir,
        &server_id,
    );

    let is_read_only = match role {
        SessionRole::Source => tx_read_only != 0,
        SessionRole::Destination => tx_read_only != 0,
    };

    let server = ServerIdentity {
        engine: Some("mariadb".into()),
        version: Some(version),
        server_uuid: fingerprint.clone(),
        database: Some(database_name.clone()),
        database_id: Some(database_name.clone()),
        current_user: Some(current_user.clone()),
        read_only: Some(is_read_only),
        can_write: None,
    };

    let host = if hostname.is_empty() {
        profile.host.clone()
    } else {
        hostname.clone()
    };
    let port = if server_port_u32 > 0 {
        server_port_u32 as u16
    } else {
        profile.port
    };
    let identity_key = server.identity_key(DbEngine::Mysql, &host, port);

    Ok(SessionIdentity {
        engine: DbEngine::Mysql,
        identity_key,
        system_identifier: fingerprint,
        database_oid: database_name.clone(),
        database_name,
        current_user,
        backend_pid: (conn_id.min(i32::MAX as u64)) as i32,
        backend_start: String::new(),
        server_addr: hostname,
        server_port: port,
        transaction_read_only: is_read_only,
        host,
        port,
        server,
    })
}

async fn assert_mariadb_source_role(
    connection: &mut MySqlConnection,
    database_name: &str,
    tables: &[QualifiedName],
) -> Result<(), CoreError> {
    let tx_ro: i8 = sqlx::query_scalar("SELECT @@SESSION.tx_read_only")
        .fetch_one(&mut *connection)
        .await
        .map_err(connection_error)?;
    if tx_ro == 0 {
        return Err(CoreError::with_detail(
            ErrorCode::Operation,
            "the MariaDB source session is not read-only",
            "source_not_read_only",
        ));
    }

    let rows = sqlx::query("SHOW GRANTS FOR CURRENT_USER()")
        .fetch_all(&mut *connection)
        .await
        .map_err(connection_error)?;

    for row in rows {
        let grant = decode_mysql_string(&row, 0);
        if mariadb_grant_allows_write(&grant, database_name, tables) {
            return Err(CoreError::with_detail(
                ErrorCode::Operation,
                "the MariaDB source role is privileged or writable; use a SELECT-only role",
                "source_privileged",
            ));
        }
    }

    Ok(())
}

fn mariadb_grant_allows_write(grant: &str, database_name: &str, tables: &[QualifiedName]) -> bool {
    let upper = grant.trim().to_ascii_uppercase();
    let Some(rest) = upper.strip_prefix("GRANT ") else {
        return true;
    };
    let Some((privs_part, target_and_user)) = rest.split_once(" ON ") else {
        return true;
    };
    let Some((target_part, _)) = target_and_user.split_once(" TO ") else {
        return true;
    };

    let target = target_part.trim().replace('`', "");
    let applies = if target == "*.*" {
        true
    } else if let Some((schema, table)) = target.split_once('.') {
        let db_match = mariadb_schema_matches(schema, database_name)
            || tables
                .iter()
                .any(|t| mariadb_schema_matches(schema, &t.schema));
        if !db_match {
            false
        } else if table == "*" {
            true
        } else {
            tables.iter().any(|t| {
                mariadb_schema_matches(schema, &t.schema) && t.table.eq_ignore_ascii_case(table)
            })
        }
    } else {
        true
    };

    if !applies {
        return false;
    }

    const DISALLOWED: &[&str] = &[
        "ALL PRIVILEGES",
        "ALL",
        "SUPER",
        "INSERT",
        "UPDATE",
        "DELETE",
        "ALTER",
        "DROP",
        "CREATE",
        "INDEX",
        "REFERENCES",
        "LOCK TABLES",
        "TRIGGER",
        "EVENT",
        "EXECUTE",
        "PROCESS",
        "RELOAD",
        "SHUTDOWN",
        "FILE",
        "GRANT OPTION",
    ];

    privs_part.split(',').any(|raw| {
        let p = raw.trim();
        let base = p.split('(').next().unwrap_or(p).trim();
        DISALLOWED.contains(&base)
    })
}

fn mariadb_schema_matches(pattern: &str, candidate: &str) -> bool {
    let pat: Vec<char> = pattern.to_ascii_uppercase().chars().collect();
    let cand: Vec<char> = candidate.to_ascii_uppercase().chars().collect();
    fn rec(p: &[char], c: &[char]) -> bool {
        if p.is_empty() {
            return c.is_empty();
        }
        if p[0] == '\\' && p.len() > 1 {
            return !c.is_empty() && p[1] == c[0] && rec(&p[2..], &c[1..]);
        }
        if p[0] == '%' {
            return rec(&p[1..], c) || (!c.is_empty() && rec(p, &c[1..]));
        }
        if p[0] == '_' {
            return !c.is_empty() && rec(&p[1..], &c[1..]);
        }
        !c.is_empty() && p[0] == c[0] && rec(&p[1..], &c[1..])
    }
    rec(&pat, &cand)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::safesync::spec::Limits;

    fn identity(key: Option<&str>, pid: i32) -> SessionIdentity {
        SessionIdentity {
            engine: DbEngine::Postgres,
            identity_key: key.map(str::to_string),
            system_identifier: Some("1".into()),
            database_oid: "16384".into(),
            database_name: "db".into(),
            current_user: "dev".into(),
            backend_pid: pid,
            backend_start: "2026-01-01".into(),
            server_addr: "127.0.0.1".into(),
            server_port: 5432,
            transaction_read_only: true,
            host: "127.0.0.1".into(),
            port: 5432,
            server: ServerIdentity {
                server_uuid: Some("1".into()),
                database_id: Some("16384".into()),
                ..ServerIdentity::default()
            },
        }
    }

    #[test]
    fn a_matching_identity_key_binds() {
        bind(
            SessionRole::Source,
            &identity(Some("postgres://h:1/db#1"), 1),
            &identity(Some("postgres://h:1/db#1"), 99),
        )
        .unwrap();
    }

    #[test]
    fn a_changed_identity_key_is_refused() {
        let error = bind(
            SessionRole::Destination,
            &identity(Some("postgres://h:1/db#1"), 1),
            &identity(Some("postgres://h:1/db#2"), 1),
        )
        .unwrap_err();
        assert_eq!(error.detail.as_deref(), Some("destination_changed"));
    }

    #[test]
    fn a_missing_identity_is_unverifiable() {
        let error = bind(
            SessionRole::Source,
            &identity(None, 1),
            &identity(Some("postgres://h:1/db#1"), 1),
        )
        .unwrap_err();
        assert_eq!(error.detail.as_deref(), Some("source_changed"));
    }

    #[test]
    fn a_default_limit_set_is_documented() {
        let limits = Limits::default();
        assert_eq!(limits.timeout_ms, 60_000);
    }

    #[test]
    fn mariadb_grant_parser_distinguishes_select_from_writes() {
        let tables = vec![QualifiedName::parse_with_default("orders", "mdb_src").unwrap()];
        assert!(!mariadb_grant_allows_write(
            "GRANT USAGE ON *.* TO `mdb_ro`@`%`",
            "mdb_src",
            &tables
        ));
        assert!(!mariadb_grant_allows_write(
            "GRANT SELECT ON `mdb_src`.* TO `mdb_ro`@`%`",
            "mdb_src",
            &tables
        ));
        assert!(!mariadb_grant_allows_write(
            "GRANT INSERT ON `other_db`.* TO `mdb_ro`@`%`",
            "mdb_src",
            &tables
        ));
        assert!(mariadb_grant_allows_write(
            "GRANT ALL PRIVILEGES ON *.* TO `root`@`%`",
            "mdb_src",
            &tables
        ));
        assert!(mariadb_grant_allows_write(
            "GRANT SELECT, INSERT ON `mdb_src`.* TO `mdb_rw`@`%`",
            "mdb_src",
            &tables
        ));
        assert!(mariadb_grant_allows_write(
            "GRANT UPDATE (`status`) ON `mdb_src`.`orders` TO `mdb_rw`@`%`",
            "mdb_src",
            &tables
        ));
        assert!(mariadb_grant_allows_write(
            "GRANT INSERT ON `mdb\\_src`.* TO `mdb_rw`@`%`",
            "mdb_src",
            &tables
        ));
        assert!(mariadb_grant_allows_write(
            "GRANT DELETE ON `mdb_%`.* TO `mdb_rw`@`%`",
            "mdb_src",
            &tables
        ));
    }
}
