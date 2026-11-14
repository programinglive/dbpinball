#![cfg(feature = "integration")]

//! Real PostgreSQL tests for M4 Slice 1 sessions, inspection and planning.
//!
//! Two isolated clusters are required:
//!
//! - `DBPINBALL_TEST_PG_SOURCE_URL` — a restricted, SELECT-only source role
//! - `DBPINBALL_TEST_PG_DEST_URL` — a writable destination role
//! - `DBPINBALL_TEST_PG_SUPER_URL` — optional superuser on the source cluster,
//!   used only to set up refusal fixtures
//!
//! A skipped run is not acceptance.

use std::collections::BTreeMap;

use dbpinball_core::domain::connection::{DbConnection, DbEngine, TargetClass};
use dbpinball_core::domain::error::{CoreError, ErrorCode};
use dbpinball_core::ports::credentials::{CredentialProvider, Secret};
use dbpinball_core::safesync::{
    graph::inspect_table, service, BoundSession, InMemoryMaskKeyStore, Limits, MaskKey, Masker,
    QualifiedName, SafeSyncSpec, SessionRole,
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

    let connection = DbConnection {
        id: id.to_string(),
        name: id.to_string(),
        engine: DbEngine::Postgres,
        host: parsed.host_str()?.to_string(),
        port: parsed.port().unwrap_or(5432),
        database: parsed.path().trim_start_matches('/').to_string(),
        username: parsed.username().to_string(),
        use_ssl: false,
        target: TargetClass::Unknown,
    };

    Some(Target {
        connection,
        password: parsed.password().unwrap_or_default().to_string(),
    })
}

fn super_target() -> Option<Target> {
    let mut target = target("DBPINBALL_TEST_PG_SUPER_URL", "super")?;
    target.connection.database =
        std::env::var("DBPINBALL_TEST_PG_SUPER_DATABASE").unwrap_or_else(|_| "srcdb".to_string());
    Some(target)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn test_key() -> MaskKey {
    // A fixed synthetic key. It is never a production key.
    MaskKey::from_bytes("k-test", [7u8; 32])
}

fn key_store() -> InMemoryMaskKeyStore {
    InMemoryMaskKeyStore::with_key(test_key())
}

fn accounts_spec() -> SafeSyncSpec {
    SafeSyncSpec::parse_yaml(
        r#"
version: 1
source: source
destination: destination
selection:
  roots:
    - table: public.accounts
      filter:
        - column: id
          op: eq
          value: 0b9f4c3e-0000-0000-0000-000000000001
limits:
  max_rows_per_table: 10
  max_total_rows: 10
  max_depth: 0
  max_bytes: 1048576
  timeout_ms: 10000
masking:
  policy: development-default@1
"#,
    )
    .expect("the test spec is valid")
}

async fn exec_as(connection: &DbConnection, password: &str, sql: &str) {
    use sqlx::postgres::{PgConnectOptions, PgConnection};
    use sqlx::Connection;

    let options = PgConnectOptions::new()
        .host(&connection.host)
        .port(connection.port)
        .username(&connection.username)
        .password(password)
        .database(&connection.database);
    let mut session = PgConnection::connect_with(&options)
        .await
        .expect("the fixture connection should open");
    sqlx::query(sql)
        .execute(&mut session)
        .await
        .expect("the fixture statement should succeed");
}

// ---------------------------------------------------------------------------
// Sessions
// ---------------------------------------------------------------------------

#[tokio::test]
async fn source_session_reports_a_bound_identity() {
    let Some(target) = target("DBPINBALL_TEST_PG_SOURCE_URL", "source") else {
        eprintln!("skipped: DBPINBALL_TEST_PG_SOURCE_URL is not set");
        return;
    };
    let provider = MapProvider::new(&[("source", &target.password)]);

    let mut session = BoundSession::open(
        SessionRole::Source,
        &target.connection,
        &provider,
        &Limits::default(),
    )
    .await
    .expect("the source session should open");

    let identity = session.identity();
    assert!(identity.identity_key.is_some(), "identity must be usable");
    assert_eq!(identity.current_user, target.connection.username);
    assert!(identity.transaction_read_only);
    assert!(!identity.database_oid.is_empty());

    session.rollback().await.unwrap();
}

#[tokio::test]
async fn the_source_session_is_database_enforced_read_only() {
    let Some(target) = target("DBPINBALL_TEST_PG_SOURCE_URL", "source") else {
        eprintln!("skipped: DBPINBALL_TEST_PG_SOURCE_URL is not set");
        return;
    };
    let provider = MapProvider::new(&[("source", &target.password)]);

    let mut session = BoundSession::open(
        SessionRole::Source,
        &target.connection,
        &provider,
        &Limits::default(),
    )
    .await
    .expect("open");

    session.begin_read_only().await.expect("begin read only");

    let attempt = sqlx::query(
        "INSERT INTO public.accounts (id, email, full_name) \
         VALUES ('00000000-0000-0000-0000-00000000dead', 'x@example.com', 'X')",
    )
    .execute(session.connection())
    .await;

    assert!(
        attempt.is_err(),
        "a write on the source session must fail at the engine"
    );

    let _ = session.rollback().await;
}

#[tokio::test]
async fn a_restricted_source_role_is_accepted() {
    let Some(target) = target("DBPINBALL_TEST_PG_SOURCE_URL", "source") else {
        eprintln!("skipped: DBPINBALL_TEST_PG_SOURCE_URL is not set");
        return;
    };
    let provider = MapProvider::new(&[("source", &target.password)]);

    let mut session = BoundSession::open(
        SessionRole::Source,
        &target.connection,
        &provider,
        &Limits::default(),
    )
    .await
    .expect("open");

    session
        .assert_source_role(&[QualifiedName::parse("public.accounts").unwrap()])
        .await
        .expect("a SELECT-only role is acceptable");
}

#[tokio::test]
async fn a_superuser_source_is_refused() {
    let Some(target) = super_target() else {
        eprintln!("skipped: DBPINBALL_TEST_PG_SUPER_URL is not set");
        return;
    };
    let provider = MapProvider::new(&[("super", &target.password)]);

    let mut session = BoundSession::open(
        SessionRole::Source,
        &target.connection,
        &provider,
        &Limits::default(),
    )
    .await
    .expect("open");

    let error = session
        .assert_source_role(&[QualifiedName::parse("public.accounts").unwrap()])
        .await
        .unwrap_err();
    assert_eq!(error.detail.as_deref(), Some("source_privileged"));
}

#[tokio::test]
async fn a_source_role_with_a_write_privilege_is_refused() {
    let Some(super_target) = super_target() else {
        eprintln!("skipped: DBPINBALL_TEST_PG_SUPER_URL is not set");
        return;
    };

    exec_as(
        &super_target.connection,
        &super_target.password,
        "DO $$ BEGIN
           IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'm4src_writer') THEN
             CREATE ROLE m4src_writer LOGIN PASSWORD 'm4srcw';
           END IF;
         END $$",
    )
    .await;
    for statement in [
        "GRANT CONNECT ON DATABASE srcdb TO m4src_writer",
        "GRANT USAGE ON SCHEMA public TO m4src_writer",
        "GRANT SELECT, UPDATE ON public.accounts TO m4src_writer",
    ] {
        exec_as(&super_target.connection, &super_target.password, statement).await;
    }

    let mut writer = super_target.connection.clone();
    writer.id = "writer".to_string();
    writer.username = "m4src_writer".to_string();
    let provider = MapProvider::new(&[("writer", "m4srcw")]);

    let mut session =
        BoundSession::open(SessionRole::Source, &writer, &provider, &Limits::default())
            .await
            .expect("open");

    let error = session
        .assert_source_role(&[QualifiedName::parse("public.accounts").unwrap()])
        .await
        .unwrap_err();
    assert_eq!(error.detail.as_deref(), Some("source_privileged"));
}

#[tokio::test]
async fn a_non_postgres_engine_is_refused() {
    use dbpinball_core::safesync::session::SessionRole as Role;

    let connection = DbConnection {
        id: "mysql".into(),
        name: "mysql".into(),
        engine: DbEngine::Mysql,
        host: "127.0.0.1".into(),
        port: 3306,
        database: "app".into(),
        username: "u".into(),
        use_ssl: false,
        target: TargetClass::Unknown,
    };
    let provider = MapProvider::new(&[("mysql", "x")]);

    let error = BoundSession::open(Role::Source, &connection, &provider, &Limits::default())
        .await
        .unwrap_err();
    assert_eq!(error.detail.as_deref(), Some("unsupported_engine"));
}

#[tokio::test]
async fn a_bad_password_is_redacted() {
    let Some(target) = target("DBPINBALL_TEST_PG_SOURCE_URL", "source") else {
        eprintln!("skipped: DBPINBALL_TEST_PG_SOURCE_URL is not set");
        return;
    };
    let provider = MapProvider::new(&[("source", "wrong-password-value")]);

    let error = BoundSession::open(
        SessionRole::Source,
        &target.connection,
        &provider,
        &Limits::default(),
    )
    .await
    .unwrap_err();

    assert_eq!(error.code, ErrorCode::Connection);
    assert!(!error.message.contains("wrong-password-value"));
}

// ---------------------------------------------------------------------------
// Inspection
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_view_is_refused() {
    let Some(super_target) = super_target() else {
        eprintln!("skipped: DBPINBALL_TEST_PG_SUPER_URL is not set");
        return;
    };
    exec_as(
        &super_target.connection,
        &super_target.password,
        "CREATE OR REPLACE VIEW public.accounts_view AS SELECT * FROM public.accounts",
    )
    .await;

    let provider = MapProvider::new(&[("super", &super_target.password)]);
    let mut session = BoundSession::open(
        SessionRole::Source,
        &super_target.connection,
        &provider,
        &Limits::default(),
    )
    .await
    .expect("open");

    let error = inspect_table(
        &mut session,
        &QualifiedName::parse("public.accounts_view").unwrap(),
    )
    .await
    .unwrap_err();
    assert_eq!(error.detail.as_deref(), Some("unsupported_relation"));
}

#[tokio::test]
async fn inspection_reports_the_three_approved_columns() {
    let Some(target) = target("DBPINBALL_TEST_PG_SOURCE_URL", "source") else {
        eprintln!("skipped: DBPINBALL_TEST_PG_SOURCE_URL is not set");
        return;
    };
    let provider = MapProvider::new(&[("source", &target.password)]);
    let mut session = BoundSession::open(
        SessionRole::Source,
        &target.connection,
        &provider,
        &Limits::default(),
    )
    .await
    .expect("open");

    let info = inspect_table(
        &mut session,
        &QualifiedName::parse("public.accounts").unwrap(),
    )
    .await
    .expect("inspect");

    assert_eq!(
        info.columns
            .iter()
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>(),
        vec!["id", "email", "full_name"]
    );
    assert_eq!(info.primary_key, vec!["id".to_string()]);
    assert_eq!(info.row_identity, vec!["id".to_string()]);
    assert!(!info.schema_hash.is_empty());
}

#[tokio::test]
async fn the_schema_hash_changes_when_a_column_is_added() {
    let Some(super_target) = super_target() else {
        eprintln!("skipped: DBPINBALL_TEST_PG_SUPER_URL is not set");
        return;
    };

    exec_as(
        &super_target.connection,
        &super_target.password,
        "DROP TABLE IF EXISTS public.drift_probe",
    )
    .await;
    exec_as(
        &super_target.connection,
        &super_target.password,
        "CREATE TABLE public.drift_probe (id uuid PRIMARY KEY, email text)",
    )
    .await;

    let provider = MapProvider::new(&[("super", &super_target.password)]);
    let mut session = BoundSession::open(
        SessionRole::Source,
        &super_target.connection,
        &provider,
        &Limits::default(),
    )
    .await
    .expect("open");

    let before = inspect_table(
        &mut session,
        &QualifiedName::parse("public.drift_probe").unwrap(),
    )
    .await
    .expect("inspect before");

    exec_as(
        &super_target.connection,
        &super_target.password,
        "ALTER TABLE public.drift_probe ADD COLUMN IF NOT EXISTS note text",
    )
    .await;

    let after = inspect_table(
        &mut session,
        &QualifiedName::parse("public.drift_probe").unwrap(),
    )
    .await
    .expect("inspect after");

    assert_ne!(before.schema_hash, after.schema_hash);
}

// ---------------------------------------------------------------------------
// Planning
// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_unenrolled_destination_is_refused() {
    let (Some(source), Some(destination)) = (
        target("DBPINBALL_TEST_PG_SOURCE_URL", "source"),
        target("DBPINBALL_TEST_PG_DEST_URL", "destination"),
    ) else {
        eprintln!("skipped: the test database URLs are not set");
        return;
    };

    let directory = tempfile::tempdir().unwrap();
    let store = security::store_for(directory.path());
    let provider = MapProvider::new(&[
        ("source", &source.password),
        ("destination", &destination.password),
    ]);

    let error = service::plan(
        &accounts_spec(),
        &source.connection,
        &destination.connection,
        &provider,
        &store,
        &key_store(),
        now_ms(),
        false,
    )
    .await
    .unwrap_err();

    assert_eq!(error.code, ErrorCode::PolicyRefusal);
    assert_eq!(error.detail.as_deref(), Some("not_enrolled"));
}

#[tokio::test]
async fn an_enrolled_destination_produces_a_value_free_plan() {
    let (Some(source), Some(destination)) = (
        target("DBPINBALL_TEST_PG_SOURCE_URL", "source"),
        target("DBPINBALL_TEST_PG_DEST_URL", "destination"),
    ) else {
        eprintln!("skipped: the test database URLs are not set");
        return;
    };

    let directory = tempfile::tempdir().unwrap();
    let store = security::store_for(directory.path());
    let provider = MapProvider::new(&[
        ("source", &source.password),
        ("destination", &destination.password),
    ]);

    security::enroll(
        &destination.connection,
        &provider,
        &store,
        DestinationCategory::VerifiedDevelopment,
        now_ms(),
    )
    .await
    .expect("enroll the destination");

    let plan = service::plan(
        &accounts_spec(),
        &source.connection,
        &destination.connection,
        &provider,
        &store,
        &key_store(),
        now_ms(),
        false,
    )
    .await
    .expect("plan");

    assert!(plan.complete);
    assert_eq!(plan.masking.policy, "development-default@1");
    assert_eq!(plan.tables.len(), 1);
    assert_eq!(
        plan.selection.roots[0]
            .filter_digest
            .starts_with("hmac-sha256:"),
        true
    );
    assert!(
        plan.tables[0].rows_estimated.is_none(),
        "metadata-only plan"
    );

    let rendered = serde_json::to_string(&plan).unwrap();
    assert!(!rendered.contains("0b9f4c3e-0000-0000-0000-000000000001"));
    assert!(!rendered.contains("alice@example.com"));
    assert!(!rendered.contains("Alice Example"));
}

#[tokio::test]
async fn the_plan_fingerprint_is_reproducible() {
    let (Some(source), Some(destination)) = (
        target("DBPINBALL_TEST_PG_SOURCE_URL", "source"),
        target("DBPINBALL_TEST_PG_DEST_URL", "destination"),
    ) else {
        eprintln!("skipped: the test database URLs are not set");
        return;
    };

    let directory = tempfile::tempdir().unwrap();
    let store = security::store_for(directory.path());
    let provider = MapProvider::new(&[
        ("source", &source.password),
        ("destination", &destination.password),
    ]);

    security::enroll(
        &destination.connection,
        &provider,
        &store,
        DestinationCategory::VerifiedDevelopment,
        now_ms(),
    )
    .await
    .expect("enroll");

    let first = service::plan(
        &accounts_spec(),
        &source.connection,
        &destination.connection,
        &provider,
        &store,
        &key_store(),
        now_ms(),
        false,
    )
    .await
    .expect("plan");

    let second = service::plan(
        &accounts_spec(),
        &source.connection,
        &destination.connection,
        &provider,
        &store,
        &key_store(),
        now_ms(),
        false,
    )
    .await
    .expect("plan");

    assert_eq!(first.plan_fingerprint, second.plan_fingerprint);
}

#[tokio::test]
async fn the_selection_bound_is_enforced() {
    let (Some(source), Some(destination)) = (
        target("DBPINBALL_TEST_PG_SOURCE_URL", "source"),
        target("DBPINBALL_TEST_PG_DEST_URL", "destination"),
    ) else {
        eprintln!("skipped: the test database URLs are not set");
        return;
    };

    let directory = tempfile::tempdir().unwrap();
    let store = security::store_for(directory.path());
    let provider = MapProvider::new(&[
        ("source", &source.password),
        ("destination", &destination.password),
    ]);

    security::enroll(
        &destination.connection,
        &provider,
        &store,
        DestinationCategory::VerifiedDevelopment,
        now_ms(),
    )
    .await
    .expect("enroll");

    let mut spec = accounts_spec();
    spec.limits.max_rows_per_table = 1;
    spec.limits.max_total_rows = 1;
    // The `in` filter selects two rows, which exceeds the limit of one.
    spec.selection.roots[0].filter[0].op = dbpinball_core::safesync::FilterOp::In;
    spec.selection.roots[0].filter[0].value = dbpinball_core::safesync::FilterValue::List(vec![
        dbpinball_core::safesync::FilterValue::Text("0b9f4c3e-0000-0000-0000-000000000001".into()),
        dbpinball_core::safesync::FilterValue::Text("0b9f4c3e-0000-0000-0000-000000000002".into()),
    ]);

    let error = service::plan(
        &spec,
        &source.connection,
        &destination.connection,
        &provider,
        &store,
        &key_store(),
        now_ms(),
        true,
    )
    .await
    .unwrap_err();

    assert_eq!(error.detail.as_deref(), Some("limit_exceeded"));
}

#[tokio::test]
async fn an_estimate_counts_rows_without_returning_values() {
    let (Some(source), Some(destination)) = (
        target("DBPINBALL_TEST_PG_SOURCE_URL", "source"),
        target("DBPINBALL_TEST_PG_DEST_URL", "destination"),
    ) else {
        eprintln!("skipped: the test database URLs are not set");
        return;
    };

    let directory = tempfile::tempdir().unwrap();
    let store = security::store_for(directory.path());
    let provider = MapProvider::new(&[
        ("source", &source.password),
        ("destination", &destination.password),
    ]);

    security::enroll(
        &destination.connection,
        &provider,
        &store,
        DestinationCategory::VerifiedDevelopment,
        now_ms(),
    )
    .await
    .expect("enroll");

    let plan = service::plan(
        &accounts_spec(),
        &source.connection,
        &destination.connection,
        &provider,
        &store,
        &key_store(),
        now_ms(),
        true,
    )
    .await
    .expect("plan");

    assert_eq!(plan.tables[0].rows_estimated, Some(1));
    let rendered = serde_json::to_string(&plan).unwrap();
    assert!(!rendered.contains("alice@example.com"));
}

#[tokio::test]
async fn the_same_database_as_source_and_destination_is_refused() {
    let Some(source) = target("DBPINBALL_TEST_PG_SOURCE_URL", "source") else {
        eprintln!("skipped: DBPINBALL_TEST_PG_SOURCE_URL is not set");
        return;
    };

    let directory = tempfile::tempdir().unwrap();
    let store = security::store_for(directory.path());
    let provider = MapProvider::new(&[
        ("source", &source.password),
        ("destination-as-source", &source.password),
    ]);

    // Enroll the source cluster as the destination, then use it as both.
    security::enroll(
        &source.connection,
        &provider,
        &store,
        DestinationCategory::VerifiedDevelopment,
        now_ms(),
    )
    .await
    .expect("enroll");

    let mut spec = accounts_spec();
    spec.destination = "destination-as-source".to_string();

    // A second connection id that resolves to the same database.
    let mut alias = source.connection.clone();
    alias.id = "destination-as-source".to_string();

    let error = service::plan(
        &spec,
        &source.connection,
        &alias,
        &provider,
        &store,
        &key_store(),
        now_ms(),
        false,
    )
    .await
    .unwrap_err();

    assert_eq!(error.detail.as_deref(), Some("same_source_and_destination"));
}

#[tokio::test]
async fn a_tampered_spec_is_refused_at_validate() {
    let (Some(source), Some(destination)) = (
        target("DBPINBALL_TEST_PG_SOURCE_URL", "source"),
        target("DBPINBALL_TEST_PG_DEST_URL", "destination"),
    ) else {
        eprintln!("skipped: the test database URLs are not set");
        return;
    };

    let directory = tempfile::tempdir().unwrap();
    let store = security::store_for(directory.path());
    let provider = MapProvider::new(&[
        ("source", &source.password),
        ("destination", &destination.password),
    ]);

    security::enroll(
        &destination.connection,
        &provider,
        &store,
        DestinationCategory::VerifiedDevelopment,
        now_ms(),
    )
    .await
    .expect("enroll");

    let plan = service::plan(
        &accounts_spec(),
        &source.connection,
        &destination.connection,
        &provider,
        &store,
        &key_store(),
        now_ms(),
        false,
    )
    .await
    .expect("plan");

    let mut tampered = accounts_spec();
    tampered.selection.roots[0].filter[0].value =
        dbpinball_core::safesync::FilterValue::Text("0b9f4c3e-0000-0000-0000-000000000002".into());

    let error = service::validate(
        &plan,
        &tampered,
        &source.connection,
        &destination.connection,
        &provider,
        &store,
        &key_store(),
        now_ms(),
    )
    .await
    .unwrap_err();

    assert_eq!(error.detail.as_deref(), Some("spec_plan_mismatch"));
}

#[tokio::test]
async fn masking_is_deterministic_across_planning_runs() {
    let Some(target) = target("DBPINBALL_TEST_PG_SOURCE_URL", "source") else {
        eprintln!("skipped: DBPINBALL_TEST_PG_SOURCE_URL is not set");
        return;
    };
    let provider = MapProvider::new(&[("source", &target.password)]);
    let mut session = BoundSession::open(
        SessionRole::Source,
        &target.connection,
        &provider,
        &Limits::default(),
    )
    .await
    .expect("open");

    let masker = Masker::from_builtin(test_key()).unwrap();
    let info = inspect_table(
        &mut session,
        &QualifiedName::parse("public.accounts").unwrap(),
    )
    .await
    .expect("inspect");

    let root = dbpinball_core::safesync::RootSelection {
        table: "public.accounts".into(),
        filter: vec![dbpinball_core::safesync::FilterPredicate {
            column: "id".into(),
            op: dbpinball_core::safesync::FilterOp::Eq,
            value: dbpinball_core::safesync::FilterValue::Text(
                "0b9f4c3e-0000-0000-0000-000000000001".into(),
            ),
        }],
    };

    let first = dbpinball_core::safesync::extract_table(
        &mut session,
        &info,
        &root,
        &masker,
        &Limits::default(),
    )
    .await
    .expect("extract");

    let second = dbpinball_core::safesync::extract_table(
        &mut session,
        &info,
        &root,
        &masker,
        &Limits::default(),
    )
    .await
    .expect("extract");

    assert_eq!(first.rows, second.rows);
    assert_eq!(first.row_count(), 1);
    // The destination will never see the source email.
    let rendered = format!("{:?}", first.rows);
    assert!(!rendered.contains("alice@example.com"));
    assert!(rendered.contains("@example.invalid"));
}
