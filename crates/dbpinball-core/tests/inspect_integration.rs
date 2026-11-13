#![cfg(feature = "integration")]

//! Integration tests for the inspect service.
//!
//! These run only when `--features integration` is passed and the matching
//! environment variable is set. They never run in the default test suite, so
//! `cargo test` stays fast and offline.

use dbpinball_core::domain::connection::{DbConnection, DbEngine};
use dbpinball_core::ports::credentials::{CredentialProvider, Secret};
use dbpinball_core::services::inspect;

struct FixedCredentialProvider(String);

impl CredentialProvider for FixedCredentialProvider {
    fn get(
        &self,
        _connection_id: &str,
    ) -> Result<Secret, dbpinball_core::domain::error::CoreError> {
        Ok(Secret::new(self.0.clone()))
    }
}

fn connection_from_url(value: &str) -> Option<(DbConnection, FixedCredentialProvider)> {
    let parsed = url::Url::parse(value).ok()?;

    let engine = match parsed.scheme() {
        "postgres" | "postgresql" => DbEngine::Postgres,
        "mysql" => DbEngine::Mysql,
        _ => return None,
    };

    let default_port = if engine == DbEngine::Postgres {
        5432
    } else {
        3306
    };

    let connection = DbConnection {
        id: "integration".to_string(),
        name: "integration".to_string(),
        engine,
        host: parsed.host_str()?.to_string(),
        port: parsed.port().unwrap_or(default_port),
        database: parsed.path().trim_start_matches('/').to_string(),
        username: parsed.username().to_string(),
        use_ssl: false,
    };

    let credentials = FixedCredentialProvider(parsed.password().unwrap_or_default().to_string());

    Some((connection, credentials))
}

#[tokio::test]
async fn postgres_connection_is_inspected() {
    let Ok(value) = std::env::var("DBPINBALL_TEST_PG_URL") else {
        eprintln!("skipping: DBPINBALL_TEST_PG_URL is not set");
        return;
    };

    let (connection, credentials) = connection_from_url(&value).expect("valid PostgreSQL URL");

    inspect::test_connection(&connection, &credentials)
        .await
        .expect("connection should succeed");

    let databases = inspect::list_databases(&connection, &credentials)
        .await
        .expect("databases should be listed");

    assert!(!databases.is_empty(), "expected at least one database");

    let tables = inspect::list_tables(&connection, &connection.database, &credentials)
        .await
        .expect("tables should be listed");

    assert!(tables.len() < 1_000_000, "sanity bound on the table list");
}

#[tokio::test]
async fn mysql_connection_is_inspected() {
    let Ok(value) = std::env::var("DBPINBALL_TEST_MYSQL_URL") else {
        eprintln!("skipping: DBPINBALL_TEST_MYSQL_URL is not set");
        return;
    };

    let (connection, credentials) = connection_from_url(&value).expect("valid MySQL URL");

    inspect::test_connection(&connection, &credentials)
        .await
        .expect("connection should succeed");

    let databases = inspect::list_databases(&connection, &credentials)
        .await
        .expect("databases should be listed");

    assert!(!databases.is_empty(), "expected at least one database");

    let tables = inspect::list_tables(&connection, &connection.database, &credentials)
        .await
        .expect("tables should be listed");

    assert!(tables.len() < 1_000_000, "sanity bound on the table list");
}
