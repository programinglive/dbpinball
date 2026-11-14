//! Privacy-safe run audit and local journal.
//!
//! A record contains counts, names and identities. It never contains a source
//! value, a masked value, a password or a mask key.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::domain::error::{CoreError, ErrorCode};

/// Per-table counts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TableAudit {
    pub schema: String,
    pub table: String,
    pub rows: u64,
    pub masked_columns: u64,
}

/// One run's audit record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditRecord {
    pub run_id: String,
    pub plan_fingerprint: String,
    /// The destination profile id, so `status --run` can find it later.
    pub destination_connection_id: String,
    pub source_identity_key: Option<String>,
    pub destination_identity_key: Option<String>,
    pub tables: Vec<TableAudit>,
    pub policy: String,
    pub key_id: String,
    pub outcome: String,
    pub started_at_ms: u64,
    pub finished_at_ms: u64,
}

/// Write the local run journal. Best-effort: the destination run table is the
/// source of truth, so a journal write failure is reported, not fatal.
pub fn write_record(directory: &Path, record: &AuditRecord) -> Result<PathBuf, CoreError> {
    let runs = directory.join("safe-sync").join("runs");
    std::fs::create_dir_all(&runs).map_err(|error| {
        CoreError::new(
            ErrorCode::Operation,
            format!("the audit directory could not be created: {error}"),
        )
    })?;

    let path = runs.join(format!("{}.json", record.run_id));
    let temporary = runs.join(format!(".{}.tmp", record.run_id));

    let body = serde_json::to_string_pretty(record).map_err(|error| {
        CoreError::new(
            ErrorCode::Operation,
            format!("the audit record could not be serialized: {error}"),
        )
    })?;

    std::fs::write(&temporary, body).map_err(|error| {
        CoreError::new(
            ErrorCode::Operation,
            format!("the audit record could not be written: {error}"),
        )
    })?;

    std::fs::rename(&temporary, &path).map_err(|error| {
        let _ = std::fs::remove_file(&temporary);
        CoreError::new(
            ErrorCode::Operation,
            format!("the audit record could not be finalized: {error}"),
        )
    })?;

    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_record_has_no_value_fields() {
        let record = AuditRecord {
            run_id: "run-1".into(),
            plan_fingerprint: "fp".into(),
            destination_connection_id: "destination".into(),
            source_identity_key: Some("postgres://h:1/db#1".into()),
            destination_identity_key: Some("postgres://h:1/dev#2".into()),
            tables: vec![TableAudit {
                schema: "public".into(),
                table: "accounts".into(),
                rows: 1,
                masked_columns: 2,
            }],
            policy: "development-default@1".into(),
            key_id: "k-test".into(),
            outcome: "completed".into(),
            started_at_ms: 1,
            finished_at_ms: 2,
        };

        let rendered = serde_json::to_string(&record).unwrap();
        assert!(rendered.contains("\"rows\":1"));
        assert!(!rendered.contains("alice@example.com"));
        assert!(!rendered.contains("Anon "));
    }

    #[test]
    fn the_record_is_written_atomically() {
        let directory = tempfile::tempdir().unwrap();
        let record = AuditRecord {
            run_id: "run-1".into(),
            plan_fingerprint: "fp".into(),
            destination_connection_id: "destination".into(),
            source_identity_key: None,
            destination_identity_key: None,
            tables: Vec::new(),
            policy: "development-default@1".into(),
            key_id: "k-test".into(),
            outcome: "completed".into(),
            started_at_ms: 1,
            finished_at_ms: 2,
        };
        let path = write_record(directory.path(), &record).unwrap();
        assert!(path.exists());
        assert!(!directory
            .path()
            .join("safe-sync/runs/.run-1.json.tmp")
            .exists());
    }
}
