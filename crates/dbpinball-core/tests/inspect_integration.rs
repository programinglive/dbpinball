#![cfg(feature = "integration")]

//! Integration tests for the inspect service.
//!
//! These run only when `--features integration` is passed and the matching
//! environment variable is set. They never run in the default test suite, so
//! `cargo test` stays fast and offline.
//!
//! The CI job provisions disposable PostgreSQL 16 and MySQL 8 service
//! containers and exports `DBPINBALL_TEST_PG_URL` / `DBPINBALL_TEST_MYSQL_URL`.

use dbpinball_core::domain::connection::{DbConnection, DbEngine};
use dbpinball_core::domain::error::ErrorCode;
use dbpinball_core::ports::credentials::{CredentialProvider, Secret};
use dbpinball_core::services::inspect;

const PROBE_TABLE: &str = "dbpinball_inspect_probe";

struct FixedCredentialProvider(String);

impl CredentialProvider for FixedCredentialProvider {
    fn get(
        &self,
        _connection_id: &str,
    ) -> Result<Secret, dbpinball_core::domain::error::CoreError> {
        Ok(Secret::new(self.0.clone()))
    }
}

struct Target {
    connection: DbConnection,
    credentials: FixedCredentialProvider,
    url: String,
}

fn target(variable: &str, engine: DbEngine) -> Option<Target> {
    let url = std::env::var(variable).ok()?;
    let parsed = url::Url::parse(&url).ok()?;

    let default_port = if engine == DbEngine::Postgres {
        5432
    } else {
        3306
    };

    let connection = DbConnection {
        id: format!("integration-{}", variable),
        name: "integration".to_string(),
        engine,
        host: parsed.host_str()?.to_string(),
        port: parsed.port().unwrap_or(default_port),
        database: parsed.path().trim_start_matches('/').to_string(),
        username: parsed.username().to_string(),
        use_ssl: false,
    };

    let credentials = FixedCredentialProvider(parsed.password().unwrap_or_default().to_string());

    Some(Target {
        connection,
        credentials,
        url,
    })
}

fn postgres() -> Option<Target> {
    target("DBPINBALL_TEST_PG_URL", DbEngine::Postgres)
}

fn mysql() -> Option<Target> {
    target("DBPINBALL_TEST_MYSQL_URL", DbEngine::Mysql)
}

async fn pg_pool(url: &str) -> sqlx::PgPool {
    sqlx::PgPool::connect(url)
        .await
        .expect("raw PostgreSQL connection should succeed")
}

async fn mysql_pool(url: &str) -> sqlx::MySqlPool {
    sqlx::MySqlPool::connect(url)
        .await
        .expect("raw MySQL connection should succeed")
}

async fn create_probe_pg(pool: &sqlx::PgPool) {
    sqlx::query(&format!(
        "CREATE TABLE IF NOT EXISTS {PROBE_TABLE} (id INT PRIMARY KEY, note VARCHAR(64))"
    ))
    .execute(pool)
    .await
    .expect("probe table should be created");

    sqlx::query(&format!(
        "INSERT INTO {PROBE_TABLE} (id, note) VALUES (1, 'synthetic') ON CONFLICT (id) DO NOTHING"
    ))
    .execute(pool)
    .await
    .expect("probe row should be inserted");
}

async fn create_probe_mysql(pool: &sqlx::MySqlPool) {
    sqlx::query(&format!(
        "CREATE TABLE IF NOT EXISTS {PROBE_TABLE} (id INT PRIMARY KEY, note VARCHAR(64))"
    ))
    .execute(pool)
    .await
    .expect("probe table should be created");

    sqlx::query(&format!(
        "INSERT IGNORE INTO {PROBE_TABLE} (id, note) VALUES (1, 'synthetic')"
    ))
    .execute(pool)
    .await
    .expect("probe row should be inserted");
}

async fn count_probe_pg(pool: &sqlx::PgPool) -> i64 {
    sqlx::query_scalar::<_, i64>(&format!("SELECT COUNT(*) FROM {PROBE_TABLE}"))
        .fetch_one(pool)
        .await
        .expect("probe row should be readable")
}

async fn count_probe_mysql(pool: &sqlx::MySqlPool) -> i64 {
    sqlx::query_scalar::<_, i64>(&format!("SELECT COUNT(*) FROM {PROBE_TABLE}"))
        .fetch_one(pool)
        .await
        .expect("probe row should be readable")
}

#[tokio::test]
async fn postgres_connection_and_listing() {
    let Some(target) = postgres() else {
        eprintln!("skipping: DBPINBALL_TEST_PG_URL is not set");
        return;
    };

    inspect::test_connection(&target.connection, &target.credentials)
        .await
        .expect("PostgreSQL connection should succeed");

    let databases = inspect::list_databases(&target.connection, &target.credentials)
        .await
        .expect("databases should be listed");

    assert!(
        databases
            .iter()
            .any(|name| name == &target.connection.database),
        "expected the test database in {databases:?}"
    );

    let pool = pg_pool(&target.url).await;
    create_probe_pg(&pool).await;

    let tables = inspect::list_tables(
        &target.connection,
        &target.connection.database,
        &target.credentials,
    )
    .await
    .expect("tables should be listed");

    assert!(
        tables.iter().any(|name| name == PROBE_TABLE),
        "expected {PROBE_TABLE} in {tables:?}"
    );

    sqlx::query(&format!("DROP TABLE IF EXISTS {PROBE_TABLE}"))
        .execute(&pool)
        .await
        .expect("probe table should be dropped");
}

#[tokio::test]
async fn mysql_connection_and_listing() {
    let Some(target) = mysql() else {
        eprintln!("skipping: DBPINBALL_TEST_MYSQL_URL is not set");
        return;
    };

    inspect::test_connection(&target.connection, &target.credentials)
        .await
        .expect("MySQL connection should succeed");

    let databases = inspect::list_databases(&target.connection, &target.credentials)
        .await
        .expect("databases should be listed");

    assert!(
        databases
            .iter()
            .any(|name| name == &target.connection.database),
        "expected the test database in {databases:?}"
    );

    let pool = mysql_pool(&target.url).await;
    create_probe_mysql(&pool).await;

    let tables = inspect::list_tables(
        &target.connection,
        &target.connection.database,
        &target.credentials,
    )
    .await
    .expect("tables should be listed");

    assert!(
        tables.iter().any(|name| name == PROBE_TABLE),
        "expected {PROBE_TABLE} in {tables:?}"
    );

    sqlx::query(&format!("DROP TABLE IF EXISTS {PROBE_TABLE}"))
        .execute(&pool)
        .await
        .expect("probe table should be dropped");
}

#[tokio::test]
async fn postgres_authentication_failure_is_a_redacted_connection_error() {
    let Some(mut target) = postgres() else {
        eprintln!("skipping: DBPINBALL_TEST_PG_URL is not set");
        return;
    };

    let wrong_password = "definitely-not-the-password";
    target.credentials = FixedCredentialProvider(wrong_password.to_string());

    let error = inspect::test_connection(&target.connection, &target.credentials)
        .await
        .expect_err("a wrong password must fail");

    assert_eq!(error.code, ErrorCode::Connection);
    assert!(!error.message.contains(wrong_password));
}

#[tokio::test]
async fn mysql_authentication_failure_is_a_redacted_connection_error() {
    let Some(mut target) = mysql() else {
        eprintln!("skipping: DBPINBALL_TEST_MYSQL_URL is not set");
        return;
    };

    let wrong_password = "definitely-not-the-password";
    target.credentials = FixedCredentialProvider(wrong_password.to_string());

    let error = inspect::test_connection(&target.connection, &target.credentials)
        .await
        .expect_err("a wrong password must fail");

    assert_eq!(error.code, ErrorCode::Connection);
    assert!(!error.message.contains(wrong_password));
}

#[tokio::test]
async fn postgres_invalid_connection_returns_a_connection_error() {
    let Some(mut target) = postgres() else {
        eprintln!("skipping: DBPINBALL_TEST_PG_URL is not set");
        return;
    };

    target.connection.port = 1;

    let error = inspect::test_connection(&target.connection, &target.credentials)
        .await
        .expect_err("an unreachable port must fail");

    assert_eq!(error.code, ErrorCode::Connection);
}

#[tokio::test]
async fn mysql_invalid_connection_returns_a_connection_error() {
    let Some(mut target) = mysql() else {
        eprintln!("skipping: DBPINBALL_TEST_MYSQL_URL is not set");
        return;
    };

    target.connection.port = 1;

    let error = inspect::test_connection(&target.connection, &target.credentials)
        .await
        .expect_err("an unreachable port must fail");

    assert_eq!(error.code, ErrorCode::Connection);
}

#[tokio::test]
async fn postgres_inspection_does_not_mutate_data() {
    let Some(target) = postgres() else {
        eprintln!("skipping: DBPINBALL_TEST_PG_URL is not set");
        return;
    };

    let pool = pg_pool(&target.url).await;
    create_probe_pg(&pool).await;

    inspect::test_connection(&target.connection, &target.credentials)
        .await
        .expect("connection should succeed");
    inspect::list_databases(&target.connection, &target.credentials)
        .await
        .expect("databases should be listed");
    inspect::list_tables(
        &target.connection,
        &target.connection.database,
        &target.credentials,
    )
    .await
    .expect("tables should be listed");

    assert_eq!(
        count_probe_pg(&pool).await,
        1,
        "inspection must not delete rows"
    );

    sqlx::query(&format!("DROP TABLE IF EXISTS {PROBE_TABLE}"))
        .execute(&pool)
        .await
        .expect("probe table should be dropped");
}

#[tokio::test]
async fn mysql_inspection_does_not_mutate_data() {
    let Some(target) = mysql() else {
        eprintln!("skipping: DBPINBALL_TEST_MYSQL_URL is not set");
        return;
    };

    let pool = mysql_pool(&target.url).await;
    create_probe_mysql(&pool).await;

    inspect::test_connection(&target.connection, &target.credentials)
        .await
        .expect("connection should succeed");
    inspect::list_databases(&target.connection, &target.credentials)
        .await
        .expect("databases should be listed");
    inspect::list_tables(
        &target.connection,
        &target.connection.database,
        &target.credentials,
    )
    .await
    .expect("tables should be listed");

    assert_eq!(
        count_probe_mysql(&pool).await,
        1,
        "inspection must not delete rows"
    );

    sqlx::query(&format!("DROP TABLE IF EXISTS {PROBE_TABLE}"))
        .execute(&pool)
        .await
        .expect("probe table should be dropped");
}
