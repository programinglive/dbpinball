#![cfg(feature = "integration")]

//! Integration tests for the M3 destination verifier against real engines.
//!
//! These run only with `--features integration` and the disposable database
//! URLs in `DBPINBALL_TEST_PG_URL` / `DBPINBALL_TEST_MYSQL_URL`. A skipped run
//! is not engine verification, and the CI job that provides the URLs is the
//! only place these count.
//!
//! Every test uses a synthetic role and a disposable database. No production
//! credential is ever used.

use dbpinball_core::domain::connection::{DbConnection, DbEngine, TargetClass};
use dbpinball_core::domain::error::CoreError;
use dbpinball_core::ports::credentials::{CredentialProvider, Secret};
use dbpinball_core::security::{
    self, DatabaseOperation, DestinationCategory, EnrolledFingerprint, EnrollmentMatch,
    EnrollmentRegistry, FreshnessPolicy, ServerIdentity,
};

struct FixedCredentialProvider(String);

impl CredentialProvider for FixedCredentialProvider {
    fn get(&self, _connection_id: &str) -> Result<Secret, CoreError> {
        Ok(Secret::new(self.0.clone()))
    }

    fn set(&self, _connection_id: &str, _secret: &Secret) -> Result<(), CoreError> {
        Ok(())
    }

    fn delete(&self, _connection_id: &str) -> Result<(), CoreError> {
        Ok(())
    }
}

struct Target {
    connection: DbConnection,
    credentials: FixedCredentialProvider,
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
        id: "m3-verifier".to_string(),
        name: "m3-verifier".to_string(),
        engine,
        host: parsed.host_str()?.to_string(),
        port: parsed.port().unwrap_or(default_port),
        database: parsed.path().trim_start_matches('/').to_string(),
        username: parsed.username().to_string(),
        use_ssl: false,
        target: TargetClass::Unknown,
    };

    let credentials = FixedCredentialProvider(parsed.password().unwrap_or_default().to_string());

    Some(Target {
        connection,
        credentials,
    })
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn enrollment_for(
    identity: &security::DestinationIdentity,
    category: DestinationCategory,
) -> EnrolledFingerprint {
    EnrolledFingerprint {
        identity_key: identity.identity_key.clone().expect("identity key"),
        engine: format!("{:?}", identity.engine).to_ascii_lowercase(),
        database: identity.server.database.clone().unwrap_or_default(),
        category,
        enrolled_at_ms: now_ms(),
        note: None,
    }
}

// ---------------------------------------------------------------------------
// PostgreSQL
// ---------------------------------------------------------------------------

#[tokio::test]
async fn postgres_verifier_reports_a_real_identity() {
    let Some(target) = target("DBPINBALL_TEST_PG_URL", DbEngine::Postgres) else {
        eprintln!("skipped: DBPINBALL_TEST_PG_URL is not set");
        return;
    };

    let identity = security::verify_destination(&target.connection, &target.credentials)
        .await
        .expect("the verifier should reach the database");

    assert_eq!(identity.engine, DbEngine::Postgres);
    assert!(
        identity.identity_key.is_some(),
        "PostgreSQL must report a usable identity under a normal role"
    );
    assert!(identity.server.is_identified());
    assert_eq!(
        identity.server.database.as_deref(),
        Some(target.connection.database.as_str())
    );
    assert!(!identity
        .server
        .current_user
        .as_deref()
        .unwrap_or("")
        .is_empty());
}

#[tokio::test]
async fn postgres_read_only_transaction_refuses_ddl() {
    let Some(_target) = target("DBPINBALL_TEST_PG_URL", DbEngine::Postgres) else {
        eprintln!("skipped: DBPINBALL_TEST_PG_URL is not set");
        return;
    };

    let url = std::env::var("DBPINBALL_TEST_PG_URL").unwrap();
    let pool = sqlx::PgPool::connect(&url).await.expect("connect");

    let mut tx = pool.begin().await.expect("begin");
    sqlx::query("SET TRANSACTION READ ONLY")
        .execute(&mut *tx)
        .await
        .expect("set read only");

    let attempt = sqlx::query("CREATE TABLE dbpinball_rw_probe (x int)")
        .execute(&mut *tx)
        .await;

    assert!(
        attempt.is_err(),
        "PostgreSQL must refuse DDL inside a read-only transaction"
    );

    let _ = tx.rollback().await;
}
#[tokio::test]
async fn postgres_policy_allows_an_enrolled_development_destination() {
    let Some(target) = target("DBPINBALL_TEST_PG_URL", DbEngine::Postgres) else {
        eprintln!("skipped: DBPINBALL_TEST_PG_URL is not set");
        return;
    };

    let identity = security::verify_destination(&target.connection, &target.credentials)
        .await
        .expect("verify");

    let enrolled = enrollment_for(&identity, DestinationCategory::VerifiedDevelopment);

    let decision = security::evaluate(
        &security::OperationContext::new(DatabaseOperation::DatabaseRestore, now_ms())
            .with_destination(&identity)
            .with_enrollment(&enrolled),
    );

    assert!(
        decision.is_allowed(),
        "an enrolled dev destination is allowed"
    );
}

#[tokio::test]
async fn postgres_policy_requires_verification_when_not_enrolled() {
    let Some(target) = target("DBPINBALL_TEST_PG_URL", DbEngine::Postgres) else {
        eprintln!("skipped: DBPINBALL_TEST_PG_URL is not set");
        return;
    };

    let identity = security::verify_destination(&target.connection, &target.credentials)
        .await
        .expect("verify");

    let decision = security::evaluate(
        &security::OperationContext::new(DatabaseOperation::DatabaseRestore, now_ms())
            .with_destination(&identity),
    );

    assert_eq!(
        decision.code(),
        security::PolicyCode::NotEnrolled,
        "a real unenrolled destination must not be writable"
    );
}

#[tokio::test]
async fn postgres_policy_denies_a_changed_database_identity() {
    let Some(target) = target("DBPINBALL_TEST_PG_URL", DbEngine::Postgres) else {
        eprintln!("skipped: DBPINBALL_TEST_PG_URL is not set");
        return;
    };

    let identity = security::verify_destination(&target.connection, &target.credentials)
        .await
        .expect("verify");

    // Enroll the real identity, then change only the server component. That is
    // what a different cluster (or a replaced server) looks like.
    let mut enrolled = enrollment_for(&identity, DestinationCategory::VerifiedDevelopment);
    let real_key = identity.identity_key.clone().expect("identity key");
    let server_suffix = real_key.rsplit('#').next().unwrap_or("").to_string();
    enrolled.identity_key = real_key.replace(&format!("#{server_suffix}"), "#9999999999999999999");
    assert_ne!(enrolled.identity_key, real_key);

    let decision = security::evaluate(
        &security::OperationContext::new(DatabaseOperation::DatabaseRestore, now_ms())
            .with_destination(&identity)
            .with_enrollment(&enrolled),
    );

    assert_eq!(decision.code(), security::PolicyCode::DestinationChanged);
}

#[tokio::test]
async fn postgres_enrollment_round_trips_through_a_real_identity() {
    let Some(target) = target("DBPINBALL_TEST_PG_URL", DbEngine::Postgres) else {
        eprintln!("skipped: DBPINBALL_TEST_PG_URL is not set");
        return;
    };

    let directory = tempfile::tempdir().unwrap();
    let store = security::store_for(directory.path());

    let fingerprint = security::enroll(
        &target.connection,
        &target.credentials,
        &store,
        DestinationCategory::VerifiedDevelopment,
        now_ms(),
    )
    .await
    .expect("enrollment should succeed against a real database");

    let registry = store.load().unwrap();
    let stored = registry.get(&target.connection.id).expect("stored");
    assert_eq!(stored.identity_key, fingerprint.identity_key);

    // A fresh verification matches the enrollment.
    let (identity, matched) = security::verify(
        &target.connection,
        &target.credentials,
        &store,
        FreshnessPolicy::default(),
        now_ms(),
    )
    .await
    .expect("verify");

    assert_eq!(matched, EnrollmentMatch::Matches);
    assert_eq!(identity.identity_key, Some(fingerprint.identity_key));
}

#[tokio::test]
async fn postgres_stale_evidence_requires_reverification() {
    let Some(target) = target("DBPINBALL_TEST_PG_URL", DbEngine::Postgres) else {
        eprintln!("skipped: DBPINBALL_TEST_PG_URL is not set");
        return;
    };

    let identity = security::verify_destination(&target.connection, &target.credentials)
        .await
        .expect("verify");
    let enrolled = enrollment_for(&identity, DestinationCategory::VerifiedDevelopment);

    let mut context = security::OperationContext::new(DatabaseOperation::DatabaseRestore, now_ms())
        .with_destination(&identity)
        .with_enrollment(&enrolled);
    context.freshness = FreshnessPolicy { max_age_ms: 1 };

    // One second later the evidence is no longer fresh.
    context.now_ms = now_ms() + 5_000;

    assert_eq!(
        security::evaluate(&context).code(),
        security::PolicyCode::StaleVerification
    );
}

// ---------------------------------------------------------------------------
// MySQL
// ---------------------------------------------------------------------------

#[tokio::test]
async fn mysql_verifier_reports_a_real_identity() {
    let Some(target) = target("DBPINBALL_TEST_MYSQL_URL", DbEngine::Mysql) else {
        eprintln!("skipped: DBPINBALL_TEST_MYSQL_URL is not set");
        return;
    };

    let identity = security::verify_destination(&target.connection, &target.credentials)
        .await
        .expect("the verifier should reach the database");

    assert_eq!(identity.engine, DbEngine::Mysql);
    assert!(
        identity.identity_key.is_some(),
        "MySQL must report a server_uuid under a normal role"
    );
    assert!(identity.server.is_identified());
    assert!(!identity
        .server
        .current_user
        .as_deref()
        .unwrap_or("")
        .is_empty());
}

#[tokio::test]
async fn mysql_read_only_transaction_refuses_dml() {
    let Some(target) = target("DBPINBALL_TEST_MYSQL_URL", DbEngine::Mysql) else {
        eprintln!("skipped: DBPINBALL_TEST_MYSQL_URL is not set");
        return;
    };

    let url = std::env::var("DBPINBALL_TEST_MYSQL_URL").unwrap();
    let pool = sqlx::MySqlPool::connect(&url).await.expect("connect");

    // A transactional table on purpose: MySQL only enforces READ ONLY for
    // engines that participate in transactions.
    let table = format!("dbpinball_ro_probe_{}", std::process::id());
    let _ = sqlx::query(&format!("CREATE TABLE {table} (x INT) ENGINE=InnoDB"))
        .execute(&pool)
        .await;

    // A pool gives each statement its own pooled connection, so the mode must
    // be applied on the same connection that runs the statement.
    let mut conn = pool.acquire().await.expect("acquire");
    sqlx::query("SET TRANSACTION READ ONLY")
        .execute(&mut *conn)
        .await
        .expect("set read only");

    let attempt = sqlx::query(&format!("INSERT INTO {table} VALUES (1)"))
        .execute(&mut *conn)
        .await;
    assert!(
        attempt.is_err(),
        "MySQL must refuse DML inside a read-only transaction"
    );
    drop(conn);

    let _ = sqlx::query(&format!("DROP TABLE IF EXISTS {table}"))
        .execute(&pool)
        .await;
}

#[tokio::test]
async fn mysql_policy_requires_verification_when_not_enrolled() {
    let Some(target) = target("DBPINBALL_TEST_MYSQL_URL", DbEngine::Mysql) else {
        eprintln!("skipped: DBPINBALL_TEST_MYSQL_URL is not set");
        return;
    };

    let identity = security::verify_destination(&target.connection, &target.credentials)
        .await
        .expect("verify");

    let decision = security::evaluate(
        &security::OperationContext::new(DatabaseOperation::DatabaseRestore, now_ms())
            .with_destination(&identity),
    );

    assert_eq!(decision.code(), security::PolicyCode::NotEnrolled);
}

#[tokio::test]
async fn mysql_enrollment_round_trips_through_a_real_identity() {
    let Some(target) = target("DBPINBALL_TEST_MYSQL_URL", DbEngine::Mysql) else {
        eprintln!("skipped: DBPINBALL_TEST_MYSQL_URL is not set");
        return;
    };

    let directory = tempfile::tempdir().unwrap();
    let store = security::store_for(directory.path());

    let fingerprint = security::enroll(
        &target.connection,
        &target.credentials,
        &store,
        DestinationCategory::VerifiedDisposable,
        now_ms(),
    )
    .await
    .expect("enrollment should succeed against a real database");

    let (_, matched) = security::verify(
        &target.connection,
        &target.credentials,
        &store,
        FreshnessPolicy::default(),
        now_ms(),
    )
    .await
    .expect("verify");

    assert_eq!(matched, EnrollmentMatch::Matches);
    assert!(fingerprint.identity_key.contains("mysql://"));
}

#[tokio::test]
async fn mysql_identity_key_changes_when_the_database_changes() {
    let Some(mut target) = target("DBPINBALL_TEST_MYSQL_URL", DbEngine::Mysql) else {
        eprintln!("skipped: DBPINBALL_TEST_MYSQL_URL is not set");
        return;
    };

    let first = security::verify_destination(&target.connection, &target.credentials)
        .await
        .expect("verify first database");

    // Point at another database on the same server; the identity must differ.
    target.connection.database = "information_schema".to_string();
    let second = security::verify_destination(&target.connection, &target.credentials)
        .await
        .expect("verify second database");

    assert_ne!(
        first.identity_key, second.identity_key,
        "a different database on the same server must have a different identity"
    );
}

// ---------------------------------------------------------------------------
// Enrollment registry integrity (no database required)
// ---------------------------------------------------------------------------

#[test]
fn an_enrollment_registry_survives_a_corrupt_file_without_lying() {
    let directory = tempfile::tempdir().unwrap();
    let store = security::store_for(directory.path());

    std::fs::create_dir_all(directory.path().join("security")).unwrap();
    std::fs::write(store.path(), "{ not json").unwrap();

    // A corrupt registry must be an error, never a silent empty registry that
    // would make an enrolled destination look unenrolled.
    assert!(store.load().is_err());
}

#[test]
fn an_enrollment_registry_does_not_store_a_secret() {
    let directory = tempfile::tempdir().unwrap();
    let store = security::store_for(directory.path());

    let mut registry = EnrollmentRegistry::default();
    registry.enroll(
        "conn",
        EnrolledFingerprint {
            identity_key: "postgres://127.0.0.1:5432/dev#1".into(),
            engine: "postgres".into(),
            database: "dev".into(),
            category: DestinationCategory::VerifiedDevelopment,
            enrolled_at_ms: 1,
            note: None,
        },
    );
    store.save(&registry).unwrap();

    let contents = std::fs::read_to_string(store.path()).unwrap();
    assert!(!contents.to_ascii_lowercase().contains("password"));
    assert!(!contents.contains("m3devpass"));
}

#[test]
fn a_server_without_identity_cannot_be_compared() {
    let server = ServerIdentity {
        database: Some("dev".into()),
        ..ServerIdentity::default()
    };
    assert!(!server.is_identified());
    assert!(server
        .identity_key(DbEngine::Postgres, "127.0.0.1", 5432)
        .is_none());
}

// ---------------------------------------------------------------------------
// Bypass attempts: the policy must be enforced at the execution boundary
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_declared_local_label_cannot_authorize_a_real_write() {
    let Some(mut target) = target("DBPINBALL_TEST_PG_URL", DbEngine::Postgres) else {
        eprintln!("skipped: DBPINBALL_TEST_PG_URL is not set");
        return;
    };

    // The exact bypass the milestone calls out: a profile that claims `local`.
    target.connection.target = TargetClass::Local;

    let identity = security::verify_destination(&target.connection, &target.credentials)
        .await
        .expect("verify");

    let decision = security::evaluate(
        &security::OperationContext::new(DatabaseOperation::DatabaseRestore, now_ms())
            .with_destination(&identity)
            .with_declared_target(target.connection.target),
    );

    assert!(
        !decision.is_allowed(),
        "a `local` label must never authorize a write on its own"
    );
    assert_eq!(decision.code(), security::PolicyCode::NotEnrolled);
}

#[tokio::test]
async fn a_localhost_tunnel_cannot_reuse_another_servers_fingerprint() {
    let Some(target) = target("DBPINBALL_TEST_PG_URL", DbEngine::Postgres) else {
        eprintln!("skipped: DBPINBALL_TEST_PG_URL is not set");
        return;
    };

    let identity = security::verify_destination(&target.connection, &target.credentials)
        .await
        .expect("verify");

    // An enrollment captured from a different cluster, reached through the same
    // 127.0.0.1 endpoint. The endpoint matches; the server identity does not.
    let mut enrolled = enrollment_for(&identity, DestinationCategory::VerifiedDevelopment);
    enrolled.identity_key = "postgres://127.0.0.1:55432/m3dev#1111111111111111111".to_string();

    let decision = security::evaluate(
        &security::OperationContext::new(DatabaseOperation::DatabaseRestore, now_ms())
            .with_destination(&identity)
            .with_enrollment(&enrolled),
    );

    assert_eq!(
        decision.code(),
        security::PolicyCode::DestinationChanged,
        "a tunnel to another server must not inherit a fingerprint"
    );
}

#[tokio::test]
async fn a_reversed_copy_is_denied_against_real_identities() {
    let Some(target) = target("DBPINBALL_TEST_PG_URL", DbEngine::Postgres) else {
        eprintln!("skipped: DBPINBALL_TEST_PG_URL is not set");
        return;
    };

    let identity = security::verify_destination(&target.connection, &target.credentials)
        .await
        .expect("verify");
    let enrolled = enrollment_for(&identity, DestinationCategory::VerifiedDevelopment);

    // Source and destination are the same real database.
    let decision = security::evaluate(
        &security::OperationContext::new(DatabaseOperation::DatabaseCopy, now_ms())
            .with_source(&identity)
            .with_destination(&identity)
            .with_enrollment(&enrolled),
    );

    assert_eq!(
        decision.code(),
        security::PolicyCode::SameSourceAndDestination
    );
}

#[tokio::test]
async fn a_production_like_enrollment_is_never_writable() {
    let Some(target) = target("DBPINBALL_TEST_MYSQL_URL", DbEngine::Mysql) else {
        eprintln!("skipped: DBPINBALL_TEST_MYSQL_URL is not set");
        return;
    };

    let identity = security::verify_destination(&target.connection, &target.credentials)
        .await
        .expect("verify");
    let enrolled = enrollment_for(&identity, DestinationCategory::ProductionLike);

    let decision = security::evaluate(
        &security::OperationContext::new(DatabaseOperation::DatabaseRestore, now_ms())
            .with_destination(&identity)
            .with_enrollment(&enrolled),
    );

    assert_eq!(decision.code(), security::PolicyCode::DestinationNotAllowed);
}
