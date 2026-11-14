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
