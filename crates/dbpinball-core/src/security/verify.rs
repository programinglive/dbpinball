//! Engine-specific destination identity verification.
//!
//! Each engine reports a different set of facts. This module asks the server
//! what it is, records what it answered, and never fills a gap with a guess.
//! A missing answer becomes `None`, and the policy treats an unidentified
//! server as unverifiable.

use crate::domain::connection::{DbConnection, DbEngine};
use crate::domain::error::{CoreError, ErrorCode};
use crate::ports::credentials::CredentialProvider;
use crate::security::destination::{DestinationIdentity, ServerIdentity};
use crate::services::inspect;

/// Read the identity of the database a connection points at.
pub async fn verify_destination(
    connection: &DbConnection,
    credentials: &dyn CredentialProvider,
) -> Result<DestinationIdentity, CoreError> {
    match connection.engine {
        DbEngine::Mysql => verify_mysql(connection, credentials).await,
        DbEngine::Postgres => verify_postgres(connection, credentials).await,
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

async fn verify_postgres(
    connection: &DbConnection,
    credentials: &dyn CredentialProvider,
) -> Result<DestinationIdentity, CoreError> {
    let database = if connection.database.trim().is_empty() {
        "postgres"
    } else {
        connection.database.as_str()
    };

    let pool = inspect::connect_postgres(connection, database, credentials).await?;

    // `pg_control_system()` needs elevated privileges, so it is attempted and
    // allowed to fail. The database OID plus the reported address is the
    // fallback identity.
    let row = sqlx::query(
        "SELECT current_database()::text, \
                current_user::text, \
                COALESCE(inet_server_addr()::text, ''), \
                COALESCE(inet_server_port(), 0)::int, \
                version(), \
                (SELECT oid::text FROM pg_database WHERE datname = current_database()), \
                current_setting('transaction_read_only')",
    )
    .fetch_one(&pool)
    .await
    .map_err(|error| {
        CoreError::redacted(
            ErrorCode::Connection,
            format!("destination identity query failed: {error}"),
        )
    })?;

    use sqlx::Row;
    let database_name: String = row.try_get(0).unwrap_or_default();
    let current_user: String = row.try_get(1).unwrap_or_default();
    let server_addr: String = row.try_get(2).unwrap_or_default();
    let server_port: i32 = row.try_get(3).unwrap_or(0);
    let version: String = row.try_get(4).unwrap_or_default();
    let database_oid: String = row.try_get(5).unwrap_or_default();
    let read_only_setting: String = row.try_get(6).unwrap_or_default();

    let cluster_identifier = postgres_cluster_identifier(&pool).await;

    let server = ServerIdentity {
        engine: Some("postgres".into()),
        version: Some(version),
        server_uuid: cluster_identifier,
        database: Some(database_name.clone()),
        database_id: Some(database_oid),
        current_user: Some(current_user),
        read_only: Some(read_only_setting.eq_ignore_ascii_case("on")),
        can_write: None,
    };

    let host = if server_addr.is_empty() {
        connection.host.clone()
    } else {
        server_addr
    };
    let port = if server_port > 0 {
        server_port as u16
    } else {
        connection.port
    };

    Ok(DestinationIdentity {
        engine: DbEngine::Postgres,
        identity_key: server.identity_key(DbEngine::Postgres, &host, port),
        host,
        port,
        server,
        observed_at_ms: now_ms(),
    })
}

/// Best-effort cluster identifier.
///
/// `pg_control_system()` is the strongest evidence but is restricted. When it
/// is unavailable the function returns `None`, and the policy treats the
/// destination as unverifiable rather than inventing an identity.
async fn postgres_cluster_identifier(pool: &sqlx::PgPool) -> Option<String> {
    let value: Result<String, sqlx::Error> =
        sqlx::query_scalar("SELECT system_identifier::text FROM pg_control_system()")
            .fetch_one(pool)
            .await;

    match value {
        Ok(identifier) if !identifier.trim().is_empty() => Some(identifier),
        _ => None,
    }
}

async fn verify_mysql(
    connection: &DbConnection,
    credentials: &dyn CredentialProvider,
) -> Result<DestinationIdentity, CoreError> {
    let database = if connection.database.trim().is_empty() {
        "information_schema"
    } else {
        connection.database.as_str()
    };

    let pool = inspect::connect_mysql(connection, database, credentials).await?;

    let row = sqlx::query(
        "SELECT COALESCE(DATABASE(), ''), \
                CURRENT_USER(), \
                COALESCE(@@hostname, ''), \
                @@port, \
                VERSION(), \
                @@read_only",
    )
    .fetch_one(&pool)
    .await
    .map_err(|error| {
        CoreError::redacted(
            ErrorCode::Connection,
            format!("destination identity query failed: {error}"),
        )
    })?;

    use sqlx::Row;
    let database_name: String = row.try_get(0).unwrap_or_default();
    let current_user: String = row.try_get(1).unwrap_or_default();
    let hostname: String = row.try_get(2).unwrap_or_default();
    let port: u32 = row
        .try_get::<u32, _>(3)
        .or_else(|_| row.try_get::<u64, _>(3).map(|v| v as u32))
        .or_else(|_| row.try_get::<i64, _>(3).map(|v| v.max(0) as u32))
        .unwrap_or(0);
    let version: String = row.try_get(4).unwrap_or_default();
    let read_only: i8 = row.try_get(5).unwrap_or(0);

    let is_mariadb = is_mariadb_version(&version);
    let (engine_label, server_uuid) = if is_mariadb {
        let mdb_row = sqlx::query(
            "SELECT COALESCE(CAST(@@datadir AS CHAR CHARACTER SET utf8mb4), ''), \
                    COALESCE(CAST(@@server_id AS CHAR), '')",
        )
        .fetch_one(&pool)
        .await
        .map_err(|error| {
            CoreError::redacted(
                ErrorCode::Connection,
                format!("destination identity query failed: {error}"),
            )
        })?;
        let datadir: String = mdb_row
            .try_get::<String, _>(0)
            .or_else(|_| {
                mdb_row
                    .try_get::<Vec<u8>, _>(0)
                    .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            })
            .unwrap_or_default();
        let server_id: String = mdb_row
            .try_get::<String, _>(1)
            .or_else(|_| {
                mdb_row
                    .try_get::<Vec<u8>, _>(1)
                    .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            })
            .unwrap_or_default();
        (
            "mariadb",
            mariadb_server_fingerprint(&hostname, port, &datadir, &server_id),
        )
    } else {
        let mysql_row = sqlx::query("SELECT COALESCE(@@server_uuid, '')")
            .fetch_one(&pool)
            .await
            .map_err(|error| {
                CoreError::redacted(
                    ErrorCode::Connection,
                    format!("destination identity query failed: {error}"),
                )
            })?;
        let raw_uuid: String = mysql_row.try_get(0).unwrap_or_default();
        let uuid = if raw_uuid.trim().is_empty() {
            None
        } else {
            Some(raw_uuid)
        };
        ("mysql", uuid)
    };

    let server = ServerIdentity {
        engine: Some(engine_label.into()),
        version: Some(version),
        server_uuid,
        database: Some(database_name.clone()),
        database_id: Some(database_name),
        current_user: Some(current_user),
        read_only: Some(read_only != 0),
        can_write: None,
    };

    let host = if hostname.is_empty() {
        connection.host.clone()
    } else {
        hostname
    };
    let port = if port > 0 {
        port as u16
    } else {
        connection.port
    };

    Ok(DestinationIdentity {
        engine: DbEngine::Mysql,
        identity_key: server.identity_key(DbEngine::Mysql, &host, port),
        host,
        port,
        server,
        observed_at_ms: now_ms(),
    })
}

pub(crate) fn is_mariadb_version(version: &str) -> bool {
    version.to_ascii_lowercase().contains("mariadb")
}

/// Build a collision-resistant server identity fingerprint for MariaDB, which
/// does not expose MySQL's `@@server_uuid`. Fails closed (`None`) if any
/// required server property is missing or ambiguous rather than falling back to
/// `@@server_id` alone.
pub(crate) fn mariadb_server_fingerprint(
    hostname: &str,
    port: u32,
    datadir: &str,
    server_id: &str,
) -> Option<String> {
    use sha2::{Digest, Sha256};

    let host = hostname.trim();
    let dir = datadir.trim();
    let sid = server_id.trim();
    if host.is_empty() || dir.is_empty() || port == 0 {
        return None;
    }

    let mut hasher = Sha256::new();
    hasher.update(b"dbpinball/mariadb-server-identity/v1|");
    hasher.update(host.as_bytes());
    hasher.update(b"|");
    hasher.update(port.to_string().as_bytes());
    hasher.update(b"|");
    hasher.update(dir.as_bytes());
    hasher.update(b"|");
    hasher.update(sid.as_bytes());
    let digest = hasher.finalize();

    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    Some(format!("mariadb-fp:{hex}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_postgres_identity_key_includes_the_cluster_and_database() {
        let server = ServerIdentity {
            server_uuid: Some("12345".into()),
            database_id: Some("16384".into()),
            ..ServerIdentity::default()
        };
        let key = server
            .identity_key(DbEngine::Postgres, "127.0.0.1", 5432)
            .unwrap();
        assert!(key.starts_with("postgres://127.0.0.1:5432/16384#12345"));
    }

    #[test]
    fn a_mysql_identity_key_is_engine_scoped() {
        let server = ServerIdentity {
            server_uuid: Some("uuid-1".into()),
            database_id: Some("app".into()),
            ..ServerIdentity::default()
        };
        let key = server
            .identity_key(DbEngine::Mysql, "db.internal", 3306)
            .unwrap();
        assert!(key.starts_with("mysql://db.internal:3306/app#uuid-1"));
    }

    #[test]
    fn a_missing_server_uuid_produces_no_identity() {
        let server = ServerIdentity {
            server_uuid: None,
            database_id: Some("app".into()),
            ..ServerIdentity::default()
        };
        assert!(server
            .identity_key(DbEngine::Postgres, "127.0.0.1", 5432)
            .is_none());
    }

    #[test]
    fn mariadb_version_detection_distinguishes_mariadb_from_mysql() {
        assert!(is_mariadb_version("10.11.19-MariaDB-0+deb12u1-log"));
        assert!(is_mariadb_version("11.4.2-mariadb"));
        assert!(!is_mariadb_version("8.4.0"));
        assert!(!is_mariadb_version("8.0.39-0ubuntu0.24.04.1"));
    }

    #[test]
    fn mariadb_fingerprint_is_deterministic_sensitive_and_fails_closed() {
        let fp1 = mariadb_server_fingerprint("bw-server-clean", 3306, "/var/lib/mysql/", "1")
            .expect("valid MariaDB identity");
        let fp2 = mariadb_server_fingerprint("bw-server-clean", 3306, "/var/lib/mysql/", "1")
            .expect("deterministic");
        assert_eq!(fp1, fp2);
        assert!(fp1.starts_with("mariadb-fp:"));

        let different_host =
            mariadb_server_fingerprint("other-host", 3306, "/var/lib/mysql/", "1").unwrap();
        let different_dir =
            mariadb_server_fingerprint("bw-server-clean", 3306, "/var/lib/mysql2/", "1").unwrap();
        let different_port =
            mariadb_server_fingerprint("bw-server-clean", 3307, "/var/lib/mysql/", "1").unwrap();
        assert_ne!(fp1, different_host);
        assert_ne!(fp1, different_dir);
        assert_ne!(fp1, different_port);

        // Fail closed when required identity components are missing or invalid.
        assert!(mariadb_server_fingerprint("", 3306, "/var/lib/mysql/", "1").is_none());
        assert!(mariadb_server_fingerprint("bw-server-clean", 3306, "", "1").is_none());
        assert!(mariadb_server_fingerprint("bw-server-clean", 0, "/var/lib/mysql/", "1").is_none());
    }
}
