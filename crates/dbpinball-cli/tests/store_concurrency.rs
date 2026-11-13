//! Cross-process store coordination.
//!
//! These tests spawn the real `dbpinball` binary, not an in-process task: the
//! lock is an operating-system file lock, and only separate processes exercise
//! that guarantee.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::json;

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_dbpinball")
}

fn profile(id: &str, database: &str) -> serde_json::Value {
    json!({
        "id": id,
        "name": format!("profile-{id}"),
        "engine": "postgres",
        "host": "127.0.0.1",
        "port": 5432,
        "database": database,
        "username": "postgres",
        "useSsl": false
    })
}

fn write_import(dir: &Path, name: &str, records: &[serde_json::Value]) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, serde_json::to_string(records).unwrap()).unwrap();
    path
}

fn import_command(config_dir: &Path, file: &Path) -> Command {
    let mut command = Command::new(binary());
    command
        .arg("--config-dir")
        .arg(config_dir)
        .arg("--json")
        .arg("connections")
        .arg("import")
        .arg(file)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

fn read_store(config_dir: &Path) -> serde_json::Value {
    let text = fs::read_to_string(config_dir.join("connections.json")).unwrap();
    serde_json::from_str(&text).unwrap()
}

#[test]
fn two_processes_do_not_lose_each_others_import() {
    let directory = tempfile::tempdir().unwrap();
    let config = directory.path();

    let first = write_import(config, "first.json", &[profile("a", "alpha")]);
    let second = write_import(config, "second.json", &[profile("b", "beta")]);

    let mut one = import_command(config, &first).spawn().unwrap();
    let mut two = import_command(config, &second).spawn().unwrap();

    let one_status = one.wait().unwrap();
    let two_status = two.wait().unwrap();

    assert!(one_status.success(), "first import failed");
    assert!(two_status.success(), "second import failed");

    let store = read_store(config);
    let ids: Vec<String> = store["connections"]
        .as_array()
        .unwrap()
        .iter()
        .map(|record| record["id"].as_str().unwrap().to_string())
        .collect();

    assert!(ids.contains(&"a".to_string()), "first record was lost");
    assert!(ids.contains(&"b".to_string()), "second record was lost");
    assert_eq!(ids.len(), 2);
    assert_eq!(store["revision"].as_u64().unwrap(), 2);
}

#[test]
fn a_live_lock_owner_causes_a_busy_exit() {
    let directory = tempfile::tempdir().unwrap();
    let config = directory.path();
    let file = write_import(config, "one.json", &[profile("a", "alpha")]);

    // The test process is alive, so the child must not steal the lock.
    let lock = json!({
        "pid": std::process::id(),
        "host": hostname(),
        "started_at": 0
    });
    fs::write(
        config.join("connections.lock"),
        serde_json::to_string(&lock).unwrap(),
    )
    .unwrap();

    let output = import_command(config, &file).output().unwrap();

    assert_eq!(output.status.code(), Some(9), "expected the busy exit code");
    assert!(!config.join("connections.json").exists());
}

#[test]
fn a_dead_lock_owner_is_recovered() {
    let directory = tempfile::tempdir().unwrap();
    let config = directory.path();
    let file = write_import(config, "one.json", &[profile("a", "alpha")]);

    // A PID that cannot be alive on any supported platform.
    let lock = json!({
        "pid": 2147483646u32,
        "host": hostname(),
        "started_at": 0
    });
    fs::write(
        config.join("connections.lock"),
        serde_json::to_string(&lock).unwrap(),
    )
    .unwrap();

    let output = import_command(config, &file).output().unwrap();

    assert!(output.status.success(), "a dead owner must be recovered");
    assert_eq!(
        read_store(config)["connections"].as_array().unwrap().len(),
        1
    );
}

#[test]
fn a_lock_from_another_host_is_not_stolen() {
    let directory = tempfile::tempdir().unwrap();
    let config = directory.path();
    let file = write_import(config, "one.json", &[profile("a", "alpha")]);

    let lock = json!({
        "pid": 1,
        "host": "another-machine",
        "started_at": 0
    });
    fs::write(
        config.join("connections.lock"),
        serde_json::to_string(&lock).unwrap(),
    )
    .unwrap();

    let output = import_command(config, &file).output().unwrap();

    assert_eq!(output.status.code(), Some(9));
}

fn hostname() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "unknown".to_string())
}
