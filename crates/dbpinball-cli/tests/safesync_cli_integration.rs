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

fn cli_command(config: &Path, args: &[&str], source: &Target, destination: &Target) -> Command {
    let mut command = Command::new(binary());
    command
        .args(["--config-dir", &config.to_string_lossy()])
        .args(["--credential-source", "env", "--mask-key-source", "env"])
        .arg("--json")
        .args(args)
        .env("DBPINBALL_PASSWORD_SOURCE", &source.password)
        .env("DBPINBALL_PASSWORD_DESTINATION", &destination.password)
        .env("DBPINBALL_PASSWORD_SUPER", &source.password)
        .env("DBPINBALL_PASSWORD_MYSQL", "x")
        .env("DBPINBALL_SAFE_SYNC_KEY", KEY_HEX);
    command
}

fn cli(config: &Path, args: &[&str], source: &Target, destination: &Target) -> Output {
    cli_command(config, args, source, destination)
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

/// Re-seal a plan with the test mask key, so a test can change a field and keep
/// a valid authorization envelope.
fn seal_plan(plan: &mut dbpinball_core::safesync::DryRunPlan) {
    use dbpinball_core::safesync::{resolve_policy, MaskKey, Masker};
    let key = MaskKey::from_hex(KEY_HEX).unwrap();
    let policy = resolve_policy(&plan.masking.policy).unwrap();
    let masker = Masker::new(policy, key).unwrap();
    plan.seal(&masker);
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
    // A run with no identity column needs no sequence movement.
    assert_eq!(body["data"]["sequenceStatus"], "not_required");

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

    // A legitimately expired plan still has a valid authorization envelope for
    // its content, so the expiry check, not the integrity check, must fire.
    let mut plan: dbpinball_core::safesync::DryRunPlan =
        serde_json::from_str(&std::fs::read_to_string(&plan_path).unwrap()).unwrap();
    plan.expires_at_ms = plan.created_at_ms + 1;
    plan.plan_fingerprint = plan.fingerprint();
    seal_plan(&mut plan);
    let fingerprint = plan.plan_fingerprint.clone();
    std::fs::write(&plan_path, serde_json::to_string(&plan).unwrap()).unwrap();

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
async fn an_extended_expiry_is_refused() {
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
    // Extend the lifetime well beyond the maximum window.
    let created = plan_json["created_at_ms"].as_u64().unwrap_or(0);
    plan_json["expires_at_ms"] = Value::from(created + 3_600_001);
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
    assert_eq!(envelope(&output)["error"]["detail"], "plan_expiry_invalid");
    assert_eq!(destination_rows(&destination).await.len(), 0);
}

#[tokio::test]
async fn a_shifted_issuance_is_refused() {
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

    // Shift both timestamps forward and recompute the public fingerprint. The
    // authorization envelope still covers the original issuance, so this is
    // refused even though the fingerprint is self-consistent.
    let mut plan: dbpinball_core::safesync::DryRunPlan =
        serde_json::from_str(&std::fs::read_to_string(&plan_path).unwrap()).unwrap();
    plan.created_at_ms += 5_000;
    plan.expires_at_ms += 5_000;
    plan.plan_fingerprint = plan.fingerprint();
    let fingerprint = plan.plan_fingerprint.clone();
    std::fs::write(&plan_path, serde_json::to_string(&plan).unwrap()).unwrap();

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
    assert_eq!(envelope(&output)["error"]["detail"], "plan_tampered");
    assert_eq!(destination_rows(&destination).await.len(), 0);
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
async fn a_failed_insert_rolls_back_the_destination() {
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

    // A destination trigger rejects the masked row during the insert, after the
    // transaction has begun. A trigger is not part of the schema hash, so this
    // reaches the insert rather than failing the drift check.
    let mut connection = connect(&destination).await;
    let _ = sqlx::query("DROP TRIGGER IF EXISTS m4_reject_account ON public.accounts")
        .execute(&mut connection)
        .await;
    sqlx::query(
        "CREATE OR REPLACE FUNCTION public.m4_reject_account_row() RETURNS trigger
         LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'rejected'; END $$",
    )
    .execute(&mut connection)
    .await
    .unwrap();
    sqlx::query(
        "CREATE TRIGGER m4_reject_account BEFORE INSERT ON public.accounts
         FOR EACH ROW EXECUTE FUNCTION public.m4_reject_account_row()",
    )
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
    assert_eq!(code(&output), 5);
    assert_eq!(
        envelope(&output)["error"]["detail"],
        "destination_import_failed"
    );

    // The failed transaction left nothing behind.
    assert_eq!(destination_rows(&destination).await.len(), 0);

    sqlx::query("DROP TRIGGER m4_reject_account ON public.accounts")
        .execute(&mut connection)
        .await
        .unwrap();
    sqlx::query("DROP FUNCTION public.m4_reject_account_row()")
        .execute(&mut connection)
        .await
        .unwrap();
}

#[tokio::test]
async fn the_source_is_never_mutated() {
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

    let mut source_connection = connect(&source).await;
    let before: (i64,) = sqlx::query_as("SELECT count(*) FROM public.accounts")
        .fetch_one(&mut source_connection)
        .await
        .unwrap();
    let before_rows: Vec<(String, String, String)> =
        sqlx::query_as("SELECT id::text, email, full_name FROM public.accounts ORDER BY id")
            .fetch_all(&mut source_connection)
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
    assert_eq!(code(&output), 0);

    let after: (i64,) = sqlx::query_as("SELECT count(*) FROM public.accounts")
        .fetch_one(&mut source_connection)
        .await
        .unwrap();
    let after_rows: Vec<(String, String, String)> =
        sqlx::query_as("SELECT id::text, email, full_name FROM public.accounts ORDER BY id")
            .fetch_all(&mut source_connection)
            .await
            .unwrap();

    assert_eq!(before, after);
    assert_eq!(before_rows, after_rows);
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

    // No plaintext artifact anywhere under the config directory, including the
    // audit journal and the plan file.
    let runs = config.join("safe-sync").join("runs");
    for entry in std::fs::read_dir(&runs).unwrap() {
        let path = entry.unwrap().path();
        let body = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(!body.contains("alice@example.com"), "artifact: {path:?}");
        assert!(!body.contains("Alice Example"), "artifact: {path:?}");
        assert!(!body.contains(KEY_HEX), "artifact: {path:?}");
    }
    let plan_body = std::fs::read_to_string(&plan_path).unwrap();
    assert!(!plan_body.contains("alice@example.com"));
    assert!(!plan_body.contains("0b9f4c3e-0000-0000-0000-000000000001"));
}

#[tokio::test]
async fn a_rotated_key_invalidates_the_plan() {
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

    // A different synthetic key must not be able to run a plan produced under
    // the old key: the pseudonyms would change.
    let rotated = "0808080808080808080808080808080808080808080808080808080808080808";
    let output = cli_command(
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
    )
    .env("DBPINBALL_SAFE_SYNC_KEY", rotated)
    .stdin(Stdio::null())
    .output()
    .expect("cli");

    assert_eq!(code(&output), 8);
    assert_eq!(envelope(&output)["error"]["detail"], "mask_key_changed");
    assert_eq!(destination_rows(&destination).await.len(), 0);
}

#[tokio::test]
async fn concurrent_executions_do_not_duplicate() {
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

    let args = [
        "safe-sync",
        "execute",
        "--plan",
        &plan_path.to_string_lossy(),
        "--spec",
        &spec.to_string_lossy(),
        "--ack",
        &fingerprint,
        "--yes",
    ];

    // Two independent processes, same plan, same destination.
    let mut first = cli_command(&config, &args, &source, &destination);
    let mut second = cli_command(&config, &args, &source, &destination);
    let child_a = first
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    let child_b = second
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");

    let a = child_a.wait_with_output().unwrap();
    let b = child_b.wait_with_output().unwrap();

    assert_eq!(
        code(&a),
        0,
        "first run failed: {}",
        String::from_utf8_lossy(&a.stdout)
    );
    assert_eq!(
        code(&b),
        0,
        "second run failed: {}",
        String::from_utf8_lossy(&b.stdout)
    );
    // Exactly one row: the second run is a no-op, not a duplicate.
    assert_eq!(destination_rows(&destination).await.len(), 1);
}

#[tokio::test]
async fn replace_does_not_touch_unrelated_tables() {
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
    sqlx::query("CREATE TABLE IF NOT EXISTS public.unrelated (id int PRIMARY KEY, note text)")
        .execute(&mut connection)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO public.unrelated VALUES (1, 'keep me') ON CONFLICT (id) DO UPDATE SET note = 'keep me'",
    )
    .execute(&mut connection)
    .await
    .unwrap();

    // A destructive plan must be produced with `--replace`; a plan reviewed as
    // non-destructive refuses the flag at execute time.
    let spec = spec_file(&config, "0b9f4c3e-0000-0000-0000-000000000001");
    let plan_path = config.join("plan.json");
    let planned = cli(
        &config,
        &[
            "safe-sync",
            "plan",
            "--spec",
            &spec.to_string_lossy(),
            "--replace",
            "--out",
            &plan_path.to_string_lossy(),
        ],
        &source,
        &destination,
    );
    assert_eq!(code(&planned), 0);
    let plan_json: Value =
        serde_json::from_str(&std::fs::read_to_string(&plan_path).unwrap()).unwrap();
    let fingerprint = plan_json["plan_fingerprint"].as_str().unwrap().to_string();
    assert_eq!(plan_json["import"]["conflict_policy"], "replace");

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
            "--replace",
            "--yes",
        ],
        &source,
        &destination,
    );
    assert_eq!(
        code(&output),
        0,
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(destination_rows(&destination).await.len(), 1);

    let unrelated: (i64,) = sqlx::query_as("SELECT count(*) FROM public.unrelated")
        .fetch_one(&mut connection)
        .await
        .unwrap();
    let note: (String,) = sqlx::query_as("SELECT note FROM public.unrelated WHERE id = 1")
        .fetch_one(&mut connection)
        .await
        .unwrap();
    assert_eq!(unrelated.0, 1);
    assert_eq!(note.0, "keep me");
}

#[tokio::test]
async fn the_environment_key_source_cannot_rotate() {
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

    let output = cli(&config, &["safe-sync", "key-rotate"], &source, &destination);
    assert_eq!(code(&output), 5);
    assert_eq!(envelope(&output)["error"]["detail"], "mask_key_read_only");
}

#[tokio::test]
async fn a_tampered_spec_is_refused_at_execute() {
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

    // Replace the reviewed filter with a different row. The plan fingerprint is
    // unchanged, so only the spec-to-plan binding can catch this.
    let spec = spec_file(&config, "0b9f4c3e-0000-0000-0000-000000000002");

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
    assert_eq!(code(&output), 2);
    assert_eq!(envelope(&output)["error"]["detail"], "spec_plan_mismatch");
    assert_eq!(destination_rows(&destination).await.len(), 0);
}

#[tokio::test]
async fn a_tampered_plan_is_refused() {
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

    // Edit a field without recomputing the fingerprint.
    plan_json["tables"][0]["schema_hash"] = Value::from("tampered");
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
    assert_eq!(envelope(&output)["error"]["detail"], "plan_tampered");
    assert_eq!(destination_rows(&destination).await.len(), 0);
}

#[tokio::test]
async fn replace_requires_a_replace_plan() {
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

    // The plan records `refuse`; `--replace` must be refused.
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
            "--replace",
            "--yes",
        ],
        &source,
        &destination,
    );
    assert_eq!(code(&output), 2);
    assert_eq!(
        envelope(&output)["error"]["detail"],
        "conflict_policy_mismatch"
    );
}

fn orders_spec(directory: &Path, id: &str) -> PathBuf {
    let path = directory.join(format!("orders-{id}.yaml"));
    let body = format!(
        r#"version: 1
source: source
destination: destination
selection:
  roots:
    - table: public.orders
      filter:
        - column: id
          op: eq
          value: {id}
  relationships:
    depth: 1
    direction: parents
limits:
  max_rows_per_table: 100
  max_total_rows: 100
  max_depth: 3
  max_bytes: 1048576
  timeout_ms: 15000
masking:
  policy: development-default@2
"#
    );
    std::fs::write(&path, body).unwrap();
    path
}

#[tokio::test]
async fn two_overlapping_plans_do_not_corrupt_the_destination() {
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

    // Start from an empty relationship slice.
    let mut connection = connect(&destination).await;
    let _ = sqlx::query("BEGIN").execute(&mut connection).await;
    let _ = sqlx::query("SET CONSTRAINTS ALL DEFERRED")
        .execute(&mut connection)
        .await;
    for table in ["orders", "customers"] {
        let _ = sqlx::query(&format!("DELETE FROM public.{table}"))
            .execute(&mut connection)
            .await;
    }
    let _ = sqlx::query("COMMIT").execute(&mut connection).await;

    // Two different plans: different root rows, different fingerprints,
    // overlapping destination tables.
    let spec_a = orders_spec(&config, "0b9f4c3e-0000-0000-0000-000000000001");
    let spec_b = orders_spec(&config, "0b9f4c3e-0000-0000-0000-000000000002");

    let mut fingerprints = Vec::new();
    for spec in [&spec_a, &spec_b] {
        let planned = cli(
            &config,
            &[
                "safe-sync",
                "plan",
                "--spec",
                &spec.to_string_lossy(),
                "--out",
                &config
                    .join(format!("plan-{}.json", fingerprints.len()))
                    .to_string_lossy(),
            ],
            &source,
            &destination,
        );
        assert_eq!(
            code(&planned),
            0,
            "{}",
            String::from_utf8_lossy(&planned.stdout)
        );
        let path = config.join(format!("plan-{}.json", fingerprints.len()));
        let json: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        fingerprints.push(json["plan_fingerprint"].as_str().unwrap().to_string());
    }
    assert_ne!(fingerprints[0], fingerprints[1]);

    let plan_a = config.join("plan-0.json");
    let plan_b = config.join("plan-1.json");
    let args_a = [
        "safe-sync",
        "execute",
        "--plan",
        &plan_a.to_string_lossy(),
        "--spec",
        &spec_a.to_string_lossy(),
        "--ack",
        &fingerprints[0],
        "--yes",
    ];
    let args_b = [
        "safe-sync",
        "execute",
        "--plan",
        &plan_b.to_string_lossy(),
        "--spec",
        &spec_b.to_string_lossy(),
        "--ack",
        &fingerprints[1],
        "--yes",
    ];

    let mut first = cli_command(&config, &args_a, &source, &destination);
    let mut second = cli_command(&config, &args_b, &source, &destination);
    let child_a = first
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    let child_b = second
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    let a = child_a.wait_with_output().unwrap();
    let b = child_b.wait_with_output().unwrap();

    let successes = [code(&a), code(&b)]
        .iter()
        .filter(|code| **code == 0)
        .count();
    assert_eq!(
        successes,
        1,
        "exactly one plan may import: a={:?} b={:?}",
        String::from_utf8_lossy(&a.stdout),
        String::from_utf8_lossy(&b.stdout)
    );

    // The loser must refuse deterministically, not partially write.
    let loser = if code(&a) == 0 { &b } else { &a };
    assert_eq!(code(loser), 8);
    assert_eq!(envelope(loser)["error"]["detail"], "destination_not_empty");

    // No duplicates, and the imported order's parent exists.
    let (orders,): (i64,) = sqlx::query_as("SELECT count(*) FROM public.orders")
        .fetch_one(&mut connection)
        .await
        .unwrap();
    let (customers,): (i64,) = sqlx::query_as("SELECT count(*) FROM public.customers")
        .fetch_one(&mut connection)
        .await
        .unwrap();
    assert_eq!(orders, 1, "one plan's root row only");
    assert_eq!(customers, 1, "one parent row, no duplicate");

    let broken: (i64,) = sqlx::query_as(
        "SELECT count(*) FROM public.orders o
         LEFT JOIN public.customers c ON c.code = o.customer_code
         WHERE c.code IS NULL",
    )
    .fetch_one(&mut connection)
    .await
    .unwrap();
    assert_eq!(broken.0, 0);
}

#[tokio::test]
async fn an_identity_plan_reports_the_sequence_status() {
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

    // Clear the identity tables and put the sequence behind the imported id.
    let mut connection = connect(&destination).await;
    sqlx::query("DELETE FROM public.identity_children")
        .execute(&mut connection)
        .await
        .unwrap();
    sqlx::query("DELETE FROM public.identity_parents")
        .execute(&mut connection)
        .await
        .unwrap();
    sqlx::query("SELECT setval('public.identity_parents_id_seq', 1, false)")
        .execute(&mut connection)
        .await
        .unwrap();
    enroll(&config, &source, &destination);

    let spec = config.join("identity-spec.yaml");
    std::fs::write(
        &spec,
        r#"version: 1
source: source
destination: destination
selection:
  roots:
    - table: public.identity_children
      filter:
        - column: id
          op: eq
          value: 0b9f4c3e-0000-0000-0000-0000000000c1
  relationships:
    depth: 1
limits:
  max_rows_per_table: 10
  max_total_rows: 10
  max_depth: 2
  max_bytes: 1048576
  timeout_ms: 10000
masking:
  policy: development-default@2
"#,
    )
    .unwrap();

    let plan_path = config.join("identity-plan.json");
    let output = cli(
        &config,
        &[
            "safe-sync",
            "plan",
            "--spec",
            &spec.to_string_lossy(),
            "--out",
            &plan_path.to_string_lossy(),
        ],
        &source,
        &destination,
    );
    assert_eq!(
        code(&output),
        0,
        "plan failed: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let plan_json: Value =
        serde_json::from_str(&std::fs::read_to_string(&plan_path).unwrap()).unwrap();
    let fingerprint = plan_json["plan_fingerprint"].as_str().unwrap().to_string();

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
        "execute failed: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    // The owner can take the lock, so the sequence is raised and the run is
    // reported as completed, not as an unreconciled success.
    assert_eq!(envelope(&output)["data"]["sequenceStatus"], "completed");
}

#[tokio::test]
async fn a_cached_sequence_is_not_reported_as_success() {
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

    let mut connection = connect(&destination).await;
    sqlx::query("DELETE FROM public.identity_children")
        .execute(&mut connection)
        .await
        .unwrap();
    sqlx::query("DELETE FROM public.identity_parents")
        .execute(&mut connection)
        .await
        .unwrap();
    sqlx::query("SELECT setval('public.identity_parents_id_seq', 1, false)")
        .execute(&mut connection)
        .await
        .unwrap();
    // A preallocating sequence cannot be proven quiescent, so the run must not
    // claim success after committing the rows.
    sqlx::query("ALTER SEQUENCE public.identity_parents_id_seq CACHE 10")
        .execute(&mut connection)
        .await
        .unwrap();
    enroll(&config, &source, &destination);

    let spec = config.join("cached-spec.yaml");
    std::fs::write(
        &spec,
        r#"version: 1
source: source
destination: destination
selection:
  roots:
    - table: public.identity_children
      filter:
        - column: id
          op: eq
          value: 0b9f4c3e-0000-0000-0000-0000000000c1
  relationships:
    depth: 1
limits:
  max_rows_per_table: 10
  max_total_rows: 10
  max_depth: 2
  max_bytes: 1048576
  timeout_ms: 10000
masking:
  policy: development-default@2
"#,
    )
    .unwrap();

    let plan_path = config.join("cached-plan.json");
    let output = cli(
        &config,
        &[
            "safe-sync",
            "plan",
            "--spec",
            &spec.to_string_lossy(),
            "--out",
            &plan_path.to_string_lossy(),
        ],
        &source,
        &destination,
    );
    assert_eq!(
        code(&output),
        0,
        "plan failed: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let plan_json: Value =
        serde_json::from_str(&std::fs::read_to_string(&plan_path).unwrap()).unwrap();
    let fingerprint = plan_json["plan_fingerprint"].as_str().unwrap().to_string();

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
        6,
        "an unreconciled sequence must exit 6, got: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let body = envelope(&output);
    assert_eq!(
        body["ok"], false,
        "an unreconciled sequence must not be reported as ok"
    );
    assert_eq!(body["error"]["code"], "sequence_reconciliation_required");
    assert_eq!(body["data"]["sequenceStatus"], "reconciliation_required");
    assert!(
        body["data"]["rowsWritten"].as_u64().unwrap() >= 1,
        "the committed rows must still be reported"
    );

    // Restore the ordinary identity cache for the tests that follow.
    sqlx::query("ALTER SEQUENCE public.identity_parents_id_seq CACHE 1")
        .execute(&mut connection)
        .await
        .unwrap();
}
