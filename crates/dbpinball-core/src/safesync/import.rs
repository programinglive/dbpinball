//! Transactional destination import with a verifiable idempotency contract.
//!
//! The run is one transaction. The completed-run record lives in the same
//! transaction as the data, and it stores a digest of the destination rows the
//! run produced. A repeat is a no-op only when that digest still matches, so a
//! changed destination is refused instead of skipped.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::Row;

use crate::domain::error::{CoreError, ErrorCode};
use crate::safesync::graph::inspect_table;
use crate::safesync::mask::{hex_lower, MaskedValue};
use crate::safesync::plan::DryRunPlan;
use crate::safesync::planner::{quote_ident, TableRows};
use crate::safesync::session::BoundSession;
use crate::safesync::spec::QualifiedName;

/// The destination run table. It is created inside the data transaction.
pub const RUN_TABLE: &str = "public.dbpinball_safe_sync_runs";

/// What to do when the destination is not empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictPolicy {
    Refuse,
    Replace,
}

impl ConflictPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            ConflictPolicy::Refuse => "refuse",
            ConflictPolicy::Replace => "replace",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TableImport {
    pub schema: String,
    pub table: String,
    pub rows: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    pub run_id: String,
    pub already_completed: bool,
    pub rows_written: u64,
    pub destination_digest: String,
    pub tables: Vec<TableImport>,
}

/// Import masked rows into the destination inside one transaction.
pub async fn run(
    session: &mut BoundSession,
    plan: &DryRunPlan,
    tables: &[TableRows],
    policy: ConflictPolicy,
    cancel: Arc<AtomicBool>,
) -> Result<ImportReport, CoreError> {
    // The run table is created in autocommit before the data transaction. It is
    // its own transaction, so a failed CREATE leaves the connection usable and
    // the session lock is released cleanly; it also keeps the DDL out of the
    // data transaction.
    setup_run_table(session).await?;

    session.begin_destination_transaction().await?;

    match run_inner(session, plan, tables, policy, &cancel).await {
        Ok(report) => {
            sqlx::query("COMMIT")
                .execute(&mut *session.connection())
                .await
                .map_err(connection_error)?;
            Ok(report)
        }
        Err(error) => {
            let _ = sqlx::query("ROLLBACK")
                .execute(&mut *session.connection())
                .await;
            Err(error)
        }
    }
}

/// A session-scoped advisory lock used only to serialise the run-table DDL.
const RUN_TABLE_LOCK_KEY: i64 = 0x6470_6269_6e62_6c31; // "dpbinbl1"

/// Create the run table under a short session lock, outside any data
/// transaction.
async fn setup_run_table(session: &mut BoundSession) -> Result<(), CoreError> {
    sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(RUN_TABLE_LOCK_KEY)
        .execute(&mut *session.connection())
        .await
        .map_err(connection_error)?;

    let setup = ensure_run_table(session).await;

    let _ = sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(RUN_TABLE_LOCK_KEY)
        .execute(&mut *session.connection())
        .await;

    setup
}

/// A 64-bit lock key derived from the plan fingerprint.
///
/// Scope: the destination database, for this plan. Taken as a transaction-scoped
/// advisory lock, so PostgreSQL releases it on commit or rollback and a dropped
/// connection cannot leave it behind.
fn plan_lock_key(fingerprint: &str) -> i64 {
    let digest = crate::safesync::mask::digest(fingerprint.as_bytes());
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&digest[..8]);
    i64::from_be_bytes(bytes)
}

async fn run_inner(
    session: &mut BoundSession,
    plan: &DryRunPlan,
    tables: &[TableRows],
    policy: ConflictPolicy,
    cancel: &Arc<AtomicBool>,
) -> Result<ImportReport, CoreError> {
    // Serialize concurrent runs of the same plan. The lock is transaction-scoped,
    // so it is released by the commit or rollback at the end of `run`.
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(plan_lock_key(&plan.plan_fingerprint))
        .execute(&mut *session.connection())
        .await
        .map_err(connection_error)?;

    // Re-assert the destination on the execution session, inside the transaction
    // that will write.
    if session.identity().identity_key != plan.destination.identity_key {
        return Err(CoreError::with_detail(
            ErrorCode::Conflict,
            "the destination identity changed since the plan was made",
            "destination_changed",
        ));
    }

    // Lock the selected tables so the integrity check and the write see one
    // stable state. `SHARE ROW EXCLUSIVE` blocks other writers, not readers.
    //
    // The order is sorted, not the plan order: two plans with the same tables in
    // a different order must still take their locks in one canonical order or
    // they can deadlock. The insert order stays as the plan defines it.
    let mut locked: Vec<&QualifiedName> = tables.iter().map(|table| &table.table).collect();
    locked.sort_by(|left, right| (&left.schema, &left.table).cmp(&(&right.schema, &right.table)));
    for table in locked {
        let statement = format!(
            "LOCK TABLE {}.{} IN SHARE ROW EXCLUSIVE MODE",
            quote_ident(&table.schema),
            quote_ident(&table.table)
        );
        sqlx::query(&statement)
            .execute(&mut *session.connection())
            .await
            .map_err(connection_error)?;
    }

    // Refuse destination schema drift before touching any row.
    for planned in &plan.tables {
        let name = crate::safesync::QualifiedName {
            schema: planned.schema.clone(),
            table: planned.table.clone(),
        };
        let live = inspect_table(session, &name).await?;
        if live.schema_hash != planned.schema_hash {
            return Err(CoreError::with_detail(
                ErrorCode::Conflict,
                format!("the schema of {} changed since the plan was made", name),
                "schema_drift",
            ));
        }
    }

    if cancel.load(Ordering::Relaxed) {
        return Err(cancelled());
    }

    let (current_digest, current_rows) = destination_digest(session, tables).await?;

    // A completed run with this fingerprint is a no-op only while the
    // destination still matches the digest recorded with that run.
    if let Some((stored_digest, stored_rows, run_id)) =
        completed_run(session, &plan.plan_fingerprint).await?
    {
        if stored_digest == current_digest && stored_rows == current_rows {
            return Ok(ImportReport {
                run_id,
                already_completed: true,
                rows_written: 0,
                destination_digest: current_digest,
                tables: tables
                    .iter()
                    .map(|table| TableImport {
                        schema: table.table.schema.clone(),
                        table: table.table.table.clone(),
                        rows: 0,
                    })
                    .collect(),
            });
        }

        return Err(CoreError::with_detail(
            ErrorCode::Conflict,
            "the destination changed since the completed run",
            "destination_drifted",
        ));
    }

    if current_rows > 0 && policy == ConflictPolicy::Refuse {
        return Err(CoreError::with_detail(
            ErrorCode::Conflict,
            "the destination already contains rows in the selected tables",
            "destination_not_empty",
        ));
    }

    if policy == ConflictPolicy::Replace {
        for table in tables {
            let statement = format!(
                "DELETE FROM {}.{}",
                quote_ident(&table.table.schema),
                quote_ident(&table.table.table)
            );
            sqlx::query(&statement)
                .execute(&mut *session.connection())
                .await
                .map_err(connection_error)?;
        }
    }

    let mut written = Vec::new();
    for table in tables {
        let mut rows = 0u64;
        for row in &table.rows {
            if cancel.load(Ordering::Relaxed) {
                return Err(cancelled());
            }
            insert_row(session, table, row).await?;
            rows += 1;
        }
        written.push(TableImport {
            schema: table.table.schema.clone(),
            table: table.table.table.clone(),
            rows,
        });
    }

    let (digest, total) = destination_digest(session, tables).await?;
    let run_id = uuid::Uuid::new_v4().to_string();

    sqlx::query(&format!(
        "INSERT INTO {RUN_TABLE}
           (run_id, plan_fingerprint, spec_hash, source_identity_key,
            destination_identity_key, schema_hash, masking_policy, mask_key_id,
            conflict_policy, import_mode, destination_digest, rows_written, status)
         VALUES ($1::uuid, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, 'completed')"
    ))
    .bind(&run_id)
    .bind(&plan.plan_fingerprint)
    .bind(&plan.spec_hash)
    .bind(plan.source.identity_key.clone())
    .bind(plan.destination.identity_key.clone())
    .bind(
        plan.tables
            .iter()
            .map(|t| t.schema_hash.clone())
            .collect::<Vec<_>>()
            .join(","),
    )
    .bind(&plan.masking.policy)
    .bind(&plan.masking.key_id)
    .bind(policy.as_str())
    .bind(&plan.import.mode)
    .bind(&digest)
    .bind(total as i64)
    .execute(&mut *session.connection())
    .await
    .map_err(connection_error)?;

    Ok(ImportReport {
        run_id,
        already_completed: false,
        rows_written: total,
        destination_digest: digest,
        tables: written,
    })
}

async fn ensure_run_table(session: &mut BoundSession) -> Result<(), CoreError> {
    sqlx::query(&format!(
        "CREATE TABLE IF NOT EXISTS {RUN_TABLE} (
           run_id uuid PRIMARY KEY,
           plan_fingerprint text NOT NULL,
           spec_hash text NOT NULL,
           source_identity_key text,
           destination_identity_key text,
           schema_hash text NOT NULL,
           masking_policy text NOT NULL,
           mask_key_id text NOT NULL,
           conflict_policy text NOT NULL,
           import_mode text NOT NULL,
           destination_digest text NOT NULL,
           rows_written bigint NOT NULL,
           status text NOT NULL,
           finished_at timestamptz NOT NULL DEFAULT now()
         )"
    ))
    .execute(&mut *session.connection())
    .await
    .map_err(connection_error)?;
    Ok(())
}

async fn completed_run(
    session: &mut BoundSession,
    fingerprint: &str,
) -> Result<Option<(String, u64, String)>, CoreError> {
    let row = sqlx::query(&format!(
        "SELECT destination_digest, rows_written, run_id::text
         FROM {RUN_TABLE}
         WHERE plan_fingerprint = $1 AND status = 'completed'
         ORDER BY finished_at DESC
         LIMIT 1"
    ))
    .bind(fingerprint)
    .fetch_optional(&mut *session.connection())
    .await
    .map_err(connection_error)?;

    Ok(row.map(|row| {
        (
            row.try_get::<String, _>(0).unwrap_or_default(),
            row.try_get::<i64, _>(1).unwrap_or(0) as u64,
            row.try_get::<String, _>(2).unwrap_or_default(),
        )
    }))
}

async fn insert_row(
    session: &mut BoundSession,
    table: &TableRows,
    row: &[MaskedValue],
) -> Result<(), CoreError> {
    let columns = table
        .columns
        .iter()
        .map(|column| quote_ident(column))
        .collect::<Vec<_>>()
        .join(", ");

    let mut placeholders = Vec::with_capacity(table.types.len());
    for (index, type_name) in table.types.iter().enumerate() {
        let cast = cast_for(type_name)?;
        placeholders.push(format!("CAST(${} AS {})", index + 1, cast));
    }

    let sql = format!(
        "INSERT INTO {}.{} ({}) VALUES ({})",
        quote_ident(&table.table.schema),
        quote_ident(&table.table.table),
        columns,
        placeholders.join(", ")
    );

    let mut query = sqlx::query(&sql);
    for value in row {
        query = query.bind(value.as_text().map(str::to_string));
    }

    query
        .execute(&mut *session.connection())
        .await
        .map_err(connection_error)?;
    Ok(())
}

/// A stable digest of the selected destination rows.
///
/// Only the selected tables are included; unrelated rows elsewhere are not.
/// Values are already masked, so the digest never encodes source plaintext.
async fn destination_digest(
    session: &mut BoundSession,
    tables: &[TableRows],
) -> Result<(String, u64), CoreError> {
    let mut hasher = Sha256::new();
    let mut total = 0u64;

    for table in tables {
        hasher.update(b"table");
        push_field(&mut hasher, &table.table.schema);
        push_field(&mut hasher, &table.table.table);

        let columns = table
            .columns
            .iter()
            .map(|column| format!("{}::text", quote_ident(column)))
            .collect::<Vec<_>>()
            .join(", ");

        // A stable order is required so the digest does not depend on physical
        // row order. The declared row identity is that order.
        let order = if table.row_identity.is_empty() {
            "1".to_string()
        } else {
            table
                .row_identity
                .iter()
                .map(|column| quote_ident(column))
                .collect::<Vec<_>>()
                .join(", ")
        };

        let statement = format!(
            "SELECT {} FROM {}.{} ORDER BY {}",
            columns,
            quote_ident(&table.table.schema),
            quote_ident(&table.table.table),
            order
        );

        let rows = sqlx::query(&statement)
            .fetch_all(&mut *session.connection())
            .await
            .map_err(connection_error)?;

        for row in rows {
            hasher.update(b"row");
            for index in 0..table.columns.len() {
                let value: Option<String> = row.try_get(index).map_err(|error| {
                    CoreError::with_detail(
                        ErrorCode::Operation,
                        format!("a destination value could not be read: {error}"),
                        "destination_value_unreadable",
                    )
                })?;
                // Distinguish SQL NULL from an empty string; otherwise a NULL
                // that became '' would hash identically and look unchanged.
                match value {
                    Some(value) => {
                        hasher.update([1u8]);
                        push_field(&mut hasher, &value);
                    }
                    None => {
                        hasher.update([0u8]);
                        hasher.update(0u64.to_le_bytes());
                    }
                }
            }
            total += 1;
        }
    }

    Ok((hex_lower(&hasher.finalize()), total))
}

fn push_field(hasher: &mut Sha256, value: &str) {
    hasher.update((value.len() as u64).to_le_bytes());
    hasher.update(value.as_bytes());
}

fn cast_for(type_name: &str) -> Result<&'static str, CoreError> {
    match type_name {
        "uuid" => Ok("uuid"),
        "text" | "character varying" | "character" | "varchar" | "citext" | "name" => Ok("text"),
        other => Err(CoreError::with_detail(
            ErrorCode::Unsupported,
            format!("column type '{other}' is not supported in this slice"),
            "unsupported_type",
        )),
    }
}

fn cancelled() -> CoreError {
    CoreError::with_detail(ErrorCode::Operation, "the run was cancelled", "cancelled")
}

fn connection_error(error: sqlx::Error) -> CoreError {
    CoreError::with_detail(
        ErrorCode::Operation,
        format!("destination import failed: {error}"),
        "destination_import_failed",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cast_whitelist_refuses_unknown_types() {
        assert_eq!(cast_for("uuid").unwrap(), "uuid");
        assert_eq!(cast_for("text").unwrap(), "text");
        assert_eq!(
            cast_for("jsonb").unwrap_err().detail.as_deref(),
            Some("unsupported_type")
        );
    }

    #[test]
    fn a_cancelled_run_is_a_typed_error() {
        assert_eq!(cancelled().detail.as_deref(), Some("cancelled"));
    }

    #[test]
    fn the_plan_lock_key_is_stable_and_fingerprint_scoped() {
        let first = plan_lock_key("fingerprint-a");
        assert_eq!(first, plan_lock_key("fingerprint-a"));
        assert_ne!(first, plan_lock_key("fingerprint-b"));
    }

    #[test]
    fn the_plan_lock_key_differs_from_the_ddl_lock_key() {
        // A collision would serialise unrelated work or, worse, share a lock
        // between a plan and the run-table setup.
        assert_ne!(plan_lock_key("fingerprint-a"), RUN_TABLE_LOCK_KEY);
    }
}
