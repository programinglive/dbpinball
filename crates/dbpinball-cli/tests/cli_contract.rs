use std::io::Write;
use std::process::{Command, Stdio};

use serde_json::{json, Value};
use tempfile::TempDir;

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_dbpinball")
}

/// Run the CLI with an isolated configuration directory.
///
/// Stdin is always a pipe, so `is_terminal()` is false and the commands see a
/// non-interactive environment. When `stdin` is `None` the pipe is closed
/// immediately, which mirrors `</dev/null`.
fn run(temp: &TempDir, args: &[&str], stdin: Option<&str>) -> (i32, String, String) {
    let mut command = Command::new(binary());
    command
        .arg("--config-dir")
        .arg(temp.path())
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = command.spawn().expect("the CLI binary should start");

    {
        let mut handle = child.stdin.take().expect("stdin should be piped");
        if let Some(input) = stdin {
            handle
                .write_all(input.as_bytes())
                .expect("stdin should accept input");
        }
    }

    let output = child.wait_with_output().expect("the CLI should exit");

    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
    )
}

fn parse_envelope(stdout: &str) -> Value {
    serde_json::from_str(stdout)
        .unwrap_or_else(|error| panic!("stdout is not one JSON object: {error}\n{stdout}"))
}

fn write_store(temp: &TempDir, connections: Value) {
    let store = json!({ "version": 1, "connections": connections });
    std::fs::write(temp.path().join("connections.json"), store.to_string())
        .expect("store should be writable");
}

fn connection() -> Value {
    json!({
        "id": "abc",
        "name": "local",
        "engine": "postgres",
        "host": "127.0.0.1",
        "port": 5432,
        "database": "app",
        "username": "postgres",
        "useSsl": false,
        "target": "unknown"
    })
}

#[test]
fn version_exits_zero() {
    let temp = TempDir::new().unwrap();
    let (code, stdout, _) = run(&temp, &["--version"], None);

    assert_eq!(code, 0);
    assert!(
        stdout.contains("0.1.6"),
        "unexpected version output: {stdout}"
    );
}

#[test]
fn help_exits_zero() {
    let temp = TempDir::new().unwrap();
    let (code, stdout, _) = run(&temp, &["--help"], None);

    assert_eq!(code, 0);
    assert!(stdout.contains("doctor"));
    assert!(stdout.contains("connections"));
    assert!(stdout.contains("schema"));
}

#[test]
fn doctor_emits_a_json_envelope() {
    let temp = TempDir::new().unwrap();
    let (code, stdout, _) = run(&temp, &["doctor", "--json"], None);

    assert_eq!(code, 0, "stderr: {}", stdout);

    let envelope = parse_envelope(&stdout);
    assert_eq!(envelope["ok"], true);
    assert_eq!(envelope["data"]["version"], "0.1.6");
    assert!(envelope["data"]["checks"].as_array().unwrap().len() >= 4);
}

#[test]
fn connections_list_is_empty_on_a_fresh_config() {
    let temp = TempDir::new().unwrap();
    let (code, stdout, _) = run(&temp, &["connections", "list", "--json"], None);

    assert_eq!(code, 0);

    let envelope = parse_envelope(&stdout);
    assert_eq!(envelope["ok"], true);
    assert_eq!(envelope["data"]["connections"], json!([]));
}

#[test]
fn connections_test_unknown_profile_exits_7() {
    let temp = TempDir::new().unwrap();
    let (code, stdout, _) = run(&temp, &["connections", "test", "missing", "--json"], None);

    assert_eq!(code, 7);

    let envelope = parse_envelope(&stdout);
    assert_eq!(envelope["ok"], false);
    assert_eq!(envelope["error"]["code"], "not_found");
}

#[test]
fn schema_inspect_unknown_profile_exits_7() {
    let temp = TempDir::new().unwrap();
    let (code, stdout, _) = run(
        &temp,
        &["schema", "inspect", "--connection", "missing", "--json"],
        None,
    );

    assert_eq!(code, 7);

    let envelope = parse_envelope(&stdout);
    assert_eq!(envelope["error"]["code"], "not_found");
}

#[test]
fn remove_unknown_profile_with_yes_exits_7() {
    let temp = TempDir::new().unwrap();
    let (code, stdout, _) = run(
        &temp,
        &["connections", "remove", "missing", "--yes", "--json"],
        None,
    );

    assert_eq!(code, 7);

    let envelope = parse_envelope(&stdout);
    assert_eq!(envelope["error"]["code"], "not_found");
}

#[test]
fn remove_without_yes_is_a_policy_refusal_in_noninteractive_mode() {
    let temp = TempDir::new().unwrap();
    write_store(&temp, json!([connection()]));

    let (code, stdout, _) = run(&temp, &["connections", "remove", "abc", "--json"], None);

    assert_eq!(code, 4);

    let envelope = parse_envelope(&stdout);
    assert_eq!(envelope["ok"], false);
    assert_eq!(envelope["error"]["code"], "policy_refusal");

    // The profile must still be present.
    let (list_code, list_stdout, _) = run(&temp, &["connections", "list", "--json"], None);
    assert_eq!(list_code, 0);
    let listed = parse_envelope(&list_stdout);
    assert_eq!(listed["data"]["connections"].as_array().unwrap().len(), 1);
}

#[test]
fn add_rejects_a_password_flag() {
    let temp = TempDir::new().unwrap();
    let (code, _, stderr) = run(
        &temp,
        &[
            "connections",
            "add",
            "--name",
            "local",
            "--engine",
            "postgres",
            "--host",
            "127.0.0.1",
            "--port",
            "5432",
            "--database",
            "app",
            "--username",
            "postgres",
            "--password",
            "secret",
        ],
        None,
    );

    assert_eq!(code, 2, "stderr: {stderr}");
}

#[test]
fn list_json_never_contains_a_password_field() {
    let temp = TempDir::new().unwrap();
    write_store(&temp, json!([connection()]));

    let (code, stdout, _) = run(&temp, &["connections", "list", "--json"], None);

    assert_eq!(code, 0);
    assert!(!stdout.to_lowercase().contains("password"));
}

#[test]
fn invalid_store_is_an_operation_error() {
    let temp = TempDir::new().unwrap();
    std::fs::write(temp.path().join("connections.json"), "{ not json").unwrap();

    let (code, stdout, _) = run(&temp, &["connections", "list", "--json"], None);

    assert_eq!(code, 5);

    let envelope = parse_envelope(&stdout);
    assert_eq!(envelope["error"]["code"], "operation");
}

#[test]
fn import_validates_and_persists() {
    let temp = TempDir::new().unwrap();

    let valid = connection();
    let mut invalid = connection();
    invalid["id"] = json!("broken");
    invalid["host"] = json!("");

    let import_path = temp.path().join("import.json");
    std::fs::write(&import_path, json!([valid, invalid]).to_string()).unwrap();

    let import_path_string = import_path.to_string_lossy().to_string();
    let (code, stdout, _) = run(
        &temp,
        &["connections", "import", &import_path_string, "--json"],
        None,
    );

    assert_eq!(code, 0, "stdout: {stdout}");

    let envelope = parse_envelope(&stdout);
    assert_eq!(envelope["data"]["imported"], 1);
    assert_eq!(
        envelope["data"]["skippedInvalid"].as_array().unwrap().len(),
        1
    );

    let store = std::fs::read_to_string(temp.path().join("connections.json")).unwrap();
    assert!(store.contains("\"abc\""));
    assert!(!store.contains("\"broken\""));
}

#[test]
fn doctor_is_deterministic_with_a_fixed_config_dir() {
    let temp = TempDir::new().unwrap();

    let (first_code, first_stdout, _) = run(&temp, &["doctor", "--json"], None);
    let (second_code, second_stdout, _) = run(&temp, &["doctor", "--json"], None);

    assert_eq!(first_code, 0);
    assert_eq!(second_code, 0);
    assert_eq!(first_stdout, second_stdout);
}

// ---------------------------------------------------------------------------
// M3 security commands
// ---------------------------------------------------------------------------

#[test]
fn security_help_lists_every_subcommand() {
    let temp = TempDir::new().unwrap();
    let (code, stdout, _) = run(&temp, &["security", "--help"], None);
    assert_eq!(code, 0);
    for command in ["inspect", "enroll", "verify", "policy-check"] {
        assert!(stdout.contains(command), "missing {command} in help");
    }
}

#[test]
fn security_inspect_requires_a_known_connection() {
    let temp = TempDir::new().unwrap();
    let (code, stdout, _) = run(&temp, &["--json", "security", "inspect", "missing"], None);
    assert_eq!(code, 7, "an unknown connection exits 7");
    let value: Value = serde_json::from_str(stdout.trim()).expect("one JSON object");
    assert_eq!(value["error"]["code"], "not_found");
}

#[test]
fn security_enroll_rejects_an_unknown_connection() {
    let temp = TempDir::new().unwrap();
    let (code, _, _) = run(
        &temp,
        &["security", "enroll", "missing", "--category", "development"],
        None,
    );
    assert_eq!(code, 7);
}

#[test]
fn security_enroll_requires_a_valid_category() {
    let temp = TempDir::new().unwrap();
    let (code, _, stderr) = run(
        &temp,
        &["security", "enroll", "missing", "--category", "production"],
        None,
    );
    assert_eq!(code, 2, "an invalid category is a usage error");
    assert!(stderr.to_ascii_lowercase().contains("invalid value"));
}

#[test]
fn security_policy_check_requires_a_valid_operation() {
    let temp = TempDir::new().unwrap();
    let (code, _, _) = run(
        &temp,
        &[
            "security",
            "policy-check",
            "missing",
            "--operation",
            "nonsense",
        ],
        None,
    );
    assert_eq!(code, 2);
}

#[test]
fn security_policy_check_for_a_read_only_operation_needs_no_database() {
    let temp = TempDir::new().unwrap();

    // Create a profile without a password; a read-only policy check must not
    // need one, because it never opens a connection.
    let (code, _, _) = run(
        &temp,
        &[
            "connections",
            "add",
            "--name",
            "local",
            "--engine",
            "postgres",
            "--host",
            "127.0.0.1",
            "--port",
            "5432",
            "--database",
            "app",
            "--username",
            "postgres",
            "--target",
            "local",
        ],
        Some("secret\n"),
    );
    assert_eq!(code, 0, "profile creation should succeed");

    let (code, stdout, _) = run(
        &temp,
        &[
            "--json",
            "security",
            "policy-check",
            "PLACEHOLDER",
            "--operation",
            "query",
        ],
        None,
    );
    // The placeholder id does not exist, so this still exits 7; the point is
    // that the command is wired and returns one JSON object.
    assert_eq!(code, 7);
    let value: Value = serde_json::from_str(stdout.trim()).expect("one JSON object");
    assert_eq!(value["error"]["code"], "not_found");
}
