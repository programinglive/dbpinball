#![cfg(feature = "integration")]

//! Real PostgreSQL acceptance for the `safe-sync` CLI.
//!
//! Requires `DBPINBALL_TEST_PG_SOURCE_URL` (a restricted SELECT-only role),
//! `DBPINBALL_TEST_PG_DEST_URL` (a writable destination) and optionally
//! `DBPINBALL_TEST_PG_SUPER_URL` on the source cluster.
//!
//! Tests are serialized because they share the destination database.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::Value;

use dbpinball_core::domain::connection::{DbConnection, DbEngine, TargetClass};

const KEY_HEX: &str = "0707070707070707070707070707070707070707070707070707070707070707";

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_dbpinball")
}

fn serial() -> &'static tokio::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

#[derive(Clone)]
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

/// Create an isolated config directory with the given profiles imported.
fn config_with(directory: &Path, profiles: &[&Target]) -> PathBuf {
    let entries: Vec<Value> = profiles
        .iter()
        .map(|target| {
            serde_json::json!({
                "id": target.connection.id,
                "name": target.connection.name,
                "engine": "postgres",
                "host": target.connection.host,
                "port": target.connection.port,
                "database": target.connection.database,
                "username": target.connection.username,
                "useSsl": false
            })
        })
        .collect();

    let file = directory.join("profiles.json");
    std::fs::write(&file, serde_json::to_string(&entries).unwrap()).unwrap();

    let output = Command::new(binary())
        .args(["--config-dir", &directory.to_string_lossy()])
        .args(["connections", "import"])
        .arg(&file)
        .stdin(Stdio::null())
        .output()
        .expect("import");
    assert!(
        output.status.success(),
        "profile import failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    directory.to_path_buf()
}

fn cli(config: &Path, args: &[&str], source: &Target, destination: &Target) -> Output {
    Command::new(binary())
        .args(["--config-dir", &config.to_string_lossy()])
        .args(["--credential-source", "env", "--mask-key-source", "env"])
        .arg("--json")
        .args(args)
        .env("DBPINBALL_PASSWORD_SOURCE", &source.password)
        .env("DBPINBALL_PASSWORD_DESTINATION", &destination.password)
        .env("DBPINBALL_PASSWORD_SUPER", &source.password)
        .env("DBPINBALL_PASSWORD_MYSQL", "x")
        .env("DBPINBALL_SAFE_SYNC_KEY", KEY_HEX)
        .stdin(Stdio::null())
        .output()
        .expect("cli")
}

fn envelope(output: &Output) -> Value {
    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(stdout.trim()).unwrap_or_else(|_| {
        panic!(
            "expected one JSON envelope, got: {stdout}\nstderr: {}",
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn code(output: &Output) -> i32 {
    output.status.code().unwrap_or(-1)
}

fn spec_file(directory: &Path, id: &str) -> PathBuf {
    let path = directory.join("spec.yaml");
    let body = format!(
        r#"version: 1
source: source
destination: destination
selection:
  roots:
    - table: public.accounts
      filter:
        - column: id
          op: eq
          value: {id}
limits:
  max_rows_per_table: 10
  max_total_rows: 10
  max_depth: 0
  max_bytes: 1048576
  timeout_ms: 10000
masking:
  policy: development-default@1
"#
    );
    std::fs::write(&path, body).unwrap();
    path
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

async fn reset_destination(destination: &Target) {
    let mut connection = connect(destination).await;
    sqlx::query("DELETE FROM public.accounts")
        .execute(&mut connection)
        .await
        .expect("truncate the destination");
    let _ = sqlx::query("DELETE FROM public.dbpinball_safe_sync_runs")
        .execute(&mut connection)
        .await;
}

async fn destination_rows(destination: &Target) -> Vec<(String, String, String)> {
    let mut connection = connect(destination).await;
    sqlx::query_as("SELECT id::text, email, full_name FROM public.accounts ORDER BY id")
        .fetch_all(&mut connection)
        .await
        .expect("read the destination")
}

fn enroll(config: &Path, source: &Target, destination: &Target) {
    let output = cli(
        config,
        &[
            "security",
            "enroll",
            "destination",
            "--category",
            "development",
        ],
        source,
        destination,
    );
    assert_eq!(
        code(&output),
        0,
        "enrollment failed: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}

fn plan(config: &Path, source: &Target, destination: &Target, id: &str) -> (Value, PathBuf) {
    let spec = spec_file(config, id);
    let plan_path = config.join("plan.json");
    let output = cli(
        config,
        &[
            "safe-sync",
            "plan",
            "--spec",
            &spec.to_string_lossy(),
            "--out",
            &plan_path.to_string_lossy(),
        ],
        source,
        destination,
    );
    assert_eq!(
        code(&output),
        0,
        "plan failed: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    (envelope(&output), plan_path)
}

// ---------------------------------------------------------------------------
// Positive path
// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_end_to_end_masked_import_succeeds() {
    let _guard = serial().lock().await;
    let (Some(source), Some(destination)) = (
        target("DBPINBALL_TEST_PG_SOURCE_URL", "source"),
        target("DBPINBALL_TEST_PG_DEST_URL", "destination"),
    ) else {
        eprintln!("skipped: the test database URLs are not set");
        return;
    };

    let directory = tempfile::tempdir().unwrap();
    let config = config_with(directory.path(), &[&source, &destination]);
    reset_destination(&destination).await;
    enroll(&config, &source, &destination);

    let (_plan_envelope, plan_path) = plan(
        &config,
        &source,
        &destination,
        "0b9f4c3e-0000-0000-0000-000000000001",
    );
    let plan_json: Value =
        serde_json::from_str(&std::fs::read_to_string(&plan_path).unwrap()).unwrap();
    let fingerprint = plan_json["plan_fingerprint"].as_str().unwrap().to_string();

    // The plan must not contain the source email or the filter value.
    let rendered = std::fs::read_to_string(&plan_path).unwrap();
    assert!(!rendered.contains("alice@example.com"));
    assert!(!rendered.contains("0b9f4c3e-0000-0000-0000-000000000001"));

    let spec = config.join("spec.yaml");
    let output = cli(
        &config,
        &[
            "safe-sync",
            "execute",
            "--plan",
            &plan_path.to_string_lossy(),
            "--spec",
            &spec.to_string_lossy(),
            "--ack",
            &fingerprint,
            "--yes",
        ],
        &source,
        &destination,
    );
    assert_eq!(
        code(&output),
        0,
        "execute failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let body = envelope(&output);
    assert_eq!(body["data"]["status"], "completed");
    assert_eq!(body["data"]["rowsWritten"], 1);

    let rows = destination_rows(&destination).await;
    assert_eq!(rows.len(), 1);
    let (id, email, full_name) = &rows[0];
    assert_eq!(id, "0b9f4c3e-0000-0000-0000-000000000001");
    assert!(email.ends_with("@example.invalid"), "email: {email}");
    assert_ne!(email, "alice@example.com");
    assert!(full_name.starts_with("Anon "), "name: {full_name}");
    assert_ne!(full_name, "Alice Example");

    // A repeat is a no-op, not a duplicate.
    let repeat = cli(
        &config,
        &[
            "safe-sync",
            "execute",
            "--plan",
            &plan_path.to_string_lossy(),
            "--spec",
            &spec.to_string_lossy(),
            "--ack",
            &fingerprint,
            "--yes",
        ],
        &source,
        &destination,
    );
    assert_eq!(code(&repeat), 0);
    assert_eq!(envelope(&repeat)["data"]["alreadyCompleted"], true);
    assert_eq!(destination_rows(&destination).await.len(), 1);

    // Status reads the run back from the destination via the local journal.
    let run_id = envelope(&repeat)["data"]["runId"]
        .as_str()
        .unwrap()
        .to_string();
    let status = cli(
        &config,
        &["safe-sync", "status", "--run", &run_id],
        &source,
        &destination,
    );
    assert_eq!(
        code(&status),
        0,
        "{}",
        String::from_utf8_lossy(&status.stdout)
    );
    assert_eq!(envelope(&status)["data"]["status"], "completed");
}

#[tokio::test]
async fn a_changed_destination_is_destination_drifted() {
    let _guard = serial().lock().await;
    let (Some(source), Some(destination)) = (
        target("DBPINBALL_TEST_PG_SOURCE_URL", "source"),
        target("DBPINBALL_TEST_PG_DEST_URL", "destination"),
    ) else {
        eprintln!("skipped: the test database URLs are not set");
        return;
    };

    let directory = tempfile::tempdir().unwrap();
    let config = config_with(directory.path(), &[&source, &destination]);
    reset_destination(&destination).await;
    enroll(&config, &source, &destination);

    let (_envelope, plan_path) = plan(
        &config,
        &source,
        &destination,
        "0b9f4c3e-0000-0000-0000-000000000001",
    );
    let plan_json: Value =
        serde_json::from_str(&std::fs::read_to_string(&plan_path).unwrap()).unwrap();
    let fingerprint = plan_json["plan_fingerprint"].as_str().unwrap().to_string();
    let spec = config.join("spec.yaml");

    let first = cli(
        &config,
        &[
            "safe-sync",
            "execute",
            "--plan",
            &plan_path.to_string_lossy(),
            "--spec",
            &spec.to_string_lossy(),
            "--ack",
            &fingerprint,
            "--yes",
        ],
        &source,
        &destination,
    );
    assert_eq!(code(&first), 0);

    // Mutate the destination behind the run's back.
    let mut connection = connect(&destination).await;
    sqlx::query("UPDATE public.accounts SET email = 'tampered@example.invalid'")
        .execute(&mut connection)
        .await
        .unwrap();

    let repeat = cli(
        &config,
        &[
            "safe-sync",
            "execute",
            "--plan",
            &plan_path.to_string_lossy(),
            "--spec",
            &spec.to_string_lossy(),
            "--ack",
            &fingerprint,
            "--yes",
        ],
        &source,
        &destination,
    );
    assert_eq!(code(&repeat), 8);
    assert_eq!(envelope(&repeat)["error"]["code"], "conflict");
}

// ---------------------------------------------------------------------------
// Refusals
// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_unenrolled_destination_exits_four() {
    let _guard = serial().lock().await;
    let (Some(source), Some(destination)) = (
        target("DBPINBALL_TEST_PG_SOURCE_URL", "source"),
        target("DBPINBALL_TEST_PG_DEST_URL", "destination"),
    ) else {
        eprintln!("skipped: the test database URLs are not set");
        return;
    };

    let directory = tempfile::tempdir().unwrap();
    let config = config_with(directory.path(), &[&source, &destination]);
    reset_destination(&destination).await;

    let spec = spec_file(&config, "0b9f4c3e-0000-0000-0000-000000000001");
    let output = cli(
        &config,
        &["safe-sync", "plan", "--spec", &spec.to_string_lossy()],
        &source,
        &destination,
    );
    assert_eq!(code(&output), 4);
    assert_eq!(envelope(&output)["error"]["code"], "policy_refusal");
}

#[tokio::test]
async fn execute_without_ack_or_yes_is_refused() {
    let _guard = serial().lock().await;
    let (Some(source), Some(destination)) = (
        target("DBPINBALL_TEST_PG_SOURCE_URL", "source"),
        target("DBPINBALL_TEST_PG_DEST_URL", "destination"),
    ) else {
        eprintln!("skipped: the test database URLs are not set");
        return;
    };

    let directory = tempfile::tempdir().unwrap();
    let config = config_with(directory.path(), &[&source, &destination]);
    reset_destination(&destination).await;
    enroll(&config, &source, &destination);
    let (_envelope, plan_path) = plan(
        &config,
        &source,
        &destination,
        "0b9f4c3e-0000-0000-0000-000000000001",
    );
    let spec = config.join("spec.yaml");

    // Missing --ack is a usage error from the parser.
    let missing_ack = cli(
        &config,
        &[
            "safe-sync",
            "execute",
            "--plan",
            &plan_path.to_string_lossy(),
            "--spec",
            &spec.to_string_lossy(),
            "--yes",
        ],
        &source,
        &destination,
    );
    assert_eq!(code(&missing_ack), 2);

    // A wrong acknowledgement is refused.
    let wrong = cli(
        &config,
        &[
            "safe-sync",
            "execute",
            "--plan",
            &plan_path.to_string_lossy(),
            "--spec",
            &spec.to_string_lossy(),
            "--ack",
            "deadbeef",
            "--yes",
        ],
        &source,
        &destination,
    );
    assert_eq!(code(&wrong), 2);
    assert_eq!(
        envelope(&wrong)["error"]["detail"],
        "acknowledgement_mismatch"
    );
}

#[tokio::test]
async fn an_expired_plan_exits_eight() {
    let _guard = serial().lock().await;
    let (Some(source), Some(destination)) = (
        target("DBPINBALL_TEST_PG_SOURCE_URL", "source"),
        target("DBPINBALL_TEST_PG_DEST_URL", "destination"),
    ) else {
        eprintln!("skipped: the test database URLs are not set");
        return;
    };

    let directory = tempfile::tempdir().unwrap();
    let config = config_with(directory.path(), &[&source, &destination]);
    reset_destination(&destination).await;
    enroll(&config, &source, &destination);
    let (_envelope, plan_path) = plan(
        &config,
        &source,
        &destination,
        "0b9f4c3e-0000-0000-0000-000000000001",
    );

    let mut plan_json: Value =
        serde_json::from_str(&std::fs::read_to_string(&plan_path).unwrap()).unwrap();
    let fingerprint = plan_json["plan_fingerprint"].as_str().unwrap().to_string();
    plan_json["expires_at_ms"] = Value::from(1u64);
    std::fs::write(&plan_path, serde_json::to_string(&plan_json).unwrap()).unwrap();

    let spec = config.join("spec.yaml");
    let output = cli(
        &config,
        &[
            "safe-sync",
            "execute",
            "--plan",
            &plan_path.to_string_lossy(),
            "--spec",
            &spec.to_string_lossy(),
            "--ack",
            &fingerprint,
            "--yes",
        ],
        &source,
        &destination,
    );
    assert_eq!(code(&output), 8);
    assert_eq!(envelope(&output)["error"]["detail"], "plan_expired");
}

#[tokio::test]
async fn a_non_empty_destination_is_refused() {
    let _guard = serial().lock().await;
    let (Some(source), Some(destination)) = (
        target("DBPINBALL_TEST_PG_SOURCE_URL", "source"),
        target("DBPINBALL_TEST_PG_DEST_URL", "destination"),
    ) else {
        eprintln!("skipped: the test database URLs are not set");
        return;
    };

    let directory = tempfile::tempdir().unwrap();
    let config = config_with(directory.path(), &[&source, &destination]);
    reset_destination(&destination).await;
    enroll(&config, &source, &destination);

    let mut connection = connect(&destination).await;
    sqlx::query(
        "INSERT INTO public.accounts VALUES ('00000000-0000-0000-0000-000000000009', 'x@example.invalid', 'Anon x')",
    )
    .execute(&mut connection)
    .await
    .unwrap();

    let (_envelope, plan_path) = plan(
        &config,
        &source,
        &destination,
        "0b9f4c3e-0000-0000-0000-000000000001",
    );
    let plan_json: Value =
        serde_json::from_str(&std::fs::read_to_string(&plan_path).unwrap()).unwrap();
    let fingerprint = plan_json["plan_fingerprint"].as_str().unwrap().to_string();
    let spec = config.join("spec.yaml");

    let output = cli(
        &config,
        &[
            "safe-sync",
            "execute",
            "--plan",
            &plan_path.to_string_lossy(),
            "--spec",
            &spec.to_string_lossy(),
            "--ack",
            &fingerprint,
            "--yes",
        ],
        &source,
        &destination,
    );
    assert_eq!(code(&output), 8);
    assert_eq!(
        envelope(&output)["error"]["detail"],
        "destination_not_empty"
    );
}

#[tokio::test]
async fn schema_drift_on_the_destination_is_refused() {
    let _guard = serial().lock().await;
    let (Some(source), Some(destination)) = (
        target("DBPINBALL_TEST_PG_SOURCE_URL", "source"),
        target("DBPINBALL_TEST_PG_DEST_URL", "destination"),
    ) else {
        eprintln!("skipped: the test database URLs are not set");
        return;
    };

    let directory = tempfile::tempdir().unwrap();
    let config = config_with(directory.path(), &[&source, &destination]);
    reset_destination(&destination).await;
    enroll(&config, &source, &destination);
    let (_envelope, plan_path) = plan(
        &config,
        &source,
        &destination,
        "0b9f4c3e-0000-0000-0000-000000000001",
    );
    let plan_json: Value =
        serde_json::from_str(&std::fs::read_to_string(&plan_path).unwrap()).unwrap();
    let fingerprint = plan_json["plan_fingerprint"].as_str().unwrap().to_string();
    let spec = config.join("spec.yaml");

    let mut connection = connect(&destination).await;
    sqlx::query("ALTER TABLE public.accounts ADD COLUMN note text")
        .execute(&mut connection)
        .await
        .unwrap();

    let output = cli(
        &config,
        &[
            "safe-sync",
            "execute",
            "--plan",
            &plan_path.to_string_lossy(),
            "--spec",
            &spec.to_string_lossy(),
            "--ack",
            &fingerprint,
            "--yes",
        ],
        &source,
        &destination,
    );
    assert_eq!(code(&output), 8);
    assert_eq!(envelope(&output)["error"]["detail"], "schema_drift");

    sqlx::query("ALTER TABLE public.accounts DROP COLUMN note")
        .execute(&mut connection)
        .await
        .unwrap();
}

#[tokio::test]
async fn a_privileged_source_is_refused() {
    let _guard = serial().lock().await;
    let (Some(_source), Some(destination)) = (
        target("DBPINBALL_TEST_PG_SOURCE_URL", "source"),
        target("DBPINBALL_TEST_PG_DEST_URL", "destination"),
    ) else {
        eprintln!("skipped: the test database URLs are not set");
        return;
    };
    let Some(super_target) = target("DBPINBALL_TEST_PG_SUPER_URL", "super") else {
        eprintln!("skipped: DBPINBALL_TEST_PG_SUPER_URL is not set");
        return;
    };

    let directory = tempfile::tempdir().unwrap();
    let config = config_with(directory.path(), &[&super_target, &destination]);
    reset_destination(&destination).await;
    enroll(&config, &super_target, &destination);

    let spec = spec_file(&config, "0b9f4c3e-0000-0000-0000-000000000001");
    // The spec names the restricted source, so point it at the super profile by
    // rewriting the spec's source id.
    let body = std::fs::read_to_string(&spec)
        .unwrap()
        .replace("source: source", "source: super");
    std::fs::write(&spec, body).unwrap();

    let output = cli(
        &config,
        &["safe-sync", "plan", "--spec", &spec.to_string_lossy()],
        &super_target,
        &destination,
    );
    assert_eq!(code(&output), 5);
    assert_eq!(envelope(&output)["error"]["detail"], "source_privileged");
}

#[tokio::test]
async fn a_missing_mask_key_is_refused() {
    let _guard = serial().lock().await;
    let (Some(source), Some(destination)) = (
        target("DBPINBALL_TEST_PG_SOURCE_URL", "source"),
        target("DBPINBALL_TEST_PG_DEST_URL", "destination"),
    ) else {
        eprintln!("skipped: the test database URLs are not set");
        return;
    };

    let directory = tempfile::tempdir().unwrap();
    let config = config_with(directory.path(), &[&source, &destination]);
    reset_destination(&destination).await;
    enroll(&config, &source, &destination);

    let spec = spec_file(&config, "0b9f4c3e-0000-0000-0000-000000000001");
    let output = Command::new(binary())
        .args(["--config-dir", &config.to_string_lossy()])
        .args([
            "--credential-source",
            "env",
            "--mask-key-source",
            "env",
            "--json",
        ])
        .args(["safe-sync", "plan", "--spec", &spec.to_string_lossy()])
        .env("DBPINBALL_PASSWORD_SOURCE", &source.password)
        .env("DBPINBALL_PASSWORD_DESTINATION", &destination.password)
        .env_remove("DBPINBALL_SAFE_SYNC_KEY")
        .stdin(Stdio::null())
        .output()
        .expect("cli");

    assert_eq!(code(&output), 5);
    assert_eq!(envelope(&output)["error"]["detail"], "mask_key_missing");
}

#[tokio::test]
async fn a_non_postgres_source_is_refused() {
    let _guard = serial().lock().await;
    let (Some(source), Some(destination)) = (
        target("DBPINBALL_TEST_PG_SOURCE_URL", "source"),
        target("DBPINBALL_TEST_PG_DEST_URL", "destination"),
    ) else {
        eprintln!("skipped: the test database URLs are not set");
        return;
    };

    let directory = tempfile::tempdir().unwrap();
    let config = config_with(directory.path(), &[&source, &destination]);
    // A MySQL profile that is never connected to: the engine check fires first.
    let mysql = serde_json::json!([{
        "id": "mysql", "name": "mysql", "engine": "mysql", "host": "127.0.0.1",
        "port": 3306, "database": "app", "username": "u", "useSsl": false
    }]);
    let file = directory.path().join("mysql.json");
    std::fs::write(&file, serde_json::to_string(&mysql).unwrap()).unwrap();
    let imported = Command::new(binary())
        .args(["--config-dir", &config.to_string_lossy()])
        .args(["connections", "import"])
        .arg(&file)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(imported.status.success());

    let output = cli(
        &config,
        &["safe-sync", "inspect", "--source", "mysql"],
        &source,
        &destination,
    );
    assert_eq!(code(&output), 5);
    assert_eq!(envelope(&output)["error"]["detail"], "unsupported_engine");
}

#[tokio::test]
async fn cli_outputs_contain_no_secrets_or_source_values() {
    let _guard = serial().lock().await;
    let (Some(source), Some(destination)) = (
        target("DBPINBALL_TEST_PG_SOURCE_URL", "source"),
        target("DBPINBALL_TEST_PG_DEST_URL", "destination"),
    ) else {
        eprintln!("skipped: the test database URLs are not set");
        return;
    };

    let directory = tempfile::tempdir().unwrap();
    let config = config_with(directory.path(), &[&source, &destination]);
    reset_destination(&destination).await;
    enroll(&config, &source, &destination);
    let (_envelope, plan_path) = plan(
        &config,
        &source,
        &destination,
        "0b9f4c3e-0000-0000-0000-000000000001",
    );
    let plan_json: Value =
        serde_json::from_str(&std::fs::read_to_string(&plan_path).unwrap()).unwrap();
    let fingerprint = plan_json["plan_fingerprint"].as_str().unwrap().to_string();
    let spec = config.join("spec.yaml");

    let output = cli(
        &config,
        &[
            "safe-sync",
            "execute",
            "--plan",
            &plan_path.to_string_lossy(),
            "--spec",
            &spec.to_string_lossy(),
            "--ack",
            &fingerprint,
            "--yes",
        ],
        &source,
        &destination,
    );
    assert_eq!(code(&output), 0);

    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!combined.contains("alice@example.com"));
    assert!(!combined.contains("Alice Example"));
    assert!(!combined.contains(&source.password));
    assert!(!combined.contains(KEY_HEX));
    assert!(!combined.contains("0b9f4c3e-0000-0000-0000-000000000001"));
}
