#![cfg(feature = "integration")]

//! Real PostgreSQL integration tests for M4 S2-c typed range filters.
//!
//! Two isolated clusters are required:
//!
//! - `DBPINBALL_TEST_PG_SOURCE_URL` — a restricted, SELECT-only source role
//! - `DBPINBALL_TEST_PG_DEST_URL` — a writable destination role
//! - `DBPINBALL_TEST_PG_SUPER_URL` — superuser on the source cluster used to
//!   create isolated test schemas and grant SELECT to the source role
//!
//! A skipped run is not acceptance.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use dbpinball_core::domain::connection::{DbConnection, DbEngine, TargetClass};
use dbpinball_core::domain::error::{CoreError, ErrorCode};
use dbpinball_core::ports::credentials::{CredentialProvider, Secret};
use dbpinball_core::safesync::{
    extract_table, inspect_table, service, BoundSession, ColumnRule, DryRunPlan, ExecuteRequest,
    FilterOp, FilterPredicate, FilterValue, InMemoryMaskKeyStore, Limits, MaskKey, MaskStrategy,
    Masker, MaskingPolicy, QualifiedName, RootSelection, RunReport, SafeSyncSpec, SessionRole,
};
use dbpinball_core::security::{self, DestinationCategory};
use sqlx::postgres::{PgConnectOptions, PgConnection};
use sqlx::Connection;

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

fn serial() -> &'static tokio::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

impl RangeCluster {
    async fn locked() -> Option<(tokio::sync::MutexGuard<'static, ()>, Self)> {
        let guard = serial().lock().await;
        let cluster = Self::setup().await?;
        Some((guard, cluster))
    }
}

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
    let Ok(url) = std::env::var(variable) else {
        assert!(
            std::env::var_os("CI").is_none(),
            "{variable} must be set when CI is set: a skipped database test is not a pass"
        );
        return None;
    };
    let parsed = match url::Url::parse(&url) {
        Ok(parsed) => parsed,
        Err(error) => {
            assert!(
                std::env::var_os("CI").is_none(),
                "{variable} is not a valid URL when CI is set: {error}"
            );
            return None;
        }
    };
    let Some(host) = parsed.host_str() else {
        assert!(
            std::env::var_os("CI").is_none(),
            "{variable} has no host when CI is set"
        );
        return None;
    };

    let connection = DbConnection {
        id: id.to_string(),
        name: id.to_string(),
        engine: DbEngine::Postgres,
        host: host.to_string(),
        port: parsed.port().unwrap_or(5432),
        database: parsed.path().trim_start_matches('/').to_string(),
        username: parsed.username().to_string(),
        use_ssl: false,
        target: TargetClass::Unknown,
        ssh_tunnel: None,
    };

    Some(Target {
        connection,
        password: parsed.password().unwrap_or_default().to_string(),
    })
}

fn super_target(source: &Target) -> Target {
    if let Some(mut sup) = target("DBPINBALL_TEST_PG_SUPER_URL", "super") {
        sup.connection.database = source.connection.database.clone();
        return sup;
    }
    Target {
        connection: source.connection.clone(),
        password: source.password.clone(),
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn test_key() -> MaskKey {
    MaskKey::from_bytes("k-s2c-test", [9u8; 32])
}

fn key_store() -> InMemoryMaskKeyStore {
    InMemoryMaskKeyStore::with_key(test_key())
}

async fn connect(target: &Target) -> PgConnection {
    let options = PgConnectOptions::new()
        .host(&target.connection.host)
        .port(target.connection.port)
        .username(&target.connection.username)
        .password(&target.password)
        .database(&target.connection.database);
    PgConnection::connect_with(&options)
        .await
        .expect("fixture connection should open")
}

async fn exec_batch(target: &Target, sql: &str) {
    let mut conn = connect(target).await;
    sqlx::raw_sql(sql)
        .execute(&mut conn)
        .await
        .expect("fixture SQL batch should succeed");
}

fn unique_schema() -> String {
    let n = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    format!("s2c_{}_{}", std::process::id(), n)
}

struct RangeCluster {
    source: Target,
    destination: Target,
    super_source: Target,
    schema: String,
    directory: tempfile::TempDir,
}

impl RangeCluster {
    async fn setup() -> Option<Self> {
        let source = target("DBPINBALL_TEST_PG_SOURCE_URL", "source")?;
        let destination = target("DBPINBALL_TEST_PG_DEST_URL", "destination")?;
        let super_source = super_target(&source);
        let schema = unique_schema();
        let directory = tempfile::tempdir().unwrap();

        let src_role = &source.connection.username;
        let create_src = format!(
            r#"
CREATE SCHEMA "{schema}";
CREATE TYPE "{schema}".order_status AS ENUM ('pending', 'paid');
CREATE TABLE "{schema}".customers (
    id uuid PRIMARY KEY,
    email text NOT NULL
);
CREATE TABLE "{schema}".orders (
    id uuid PRIMARY KEY,
    customer_id uuid NOT NULL,
    seq_small smallint NOT NULL,
    seq_int integer NOT NULL,
    seq_big bigint NOT NULL,
    amount numeric,
    order_date date,
    scheduled_at timestamp without time zone,
    created_at timestamptz,
    active boolean NOT NULL,
    note text,
    status "{schema}".order_status NOT NULL
);
INSERT INTO "{schema}".customers (id, email) VALUES
    ('11111111-1111-1111-1111-111111111111', 'alice@example.com'),
    ('22222222-2222-2222-2222-222222222222', 'bob@example.com');
INSERT INTO "{schema}".orders (
    id, customer_id, seq_small, seq_int, seq_big, amount, order_date,
    scheduled_at, created_at, active, note, status
) VALUES
    ('aaaaaaaa-0000-0000-0000-000000000001', '11111111-1111-1111-1111-111111111111',
     10, 100, 1000, 1.10, '2026-01-01', '2026-01-01 09:00:00', '2026-01-01T00:00:00Z',
     true, 'row1', 'pending'),
    ('aaaaaaaa-0000-0000-0000-000000000002', '11111111-1111-1111-1111-111111111111',
     20, 200, 2000, 1.10000000000000001, '2026-01-05', '2026-01-05 12:30:00', '2026-01-05T05:00:00Z',
     true, 'row2', 'paid'),
    ('aaaaaaaa-0000-0000-0000-000000000003', '22222222-2222-2222-2222-222222222222',
     30, 300, 3000, 25.50, '2026-01-10', '2026-01-10 18:00:00', '2026-01-10T12:00:00Z',
     false, 'row3', 'paid'),
    ('aaaaaaaa-0000-0000-0000-000000000004', '22222222-2222-2222-2222-222222222222',
     0, 0, 0, -0.0, NULL, NULL, NULL,
     true, 'row4_nulls', 'pending');
GRANT USAGE ON SCHEMA "{schema}" TO "{src_role}";
GRANT SELECT ON ALL TABLES IN SCHEMA "{schema}" TO "{src_role}";
"#
        );
        exec_batch(&super_source, &create_src).await;

        let create_dst = format!(
            r#"
CREATE SCHEMA "{schema}";
CREATE TYPE "{schema}".order_status AS ENUM ('pending', 'paid');
CREATE TABLE "{schema}".customers (
    id uuid PRIMARY KEY,
    email text NOT NULL
);
CREATE TABLE "{schema}".orders (
    id uuid PRIMARY KEY,
    customer_id uuid NOT NULL,
    seq_small smallint NOT NULL,
    seq_int integer NOT NULL,
    seq_big bigint NOT NULL,
    amount numeric,
    order_date date,
    scheduled_at timestamp without time zone,
    created_at timestamptz,
    active boolean NOT NULL,
    note text,
    status "{schema}".order_status NOT NULL
);
"#
        );
        exec_batch(&destination, &create_dst).await;

        let store = security::store_for(directory.path());
        let provider = self_provider(&source, &destination);
        security::enroll(
            &destination.connection,
            &provider,
            &store,
            DestinationCategory::VerifiedDevelopment,
            now_ms(),
        )
        .await
        .expect("enroll destination");

        Some(Self {
            source,
            destination,
            super_source,
            schema,
            directory,
        })
    }

    fn provider(&self) -> MapProvider {
        self_provider(&self.source, &self.destination)
    }

    fn store(&self) -> dbpinball_core::security::EnrollmentStore {
        security::store_for(self.directory.path())
    }

    fn orders_table(&self) -> String {
        format!("{}.orders", self.schema)
    }
}

async fn run_execute(
    c: &RangeCluster,
    spec: &SafeSyncSpec,
    plan: &DryRunPlan,
    replace: bool,
) -> Result<RunReport, CoreError> {
    service::execute(
        ExecuteRequest {
            plan,
            spec,
            acknowledgement: &plan.plan_fingerprint,
            replace,
            source: &c.source.connection,
            destination: &c.destination.connection,
        },
        &c.provider(),
        &c.store(),
        &key_store(),
        None,
        now_ms(),
        Arc::new(AtomicBool::new(false)),
    )
    .await
}

fn self_provider(source: &Target, destination: &Target) -> MapProvider {
    MapProvider::new(&[
        ("source", &source.password),
        ("destination", &destination.password),
    ])
}

async fn extract_ids(
    cluster: &RangeCluster,
    filter: Vec<FilterPredicate>,
) -> Result<Vec<String>, CoreError> {
    let provider = cluster.provider();
    let mut session = BoundSession::open(
        SessionRole::Source,
        &cluster.source.connection,
        &provider,
        &Limits::default(),
    )
    .await?;
    let info = inspect_table(
        &mut session,
        &QualifiedName::parse(&cluster.orders_table()).unwrap(),
    )
    .await?;
    let mut policy = MaskingPolicy::builtin_default();
    for col in &info.columns {
        let strategy = match col.name.as_str() {
            "id" | "customer_id" => MaskStrategy::Keep,
            "note" => MaskStrategy::Redact,
            _ => MaskStrategy::Null,
        };
        policy.rules.push(ColumnRule {
            schema: info.schema.clone(),
            table: info.table.clone(),
            column: col.name.clone(),
            strategy,
            max_len: None,
            key_domain: None,
            key_position: 0,
        });
    }
    // Note: Masker::coverage is not called by extract_table directly, so
    // extract_table works even on non-nullable columns when testing selection.
    let masker = Masker::new(policy, test_key())?;
    let root = RootSelection {
        table: cluster.orders_table(),
        filter,
    };
    let rows = extract_table(&mut session, &info, &root, &masker, &Limits::default()).await?;
    Ok(rows
        .rows
        .into_iter()
        .map(|r| r[0].as_text().unwrap().to_string())
        .collect())
}

fn pred(column: &str, op: FilterOp, value: FilterValue) -> FilterPredicate {
    FilterPredicate {
        column: column.into(),
        op,
        value,
    }
}

/// Set up `public.ranged_customers` and `public.ranged_orders` covered by
/// `development-default@2` for full end-to-end `service::plan` + `service::execute`.
async fn setup_e2e_tables(cluster: &RangeCluster) -> String {
    let src_role = &cluster.source.connection.username;
    let src_sql = format!(
        r#"
DROP TABLE IF EXISTS public.ranged_orders, public.ranged_customers CASCADE;
CREATE TABLE public.ranged_customers (
    id uuid PRIMARY KEY,
    email text NOT NULL
);
CREATE TABLE public.ranged_orders (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    customer_id uuid NOT NULL REFERENCES public.ranged_customers(id),
    amount numeric,
    order_date date,
    scheduled_at timestamp without time zone,
    created_at timestamptz,
    note text
);
INSERT INTO public.ranged_customers (id, email) VALUES
    ('11111111-1111-1111-1111-111111111111', 'alice@example.com'),
    ('22222222-2222-2222-2222-222222222222', 'bob@example.com');
INSERT INTO public.ranged_orders OVERRIDING SYSTEM VALUE VALUES
    (10, '11111111-1111-1111-1111-111111111111',
     1.10, '2026-01-01', NULL, '2026-01-01T00:00:00Z', 'first'),
    (20, '22222222-2222-2222-2222-222222222222',
     25.50, '2026-01-10', '2026-01-10 18:00:00', '2026-01-10T12:00:00Z', 'second'),
    (30, '22222222-2222-2222-2222-222222222222',
     NULL, NULL, NULL, NULL, 'third');
GRANT SELECT ON public.ranged_customers, public.ranged_orders TO "{src_role}";
"#
    );
    exec_batch(&cluster.super_source, &src_sql).await;

    let dst_sql = r#"
DROP TABLE IF EXISTS public.ranged_orders, public.ranged_customers CASCADE;
DROP SCHEMA IF EXISTS dbpinball_safesync CASCADE;
CREATE TABLE public.ranged_customers (
    id uuid PRIMARY KEY,
    email text NOT NULL
);
CREATE TABLE public.ranged_orders (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    customer_id uuid NOT NULL REFERENCES public.ranged_customers(id),
    amount numeric,
    order_date date,
    scheduled_at timestamp without time zone,
    created_at timestamptz,
    note text
);
"#;
    exec_batch(&cluster.destination, dst_sql).await;
    "public.ranged_orders".to_string()
}

fn e2e_spec(table: &str, filter: Vec<FilterPredicate>, depth: u8) -> SafeSyncSpec {
    let filter_yaml = serde_yaml::to_string(&filter).unwrap();
    let indented: String = filter_yaml
        .lines()
        .map(|line| format!("        {line}\n"))
        .collect();
    let yaml = format!(
        r#"
version: 1
source: source
destination: destination
selection:
  roots:
    - table: {table}
      filter:
{indented}
  relationships:
    depth: {depth}
limits:
  max_rows_per_table: 20
  max_total_rows: 40
  max_depth: 2
  max_bytes: 1048576
  timeout_ms: 10000
masking:
  policy: development-default@2
"#
    );
    SafeSyncSpec::parse_yaml(&yaml).expect("valid e2e spec")
}

// ---------------------------------------------------------------------------
// 30 real PostgreSQL integration tests for M4 S2-c
// ---------------------------------------------------------------------------

#[tokio::test]
async fn t01_gte_returns_inclusive_lower_bound() {
    let Some((_guard, c)) = RangeCluster::locked().await else {
        return;
    };
    let ids = extract_ids(
        &c,
        vec![pred("seq_int", FilterOp::Gte, FilterValue::Int(200))],
    )
    .await
    .unwrap();
    assert_eq!(ids.len(), 2);
}

#[tokio::test]
async fn t02_lte_returns_inclusive_upper_bound() {
    let Some((_guard, c)) = RangeCluster::locked().await else {
        return;
    };
    let ids = extract_ids(
        &c,
        vec![pred("seq_int", FilterOp::Lte, FilterValue::Int(100))],
    )
    .await
    .unwrap();
    assert_eq!(ids.len(), 2); // 0 and 100
}

#[tokio::test]
async fn t03_gte_and_lte_closed_window_includes_both_bounds() {
    let Some((_guard, c)) = RangeCluster::locked().await else {
        return;
    };
    let ids = extract_ids(
        &c,
        vec![
            pred("seq_int", FilterOp::Gte, FilterValue::Int(100)),
            pred("seq_int", FilterOp::Lte, FilterValue::Int(300)),
        ],
    )
    .await
    .unwrap();
    assert_eq!(ids.len(), 3);
}

#[tokio::test]
async fn t04_bound_outside_data_returns_empty_or_full_without_error() {
    let Some((_guard, c)) = RangeCluster::locked().await else {
        return;
    };
    let empty = extract_ids(
        &c,
        vec![pred("seq_int", FilterOp::Gte, FilterValue::Int(9999))],
    )
    .await
    .unwrap();
    assert!(empty.is_empty());
    let full = extract_ids(
        &c,
        vec![pred("seq_int", FilterOp::Gte, FilterValue::Int(-1))],
    )
    .await
    .unwrap();
    assert_eq!(full.len(), 4);
}

#[tokio::test]
async fn t05_integer_overflow_for_column_type_is_refused() {
    let Some((_guard, c)) = RangeCluster::locked().await else {
        return;
    };
    let err = extract_ids(
        &c,
        vec![pred("seq_small", FilterOp::Gte, FilterValue::Int(40_000))],
    )
    .await
    .unwrap_err();
    assert_eq!(err.detail.as_deref(), Some("invalid_filter"));
}

#[tokio::test]
async fn t06_numeric_equivalence_and_high_precision_not_rounded() {
    let Some((_guard, c)) = RangeCluster::locked().await else {
        return;
    };
    let t_ord = setup_e2e_tables(&c).await;
    let p_a = service::plan(
        &e2e_spec(
            &t_ord,
            vec![pred(
                "amount",
                FilterOp::Gte,
                FilterValue::Text("1.10".into()),
            )],
            0,
        ),
        &c.source.connection,
        &c.destination.connection,
        &c.provider(),
        &c.store(),
        &key_store(),
        now_ms(),
        false,
        false,
    )
    .await
    .unwrap();
    let p_b = service::plan(
        &e2e_spec(
            &t_ord,
            vec![pred(
                "amount",
                FilterOp::Gte,
                FilterValue::Text("1.1".into()),
            )],
            0,
        ),
        &c.source.connection,
        &c.destination.connection,
        &c.provider(),
        &c.store(),
        &key_store(),
        now_ms(),
        false,
        false,
    )
    .await
    .unwrap();
    assert_eq!(p_a.spec_hash, p_b.spec_hash);

    let exact = extract_ids(
        &c,
        vec![
            pred(
                "amount",
                FilterOp::Gte,
                FilterValue::Text("1.10000000000000001".into()),
            ),
            pred(
                "amount",
                FilterOp::Lte,
                FilterValue::Text("1.10000000000000001".into()),
            ),
        ],
    )
    .await
    .unwrap();
    assert_eq!(exact, vec!["aaaaaaaa-0000-0000-0000-000000000002"]);
}

#[tokio::test]
async fn t07_numeric_negative_zero_matches_zero() {
    let Some((_guard, c)) = RangeCluster::locked().await else {
        return;
    };
    let ids = extract_ids(
        &c,
        vec![
            pred("amount", FilterOp::Gte, FilterValue::Text("-0.0".into())),
            pred("amount", FilterOp::Lte, FilterValue::Text("0".into())),
        ],
    )
    .await
    .unwrap();
    assert_eq!(ids, vec!["aaaaaaaa-0000-0000-0000-000000000004"]);
}

#[tokio::test]
async fn t08_date_accepts_iso_and_refuses_time_part() {
    let Some((_guard, c)) = RangeCluster::locked().await else {
        return;
    };
    let ids = extract_ids(
        &c,
        vec![pred(
            "order_date",
            FilterOp::Gte,
            FilterValue::Text("2026-01-05".into()),
        )],
    )
    .await
    .unwrap();
    assert_eq!(ids.len(), 2);

    let err = extract_ids(
        &c,
        vec![pred(
            "order_date",
            FilterOp::Gte,
            FilterValue::Text("2026-01-05T00:00:00".into()),
        )],
    )
    .await
    .unwrap_err();
    assert_eq!(err.detail.as_deref(), Some("invalid_filter"));
}

#[tokio::test]
async fn t09_timestamp_without_tz_works_and_refuses_offset() {
    let Some((_guard, c)) = RangeCluster::locked().await else {
        return;
    };
    let ids = extract_ids(
        &c,
        vec![pred(
            "scheduled_at",
            FilterOp::Gte,
            FilterValue::Text("2026-01-05 12:30:00".into()),
        )],
    )
    .await
    .unwrap();
    assert_eq!(ids.len(), 2);

    let err = extract_ids(
        &c,
        vec![pred(
            "scheduled_at",
            FilterOp::Gte,
            FilterValue::Text("2026-01-05 12:30:00Z".into()),
        )],
    )
    .await
    .unwrap_err();
    assert_eq!(err.detail.as_deref(), Some("invalid_filter"));
}

#[tokio::test]
async fn t10_timestamptz_refuses_missing_offset() {
    let Some((_guard, c)) = RangeCluster::locked().await else {
        return;
    };
    let err = extract_ids(
        &c,
        vec![pred(
            "created_at",
            FilterOp::Gte,
            FilterValue::Text("2026-01-01 00:00:00".into()),
        )],
    )
    .await
    .unwrap_err();
    assert_eq!(err.detail.as_deref(), Some("invalid_filter"));
}

#[tokio::test]
async fn t11_timestamptz_refuses_zone_name() {
    let Some((_guard, c)) = RangeCluster::locked().await else {
        return;
    };
    let err = extract_ids(
        &c,
        vec![pred(
            "created_at",
            FilterOp::Gte,
            FilterValue::Text("2026-01-01T00:00:00 Europe/Paris".into()),
        )],
    )
    .await
    .unwrap_err();
    assert_eq!(err.detail.as_deref(), Some("invalid_filter"));
}

#[tokio::test]
async fn t12_timestamptz_normalizes_offset_to_same_digest_and_rows() {
    let Some((_guard, c)) = RangeCluster::locked().await else {
        return;
    };
    let t_ord = setup_e2e_tables(&c).await;
    let p_offset = service::plan(
        &e2e_spec(
            &t_ord,
            vec![pred(
                "created_at",
                FilterOp::Gte,
                FilterValue::Text("2026-01-01T07:00:00+07:00".into()),
            )],
            0,
        ),
        &c.source.connection,
        &c.destination.connection,
        &c.provider(),
        &c.store(),
        &key_store(),
        now_ms(),
        false,
        false,
    )
    .await
    .unwrap();
    let p_utc = service::plan(
        &e2e_spec(
            &t_ord,
            vec![pred(
                "created_at",
                FilterOp::Gte,
                FilterValue::Text("2026-01-01T00:00:00Z".into()),
            )],
            0,
        ),
        &c.source.connection,
        &c.destination.connection,
        &c.provider(),
        &c.store(),
        &key_store(),
        now_ms(),
        false,
        false,
    )
    .await
    .unwrap();
    assert_eq!(p_offset.spec_hash, p_utc.spec_hash);

    let rows_a = extract_ids(
        &c,
        vec![pred(
            "created_at",
            FilterOp::Gte,
            FilterValue::Text("2026-01-01T07:00:00+07:00".into()),
        )],
    )
    .await
    .unwrap();
    let rows_b = extract_ids(
        &c,
        vec![pred(
            "created_at",
            FilterOp::Gte,
            FilterValue::Text("2026-01-01T00:00:00Z".into()),
        )],
    )
    .await
    .unwrap();
    assert_eq!(rows_a, rows_b);
    assert_eq!(rows_a.len(), 3);
}

#[tokio::test]
async fn t13_null_column_never_satisfies_range_and_typed_null_round_trips() {
    let Some((_guard, c)) = RangeCluster::locked().await else {
        return;
    };
    let t_ord = setup_e2e_tables(&c).await;
    let spec = e2e_spec(
        &t_ord,
        vec![pred("amount", FilterOp::Gte, FilterValue::Text("0".into()))],
        1,
    );
    let plan = service::plan(
        &spec,
        &c.source.connection,
        &c.destination.connection,
        &c.provider(),
        &c.store(),
        &key_store(),
        now_ms(),
        false,
        false,
    )
    .await
    .unwrap();
    let report = run_execute(&c, &spec, &plan, false).await.unwrap();
    assert_eq!(report.rows_written, 4); // 2 orders + 2 customers; NULL amount row (id=30) excluded

    let mut dst = connect(&c.destination).await;
    let null_round_tripped: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM public.ranged_orders WHERE id = 10 AND scheduled_at IS NULL",
    )
    .fetch_one(&mut dst)
    .await
    .unwrap();
    assert_eq!(null_round_tripped, 1);
}

#[tokio::test]
async fn t14_range_on_text_boolean_uuid_or_custom_enum_is_refused() {
    let Some((_guard, c)) = RangeCluster::locked().await else {
        return;
    };
    for (col, val) in [
        ("note", FilterValue::Text("a".into())),
        ("active", FilterValue::Bool(true)),
        (
            "id",
            FilterValue::Text("aaaaaaaa-0000-0000-0000-000000000001".into()),
        ),
        ("status", FilterValue::Text("pending".into())),
    ] {
        let err = extract_ids(&c, vec![pred(col, FilterOp::Gte, val)])
            .await
            .unwrap_err();
        assert_eq!(err.detail.as_deref(), Some("invalid_filter"), "col={col}");
    }
}

#[tokio::test]
async fn t15_reordered_and_duplicate_predicates_produce_same_spec_hash_and_fingerprint() {
    let Some((_guard, c)) = RangeCluster::locked().await else {
        return;
    };
    let t_ord = setup_e2e_tables(&c).await;
    let s1 = e2e_spec(
        &t_ord,
        vec![
            pred("amount", FilterOp::Gte, FilterValue::Text("1.10".into())),
            pred(
                "order_date",
                FilterOp::Lte,
                FilterValue::Text("2026-01-10".into()),
            ),
            pred("amount", FilterOp::Gte, FilterValue::Text("1.1".into())),
        ],
        0,
    );
    let s2 = e2e_spec(
        &t_ord,
        vec![
            pred(
                "order_date",
                FilterOp::Lte,
                FilterValue::Text("2026-01-10".into()),
            ),
            pred("amount", FilterOp::Gte, FilterValue::Text("1.1".into())),
        ],
        0,
    );
    let p1 = service::plan(
        &s1,
        &c.source.connection,
        &c.destination.connection,
        &c.provider(),
        &c.store(),
        &key_store(),
        now_ms(),
        false,
        false,
    )
    .await
    .unwrap();
    let p2 = service::plan(
        &s2,
        &c.source.connection,
        &c.destination.connection,
        &c.provider(),
        &c.store(),
        &key_store(),
        now_ms(),
        false,
        false,
    )
    .await
    .unwrap();
    assert_eq!(p1.spec_hash, p2.spec_hash);
}

#[tokio::test]
async fn t16_changed_bound_invalidates_plan_before_connect() {
    let Some((_guard, c)) = RangeCluster::locked().await else {
        return;
    };
    let t_ord = setup_e2e_tables(&c).await;
    let s1 = e2e_spec(
        &t_ord,
        vec![pred("id", FilterOp::Gte, FilterValue::Int(10))],
        0,
    );
    let plan = service::plan(
        &s1,
        &c.source.connection,
        &c.destination.connection,
        &c.provider(),
        &c.store(),
        &key_store(),
        now_ms(),
        false,
        false,
    )
    .await
    .unwrap();
    let s2 = e2e_spec(
        &t_ord,
        vec![pred("id", FilterOp::Gte, FilterValue::Int(20))],
        0,
    );
    let err = service::validate(
        &plan,
        &s2,
        &c.source.connection,
        &c.destination.connection,
        &c.provider(),
        &c.store(),
        &key_store(),
        now_ms(),
    )
    .await
    .unwrap_err();
    assert_eq!(err.detail.as_deref(), Some("spec_plan_mismatch"));
}

#[tokio::test]
async fn t17_changed_operator_invalidates_plan_before_connect() {
    let Some((_guard, c)) = RangeCluster::locked().await else {
        return;
    };
    let t_ord = setup_e2e_tables(&c).await;
    let s1 = e2e_spec(
        &t_ord,
        vec![pred("id", FilterOp::Gte, FilterValue::Int(10))],
        0,
    );
    let plan = service::plan(
        &s1,
        &c.source.connection,
        &c.destination.connection,
        &c.provider(),
        &c.store(),
        &key_store(),
        now_ms(),
        false,
        false,
    )
    .await
    .unwrap();
    let s2 = e2e_spec(
        &t_ord,
        vec![pred("id", FilterOp::Lte, FilterValue::Int(10))],
        0,
    );
    let err = service::validate(
        &plan,
        &s2,
        &c.source.connection,
        &c.destination.connection,
        &c.provider(),
        &c.store(),
        &key_store(),
        now_ms(),
    )
    .await
    .unwrap_err();
    assert_eq!(err.detail.as_deref(), Some("spec_plan_mismatch"));
}

#[tokio::test]
async fn t18_tampered_plan_file_fails_v3_hmac() {
    let Some((_guard, c)) = RangeCluster::locked().await else {
        return;
    };
    let t_ord = setup_e2e_tables(&c).await;
    let spec = e2e_spec(
        &t_ord,
        vec![pred("id", FilterOp::Gte, FilterValue::Int(10))],
        0,
    );
    let mut plan: DryRunPlan = service::plan(
        &spec,
        &c.source.connection,
        &c.destination.connection,
        &c.provider(),
        &c.store(),
        &key_store(),
        now_ms(),
        false,
        false,
    )
    .await
    .unwrap();
    plan.selection.roots[0].filter_digest = "hmac-sha256:tampered".into();
    plan.plan_fingerprint = plan.fingerprint();
    let err = service::validate(
        &plan,
        &spec,
        &c.source.connection,
        &c.destination.connection,
        &c.provider(),
        &c.store(),
        &key_store(),
        now_ms(),
    )
    .await
    .unwrap_err();
    assert_eq!(err.detail.as_deref(), Some("plan_tampered"));
    assert!(err.message.contains("plan authorization is invalid"));
}

#[tokio::test]
async fn t19_canary_literal_never_leaks_into_plan_or_errors() {
    let Some((_guard, c)) = RangeCluster::locked().await else {
        return;
    };
    let t_ord = setup_e2e_tables(&c).await;
    let canary = "2026-01-01T01:23:45.654321Z";
    let spec = e2e_spec(
        &t_ord,
        vec![pred(
            "created_at",
            FilterOp::Gte,
            FilterValue::Text(canary.into()),
        )],
        0,
    );
    let plan = service::plan(
        &spec,
        &c.source.connection,
        &c.destination.connection,
        &c.provider(),
        &c.store(),
        &key_store(),
        now_ms(),
        false,
        false,
    )
    .await
    .unwrap();
    let plan_json = serde_json::to_string(&plan).unwrap();
    assert!(!plan_json.contains(canary));
    assert!(!plan_json.contains("01:23:45"));
}

#[tokio::test]
async fn t20_predicate_cap_eight_allowed_nine_refused() {
    let Some((_guard, c)) = RangeCluster::locked().await else {
        return;
    };
    let t_ord = setup_e2e_tables(&c).await;
    let eight: Vec<FilterPredicate> = (0..8)
        .map(|i| pred("id", FilterOp::Gte, FilterValue::Int(i)))
        .collect();
    let ok = service::plan(
        &e2e_spec(&t_ord, eight, 0),
        &c.source.connection,
        &c.destination.connection,
        &c.provider(),
        &c.store(),
        &key_store(),
        now_ms(),
        false,
        false,
    )
    .await;
    assert!(ok.is_ok());

    let nine: Vec<FilterPredicate> = (0..9)
        .map(|i| pred("id", FilterOp::Gte, FilterValue::Int(i)))
        .collect();
    let yaml = serde_yaml::to_string(&e2e_spec(
        &t_ord,
        vec![pred("id", FilterOp::Gte, FilterValue::Int(0))],
        0,
    ))
    .unwrap();
    let mut parsed: serde_yaml::Value = serde_yaml::from_str(&yaml).unwrap();
    parsed["selection"]["roots"][0]["filter"] = serde_yaml::to_value(&nine).unwrap();
    let err = SafeSyncSpec::parse_yaml(&serde_yaml::to_string(&parsed).unwrap()).unwrap_err();
    assert_eq!(err.detail.as_deref(), Some("limit_exceeded"));
}

#[tokio::test]
async fn t21_row_limit_applies_to_ranged_result() {
    let Some((_guard, c)) = RangeCluster::locked().await else {
        return;
    };
    let t_ord = setup_e2e_tables(&c).await;
    let mut spec = e2e_spec(
        &t_ord,
        vec![pred("id", FilterOp::Gte, FilterValue::Int(0))],
        0,
    );
    spec.limits.max_rows_per_table = 1;
    let err = service::plan(
        &spec,
        &c.source.connection,
        &c.destination.connection,
        &c.provider(),
        &c.store(),
        &key_store(),
        now_ms(),
        true,
        false,
    )
    .await
    .unwrap_err();
    assert_eq!(err.detail.as_deref(), Some("limit_exceeded"));
}

#[tokio::test]
async fn t22_source_session_remains_read_only_during_ranged_extraction() {
    let Some((_guard, c)) = RangeCluster::locked().await else {
        return;
    };
    let provider = c.provider();
    let mut session = BoundSession::open(
        SessionRole::Source,
        &c.source.connection,
        &provider,
        &Limits::default(),
    )
    .await
    .unwrap();
    session.begin_read_only().await.unwrap();
    let ro: String = sqlx::query_scalar("SHOW transaction_read_only")
        .fetch_one(&mut *session.connection())
        .await
        .unwrap();
    let _ = session.rollback().await;
    assert_eq!(ro, "on");
}

#[tokio::test]
async fn t23_and_across_multiple_columns_returns_intersection() {
    let Some((_guard, c)) = RangeCluster::locked().await else {
        return;
    };
    let ids = extract_ids(
        &c,
        vec![
            pred("seq_int", FilterOp::Gte, FilterValue::Int(100)),
            pred(
                "order_date",
                FilterOp::Lte,
                FilterValue::Text("2026-01-05".into()),
            ),
        ],
    )
    .await
    .unwrap();
    assert_eq!(ids.len(), 2);
}

#[tokio::test]
async fn t24_value_free_plan_records_filter_types_and_contains_values_false() {
    let Some((_guard, c)) = RangeCluster::locked().await else {
        return;
    };
    let t_ord = setup_e2e_tables(&c).await;
    let spec = e2e_spec(
        &t_ord,
        vec![pred(
            "amount",
            FilterOp::Gte,
            FilterValue::Text("1.10".into()),
        )],
        0,
    );
    let plan = service::plan(
        &spec,
        &c.source.connection,
        &c.destination.connection,
        &c.provider(),
        &c.store(),
        &key_store(),
        now_ms(),
        false,
        false,
    )
    .await
    .unwrap();
    assert!(!plan.selection.roots[0].contains_values);
    assert_eq!(plan.selection.roots[0].filter_types, vec!["numeric"]);
}

#[tokio::test]
async fn t25_bounded_fk_traversal_pulls_only_parents_for_ranged_children() {
    let Some((_guard, c)) = RangeCluster::locked().await else {
        return;
    };
    let t_ord = setup_e2e_tables(&c).await;
    let spec = e2e_spec(
        &t_ord,
        vec![pred("id", FilterOp::Gte, FilterValue::Int(20))],
        1,
    );
    let plan = service::plan(
        &spec,
        &c.source.connection,
        &c.destination.connection,
        &c.provider(),
        &c.store(),
        &key_store(),
        now_ms(),
        true,
        false,
    )
    .await
    .unwrap();
    let cust = plan
        .tables
        .iter()
        .find(|t| t.table == "ranged_customers")
        .unwrap();
    assert_eq!(cust.rows_estimated, Some(1));
}

#[tokio::test]
async fn t26_protected_destination_is_refused() {
    let Some((_guard, c)) = RangeCluster::locked().await else {
        return;
    };
    let t_ord = setup_e2e_tables(&c).await;
    let mut prod = c.destination.connection.clone();
    prod.target = TargetClass::Production;
    let spec = e2e_spec(
        &t_ord,
        vec![pred("id", FilterOp::Gte, FilterValue::Int(10))],
        0,
    );
    let err = service::plan(
        &spec,
        &c.source.connection,
        &prod,
        &c.provider(),
        &c.store(),
        &key_store(),
        now_ms(),
        false,
        false,
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::PolicyRefusal);
}

#[tokio::test]
async fn t27_masked_contents_verified_on_destination() {
    let Some((_guard, c)) = RangeCluster::locked().await else {
        return;
    };
    let t_ord = setup_e2e_tables(&c).await;
    let spec = e2e_spec(
        &t_ord,
        vec![pred("id", FilterOp::Gte, FilterValue::Int(20))],
        1,
    );
    let plan = service::plan(
        &spec,
        &c.source.connection,
        &c.destination.connection,
        &c.provider(),
        &c.store(),
        &key_store(),
        now_ms(),
        false,
        false,
    )
    .await
    .unwrap();
    run_execute(&c, &spec, &plan, false).await.unwrap();

    let mut dst = connect(&c.destination).await;
    let emails: Vec<String> = sqlx::query_scalar("SELECT email FROM public.ranged_customers")
        .fetch_all(&mut dst)
        .await
        .unwrap();
    assert_eq!(emails.len(), 1);
    assert!(emails[0].ends_with("@example.invalid"));
    assert_ne!(emails[0], "bob@example.com");
}

#[tokio::test]
async fn t28_idempotent_repeat_with_replace_conflict_policy() {
    let Some((_guard, c)) = RangeCluster::locked().await else {
        return;
    };
    let t_ord = setup_e2e_tables(&c).await;
    let spec = e2e_spec(
        &t_ord,
        vec![
            pred("id", FilterOp::Gte, FilterValue::Int(20)),
            pred("id", FilterOp::Lte, FilterValue::Int(20)),
        ],
        1,
    );
    let plan = service::plan(
        &spec,
        &c.source.connection,
        &c.destination.connection,
        &c.provider(),
        &c.store(),
        &key_store(),
        now_ms(),
        false,
        true,
    )
    .await
    .unwrap();
    for _ in 0..2 {
        run_execute(&c, &spec, &plan, true).await.unwrap();
    }
    let mut dst = connect(&c.destination).await;
    let count: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM {t_ord}"))
        .fetch_one(&mut dst)
        .await
        .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn t29_schema_drift_refuses_execution() {
    let Some((_guard, c)) = RangeCluster::locked().await else {
        return;
    };
    let t_ord = setup_e2e_tables(&c).await;
    let spec = e2e_spec(
        &t_ord,
        vec![pred("id", FilterOp::Gte, FilterValue::Int(10))],
        1,
    );
    let plan = service::plan(
        &spec,
        &c.source.connection,
        &c.destination.connection,
        &c.provider(),
        &c.store(),
        &key_store(),
        now_ms(),
        false,
        false,
    )
    .await
    .unwrap();
    exec_batch(
        &c.super_source,
        &format!("ALTER TABLE {t_ord} ALTER COLUMN note SET NOT NULL"),
    )
    .await;
    let err = run_execute(&c, &spec, &plan, false).await.unwrap_err();
    assert_eq!(err.detail.as_deref(), Some("schema_drift"));
}

#[tokio::test]
async fn t30_destination_transaction_rolls_back_on_conflict() {
    let Some((_guard, c)) = RangeCluster::locked().await else {
        return;
    };
    let t_ord = setup_e2e_tables(&c).await;
    let spec = e2e_spec(
        &t_ord,
        vec![pred("id", FilterOp::Gte, FilterValue::Int(10))],
        1,
    );
    let plan = service::plan(
        &spec,
        &c.source.connection,
        &c.destination.connection,
        &c.provider(),
        &c.store(),
        &key_store(),
        now_ms(),
        false,
        false,
    )
    .await
    .unwrap();
    // Create a unique index on destination customer_id so the second order for the same customer
    // fails mid-transaction after ranged_customers and the first order row were inserted.
    exec_batch(
        &c.destination,
        "CREATE UNIQUE INDEX ranged_orders_one_per_cust ON public.ranged_orders (customer_id)",
    )
    .await;
    let err = run_execute(&c, &spec, &plan, false).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Operation);

    let mut dst = connect(&c.destination).await;
    let cust_count: i64 = sqlx::query_scalar("SELECT count(*) FROM public.ranged_customers")
        .fetch_one(&mut dst)
        .await
        .unwrap();
    let ord_count: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM {t_ord}"))
        .fetch_one(&mut dst)
        .await
        .unwrap();
    assert_eq!(cust_count, 0);
    assert_eq!(ord_count, 0);
}

#[tokio::test]
async fn t31_changed_root_table_is_refused_before_connect() {
    let Some((_guard, c)) = RangeCluster::locked().await else {
        return;
    };
    let t_ord = setup_e2e_tables(&c).await;
    let spec = e2e_spec(
        &t_ord,
        vec![pred("id", FilterOp::Gte, FilterValue::Int(10))],
        1,
    );
    let plan = service::plan(
        &spec,
        &c.source.connection,
        &c.destination.connection,
        &c.provider(),
        &c.store(),
        &key_store(),
        now_ms(),
        false,
        false,
    )
    .await
    .unwrap();

    let mut tampered = spec.clone();
    tampered.selection.roots[0].table = "public.ranged_customers".to_string();

    let validate_err = service::validate(
        &plan,
        &tampered,
        &c.source.connection,
        &c.destination.connection,
        &c.provider(),
        &c.store(),
        &key_store(),
        now_ms(),
    )
    .await
    .unwrap_err();
    assert_eq!(validate_err.detail.as_deref(), Some("spec_plan_mismatch"));

    let execute_err = run_execute(&c, &tampered, &plan, false).await.unwrap_err();
    assert_eq!(execute_err.detail.as_deref(), Some("spec_plan_mismatch"));
}

#[tokio::test]
async fn t32_validate_refuses_nine_predicates_with_duplicate() {
    let Some((_guard, c)) = RangeCluster::locked().await else {
        return;
    };
    let t_ord = setup_e2e_tables(&c).await;
    let spec = e2e_spec(
        &t_ord,
        vec![pred("id", FilterOp::Gte, FilterValue::Int(10))],
        1,
    );
    let plan = service::plan(
        &spec,
        &c.source.connection,
        &c.destination.connection,
        &c.provider(),
        &c.store(),
        &key_store(),
        now_ms(),
        false,
        false,
    )
    .await
    .unwrap();

    let mut bloated = spec.clone();
    bloated.selection.roots[0].filter = (0..8)
        .map(|i| pred("id", FilterOp::Gte, FilterValue::Int(i)))
        .collect();
    bloated.selection.roots[0]
        .filter
        .push(pred("id", FilterOp::Gte, FilterValue::Int(0)));

    let err = service::validate(
        &plan,
        &bloated,
        &c.source.connection,
        &c.destination.connection,
        &c.provider(),
        &c.store(),
        &key_store(),
        now_ms(),
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::Usage);
    assert_eq!(err.detail.as_deref(), Some("limit_exceeded"));
}
