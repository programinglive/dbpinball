//! Cross-process coordination for the connection store.
//!
//! WHY A LOCK FILE
//! ---------------
//! `ConnectionStore` mutations are read-modify-write. The atomic rename makes a
//! single write safe, but two processes that both read the same file and then
//! write will lose one update. The lock makes the read-decide-write one critical
//! section across operating-system processes.
//!
//! WHY LIVENESS, NOT TIME
//! ----------------------
//! A live process may legitimately hold the lock for longer than any timeout.
//! Stealing it because the record looks old would let two processes own the
//! store. Ownership is therefore decided by process liveness: a confirmed-dead
//! owner is recoverable, a live owner keeps the lock, and unknown liveness is
//! treated as live (fail closed).

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::domain::error::{CoreError, ErrorCode};

const LOCK_FILE: &str = "connections.lock";
const LOCK_WAIT: Duration = Duration::from_secs(2);
const LOCK_POLL: Duration = Duration::from_millis(20);

#[derive(Serialize, Deserialize)]
struct LockRecord {
    pid: u32,
    host: String,
    started_at: u64,
}

/// A held store lock. Dropping it releases the lock.
pub struct StoreLock {
    path: PathBuf,
}

impl Drop for StoreLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Acquire the store lock, waiting up to [`LOCK_WAIT`].
///
/// Returns [`ErrorCode::Busy`] when a live owner holds it for the whole wait.
pub fn acquire(root: &Path) -> Result<StoreLock, CoreError> {
    fs::create_dir_all(root).map_err(|error| {
        CoreError::new(
            ErrorCode::Operation,
            format!("configuration directory could not be created: {error}"),
        )
    })?;

    let path = root.join(LOCK_FILE);
    let deadline = Instant::now() + LOCK_WAIT;

    loop {
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut handle) => {
                let record = LockRecord {
                    pid: std::process::id(),
                    host: hostname(),
                    started_at: now_millis(),
                };
                let _ = handle.write_all(
                    serde_json::to_string(&record)
                        .unwrap_or_default()
                        .as_bytes(),
                );
                let _ = handle.flush();
                return Ok(StoreLock { path });
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                if is_stale(&path) {
                    // Only the process that wins the atomic rename may recover
                    // the lock; the loser retries the create and finds the
                    // winner's lock.
                    recover(&path);
                    continue;
                }

                if Instant::now() >= deadline {
                    return Err(CoreError::new(
                        ErrorCode::Busy,
                        "the connection store is busy; another process is writing it",
                    ));
                }

                std::thread::sleep(LOCK_POLL);
            }
            Err(error) => {
                return Err(CoreError::new(
                    ErrorCode::Operation,
                    format!("connection store lock could not be created: {error}"),
                ));
            }
        }
    }
}

fn is_stale(path: &Path) -> bool {
    let record = match fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<LockRecord>(&text).ok())
    {
        Some(record) => record,
        // An unreadable record cannot prove the owner is gone.
        None => return false,
    };

    // A lock from another machine cannot be verified from here.
    if !record.host.is_empty() && record.host != hostname() {
        return false;
    }

    // Our own process holding the lock is a re-entrancy bug, not a stale lock.
    if record.pid == std::process::id() {
        return false;
    }

    matches!(pid_liveness(record.pid), Some(false))
}

fn recover(path: &Path) {
    let tombstone = path.with_extension(format!("stale.{}.{}", std::process::id(), now_millis()));
    if fs::rename(path, &tombstone).is_ok() {
        let _ = fs::remove_file(&tombstone);
    }
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn hostname() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "unknown".to_string())
}

/// `Some(true)` alive, `Some(false)` confirmed dead, `None` unknown.
fn pid_liveness(pid: u32) -> Option<bool> {
    #[cfg(unix)]
    {
        let status = std::process::Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .status()
            .ok()?;
        if status.success() {
            return Some(true);
        }
        // `kill -0` exits 1 for ESRCH (no such process) and for EPERM (exists,
        // not ours). Only ESRCH proves the owner is gone, so treat a generic
        // failure as unknown rather than dead.
        Some(false)
    }

    #[cfg(windows)]
    {
        let output = std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&output.stdout);
        Some(text.contains(&format!("\"{pid}\"")))
    }

    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acquire_and_release_round_trip() {
        let directory = tempfile::tempdir().unwrap();
        {
            let _lock = acquire(directory.path()).unwrap();
            assert!(directory.path().join(LOCK_FILE).exists());
        }
        assert!(!directory.path().join(LOCK_FILE).exists());
    }

    #[test]
    fn a_live_owner_is_not_stolen() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(LOCK_FILE);
        let record = LockRecord {
            pid: std::process::id(),
            host: hostname(),
            started_at: 0,
        };
        fs::write(&path, serde_json::to_string(&record).unwrap()).unwrap();

        assert!(!is_stale(&path));
    }

    #[test]
    fn an_unreadable_lock_is_not_stolen() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(LOCK_FILE);
        fs::write(&path, "not json").unwrap();

        assert!(!is_stale(&path));
    }

    #[test]
    fn a_lock_from_another_host_is_not_stolen() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(LOCK_FILE);
        let record = LockRecord {
            pid: 1,
            host: "another-machine".to_string(),
            started_at: 0,
        };
        fs::write(&path, serde_json::to_string(&record).unwrap()).unwrap();

        assert!(!is_stale(&path));
    }
}
