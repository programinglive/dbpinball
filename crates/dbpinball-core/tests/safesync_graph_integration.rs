#![cfg(feature = "integration")]

//! Real PostgreSQL tests for M4 S2-a: foreign-key graph, parent traversal,
//! canonical key masking, cycle handling and import ordering.
//!
//! Two isolated clusters are required. A skipped run is not acceptance.

use std::collections::BTreeMap;

use dbpinball_core::domain::connection::{DbConnection, DbEngine, TargetClass};
use dbpinball_core::domain::error::{CoreError, ErrorCode};
use dbpinball_core::ports::credentials::{CredentialProvider, Secret};
use dbpinball_core::safesync::{
    service, ConflictPolicy, ExecuteRequest, InMemoryMaskKeyStore, MaskKey, SafeSyncSpec,
};
use dbpinball_core::security::{self, DestinationCategory};

struct MapProvider(BTreeMap<String, String>);

impl MapProvider {
    fn new(entries: &[(&str, &str)]) -> Self {
        Self(
            entries
                .iter()
                .map(|(id, password)| (id.to_string(), password.to_string()))
                .collect(),
        )
    }
}

impl CredentialProvider for MapProvider {
    fn get(&self, connection_id: &str) -> Result<Secret, CoreError> {
        self.0
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

struct Target {
    connection: DbConnection,
    password: String,
}

fn target(variable: &str, id: &str) -> Option<Target> {
    let url = std::env::var(variable).ok()?;
    let parsed = url::Url::parse(&url).ok()?;
    Some(Target {
        connection: DbConnection {
            id: id.to_string(),
            name: id.to_string(),
            engine: DbEngine::Postgres,
            host: parsed.host_str()?.to_string(),
            port: parsed.port().unwrap_or(5432),
            database: parsed.path().trim_start_matches('/').to_string(),
            username: parsed.username().to_string(),
            use_ssl: false,
            target: TargetClass::Unknown,
        },
        password: parsed.password().unwrap_or_default().to_string(),
    })
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn serial() -> &'static tokio::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

fn key_store() -> InMemoryMaskKeyStore {
    InMemoryMaskKeyStore::with_key(MaskKey::from_bytes("k-test", [7u8; 32]))
}

fn spec(root: &str, column: &str, value: &str, depth: u8) -> SafeSyncSpec {
    let yaml = format!(
        r#"
version: 1
source: source
destination: destination
selection:
  roots:
    - table: {root}
      filter:
        - column: {column}
          op: eq
          value: {value}
  relationships:
    depth: {depth}
    direction: parents
limits:
  max_rows_per_table: 100
  max_total_rows: 300
  max_depth: 3
  max_bytes: 1048576
  timeout_ms: 15000
masking:
  policy: development-default@2
"#
    );
    SafeSyncSpec::parse_yaml(&yaml).expect("the test spec is valid")
}

async fn enroll(destination: &Target, provider: &MapProvider, directory: &std::path::Path) {
    let store = security::store_for(directory);
    security::enroll(
        &destination.connection,
        provider,
        &store,
        DestinationCategory::VerifiedDevelopment,
        now_ms(),
    )
    .await
    .expect("enroll the destination");
}

async fn connect(target: &Target) -> sqlx::PgConnection {
    use sqlx::postgres::{PgConnectOptions, PgConnection};
    use sqlx::Connection;

    let options = PgConnectOptions::new()
        .host(&target.connection.host)
        .port(target.connection.port)
        .username(&target.connection.username)
        .password(&target.password)
        .database(&target.connection.database);
    PgConnection::connect_with(&options).await.expect("connect")
}

async fn reset(destination: &Target) {
    let mut connection = connect(destination).await;
    // The run table may not exist yet on the first test.
    let _ = sqlx::query("DELETE FROM public.dbpinball_safe_sync_runs")
        .execute(&mut connection)
        .await;
    // Deferrable cycles must be deleted in one deferred transaction.
    let _ = sqlx::query("BEGIN").execute(&mut connection).await;
    let _ = sqlx::query("SET CONSTRAINTS ALL DEFERRED")
        .execute(&mut connection)
        .await;
    for table in [
        "accounts",
        "shipment_events",
        "shipments",
        "orders",
        "customers",
        "employees",
        "cycle_a",
        "cycle_b",
        "cycle_x",
        "cycle_y",
        "rls_children",
        "rls_parents",
        "identity_children",
        "identity_parents",
        "identity_optional",
        "sensitive_metrics",
    ] {
        let _ = sqlx::query(&format!("DELETE FROM public.{table}"))
            .execute(&mut connection)
            .await;
    }
    let _ = sqlx::query("COMMIT").execute(&mut connection).await;
}

async fn plan_and_execute(
    spec: &SafeSyncSpec,
    source: &Target,
    destination: &Target,
    provider: &MapProvider,
    directory: &std::path::Path,
) -> Result<dbpinball_core::safesync::DryRunPlan, CoreError> {
    let store = security::store_for(directory);
    let plan = service::plan(
        spec,
        &source.connection,
        &destination.connection,
        provider,
        &store,
        &key_store(),
        now_ms(),
        true,
        false,
    )
    .await?;

    if !plan.complete {
        return Err(CoreError::with_detail(
            ErrorCode::Operation,
            "incomplete plan",
            "limit_exceeded",
        ));
    }

    let store = security::store_for(directory);
    service::execute(
        ExecuteRequest {
            plan: &plan,
            spec,
            acknowledgement: &plan.plan_fingerprint,
            replace: false,
            source: &source.connection,
            destination: &destination.connection,
        },
        provider,
        &store,
        &key_store(),
        Some(directory),
        now_ms(),
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
    )
    .await?;

    Ok(plan)
}

async fn count(destination: &Target, table: &str) -> i64 {
    let mut connection = connect(destination).await;
    let row: (i64,) = sqlx::query_as(&format!("SELECT count(*) FROM public.{table}"))
        .fetch_one(&mut connection)
        .await
        .expect("count");
    row.0
}

/// The run-marker count, or zero when the run table has not been created yet.
///
/// The existence check is a separate statement: a subquery over the missing
/// relation would still fail at parse time inside a CASE.
async fn run_marker_count(destination: &Target) -> i64 {
    let mut connection = connect(destination).await;
    let exists: Option<String> =
        sqlx::query_scalar("SELECT to_regclass('public.dbpinball_safe_sync_runs')::text")
            .fetch_one(&mut connection)
            .await
            .expect("existence check");
    if exists.is_none() {
        return 0;
    }
    let row: (i64,) = sqlx::query_as("SELECT count(*) FROM public.dbpinball_safe_sync_runs")
        .fetch_one(&mut connection)
        .await
        .expect("run marker count");
    row.0
}

async fn fk_violations(destination: &Target, sql: &str) -> i64 {
    let mut connection = connect(destination).await;
    let row: (i64,) = sqlx::query_as(sql)
        .fetch_one(&mut connection)
        .await
        .expect("fk check");
    row.0
}

// ---------------------------------------------------------------------------
// Single and composite parents
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_single_parent_is_pulled_and_the_masked_join_holds() {
    let _guard = serial().lock().await;
    let (Some(source), Some(destination)) = (
        target("DBPINBALL_TEST_PG_SOURCE_URL", "source"),
        target("DBPINBALL_TEST_PG_DEST_URL", "destination"),
    ) else {
        eprintln!("skipped: the test database URLs are not set");
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let provider = MapProvider::new(&[
        ("source", &source.password),
        ("destination", &destination.password),
    ]);
    reset(&destination).await;
    enroll(&destination, &provider, &directory.path().to_path_buf()).await;

    let spec = spec(
        "public.orders",
        "id",
        "0b9f4c3e-0000-0000-0000-000000000001",
        1,
    );
    plan_and_execute(&spec, &source, &destination, &provider, directory.path())
        .await
        .expect("the plan should execute");

    assert_eq!(count(&destination, "orders").await, 1);
    assert_eq!(count(&destination, "customers").await, 1);

    let violations = fk_violations(
        &destination,
        "SELECT count(*) FROM public.orders o
         LEFT JOIN public.customers c ON c.code = o.customer_code
         WHERE c.code IS NULL",
    )
    .await;
    assert_eq!(violations, 0, "the masked join must satisfy the FK");

    let mut connection = connect(&destination).await;
    let (code, _email, _name): (String, String, String) =
        sqlx::query_as("SELECT code, email, name FROM public.customers")
            .fetch_one(&mut connection)
            .await
            .unwrap();
    assert_ne!(code, "C1", "the key must be masked");
}

#[tokio::test]
async fn a_composite_parent_is_pulled() {
    let _guard = serial().lock().await;
    let (Some(source), Some(destination)) = (
        target("DBPINBALL_TEST_PG_SOURCE_URL", "source"),
        target("DBPINBALL_TEST_PG_DEST_URL", "destination"),
    ) else {
        eprintln!("skipped: the test database URLs are not set");
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let provider = MapProvider::new(&[
        ("source", &source.password),
        ("destination", &destination.password),
    ]);
    reset(&destination).await;
    enroll(&destination, &provider, &directory.path().to_path_buf()).await;

    let spec = spec("public.shipment_events", "region", "EU", 1);
    plan_and_execute(&spec, &source, &destination, &provider, directory.path())
        .await
        .expect("the plan should execute");

    assert_eq!(count(&destination, "shipment_events").await, 1);
    assert_eq!(count(&destination, "shipments").await, 1);

    let violations = fk_violations(
        &destination,
        "SELECT count(*) FROM public.shipment_events e
         LEFT JOIN public.shipments s ON s.region = e.region AND s.code = e.code
         WHERE s.code IS NULL",
    )
    .await;
    assert_eq!(violations, 0, "the composite join must satisfy the FK");
}

#[tokio::test]
async fn a_nullable_foreign_key_needs_no_parent() {
    let _guard = serial().lock().await;
    let (Some(source), Some(destination)) = (
        target("DBPINBALL_TEST_PG_SOURCE_URL", "source"),
        target("DBPINBALL_TEST_PG_DEST_URL", "destination"),
    ) else {
        eprintln!("skipped: the test database URLs are not set");
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let provider = MapProvider::new(&[
        ("source", &source.password),
        ("destination", &destination.password),
    ]);
    reset(&destination).await;
    enroll(&destination, &provider, &directory.path().to_path_buf()).await;

    let spec = spec(
        "public.orders",
        "id",
        "0b9f4c3e-0000-0000-0000-000000000004",
        1,
    );
    plan_and_execute(&spec, &source, &destination, &provider, directory.path())
        .await
        .expect("a nullable FK must not require a parent");

    assert_eq!(count(&destination, "orders").await, 1);
    assert_eq!(count(&destination, "customers").await, 0);

    let mut connection = connect(&destination).await;
    let (code,): (Option<String>,) = sqlx::query_as("SELECT customer_code FROM public.orders")
        .fetch_one(&mut connection)
        .await
        .unwrap();
    assert_eq!(code, None);
}

#[tokio::test]
async fn a_self_reference_is_deferred_and_imports() {
    let _guard = serial().lock().await;
    let (Some(source), Some(destination)) = (
        target("DBPINBALL_TEST_PG_SOURCE_URL", "source"),
        target("DBPINBALL_TEST_PG_DEST_URL", "destination"),
    ) else {
        eprintln!("skipped: the test database URLs are not set");
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let provider = MapProvider::new(&[
        ("source", &source.password),
        ("destination", &destination.password),
    ]);
    reset(&destination).await;
    enroll(&destination, &provider, &directory.path().to_path_buf()).await;

    let spec = spec("public.employees", "id", "E2", 1);
    let plan = plan_and_execute(&spec, &source, &destination, &provider, directory.path())
        .await
        .expect("the self-reference should import");

    assert!(!plan.constraints.deferred.is_empty());
    assert_eq!(count(&destination, "employees").await, 2);
    let violations = fk_violations(
        &destination,
        "SELECT count(*) FROM public.employees e
         LEFT JOIN public.employees m ON m.id = e.manager_id
         WHERE e.manager_id IS NOT NULL AND m.id IS NULL",
    )
    .await;
    assert_eq!(violations, 0);
}

#[tokio::test]
async fn repeated_parents_are_deduplicated() {
    let _guard = serial().lock().await;
    let (Some(source), Some(destination)) = (
        target("DBPINBALL_TEST_PG_SOURCE_URL", "source"),
        target("DBPINBALL_TEST_PG_DEST_URL", "destination"),
    ) else {
        eprintln!("skipped: the test database URLs are not set");
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let provider = MapProvider::new(&[
        ("source", &source.password),
        ("destination", &destination.password),
    ]);
    reset(&destination).await;
    enroll(&destination, &provider, &directory.path().to_path_buf()).await;

    // Two orders reference the same customer.
    let yaml = r#"
version: 1
source: source
destination: destination
selection:
  roots:
    - table: public.orders
      filter:
        - column: id
          op: in
          value:
            - 0b9f4c3e-0000-0000-0000-000000000001
            - 0b9f4c3e-0000-0000-0000-000000000003
  relationships:
    depth: 1
limits:
  max_rows_per_table: 100
  max_total_rows: 300
  max_depth: 3
  max_bytes: 1048576
  timeout_ms: 15000
masking:
  policy: development-default@2
"#;
    let spec = SafeSyncSpec::parse_yaml(yaml).unwrap();
    plan_and_execute(&spec, &source, &destination, &provider, directory.path())
        .await
        .expect("deduplicated parents should import");

    assert_eq!(count(&destination, "orders").await, 2);
    assert_eq!(count(&destination, "customers").await, 1);
}

// ---------------------------------------------------------------------------
// Bounds, truncation and cycles
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_depth_truncated_closure_is_marked_incomplete() {
    let _guard = serial().lock().await;
    let (Some(source), Some(destination)) = (
        target("DBPINBALL_TEST_PG_SOURCE_URL", "source"),
        target("DBPINBALL_TEST_PG_DEST_URL", "destination"),
    ) else {
        eprintln!("skipped: the test database URLs are not set");
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let provider = MapProvider::new(&[
        ("source", &source.password),
        ("destination", &destination.password),
    ]);
    reset(&destination).await;
    enroll(&destination, &provider, &directory.path().to_path_buf()).await;

    // Depth 0 with a real FK cannot be complete.
    let spec = spec(
        "public.orders",
        "id",
        "0b9f4c3e-0000-0000-0000-000000000001",
        0,
    );
    let store = security::store_for(directory.path());
    let plan = service::plan(
        &spec,
        &source.connection,
        &destination.connection,
        &provider,
        &store,
        &key_store(),
        now_ms(),
        true,
        false,
    )
    .await
    .expect("planning should succeed with an incomplete plan");

    assert!(!plan.complete, "a truncated closure must be incomplete");
    assert!(plan
        .checks
        .iter()
        .any(|check| check.limit == "closure" && !check.ok));
}

#[tokio::test]
async fn a_deferrable_cycle_is_deferred_and_imports() {
    let _guard = serial().lock().await;
    let (Some(source), Some(destination)) = (
        target("DBPINBALL_TEST_PG_SOURCE_URL", "source"),
        target("DBPINBALL_TEST_PG_DEST_URL", "destination"),
    ) else {
        eprintln!("skipped: the test database URLs are not set");
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let provider = MapProvider::new(&[
        ("source", &source.password),
        ("destination", &destination.password),
    ]);
    reset(&destination).await;
    enroll(&destination, &provider, &directory.path().to_path_buf()).await;

    let spec = spec("public.cycle_a", "id", "A1", 1);
    let plan = plan_and_execute(&spec, &source, &destination, &provider, directory.path())
        .await
        .expect("a deferrable cycle should import");

    assert!(!plan.constraints.deferred.is_empty());
    assert_eq!(count(&destination, "cycle_a").await, 1);
    assert_eq!(count(&destination, "cycle_b").await, 1);
    let violations = fk_violations(
        &destination,
        "SELECT count(*) FROM public.cycle_a a
         LEFT JOIN public.cycle_b b ON b.id = a.b_id
         WHERE b.id IS NULL",
    )
    .await;
    assert_eq!(violations, 0);
}

#[tokio::test]
async fn a_non_deferrable_cycle_is_refused() {
    let _guard = serial().lock().await;
    let (Some(source), Some(destination)) = (
        target("DBPINBALL_TEST_PG_SOURCE_URL", "source"),
        target("DBPINBALL_TEST_PG_DEST_URL", "destination"),
    ) else {
        eprintln!("skipped: the test database URLs are not set");
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let provider = MapProvider::new(&[
        ("source", &source.password),
        ("destination", &destination.password),
    ]);
    reset(&destination).await;
    enroll(&destination, &provider, &directory.path().to_path_buf()).await;

    let spec = spec("public.cycle_x", "id", "X1", 1);
    let store = security::store_for(directory.path());
    let error = service::plan(
        &spec,
        &source.connection,
        &destination.connection,
        &provider,
        &store,
        &key_store(),
        now_ms(),
        true,
        false,
    )
    .await
    .unwrap_err();

    assert_eq!(
        error.detail.as_deref(),
        Some("cyclic_fk_requires_deferrable")
    );
}

// ---------------------------------------------------------------------------
// Plan binding
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_plan_binds_the_requested_depth() {
    let _guard = serial().lock().await;
    let (Some(source), Some(destination)) = (
        target("DBPINBALL_TEST_PG_SOURCE_URL", "source"),
        target("DBPINBALL_TEST_PG_DEST_URL", "destination"),
    ) else {
        eprintln!("skipped: the test database URLs are not set");
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let provider = MapProvider::new(&[
        ("source", &source.password),
        ("destination", &destination.password),
    ]);
    reset(&destination).await;
    enroll(&destination, &provider, &directory.path().to_path_buf()).await;

    let store = security::store_for(directory.path());
    let shallow = spec(
        "public.orders",
        "id",
        "0b9f4c3e-0000-0000-0000-000000000001",
        1,
    );
    let deep = spec(
        "public.orders",
        "id",
        "0b9f4c3e-0000-0000-0000-000000000001",
        2,
    );

    let first = service::plan(
        &shallow,
        &source.connection,
        &destination.connection,
        &provider,
        &store,
        &key_store(),
        now_ms(),
        true,
        false,
    )
    .await
    .unwrap();
    let store = security::store_for(directory.path());
    let second = service::plan(
        &deep,
        &source.connection,
        &destination.connection,
        &provider,
        &store,
        &key_store(),
        now_ms(),
        true,
        false,
    )
    .await
    .unwrap();

    assert_ne!(first.plan_fingerprint, second.plan_fingerprint);
}

#[tokio::test]
async fn a_changed_traversal_depth_is_refused_at_execute() {
    let _guard = serial().lock().await;
    let (Some(source), Some(destination)) = (
        target("DBPINBALL_TEST_PG_SOURCE_URL", "source"),
        target("DBPINBALL_TEST_PG_DEST_URL", "destination"),
    ) else {
        eprintln!("skipped: the test database URLs are not set");
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let provider = MapProvider::new(&[
        ("source", &source.password),
        ("destination", &destination.password),
    ]);
    reset(&destination).await;
    enroll(&destination, &provider, &directory.path().to_path_buf()).await;

    let store = security::store_for(directory.path());
    let spec = spec(
        "public.orders",
        "id",
        "0b9f4c3e-0000-0000-0000-000000000001",
        1,
    );
    let plan = service::plan(
        &spec,
        &source.connection,
        &destination.connection,
        &provider,
        &store,
        &key_store(),
        now_ms(),
        true,
        false,
    )
    .await
    .unwrap();

    // Execute with a deeper spec than the reviewed plan.
    let deeper = spec_deeper();
    let store = security::store_for(directory.path());
    let error = service::execute(
        ExecuteRequest {
            plan: &plan,
            spec: &deeper,
            acknowledgement: &plan.plan_fingerprint,
            replace: false,
            source: &source.connection,
            destination: &destination.connection,
        },
        &provider,
        &store,
        &key_store(),
        None,
        now_ms(),
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
    )
    .await
    .unwrap_err();

    assert_eq!(error.detail.as_deref(), Some("spec_plan_mismatch"));
    assert_eq!(count(&destination, "orders").await, 0);
}

fn spec_deeper() -> SafeSyncSpec {
    SafeSyncSpec::parse_yaml(
        r#"
version: 1
source: source
destination: destination
selection:
  roots:
    - table: public.orders
      filter:
        - column: id
          op: eq
          value: 0b9f4c3e-0000-0000-0000-000000000001
  relationships:
    depth: 2
    direction: parents
limits:
  max_rows_per_table: 100
  max_total_rows: 300
  max_depth: 3
  max_bytes: 1048576
  timeout_ms: 15000
masking:
  policy: development-default@2
"#,
    )
    .unwrap()
}

#[tokio::test]
async fn the_import_mode_is_recorded_and_bound() {
    let _guard = serial().lock().await;
    let (Some(source), Some(destination)) = (
        target("DBPINBALL_TEST_PG_SOURCE_URL", "source"),
        target("DBPINBALL_TEST_PG_DEST_URL", "destination"),
    ) else {
        eprintln!("skipped: the test database URLs are not set");
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let provider = MapProvider::new(&[
        ("source", &source.password),
        ("destination", &destination.password),
    ]);
    reset(&destination).await;
    enroll(&destination, &provider, &directory.path().to_path_buf()).await;

    let store = security::store_for(directory.path());
    let spec = spec("public.customers", "code", "C1", 0);
    let plan = service::plan(
        &spec,
        &source.connection,
        &destination.connection,
        &provider,
        &store,
        &key_store(),
        now_ms(),
        true,
        false,
    )
    .await
    .unwrap();

    assert_eq!(plan.import.conflict_policy, "refuse");
    assert_eq!(plan.import.mode, "single_transaction");
    assert_eq!(plan.constraints.order.len(), 1);
    let _ = ConflictPolicy::Refuse;
}

#[tokio::test]
async fn a_destination_constraint_failure_rolls_back_every_row() {
    let _guard = serial().lock().await;
    let (Some(source), Some(destination)) = (
        target("DBPINBALL_TEST_PG_SOURCE_URL", "source"),
        target("DBPINBALL_TEST_PG_DEST_URL", "destination"),
    ) else {
        eprintln!("skipped: the test database URLs are not set");
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let provider = MapProvider::new(&[
        ("source", &source.password),
        ("destination", &destination.password),
    ]);
    reset(&destination).await;
    enroll(&destination, &provider, &directory.path().to_path_buf()).await;

    // The parent inserts first, then the child fails on a trigger: the whole
    // transaction must roll back, including the already-inserted parent. A
    // trigger is not part of the schema hash, so this is an insert-time failure
    // rather than destination schema drift.
    let mut connection = connect(&destination).await;
    sqlx::query(
        "CREATE OR REPLACE FUNCTION public.m4_reject_order() RETURNS trigger
         LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'policy rejected the row'; END $$",
    )
    .execute(&mut connection)
    .await
    .unwrap();
    let _ = sqlx::query("DROP TRIGGER IF EXISTS m4_reject_orders ON public.orders")
        .execute(&mut connection)
        .await;
    sqlx::query(
        "CREATE TRIGGER m4_reject_orders BEFORE INSERT ON public.orders
         FOR EACH ROW EXECUTE FUNCTION public.m4_reject_order()",
    )
    .execute(&mut connection)
    .await
    .unwrap();

    let spec = spec(
        "public.orders",
        "id",
        "0b9f4c3e-0000-0000-0000-000000000001",
        1,
    );
    let error = plan_and_execute(&spec, &source, &destination, &provider, directory.path())
        .await
        .unwrap_err();
    assert_eq!(
        error.code,
        ErrorCode::Operation,
        "detail={:?} message={}",
        error.detail,
        error.message
    );

    assert_eq!(count(&destination, "customers").await, 0);
    assert_eq!(count(&destination, "orders").await, 0);
    assert_eq!(
        run_marker_count(&destination).await,
        0,
        "no run marker for a rolled-back import"
    );
    // The source is untouched.
    let source_rows: (i64,) = {
        let mut source_connection = connect(&source).await;
        sqlx::query_as("SELECT count(*) FROM public.orders")
            .fetch_one(&mut source_connection)
            .await
            .unwrap()
    };
    assert_eq!(source_rows.0, 4);

    sqlx::query("DROP TRIGGER m4_reject_orders ON public.orders")
        .execute(&mut connection)
        .await
        .unwrap();
    sqlx::query("DROP FUNCTION public.m4_reject_order()")
        .execute(&mut connection)
        .await
        .unwrap();
}

// ---------------------------------------------------------------------------
// RLS-hidden parent
// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_rls_hidden_parent_cannot_produce_a_complete_plan() {
    let _guard = serial().lock().await;
    let (Some(source), Some(destination)) = (
        target("DBPINBALL_TEST_PG_SOURCE_URL", "source"),
        target("DBPINBALL_TEST_PG_DEST_URL", "destination"),
    ) else {
        eprintln!("skipped: the test database URLs are not set");
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let provider = MapProvider::new(&[
        ("source", &source.password),
        ("destination", &destination.password),
    ]);
    reset(&destination).await;
    enroll(&destination, &provider, &directory.path().to_path_buf()).await;

    // C2 references R2, which row-level security hides from the source role.
    let hidden = spec("public.rls_children", "id", "C2", 1);
    let store = security::store_for(directory.path());
    let plan = service::plan(
        &hidden,
        &source.connection,
        &destination.connection,
        &provider,
        &store,
        &key_store(),
        now_ms(),
        true,
        false,
    )
    .await
    .expect("planning succeeds with an incomplete plan");

    assert!(!plan.complete, "a hidden parent must not look complete");
    assert!(plan
        .checks
        .iter()
        .any(|check| check.limit == "closure" && !check.ok));

    let store = security::store_for(directory.path());
    let error = service::execute(
        ExecuteRequest {
            plan: &plan,
            spec: &hidden,
            acknowledgement: &plan.plan_fingerprint,
            replace: false,
            source: &source.connection,
            destination: &destination.connection,
        },
        &provider,
        &store,
        &key_store(),
        None,
        now_ms(),
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
    )
    .await
    .unwrap_err();
    assert_eq!(error.detail.as_deref(), Some("limit_exceeded"));
    assert!(!error.message.contains("Hidden"));
    assert!(!error.message.contains("R2"));

    assert_eq!(count(&destination, "rls_children").await, 0);
    assert_eq!(count(&destination, "rls_parents").await, 0);

    // The visible parent path still works.
    let visible = spec("public.rls_children", "id", "C1", 1);
    plan_and_execute(&visible, &source, &destination, &provider, directory.path())
        .await
        .expect("a visible parent imports");
    assert_eq!(count(&destination, "rls_children").await, 1);
    assert_eq!(count(&destination, "rls_parents").await, 1);
}

// ---------------------------------------------------------------------------
// Bounds matrix
// ---------------------------------------------------------------------------

fn spec_yaml(root: &str, column: &str, value: &str, depth: u8, extra_limits: &str) -> SafeSyncSpec {
    let yaml = format!(
        r#"
version: 1
source: source
destination: destination
selection:
  roots:
    - table: {root}
      filter:
        - column: {column}
          op: eq
          value: {value}
  relationships:
    depth: {depth}
    direction: parents
limits:
{extra_limits}
masking:
  policy: development-default@2
"#
    );
    SafeSyncSpec::parse_yaml(&yaml).expect("the test spec is valid")
}

async fn plan_error(
    spec: &SafeSyncSpec,
    source: &Target,
    destination: &Target,
    provider: &MapProvider,
    directory: &std::path::Path,
) -> CoreError {
    let store = security::store_for(directory);
    service::plan(
        spec,
        &source.connection,
        &destination.connection,
        provider,
        &store,
        &key_store(),
        now_ms(),
        true,
        false,
    )
    .await
    .unwrap_err()
}

#[tokio::test]
async fn a_per_table_row_limit_is_refused() {
    let _guard = serial().lock().await;
    let (Some(source), Some(destination)) = (
        target("DBPINBALL_TEST_PG_SOURCE_URL", "source"),
        target("DBPINBALL_TEST_PG_DEST_URL", "destination"),
    ) else {
        eprintln!("skipped: the test database URLs are not set");
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let provider = MapProvider::new(&[
        ("source", &source.password),
        ("destination", &destination.password),
    ]);
    reset(&destination).await;
    enroll(&destination, &provider, &directory.path().to_path_buf()).await;

    // Three roots, a limit of two.
    let yaml = r#"
version: 1
source: source
destination: destination
selection:
  roots:
    - table: public.orders
      filter:
        - column: id
          op: in
          value:
            - 0b9f4c3e-0000-0000-0000-000000000001
            - 0b9f4c3e-0000-0000-0000-000000000002
            - 0b9f4c3e-0000-0000-0000-000000000003
  relationships:
    depth: 0
limits:
  max_rows_per_table: 2
  max_total_rows: 100
  max_depth: 3
  max_bytes: 1048576
  timeout_ms: 15000
masking:
  policy: development-default@2
"#;
    let spec = SafeSyncSpec::parse_yaml(yaml).unwrap();
    let error = plan_error(&spec, &source, &destination, &provider, directory.path()).await;
    assert_eq!(error.detail.as_deref(), Some("limit_exceeded"));
    assert_eq!(count(&destination, "orders").await, 0);
}

#[tokio::test]
async fn a_per_table_limit_is_cumulative_across_roots() {
    let _guard = serial().lock().await;
    let (Some(source), Some(destination)) = (
        target("DBPINBALL_TEST_PG_SOURCE_URL", "source"),
        target("DBPINBALL_TEST_PG_DEST_URL", "destination"),
    ) else {
        eprintln!("skipped: the test database URLs are not set");
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let provider = MapProvider::new(&[
        ("source", &source.password),
        ("destination", &destination.password),
    ]);
    reset(&destination).await;
    enroll(&destination, &provider, &directory.path().to_path_buf()).await;

    // Two roots on the same table, each returning one row, with a limit of one.
    let yaml = r#"
version: 1
source: source
destination: destination
selection:
  roots:
    - table: public.orders
      filter:
        - column: id
          op: eq
          value: 0b9f4c3e-0000-0000-0000-000000000001
    - table: public.orders
      filter:
        - column: id
          op: eq
          value: 0b9f4c3e-0000-0000-0000-000000000002
  relationships:
    depth: 0
limits:
  max_rows_per_table: 1
  max_total_rows: 100
  max_depth: 3
  max_bytes: 1048576
  timeout_ms: 15000
masking:
  policy: development-default@2
"#;
    let spec = SafeSyncSpec::parse_yaml(yaml).unwrap();
    let error = plan_error(&spec, &source, &destination, &provider, directory.path()).await;
    assert_eq!(error.detail.as_deref(), Some("limit_exceeded"));
    assert_eq!(count(&destination, "orders").await, 0);
}

#[tokio::test]
async fn a_global_row_limit_is_refused() {
    let _guard = serial().lock().await;
    let (Some(source), Some(destination)) = (
        target("DBPINBALL_TEST_PG_SOURCE_URL", "source"),
        target("DBPINBALL_TEST_PG_DEST_URL", "destination"),
    ) else {
        eprintln!("skipped: the test database URLs are not set");
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let provider = MapProvider::new(&[
        ("source", &source.password),
        ("destination", &destination.password),
    ]);
    reset(&destination).await;
    enroll(&destination, &provider, &directory.path().to_path_buf()).await;

    // One order plus its parent is two rows; the global limit is one.
    let spec = spec_yaml(
        "public.orders",
        "id",
        "0b9f4c3e-0000-0000-0000-000000000001",
        1,
        "  max_rows_per_table: 100\n  max_total_rows: 1\n  max_depth: 3\n  max_bytes: 1048576\n  timeout_ms: 15000",
    );
    let error = plan_error(&spec, &source, &destination, &provider, directory.path()).await;
    assert_eq!(error.detail.as_deref(), Some("limit_exceeded"));
    assert_eq!(count(&destination, "orders").await, 0);
}

#[tokio::test]
async fn a_byte_limit_is_refused() {
    let _guard = serial().lock().await;
    let (Some(source), Some(destination)) = (
        target("DBPINBALL_TEST_PG_SOURCE_URL", "source"),
        target("DBPINBALL_TEST_PG_DEST_URL", "destination"),
    ) else {
        eprintln!("skipped: the test database URLs are not set");
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let provider = MapProvider::new(&[
        ("source", &source.password),
        ("destination", &destination.password),
    ]);
    reset(&destination).await;
    enroll(&destination, &provider, &directory.path().to_path_buf()).await;

    let spec = spec_yaml(
        "public.orders",
        "id",
        "0b9f4c3e-0000-0000-0000-000000000001",
        1,
        "  max_rows_per_table: 100\n  max_total_rows: 100\n  max_depth: 3\n  max_bytes: 1\n  timeout_ms: 15000",
    );
    let error = plan_error(&spec, &source, &destination, &provider, directory.path()).await;
    assert_eq!(error.detail.as_deref(), Some("limit_exceeded"));
    assert_eq!(count(&destination, "orders").await, 0);
}

#[tokio::test]
async fn a_cancelled_run_writes_nothing() {
    let _guard = serial().lock().await;
    let (Some(source), Some(destination)) = (
        target("DBPINBALL_TEST_PG_SOURCE_URL", "source"),
        target("DBPINBALL_TEST_PG_DEST_URL", "destination"),
    ) else {
        eprintln!("skipped: the test database URLs are not set");
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let provider = MapProvider::new(&[
        ("source", &source.password),
        ("destination", &destination.password),
    ]);
    reset(&destination).await;
    enroll(&destination, &provider, &directory.path().to_path_buf()).await;

    let spec = spec(
        "public.orders",
        "id",
        "0b9f4c3e-0000-0000-0000-000000000001",
        1,
    );
    let store = security::store_for(directory.path());
    let plan = service::plan(
        &spec,
        &source.connection,
        &destination.connection,
        &provider,
        &store,
        &key_store(),
        now_ms(),
        true,
        false,
    )
    .await
    .expect("plan");

    let store = security::store_for(directory.path());
    let error = service::execute(
        ExecuteRequest {
            plan: &plan,
            spec: &spec,
            acknowledgement: &plan.plan_fingerprint,
            replace: false,
            source: &source.connection,
            destination: &destination.connection,
        },
        &provider,
        &store,
        &key_store(),
        None,
        now_ms(),
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
    )
    .await
    .unwrap_err();

    assert_eq!(error.detail.as_deref(), Some("cancelled"));
    assert_eq!(count(&destination, "orders").await, 0);
    assert_eq!(count(&destination, "customers").await, 0);
    assert_eq!(run_marker_count(&destination).await, 0);
}

// ---------------------------------------------------------------------------
// Identity and generated columns
// ---------------------------------------------------------------------------

#[tokio::test]
async fn identity_values_are_preserved_and_the_sequence_is_reconciled() {
    let _guard = serial().lock().await;
    let (Some(source), Some(destination)) = (
        target("DBPINBALL_TEST_PG_SOURCE_URL", "source"),
        target("DBPINBALL_TEST_PG_DEST_URL", "destination"),
    ) else {
        eprintln!("skipped: the test database URLs are not set");
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let provider = MapProvider::new(&[
        ("source", &source.password),
        ("destination", &destination.password),
    ]);
    reset(&destination).await;
    enroll(&destination, &provider, &directory.path().to_path_buf()).await;

    let yaml = r#"
version: 1
source: source
destination: destination
selection:
  roots:
    - table: public.identity_children
      filter:
        - column: id
          op: in
          value:
            - 0b9f4c3e-0000-0000-0000-0000000000c1
            - 0b9f4c3e-0000-0000-0000-0000000000c2
  relationships:
    depth: 1
limits:
  max_rows_per_table: 100
  max_total_rows: 100
  max_depth: 3
  max_bytes: 1048576
  timeout_ms: 15000
masking:
  policy: development-default@2
"#;
    let first = SafeSyncSpec::parse_yaml(yaml).unwrap();
    plan_and_execute(&first, &source, &destination, &provider, directory.path())
        .await
        .expect("the identity parents should import");

    assert_eq!(count(&destination, "identity_children").await, 2);
    assert_eq!(count(&destination, "identity_parents").await, 2);

    let mut connection = connect(&destination).await;
    let parents: Vec<(i64, String, String)> =
        sqlx::query_as("SELECT id, code, full_label FROM public.identity_parents ORDER BY id")
            .fetch_all(&mut connection)
            .await
            .unwrap();
    // The identity values are preserved, not regenerated.
    assert_eq!(
        parents.iter().map(|(id, _, _)| *id).collect::<Vec<_>>(),
        vec![1, 2]
    );
    for (id, code, label) in &parents {
        // The generated column is recomputed by the destination from masked values.
        assert_eq!(label, &format!("{code}:{id}"));
        assert_ne!(code, "A");
        assert_ne!(code, "B");
    }

    let broken: (i64,) = sqlx::query_as(
        "SELECT count(*) FROM public.identity_children c
         LEFT JOIN public.identity_parents p ON p.id = c.parent_id
         WHERE p.id IS NULL",
    )
    .fetch_one(&mut connection)
    .await
    .unwrap();
    assert_eq!(broken.0, 0);

    let (last,): (i64,) = sqlx::query_as("SELECT last_value FROM public.identity_parents_id_seq")
        .fetch_one(&mut connection)
        .await
        .unwrap();
    assert!(last >= 2, "the sequence must cover the imported identities");

    // Force the sequence behind the data, then replay the completed plan. The
    // reconciliation must raise it without duplicating rows.
    sqlx::query("SELECT setval('public.identity_parents_id_seq', 1, true)")
        .execute(&mut connection)
        .await
        .unwrap();
    let replay = plan_and_execute(&first, &source, &destination, &provider, directory.path())
        .await
        .expect("the repeat should reconcile and not duplicate");
    assert!(replay.complete);
    assert_eq!(count(&destination, "identity_parents").await, 2);
    let (last,): (i64,) = sqlx::query_as("SELECT last_value FROM public.identity_parents_id_seq")
        .fetch_one(&mut connection)
        .await
        .unwrap();
    assert!(
        last >= 2,
        "reconciliation must raise a sequence that is behind"
    );
}

#[tokio::test]
async fn the_unguarded_extraction_helper_is_refused_for_a_table_with_fks() {
    use dbpinball_core::safesync::{
        extract_table, graph::inspect_table, BoundSession, Limits, Masker, MaskingPolicy,
        QualifiedName, RootSelection, SessionRole,
    };

    let _guard = serial().lock().await;
    let Some(source) = target("DBPINBALL_TEST_PG_SOURCE_URL", "source") else {
        eprintln!("skipped: DBPINBALL_TEST_PG_SOURCE_URL is not set");
        return;
    };
    let provider = MapProvider::new(&[("source", &source.password)]);
    let mut session = BoundSession::open(
        SessionRole::Source,
        &source.connection,
        &provider,
        &Limits::default(),
    )
    .await
    .expect("open");

    let info = inspect_table(
        &mut session,
        &QualifiedName::parse("public.orders").unwrap(),
    )
    .await
    .expect("inspect");
    let masker = Masker::new(
        MaskingPolicy::builtin_v2(),
        MaskKey::from_bytes("k-test", [7u8; 32]),
    )
    .unwrap();
    let root = RootSelection {
        table: "public.orders".into(),
        filter: vec![dbpinball_core::safesync::FilterPredicate {
            column: "id".into(),
            op: dbpinball_core::safesync::FilterOp::Eq,
            value: dbpinball_core::safesync::FilterValue::Text(
                "0b9f4c3e-0000-0000-0000-000000000001".into(),
            ),
        }],
    };

    let error = extract_table(&mut session, &info, &root, &masker, &Limits::default())
        .await
        .unwrap_err();
    assert_eq!(
        error.detail.as_deref(),
        Some("unguarded_extraction_refused")
    );
}

// ---------------------------------------------------------------------------
// S2-b defect remediation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_nullable_identity_reference_keeps_null() {
    let _guard = serial().lock().await;
    let (Some(source), Some(destination)) = (
        target("DBPINBALL_TEST_PG_SOURCE_URL", "source"),
        target("DBPINBALL_TEST_PG_DEST_URL", "destination"),
    ) else {
        eprintln!("skipped: the test database URLs are not set");
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let provider = MapProvider::new(&[
        ("source", &source.password),
        ("destination", &destination.password),
    ]);
    reset(&destination).await;
    enroll(&destination, &provider, &directory.path().to_path_buf()).await;

    let spec = spec(
        "public.identity_optional",
        "id",
        "0b9f4c3e-0000-0000-0000-0000000000d1",
        1,
    );
    plan_and_execute(&spec, &source, &destination, &provider, directory.path())
        .await
        .expect("the nullable identity reference should import");

    let mut connection = connect(&destination).await;
    let (parent,): (Option<i64>,) = sqlx::query_as(
        "SELECT parent_id FROM public.identity_optional
         WHERE id = '0b9f4c3e-0000-0000-0000-0000000000d1'",
    )
    .fetch_one(&mut connection)
    .await
    .unwrap();
    // SQL NULL is preserved as NULL, never as an empty string.
    assert_eq!(parent, None);
}

#[tokio::test]
async fn an_ordinary_integer_cannot_be_kept() {
    let _guard = serial().lock().await;
    let (Some(source), Some(destination)) = (
        target("DBPINBALL_TEST_PG_SOURCE_URL", "source"),
        target("DBPINBALL_TEST_PG_DEST_URL", "destination"),
    ) else {
        eprintln!("skipped: the test database URLs are not set");
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let provider = MapProvider::new(&[
        ("source", &source.password),
        ("destination", &destination.password),
    ]);
    reset(&destination).await;
    enroll(&destination, &provider, &directory.path().to_path_buf()).await;

    let spec = spec(
        "public.sensitive_metrics",
        "id",
        "0b9f4c3e-0000-0000-0000-0000000000e1",
        0,
    );
    let error = plan_and_execute(&spec, &source, &destination, &provider, directory.path())
        .await
        .unwrap_err();
    assert_eq!(
        error.detail.as_deref(),
        Some("integer_keep_requires_identity")
    );
}

#[tokio::test]
async fn the_run_table_is_migrated_from_s2a() {
    let _guard = serial().lock().await;
    let (Some(source), Some(destination)) = (
        target("DBPINBALL_TEST_PG_SOURCE_URL", "source"),
        target("DBPINBALL_TEST_PG_DEST_URL", "destination"),
    ) else {
        eprintln!("skipped: the test database URLs are not set");
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let provider = MapProvider::new(&[
        ("source", &source.password),
        ("destination", &destination.password),
    ]);
    reset(&destination).await;
    enroll(&destination, &provider, &directory.path().to_path_buf()).await;

    // A pre-S2-b run table: no sequence_status column.
    let mut connection = connect(&destination).await;
    sqlx::query("DROP TABLE IF EXISTS public.dbpinball_safe_sync_runs")
        .execute(&mut connection)
        .await
        .unwrap();
    sqlx::query(
        "CREATE TABLE public.dbpinball_safe_sync_runs (
           run_id uuid PRIMARY KEY,
           plan_fingerprint text NOT NULL,
           spec_hash text NOT NULL,
           source_identity_key text,
           destination_identity_key text,
           schema_hash text NOT NULL,
           masking_policy text NOT NULL,
           mask_key_id text NOT NULL,
           conflict_policy text NOT NULL,
           import_mode text NOT NULL,
           destination_digest text NOT NULL,
           rows_written bigint NOT NULL,
           status text NOT NULL,
           finished_at timestamptz NOT NULL DEFAULT now()
         )",
    )
    .execute(&mut connection)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO public.dbpinball_safe_sync_runs
           (run_id, plan_fingerprint, spec_hash, schema_hash, masking_policy,
            mask_key_id, conflict_policy, import_mode, destination_digest,
            rows_written, status)
         VALUES ('0b9f4c3e-0000-0000-0000-0000000000f1', 'historical', 'old',
                 'old', 'old', 'old', 'refuse', 'single_transaction', 'old', 0,
                 'completed')",
    )
    .execute(&mut connection)
    .await
    .unwrap();

    let spec = spec(
        "public.accounts",
        "id",
        "0b9f4c3e-0000-0000-0000-000000000001",
        0,
    );
    plan_and_execute(&spec, &source, &destination, &provider, directory.path())
        .await
        .expect("the run table should be migrated and the import should succeed");

    let has_column: bool = sqlx::query_scalar(
        "SELECT EXISTS (
           SELECT 1 FROM pg_attribute a
           JOIN pg_class c ON c.oid = a.attrelid
           WHERE c.relname = 'dbpinball_safe_sync_runs'
             AND a.attname = 'sequence_status' AND NOT a.attisdropped)",
    )
    .fetch_one(&mut connection)
    .await
    .unwrap();
    assert!(has_column, "the migration must add sequence_status");

    let historical: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM public.dbpinball_safe_sync_runs
         WHERE plan_fingerprint = 'historical'",
    )
    .fetch_one(&mut connection)
    .await
    .unwrap();
    assert_eq!(historical, 1, "the migration must preserve existing runs");
}

#[tokio::test]
async fn a_run_without_identity_reports_not_required() {
    let _guard = serial().lock().await;
    let (Some(source), Some(destination)) = (
        target("DBPINBALL_TEST_PG_SOURCE_URL", "source"),
        target("DBPINBALL_TEST_PG_DEST_URL", "destination"),
    ) else {
        eprintln!("skipped: the test database URLs are not set");
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let provider = MapProvider::new(&[
        ("source", &source.password),
        ("destination", &destination.password),
    ]);
    reset(&destination).await;
    enroll(&destination, &provider, &directory.path().to_path_buf()).await;

    let spec = spec(
        "public.accounts",
        "id",
        "0b9f4c3e-0000-0000-0000-000000000001",
        0,
    );
    plan_and_execute(&spec, &source, &destination, &provider, directory.path())
        .await
        .expect("the import should succeed");

    let mut connection = connect(&destination).await;
    let status: String = sqlx::query_scalar(
        "SELECT sequence_status FROM public.dbpinball_safe_sync_runs
         ORDER BY finished_at DESC LIMIT 1",
    )
    .fetch_one(&mut connection)
    .await
    .unwrap();
    assert_eq!(status, "not_required");
}

#[tokio::test]
async fn an_empty_identity_table_needs_no_setval() {
    let _guard = serial().lock().await;
    let (Some(source), Some(destination)) = (
        target("DBPINBALL_TEST_PG_SOURCE_URL", "source"),
        target("DBPINBALL_TEST_PG_DEST_URL", "destination"),
    ) else {
        eprintln!("skipped: the test database URLs are not set");
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let provider = MapProvider::new(&[
        ("source", &source.password),
        ("destination", &destination.password),
    ]);
    reset(&destination).await;
    enroll(&destination, &provider, &directory.path().to_path_buf()).await;

    // A filter that matches no identity row: the destination identity table
    // stays empty, so no setval is required and setval(0) is never attempted.
    let spec = spec("public.identity_parents", "code", "ZZZ", 0);
    plan_and_execute(&spec, &source, &destination, &provider, directory.path())
        .await
        .expect("an empty identity selection should succeed");

    let mut connection = connect(&destination).await;
    let status: String = sqlx::query_scalar(
        "SELECT sequence_status FROM public.dbpinball_safe_sync_runs
         ORDER BY finished_at DESC LIMIT 1",
    )
    .fetch_one(&mut connection)
    .await
    .unwrap();
    assert_eq!(status, "not_required");
}

#[tokio::test]
async fn a_changed_generated_expression_is_refused() {
    let _guard = serial().lock().await;
    let (Some(source), Some(destination)) = (
        target("DBPINBALL_TEST_PG_SOURCE_URL", "source"),
        target("DBPINBALL_TEST_PG_DEST_URL", "destination"),
    ) else {
        eprintln!("skipped: the test database URLs are not set");
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let provider = MapProvider::new(&[
        ("source", &source.password),
        ("destination", &destination.password),
    ]);
    reset(&destination).await;
    enroll(&destination, &provider, &directory.path().to_path_buf()).await;

    let spec = spec(
        "public.identity_children",
        "id",
        "0b9f4c3e-0000-0000-0000-0000000000c1",
        1,
    );
    let store = security::store_for(directory.path());
    let plan = service::plan(
        &spec,
        &source.connection,
        &destination.connection,
        &provider,
        &store,
        &key_store(),
        now_ms(),
        true,
        false,
    )
    .await
    .expect("plan");

    // Change the destination generated expression after the plan was made.
    let mut connection = connect(&destination).await;
    sqlx::query("ALTER TABLE public.identity_parents DROP COLUMN full_label")
        .execute(&mut connection)
        .await
        .unwrap();
    sqlx::query(
        "ALTER TABLE public.identity_parents
         ADD COLUMN full_label text GENERATED ALWAYS AS (code || '#' || id) STORED",
    )
    .execute(&mut connection)
    .await
    .unwrap();

    let store = security::store_for(directory.path());
    let error = service::execute(
        ExecuteRequest {
            plan: &plan,
            spec: &spec,
            acknowledgement: &plan.plan_fingerprint,
            replace: false,
            source: &source.connection,
            destination: &destination.connection,
        },
        &provider,
        &store,
        &key_store(),
        Some(directory.path()),
        now_ms(),
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
    )
    .await
    .unwrap_err();
    assert_eq!(error.detail.as_deref(), Some("schema_drift"));

    // Restore the fixture expression for the other tests.
    sqlx::query("ALTER TABLE public.identity_parents DROP COLUMN full_label")
        .execute(&mut connection)
        .await
        .unwrap();
    sqlx::query(
        "ALTER TABLE public.identity_parents
         ADD COLUMN full_label text GENERATED ALWAYS AS (code || ':' || id) STORED",
    )
    .execute(&mut connection)
    .await
    .unwrap();
}

// ---------------------------------------------------------------------------
// Sequence reconciliation safety
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_sequence_lock_blocks_external_writers() {
    let _guard = serial().lock().await;
    let Some(destination) = target("DBPINBALL_TEST_PG_DEST_URL", "destination") else {
        eprintln!("skipped: the test database URLs are not set");
        return;
    };

    let mut holder = connect(&destination).await;
    let mut writer = connect(&destination).await;

    sqlx::query("DROP TABLE IF EXISTS public.lock_probe")
        .execute(&mut holder)
        .await
        .unwrap();
    sqlx::query(
        "CREATE TABLE public.lock_probe (
           id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY
         )",
    )
    .execute(&mut holder)
    .await
    .unwrap();

    // Hold the AccessExclusiveLock the reconciliation uses.
    sqlx::query("BEGIN").execute(&mut holder).await.unwrap();
    sqlx::query("ALTER SEQUENCE public.lock_probe_id_seq CACHE 1")
        .execute(&mut holder)
        .await
        .unwrap();

    // An external nextval must block while the lock is held.
    sqlx::query("SET statement_timeout = '1500'")
        .execute(&mut writer)
        .await
        .unwrap();
    let blocked = sqlx::query("SELECT nextval('public.lock_probe_id_seq')")
        .execute(&mut writer)
        .await
        .unwrap_err();
    let code = blocked
        .as_database_error()
        .and_then(|database| database.code())
        .map(|code| code.to_string())
        .unwrap_or_default();
    assert_eq!(code, "57014", "nextval must block on the sequence lock");

    // An external setval must block too.
    let blocked = sqlx::query("SELECT setval('public.lock_probe_id_seq', 50, true)")
        .execute(&mut writer)
        .await
        .unwrap_err();
    let code = blocked
        .as_database_error()
        .and_then(|database| database.code())
        .map(|code| code.to_string())
        .unwrap_or_default();
    assert_eq!(code, "57014", "setval must block on the sequence lock");

    sqlx::query("COMMIT").execute(&mut holder).await.unwrap();

    // After the lock is released, the writer proceeds.
    sqlx::query("SET statement_timeout = '5000'")
        .execute(&mut writer)
        .await
        .unwrap();
    let value: i64 = sqlx::query_scalar("SELECT nextval('public.lock_probe_id_seq')")
        .fetch_one(&mut writer)
        .await
        .unwrap();
    assert!(value >= 1);

    sqlx::query("DROP TABLE IF EXISTS public.lock_probe")
        .execute(&mut holder)
        .await
        .unwrap();
}

#[tokio::test]
async fn a_sequence_ahead_of_the_data_is_not_required() {
    let _guard = serial().lock().await;
    let (Some(source), Some(destination)) = (
        target("DBPINBALL_TEST_PG_SOURCE_URL", "source"),
        target("DBPINBALL_TEST_PG_DEST_URL", "destination"),
    ) else {
        eprintln!("skipped: the test database URLs are not set");
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let provider = MapProvider::new(&[
        ("source", &source.password),
        ("destination", &destination.password),
    ]);
    reset(&destination).await;
    enroll(&destination, &provider, &directory.path().to_path_buf()).await;

    // Put the destination sequence well ahead of the imported identity.
    let mut connection = connect(&destination).await;
    sqlx::query("SELECT setval('public.identity_parents_id_seq', 100, true)")
        .execute(&mut connection)
        .await
        .unwrap();

    let spec = spec(
        "public.identity_children",
        "id",
        "0b9f4c3e-0000-0000-0000-0000000000c1",
        1,
    );
    plan_and_execute(&spec, &source, &destination, &provider, directory.path())
        .await
        .expect("the import should succeed");

    let status: String = sqlx::query_scalar(
        "SELECT sequence_status FROM public.dbpinball_safe_sync_runs
         ORDER BY finished_at DESC LIMIT 1",
    )
    .fetch_one(&mut connection)
    .await
    .unwrap();
    assert_eq!(status, "not_required");

    let (last,): (i64,) = sqlx::query_as("SELECT last_value FROM public.identity_parents_id_seq")
        .fetch_one(&mut connection)
        .await
        .unwrap();
    assert!(last >= 100, "an ahead sequence must not be lowered");
}
