#![cfg(feature = "integration")]

//! Real two-instance MariaDB 10.11 Safe Sync acceptance & adversarial suite (MDB-8).
//!
//! Requires:
//! - `DBPINBALL_TEST_MARIADB_URL` (root/admin on source instance, e.g. 127.0.0.1:33306/mdb_src)
//! - `DBPINBALL_TEST_MARIADB_RO_URL` (SELECT-only user on source instance)
//! - `DBPINBALL_TEST_MARIADB_DST_URL` (admin/writable user on destination instance, e.g. 127.0.0.1:33307/mdb_dst)

use std::collections::BTreeMap;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use sqlx::mysql::MySqlPoolOptions;
use sqlx::MySqlPool;
use tempfile::tempdir;

use dbpinball_core::domain::connection::{DbConnection, DbEngine, TargetClass};
use dbpinball_core::domain::error::{CoreError, ErrorCode};
use dbpinball_core::ports::credentials::{CredentialProvider, Secret};
use dbpinball_core::safesync::key::{InMemoryMaskKeyStore, MaskKey};
use dbpinball_core::safesync::service::{self, ExecuteRequest};
use dbpinball_core::safesync::spec::{
    ExecutionOptions, FilterOp, FilterPredicate, FilterValue, Limits, MaskingSelection,
    RelationshipSpec, RootSelection, SafeSyncSpec, Selection, TraversalDirection, SPEC_VERSION,
};
use dbpinball_core::security::{self, DestinationCategory, EnrollmentStore};

#[derive(Default)]
struct InMemoryCredentialProvider(std::sync::Mutex<BTreeMap<String, String>>);

impl InMemoryCredentialProvider {
    fn insert(&self, id: &str, password: &str) {
        self.0
            .lock()
            .expect("credential lock")
            .insert(id.to_string(), password.to_string());
    }
}

impl CredentialProvider for InMemoryCredentialProvider {
    fn get(&self, connection_id: &str) -> Result<Secret, CoreError> {
        self.0
            .lock()
            .expect("credential lock")
            .get(connection_id)
            .map(|password| Secret::new(password.clone()))
            .ok_or_else(|| CoreError::new(ErrorCode::NotFound, "no credential"))
    }

    fn set(&self, _connection_id: &str, _secret: &Secret) -> Result<(), CoreError> {
        Ok(())
    }

    fn delete(&self, _connection_id: &str) -> Result<(), CoreError> {
        Ok(())
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(1_700_000_000_000)
}

fn test_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

struct ParsedConn {
    pool: MySqlPool,
    profile: DbConnection,
    password: String,
}

async fn connect_env(var: &str, id: &str, target: TargetClass) -> Option<ParsedConn> {
    let Ok(url) = std::env::var(var) else {
        assert!(
            std::env::var_os("CI").is_none(),
            "{var} must be set when CI is set: skipped MariaDB Safe Sync tests are not passes"
        );
        return None;
    };
    let parsed = url::Url::parse(&url).expect("valid MariaDB URL");
    let host = parsed.host_str().expect("host").to_string();
    let port = parsed.port().unwrap_or(3306);
    let database = parsed.path().trim_start_matches('/').to_string();
    let username = parsed.username().to_string();
    let password = parsed.password().unwrap_or_default().to_string();

    let pool = MySqlPoolOptions::new()
        .max_connections(4)
        .connect(&url)
        .await
        .expect("connect MariaDB test instance");

    Some(ParsedConn {
        pool,
        profile: DbConnection {
            id: id.to_string(),
            name: id.to_string(),
            engine: DbEngine::Mysql,
            host,
            port,
            database,
            username,
            use_ssl: false,
            target,
        },
        password,
    })
}

async fn reset_pair_schema(src_admin: &MySqlPool, dst: &MySqlPool) {
    let ddl = "
        SET FOREIGN_KEY_CHECKS = 0;
        DROP TABLE IF EXISTS mdb_orders;
        DROP TABLE IF EXISTS mdb_customers;
        DROP TABLE IF EXISTS dbpinball_safe_sync_runs;
        SET FOREIGN_KEY_CHECKS = 1;

        CREATE TABLE mdb_customers (
            tenant_id INT NOT NULL,
            customer_id BIGINT NOT NULL AUTO_INCREMENT,
            email VARCHAR(255) NOT NULL,
            note VARCHAR(255) NULL,
            PRIMARY KEY (customer_id),
            UNIQUE KEY uq_tenant_cust (tenant_id, customer_id)
        ) ENGINE=InnoDB;

        CREATE TABLE mdb_orders (
            id BIGINT NOT NULL AUTO_INCREMENT,
            tenant_id INT NOT NULL,
            customer_id BIGINT NOT NULL,
            amount DECIMAL(12,2) NOT NULL,
            ordered_on DATE NOT NULL,
            created_at DATETIME(6) NOT NULL,
            PRIMARY KEY (id),
            CONSTRAINT fk_orders_customer
                FOREIGN KEY (tenant_id, customer_id)
                REFERENCES mdb_customers (tenant_id, customer_id)
        ) ENGINE=InnoDB;
    ";
    sqlx::raw_sql(ddl)
        .execute(src_admin)
        .await
        .expect("reset src schema");
    sqlx::raw_sql(ddl)
        .execute(dst)
        .await
        .expect("reset dst schema");

    sqlx::raw_sql(
        "INSERT INTO mdb_customers (tenant_id, customer_id, email, note) VALUES
            (1, 10, 'alice@secret.test', NULL),
            (1, 20, 'bob@secret.test', 'vip');
         INSERT INTO mdb_orders (id, tenant_id, customer_id, amount, ordered_on, created_at) VALUES
            (100, 1, 10, 125.50, '2026-01-10', '2026-01-10 08:30:00.123456'),
            (101, 1, 20, 999.95, '2026-02-15', '2026-02-15 14:00:00.000000');",
    )
    .execute(src_admin)
    .await
    .expect("seed src rows");
}

fn write_mariadb_policy(dir: &std::path::Path, db: &str) -> String {
    let policy_path = dir.join("mariadb-masking-policy.yaml");
    let yaml = format!(
        r#"name: mariadb-test
version: 1
rules:
  - schema: {db}
    table: mdb_customers
    column: tenant_id
    strategy: keep
  - schema: {db}
    table: mdb_customers
    column: customer_id
    strategy: keep
  - schema: {db}
    table: mdb_customers
    column: email
    strategy: deterministic_email
  - schema: {db}
    table: mdb_customers
    column: note
    strategy: redact
  - schema: {db}
    table: mdb_orders
    column: id
    strategy: keep
  - schema: {db}
    table: mdb_orders
    column: tenant_id
    strategy: keep
  - schema: {db}
    table: mdb_orders
    column: customer_id
    strategy: keep
  - schema: {db}
    table: mdb_orders
    column: amount
    strategy: keep
  - schema: {db}
    table: mdb_orders
    column: ordered_on
    strategy: keep
  - schema: {db}
    table: mdb_orders
    column: created_at
    strategy: keep
"#
    );
    std::fs::write(&policy_path, yaml).expect("write masking policy");
    policy_path.to_string_lossy().into_owned()
}

fn base_spec(dir: &std::path::Path, src_id: &str, dst_id: &str, db: &str) -> SafeSyncSpec {
    let policy_path = write_mariadb_policy(dir, db);
    SafeSyncSpec {
        version: SPEC_VERSION,
        source: src_id.to_string(),
        destination: dst_id.to_string(),
        selection: Selection {
            roots: vec![RootSelection {
                table: format!("{db}.mdb_orders"),
                filter: vec![FilterPredicate {
                    column: "ordered_on".to_string(),
                    op: FilterOp::Gte,
                    value: FilterValue::Text("2026-02-01".to_string()),
                }],
            }],
            relationships: RelationshipSpec {
                direction: TraversalDirection::Parents,
                depth: 1,
            },
        },
        limits: Limits::default(),
        masking: MaskingSelection {
            policy: policy_path,
        },
        execution: ExecutionOptions::default(),
    }
}

async fn enroll_dst(
    dir: &std::path::Path,
    dst: &ParsedConn,
    creds: &InMemoryCredentialProvider,
) -> EnrollmentStore {
    let store = EnrollmentStore::new(dir.join("enrollments.json"));
    security::enroll(
        &dst.profile,
        creds,
        &store,
        DestinationCategory::VerifiedDevelopment,
        now_ms(),
    )
    .await
    .expect("enroll dst");
    store
}

fn key_store() -> InMemoryMaskKeyStore {
    InMemoryMaskKeyStore::with_key(
        MaskKey::from_hex("0707070707070707070707070707070707070707070707070707070707070707")
            .unwrap(),
    )
}

#[tokio::test]
async fn t01_full_inspect_plan_validate_execute_retry_and_auto_increment() {
    let _guard = test_lock().lock().await;
    let Some(src_admin) = connect_env(
        "DBPINBALL_TEST_MARIADB_URL",
        "src-admin",
        TargetClass::Production,
    )
    .await
    else {
        return;
    };
    let Some(src_ro) = connect_env(
        "DBPINBALL_TEST_MARIADB_RO_URL",
        "src-ro",
        TargetClass::Production,
    )
    .await
    else {
        return;
    };
    let Some(dst) = connect_env(
        "DBPINBALL_TEST_MARIADB_DST_URL",
        "dst-local",
        TargetClass::Local,
    )
    .await
    else {
        return;
    };

    reset_pair_schema(&src_admin.pool, &dst.pool).await;

    let creds = InMemoryCredentialProvider::default();
    creds.insert(&src_ro.profile.id, &src_ro.password);
    creds.insert(&dst.profile.id, &dst.password);

    let dir = tempdir().unwrap();
    let enrollments = enroll_dst(dir.path(), &dst, &creds).await;
    let keys = key_store();

    // 1. Inspect source & destination
    let src_id = service::inspect(&src_ro.profile, &creds, &Limits::default())
        .await
        .expect("inspect source");
    assert_eq!(src_id.server.engine.as_deref(), Some("mariadb"));
    assert!(src_id.transaction_read_only);

    // 2. Plan
    let spec = base_spec(
        dir.path(),
        &src_ro.profile.id,
        &dst.profile.id,
        &src_ro.profile.database,
    );
    let now = now_ms();
    let plan = service::plan(
        &spec,
        &src_ro.profile,
        &dst.profile,
        &creds,
        &enrollments,
        &keys,
        now,
        true,
        false,
    )
    .await
    .expect("plan");

    assert_eq!(plan.plan_version, 3);
    assert_eq!(plan.source.engine, "mariadb");
    assert_eq!(plan.destination.engine, "mariadb");
    assert!(plan.complete);

    // Value-free plan guarantee
    let plan_json = serde_json::to_string(&plan).unwrap();
    assert!(!plan_json.contains("bob@secret.test"));
    assert!(!plan_json.contains("2026-02-01"));

    // 3. Validate
    let val = service::validate(
        &plan,
        &spec,
        &src_ro.profile,
        &dst.profile,
        &creds,
        &enrollments,
        &keys,
        now,
    )
    .await
    .expect("validate");
    assert!(val.valid);

    // 4. Execute
    let cancel = Arc::new(AtomicBool::new(false));
    let report = service::execute(
        ExecuteRequest {
            plan: &plan,
            spec: &spec,
            acknowledgement: &plan.plan_fingerprint,
            replace: false,
            source: &src_ro.profile,
            destination: &dst.profile,
        },
        &creds,
        &enrollments,
        &keys,
        Some(dir.path()),
        now,
        cancel.clone(),
    )
    .await
    .expect("execute");

    assert!(!report.already_completed);
    assert_eq!(report.rows_written, 2); // 1 customer (id=20) + 1 order (id=101)
    assert_eq!(report.sequence_status, "completed");

    // Verify destination data: masked email, preserved decimal, NULL/masked note, AUTO_INCREMENT
    let email: String =
        sqlx::query_scalar("SELECT email FROM mdb_customers WHERE customer_id = 20")
            .fetch_one(&dst.pool)
            .await
            .unwrap();
    assert_ne!(email, "bob@secret.test");
    assert!(email.ends_with("@example.invalid"));

    let amount: String = sqlx::query_scalar(
        "SELECT CAST(amount AS CHAR CHARACTER SET utf8mb4) FROM mdb_orders WHERE id = 101",
    )
    .fetch_one(&dst.pool)
    .await
    .unwrap();
    assert_eq!(amount, "999.95");

    // Verify AUTO_INCREMENT was reconciled past 101 so a new insert does not collide
    sqlx::query(
        "INSERT INTO mdb_orders (tenant_id, customer_id, amount, ordered_on, created_at) \
         VALUES (1, 20, 10.00, '2026-03-01', '2026-03-01 00:00:00')",
    )
    .execute(&dst.pool)
    .await
    .expect("post-sync auto_increment insert");
    let new_id: i64 = sqlx::query_scalar("SELECT MAX(id) FROM mdb_orders")
        .fetch_one(&dst.pool)
        .await
        .unwrap();
    assert!(new_id > 101);

    // Delete the extra test row so destination matches digest for idempotent retry
    sqlx::query("DELETE FROM mdb_orders WHERE id = ?")
        .bind(new_id)
        .execute(&dst.pool)
        .await
        .unwrap();

    // 5. Idempotent retry
    let retry = service::execute(
        ExecuteRequest {
            plan: &plan,
            spec: &spec,
            acknowledgement: &plan.plan_fingerprint,
            replace: false,
            source: &src_ro.profile,
            destination: &dst.profile,
        },
        &creds,
        &enrollments,
        &keys,
        None,
        now,
        cancel,
    )
    .await
    .expect("idempotent retry");
    assert!(retry.already_completed);
    assert_eq!(retry.rows_written, 0);

    // 6. Status
    let status = service::status(&report.run_id, &dst.profile, &creds, &Limits::default())
        .await
        .expect("status");
    assert_eq!(status.status, "completed");
    assert_eq!(status.sequence_status, "completed");
}

#[tokio::test]
async fn t02_adversarial_source_and_destination_refusals() {
    let _guard = test_lock().lock().await;
    let Some(src_admin) = connect_env(
        "DBPINBALL_TEST_MARIADB_URL",
        "src-admin",
        TargetClass::Production,
    )
    .await
    else {
        return;
    };
    let Some(src_ro) = connect_env(
        "DBPINBALL_TEST_MARIADB_RO_URL",
        "src-ro",
        TargetClass::Production,
    )
    .await
    else {
        return;
    };
    let Some(dst) = connect_env(
        "DBPINBALL_TEST_MARIADB_DST_URL",
        "dst-local",
        TargetClass::Local,
    )
    .await
    else {
        return;
    };

    reset_pair_schema(&src_admin.pool, &dst.pool).await;

    let creds = InMemoryCredentialProvider::default();
    creds.insert(&src_admin.profile.id, &src_admin.password);
    creds.insert(&src_ro.profile.id, &src_ro.password);
    creds.insert(&dst.profile.id, &dst.password);

    let dir = tempdir().unwrap();
    let empty_enrollments = EnrollmentStore::new(dir.path().join("empty.json"));
    let keys = key_store();

    // Privileged/writable source user is refused
    let priv_spec = base_spec(
        dir.path(),
        &src_admin.profile.id,
        &dst.profile.id,
        &src_admin.profile.database,
    );
    let err = service::plan(
        &priv_spec,
        &src_admin.profile,
        &dst.profile,
        &creds,
        &empty_enrollments,
        &keys,
        now_ms(),
        true,
        false,
    )
    .await
    .unwrap_err();
    assert_eq!(err.detail.as_deref(), Some("source_privileged"));

    // Unenrolled destination is refused
    let spec = base_spec(
        dir.path(),
        &src_ro.profile.id,
        &dst.profile.id,
        &src_ro.profile.database,
    );
    let err = service::plan(
        &spec,
        &src_ro.profile,
        &dst.profile,
        &creds,
        &empty_enrollments,
        &keys,
        now_ms(),
        true,
        false,
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::PolicyRefusal);

    // Production-declared destination is refused even if enrolled
    let enrollments = enroll_dst(dir.path(), &dst, &creds).await;
    let mut prod_dst = dst.profile.clone();
    prod_dst.target = TargetClass::Production;
    let err = service::plan(
        &spec,
        &src_ro.profile,
        &prod_dst,
        &creds,
        &enrollments,
        &keys,
        now_ms(),
        true,
        false,
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::PolicyRefusal);
}

#[tokio::test]
async fn t03_adversarial_plan_tamper_expiry_drift_triggers_and_bounds() {
    let _guard = test_lock().lock().await;
    let Some(src_admin) = connect_env(
        "DBPINBALL_TEST_MARIADB_URL",
        "src-admin",
        TargetClass::Production,
    )
    .await
    else {
        return;
    };
    let Some(src_ro) = connect_env(
        "DBPINBALL_TEST_MARIADB_RO_URL",
        "src-ro",
        TargetClass::Production,
    )
    .await
    else {
        return;
    };
    let Some(dst) = connect_env(
        "DBPINBALL_TEST_MARIADB_DST_URL",
        "dst-local",
        TargetClass::Local,
    )
    .await
    else {
        return;
    };

    reset_pair_schema(&src_admin.pool, &dst.pool).await;

    let creds = InMemoryCredentialProvider::default();
    creds.insert(&src_ro.profile.id, &src_ro.password);
    creds.insert(&dst.profile.id, &dst.password);

    let dir = tempdir().unwrap();
    let enrollments = enroll_dst(dir.path(), &dst, &creds).await;
    let keys = key_store();
    let now = now_ms();

    let spec = base_spec(
        dir.path(),
        &src_ro.profile.id,
        &dst.profile.id,
        &src_ro.profile.database,
    );
    let plan = service::plan(
        &spec,
        &src_ro.profile,
        &dst.profile,
        &creds,
        &enrollments,
        &keys,
        now,
        true,
        false,
    )
    .await
    .unwrap();

    // Tampered plan is refused
    let mut tampered = plan.clone();
    tampered.expires_at_ms += 1_000;
    let err = service::validate(
        &tampered,
        &spec,
        &src_ro.profile,
        &dst.profile,
        &creds,
        &enrollments,
        &keys,
        now,
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::Conflict);

    // Expired plan is refused
    let err = service::validate(
        &plan,
        &spec,
        &src_ro.profile,
        &dst.profile,
        &creds,
        &enrollments,
        &keys,
        plan.expires_at_ms + 1,
    )
    .await
    .unwrap_err();
    assert_eq!(err.detail.as_deref(), Some("plan_expired"));

    // Oversized extraction bound refused
    let mut tight_spec = spec.clone();
    tight_spec.selection.roots[0].filter = vec![FilterPredicate {
        column: "tenant_id".into(),
        op: FilterOp::Eq,
        value: FilterValue::Int(1),
    }];
    tight_spec.limits.max_rows_per_table = 1;
    let err = service::plan(
        &tight_spec,
        &src_ro.profile,
        &dst.profile,
        &creds,
        &enrollments,
        &keys,
        now,
        true,
        false,
    )
    .await
    .unwrap_err();
    assert_eq!(err.detail.as_deref(), Some("limit_exceeded"));

    // Destination trigger added before execute -> refused
    sqlx::raw_sql(
        "CREATE TRIGGER tr_block_orders BEFORE INSERT ON mdb_orders FOR EACH ROW SET @x = 1;",
    )
    .execute(&dst.pool)
    .await
    .unwrap();
    let err = service::execute(
        ExecuteRequest {
            plan: &plan,
            spec: &spec,
            acknowledgement: &plan.plan_fingerprint,
            replace: false,
            source: &src_ro.profile,
            destination: &dst.profile,
        },
        &creds,
        &enrollments,
        &keys,
        None,
        now,
        Arc::new(AtomicBool::new(false)),
    )
    .await
    .unwrap_err();
    assert_eq!(err.detail.as_deref(), Some("triggers_not_supported"));
    sqlx::raw_sql("DROP TRIGGER IF EXISTS tr_block_orders;")
        .execute(&dst.pool)
        .await
        .unwrap();

    // Destination schema drift -> refused
    sqlx::raw_sql("ALTER TABLE mdb_orders ADD COLUMN extra_col VARCHAR(16) NULL;")
        .execute(&dst.pool)
        .await
        .unwrap();
    let err = service::execute(
        ExecuteRequest {
            plan: &plan,
            spec: &spec,
            acknowledgement: &plan.plan_fingerprint,
            replace: false,
            source: &src_ro.profile,
            destination: &dst.profile,
        },
        &creds,
        &enrollments,
        &keys,
        None,
        now,
        Arc::new(AtomicBool::new(false)),
    )
    .await
    .unwrap_err();
    assert_eq!(err.detail.as_deref(), Some("schema_drift"));
}

#[tokio::test]
async fn t04_adversarial_replace_rollback_and_get_lock_concurrency() {
    let _guard = test_lock().lock().await;
    let Some(src_admin) = connect_env(
        "DBPINBALL_TEST_MARIADB_URL",
        "src-admin",
        TargetClass::Production,
    )
    .await
    else {
        return;
    };
    let Some(src_ro) = connect_env(
        "DBPINBALL_TEST_MARIADB_RO_URL",
        "src-ro",
        TargetClass::Production,
    )
    .await
    else {
        return;
    };
    let Some(dst) = connect_env(
        "DBPINBALL_TEST_MARIADB_DST_URL",
        "dst-local",
        TargetClass::Local,
    )
    .await
    else {
        return;
    };

    reset_pair_schema(&src_admin.pool, &dst.pool).await;

    let creds = InMemoryCredentialProvider::default();
    creds.insert(&src_ro.profile.id, &src_ro.password);
    creds.insert(&dst.profile.id, &dst.password);

    let dir = tempdir().unwrap();
    let enrollments = enroll_dst(dir.path(), &dst, &creds).await;
    let keys = key_store();
    let now = now_ms();

    let spec = base_spec(
        dir.path(),
        &src_ro.profile.id,
        &dst.profile.id,
        &src_ro.profile.database,
    );
    let plan_replace = service::plan(
        &spec,
        &src_ro.profile,
        &dst.profile,
        &creds,
        &enrollments,
        &keys,
        now,
        true,
        true,
    )
    .await
    .unwrap();

    // Seed pre-existing row in destination
    sqlx::raw_sql(
        "INSERT INTO mdb_customers (tenant_id, customer_id, email, note) VALUES (1, 99, 'old@dst.test', NULL);",
    )
    .execute(&dst.pool)
    .await
    .unwrap();

    // Cancelled replace rolls back cleanly, preserving existing destination state
    let err = service::execute(
        ExecuteRequest {
            plan: &plan_replace,
            spec: &spec,
            acknowledgement: &plan_replace.plan_fingerprint,
            replace: true,
            source: &src_ro.profile,
            destination: &dst.profile,
        },
        &creds,
        &enrollments,
        &keys,
        None,
        now,
        Arc::new(AtomicBool::new(true)),
    )
    .await
    .unwrap_err();
    assert_eq!(err.detail.as_deref(), Some("cancelled"));

    let old_still_there: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM mdb_customers WHERE customer_id = 99")
            .fetch_one(&dst.pool)
            .await
            .unwrap();
    assert_eq!(old_still_there, 1);

    // Active replace succeeds atomically
    let rep = service::execute(
        ExecuteRequest {
            plan: &plan_replace,
            spec: &spec,
            acknowledgement: &plan_replace.plan_fingerprint,
            replace: true,
            source: &src_ro.profile,
            destination: &dst.profile,
        },
        &creds,
        &enrollments,
        &keys,
        None,
        now,
        Arc::new(AtomicBool::new(false)),
    )
    .await
    .expect("replace succeeds");
    assert_eq!(rep.rows_written, 2);

    let old_gone: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM mdb_customers WHERE customer_id = 99")
            .fetch_one(&dst.pool)
            .await
            .unwrap();
    assert_eq!(old_gone, 0);

    // Concurrent GET_LOCK contention on another connection is refused
    let lock_name = format!(
        "dbpinball_sync_{}",
        plan_replace
            .plan_fingerprint
            .chars()
            .take(32)
            .collect::<String>()
    );
    let mut holder = dst.pool.acquire().await.unwrap();
    let got: i32 = sqlx::query_scalar("SELECT GET_LOCK(?, 5)")
        .bind(&lock_name)
        .fetch_one(&mut *holder)
        .await
        .unwrap();
    assert_eq!(got, 1);

    let err = service::execute(
        ExecuteRequest {
            plan: &plan_replace,
            spec: &spec,
            acknowledgement: &plan_replace.plan_fingerprint,
            replace: true,
            source: &src_ro.profile,
            destination: &dst.profile,
        },
        &creds,
        &enrollments,
        &keys,
        None,
        now,
        Arc::new(AtomicBool::new(false)),
    )
    .await
    .unwrap_err();
    assert_eq!(err.detail.as_deref(), Some("concurrent_run_in_progress"));

    let _ = sqlx::query("SELECT RELEASE_LOCK(?)")
        .bind(&lock_name)
        .fetch_optional(&mut *holder)
        .await;
}
