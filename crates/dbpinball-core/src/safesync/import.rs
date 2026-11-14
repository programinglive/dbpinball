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

async fn run_inner(
    session: &mut BoundSession,
    plan: &DryRunPlan,
    tables: &[TableRows],
    policy: ConflictPolicy,
    cancel: &Arc<AtomicBool>,
) -> Result<ImportReport, CoreError> {
    ensure_run_table(session).await?;

    // Lock the selected tables so the integrity check and the write see one
    // stable state. `SHARE ROW EXCLUSIVE` blocks other writers, not readers.
    for table in tables {
        let statement = format!(
            "LOCK TABLE {}.{} IN SHARE ROW EXCLUSIVE MODE",
            quote_ident(&table.table.schema),
            quote_ident(&table.table.table)
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
                let value: Option<String> = row.try_get(index).unwrap_or(None);
                push_field(&mut hasher, value.as_deref().unwrap_or(""));
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
}
