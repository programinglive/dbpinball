//! A database session that owns exactly one connection and its identity.
//!
//! The identity is read on the same connection that later runs every statement,
//! so there is no pool that can hand out a different server between the check and
//! the work. A reconnect builds a new session and therefore a new authorization.

use serde::Serialize;
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
            engine: self.engine.clone(),
            host: self.host.clone(),
            port: self.port,
            identity_key: self.identity_key.clone(),
            server: self.server.clone(),
            observed_at_ms,
        }
    }
}

/// One owned connection plus the identity read from it.
pub struct BoundSession {
    role: SessionRole,
    connection: PgConnection,
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
        if profile.engine != DbEngine::Postgres {
            return Err(CoreError::with_detail(
                ErrorCode::Unsupported,
                "Safe Sync supports PostgreSQL only",
                "unsupported_engine",
            ));
        }

        let secret = credentials.get(&profile.id)?;
        let options = PgConnectOptions::new()
            .host(&profile.host)
            .port(profile.port)
            .username(&profile.username)
            .password(secret.expose())
            .database(&profile.database);

        let mut connection = PgConnection::connect_with(&options)
            .await
            .map_err(|error| {
                CoreError::redacted(
                    ErrorCode::Connection,
                    format!("the {} could not be reached: {error}", role_label(role)),
                )
            })?;

        if role == SessionRole::Source {
            apply_source_guards(&mut connection, limits).await?;
        }

        let identity = read_identity(&mut connection, role, profile).await?;

        Ok(Self {
            role,
            connection,
            identity,
        })
    }

    pub fn role(&self) -> SessionRole {
        self.role
    }

    pub fn identity(&self) -> &SessionIdentity {
        &self.identity
    }

    pub fn connection(&mut self) -> &mut PgConnection {
        &mut self.connection
    }

    /// Fail when the session is no longer the one the plan recorded.
    pub fn assert_binding(&self, expected: &SessionIdentity) -> Result<(), CoreError> {
        bind(self.role, &self.identity, expected)
    }

    /// Refuse a source role that could write, or that could bypass a row policy.
    pub async fn assert_source_role(&mut self, tables: &[QualifiedName]) -> Result<(), CoreError> {
        let (is_super, bypass_rls): (bool, bool) = sqlx::query_as(
            "SELECT rolsuper, rolbypassrls FROM pg_roles WHERE rolname = current_user",
        )
        .fetch_one(&mut self.connection)
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
            if table_writes(&mut self.connection, table).await? {
                return Err(CoreError::with_detail(
                    ErrorCode::Operation,
                    format!("the source role can write to {table}"),
                    "source_privileged",
                ));
            }
            if owns_table(&mut self.connection, table).await? {
                return Err(CoreError::with_detail(
                    ErrorCode::Operation,
                    format!("the source role owns {table}"),
                    "source_privileged",
                ));
            }
        }

        Ok(())
    }

    /// Open a read-only transaction. A write on it fails at the engine.
    pub async fn begin_read_only(&mut self) -> Result<(), CoreError> {
        sqlx::query("BEGIN READ ONLY")
            .execute(&mut self.connection)
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

    /// Open the destination transaction.
    ///
    /// `READ COMMITTED` is deliberate. The destination is protected by a
    /// transaction-scoped advisory lock per plan and by `SHARE ROW EXCLUSIVE`
    /// table locks, so every statement sees the state left by whoever held the
    /// lock before. `REPEATABLE READ` would take its snapshot before the
    /// advisory lock is granted, so a waiting run could miss the run that just
    /// committed and then collide on the primary key.
    pub async fn begin_destination_transaction(&mut self) -> Result<(), CoreError> {
        sqlx::query("BEGIN ISOLATION LEVEL READ COMMITTED")
            .execute(&mut self.connection)
            .await
            .map_err(connection_error)?;
        Ok(())
    }

    pub async fn rollback(&mut self) -> Result<(), CoreError> {
        sqlx::query("ROLLBACK")
            .execute(&mut self.connection)
            .await
            .map_err(connection_error)?;
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
}
