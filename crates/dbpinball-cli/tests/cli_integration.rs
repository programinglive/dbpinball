#![cfg(feature = "integration")]

//! CLI integration tests against disposable PostgreSQL and MySQL.
//!
//! Gated behind `--features integration` and the `DBPINBALL_TEST_*_URL`
//! variables, mirroring the core integration tests. The CI
//! `database-integration` job exports both.

use std::process::{Command, Stdio};

use serde_json::{json, Value};
use tempfile::TempDir;

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_dbpinball")
}

struct Target {
    engine: &'static str,
    host: String,
    port: u16,
    database: String,
    username: String,
    password: String,
}

fn target(variable: &str) -> Option<Target> {
    let url = std::env::var(variable).ok()?;
    let parsed = url::Url::parse(&url).ok()?;

    let engine = match parsed.scheme() {
        "postgres" | "postgresql" => "postgres",
        "mysql" => "mysql",
        _ => return None,
    };

    let default_port = if engine == "postgres" { 5432 } else { 3306 };

    Some(Target {
        engine,
        host: parsed.host_str()?.to_string(),
        port: parsed.port().unwrap_or(default_port),
        database: parsed.path().trim_start_matches('/').to_string(),
        username: parsed.username().to_string(),
        password: parsed.password().unwrap_or_default().to_string(),
    })
}

fn write_store(temp: &TempDir, target: &Target) {
    let record = json!({
        "id": "integration",
        "name": "integration",
        "engine": target.engine,
        "host": target.host,
        "port": target.port,
        "database": target.database,
        "username": target.username,
        "useSsl": false,
        "target": "unknown"
    });

    let store = json!({ "version": 1, "connections": [record] });

    std::fs::write(temp.path().join("connections.json"), store.to_string())
        .expect("store should be writable");
}

fn run(temp: &TempDir, args: &[&str], password: &str) -> (i32, String, String) {
    let output = Command::new(binary())
        .arg("--config-dir")
        .arg(temp.path())
        .arg("--credential-source")
        .arg("env")
        .args(args)
        .env("DBPINBALL_PASSWORD", password)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("the CLI binary should run");

    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
    )
}

fn envelope(stdout: &str) -> Value {
    serde_json::from_str(stdout)
        .unwrap_or_else(|error| panic!("stdout is not one JSON object: {error}\n{stdout}"))
}

fn exercise(variable: &str) {
    let Some(target) = target(variable) else {
        eprintln!("skipping: {variable} is not set");
        return;
    };

    let temp = TempDir::new().unwrap();
    write_store(&temp, &target);

    // 1. Test the connection through the CLI.
    let (code, stdout, stderr) = run(
        &temp,
        &["connections", "test", "integration", "--json"],
        &target.password,
    );
    assert_eq!(code, 0, "connection test failed: {stdout} {stderr}");

    let parsed = envelope(&stdout);
    assert_eq!(parsed["ok"], true);
    assert_eq!(parsed["data"]["reachable"], true);

    // 2. List databases.
    let (code, stdout, stderr) = run(
        &temp,
        &["schema", "inspect", "--connection", "integration", "--json"],
        &target.password,
    );
    assert_eq!(code, 0, "database listing failed: {stdout} {stderr}");

    let parsed = envelope(&stdout);
    let databases = parsed["data"]["databases"]
        .as_array()
        .expect("databases should be an array");

    assert!(
        databases.iter().any(|name| name == &target.database),
        "expected {} in {databases:?}",
        target.database
    );

    // 3. List tables.
    let (code, stdout, stderr) = run(
        &temp,
        &[
            "schema",
            "inspect",
            "--connection",
            "integration",
            "--database",
            &target.database,
            "--json",
        ],
        &target.password,
    );
    assert_eq!(code, 0, "table listing failed: {stdout} {stderr}");

    let parsed = envelope(&stdout);
    assert!(
        parsed["data"]["tables"].is_array(),
        "tables should be an array: {parsed}"
    );

    // 4. Authentication failure is a connection error with no secret leak.
    let wrong_password = "definitely-not-the-password";
    let (code, stdout, _) = run(
        &temp,
        &["connections", "test", "integration", "--json"],
        wrong_password,
    );

    assert_eq!(code, 3, "a wrong password should exit 3: {stdout}");

    let parsed = envelope(&stdout);
    assert_eq!(parsed["ok"], false);
    assert_eq!(parsed["error"]["code"], "connection");
    assert!(!stdout.contains(wrong_password));
}

#[test]
fn postgres_cli_round_trip() {
    exercise("DBPINBALL_TEST_PG_URL");
}

#[test]
fn mysql_cli_round_trip() {
    exercise("DBPINBALL_TEST_MYSQL_URL");
}
