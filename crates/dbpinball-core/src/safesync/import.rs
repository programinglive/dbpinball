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

/// The destination run table. It is created in autocommit before the data
/// transaction, so a failed CREATE leaves the connection usable.
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
        // Defer before deleting, and delete children before parents, so a
        // deferrable cycle and a parent closure can both be replaced.
        if !plan.constraints.deferred.is_empty() {
            sqlx::query("SET CONSTRAINTS ALL DEFERRED")
                .execute(&mut *session.connection())
                .await
                .map_err(connection_error)?;
        }
        for table in tables.iter().rev() {
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

    // Defer only when the plan proved every edge in the cycle is deferrable.
    if !plan.constraints.deferred.is_empty() {
        sqlx::query("SET CONSTRAINTS ALL DEFERRED")
            .execute(&mut *session.connection())
            .await
            .map_err(connection_error)?;
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

    // Force every deferred constraint to be checked before the run row is
    // written, so an inconsistent graph aborts the whole transaction.
    if !plan.constraints.deferred.is_empty() {
        sqlx::query("SET CONSTRAINTS ALL IMMEDIATE")
            .execute(&mut *session.connection())
            .await
            .map_err(connection_error)?;
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

/// Reconcile identity sequences after the row transaction has committed.
///
/// `setval` is not rolled back with the rows, so it runs after the commit as a
/// separate, idempotent step. The supported mode is deliberately restricted and
/// its safety is enforced, not assumed:
///
/// - an empty table, or a table whose imported identity values are not
///   positive, needs no movement and reports `not_required`;
/// - a sequence that already covers the imported maximum needs no movement and
///   reports `not_required`;
/// - a sequence behind the imported maximum is raised to that maximum and
///   reports `completed`;
/// - a sequence whose state cannot be read, whose lock cannot be taken (missing
///   privilege, lock timeout) or whose cache preallocates values is not touched
///   and reports `reconciliation_required`.
///
/// The read and the `setval` run inside one transaction that first changes the
/// sequence with `ALTER SEQUENCE ... CACHE <current>`. PostgreSQL documents that
/// `ALTER SEQUENCE` blocks concurrent `nextval`, `currval`, `lastval` and
/// `setval`, and the backend takes a `ShareRowExclusiveLock` on the sequence, so
/// while that lock is held the observed `last_value` cannot move between the
/// read and the write. The raise is `setval(seq, GREATEST(max, last_value),
/// true)`, so it never lowers the value it observed.
///
/// Only `CACHE 1` sequences are reconciled automatically. With a larger cache a
/// backend can hold a preallocated block whose values this session cannot see or
/// revoke, and the PostgreSQL documentation says such backends keep using their
/// cached values until they notice the change. A two-session test on PostgreSQL
/// 18 shows that the storage rewrite performed by the `ALTER` does discard those
/// blocks, but that is not part of the documented contract, so the mode refuses
/// instead of depending on it. The check is a snapshot taken before the lock: a
/// cache changed by an uncoordinated external writer mid-run is outside the
/// supported mode, which requires a quiescent destination.
pub async fn reconcile_sequences(
    session: &mut BoundSession,
    tables: &[TableRows],
    run_id: &str,
) -> Result<String, CoreError> {
    let mut raised = false;
    let mut unproven = false;

    for table in tables {
        for (index, column) in table.columns.iter().enumerate() {
            if !table.identity.get(index).copied().unwrap_or(false) {
                continue;
            }

            let qualified = format!("{}.{}", table.table.schema, table.table.table);
            let sequence: Option<String> =
                sqlx::query_scalar("SELECT pg_get_serial_sequence($1, $2)")
                    .bind(&qualified)
                    .bind(column)
                    .fetch_one(&mut *session.connection())
                    .await
                    .map_err(connection_error)?;
            let Some(sequence) = sequence else {
                continue;
            };

            let max: Option<i64> = sqlx::query_scalar(&format!(
                "SELECT MAX({})::bigint FROM {}.{}",
                quote_ident(column),
                quote_ident(&table.table.schema),
                quote_ident(&table.table.table)
            ))
            .fetch_one(&mut *session.connection())
            .await
            .map_err(connection_error)?;

            // An empty table, or a non-positive identity value, cannot collide
            // with a sequence that starts at 1. No movement is required, and
            // `setval(0)` is never attempted.
            let Some(max) = max.filter(|value| *value > 0) else {
                continue;
            };

            let state: Option<(String, String, i64)> = sqlx::query_as(
                "SELECT n.nspname::text, c.relname::text, s.seqcache
                 FROM pg_class c
                 JOIN pg_namespace n ON n.oid = c.relnamespace
                 JOIN pg_sequence s ON s.seqrelid = c.oid
                 WHERE c.oid = to_regclass($1)",
            )
            .bind(&sequence)
            .fetch_optional(&mut *session.connection())
            .await
            .map_err(connection_error)?;
            let Some((sequence_schema, sequence_name, cache)) = state else {
                // The sequence disappeared between discovery and reconciliation.
                unproven = true;
                continue;
            };

            // A sequence with a larger cache hands out preallocated blocks that
            // this session can neither observe nor revoke. PostgreSQL documents
            // that other backends keep using their cached values, so the mode
            // refuses rather than claiming a quiescence it cannot prove.
            if cache > 1 {
                unproven = true;
                continue;
            }

            let qualified_sequence = format!(
                "{}.{}",
                quote_ident(&sequence_schema),
                quote_ident(&sequence_name)
            );

            // One transaction per sequence: the lock is held until COMMIT, so
            // external writers are blocked for the read and the setval, and
            // released before the next sequence.
            sqlx::query("BEGIN")
                .execute(&mut *session.connection())
                .await
                .map_err(connection_error)?;
            // Bound the lock wait so a stuck external writer cannot hang the
            // run; a timeout is reported as reconciliation_required.
            sqlx::query("SET LOCAL lock_timeout = '5000'")
                .execute(&mut *session.connection())
                .await
                .map_err(connection_error)?;

            // The ALTER takes the ShareRowExclusiveLock and keeps the cache at
            // its current value, which the check above proved is 1.
            let locked = sqlx::query(&format!(
                "ALTER SEQUENCE {qualified_sequence} CACHE {cache}"
            ))
            .execute(&mut *session.connection())
            .await;
            if let Err(error) = locked {
                let _ = sqlx::query("ROLLBACK")
                    .execute(&mut *session.connection())
                    .await;
                if is_lock_or_privilege(&error) {
                    unproven = true;
                    continue;
                }
                return Err(connection_error(error));
            }

            let state = sqlx::query_as::<_, (i64, bool)>(&format!(
                "SELECT last_value, is_called FROM {qualified_sequence}"
            ))
            .fetch_optional(&mut *session.connection())
            .await;
            let state = match state {
                Ok(state) => state,
                Err(error) => {
                    let _ = sqlx::query("ROLLBACK")
                        .execute(&mut *session.connection())
                        .await;
                    return Err(connection_error(error));
                }
            };
            let Some((last, is_called)) = state else {
                let _ = sqlx::query("ROLLBACK")
                    .execute(&mut *session.connection())
                    .await;
                unproven = true;
                continue;
            };

            // An unused sequence is at its start value; treat it as below any
            // positive imported identity.
            let current = if is_called { last } else { 0 };
            if current < max {
                // Raise only. `GREATEST` keeps the value we observed, and the
                // lock guarantees no external writer moved it.
                let set = sqlx::query(&format!(
                    "SELECT setval($1::regclass, GREATEST($2::bigint, last_value), true)
                     FROM {qualified_sequence}"
                ))
                .bind(&sequence)
                .bind(max)
                .execute(&mut *session.connection())
                .await;
                if let Err(error) = set {
                    let _ = sqlx::query("ROLLBACK")
                        .execute(&mut *session.connection())
                        .await;
                    return Err(connection_error(error));
                }
                raised = true;
            }

            sqlx::query("COMMIT")
                .execute(&mut *session.connection())
                .await
                .map_err(connection_error)?;
        }
    }

    let status = if unproven {
        "reconciliation_required"
    } else if raised {
        "completed"
    } else {
        "not_required"
    };

    sqlx::query(&format!(
        "UPDATE {RUN_TABLE} SET sequence_status = $2 WHERE run_id = $1::uuid"
    ))
    .bind(run_id)
    .bind(status)
    .execute(&mut *session.connection())
    .await
    .map_err(connection_error)?;

    Ok(status.to_string())
}

/// True when a driver error means the sequence lock or privilege was not
/// available, so the safe action is to report `reconciliation_required`.
fn is_lock_or_privilege(error: &sqlx::Error) -> bool {
    error
        .as_database_error()
        .and_then(|database| database.code())
        .map(|code| matches!(code.as_ref(), "42501" | "55P03" | "57014"))
        .unwrap_or(false)
}

/// Read the persisted sequence status for a run.
///
/// Used when reconciliation fails, so the report reflects what the run row
/// actually says instead of assuming `pending`.
pub async fn persisted_sequence_status(
    session: &mut BoundSession,
    run_id: &str,
) -> Result<String, CoreError> {
    let status: Option<String> = sqlx::query_scalar(&format!(
        "SELECT sequence_status FROM {RUN_TABLE} WHERE run_id = $1::uuid"
    ))
    .bind(run_id)
    .fetch_optional(&mut *session.connection())
    .await
    .map_err(connection_error)?;
    Ok(status.unwrap_or_else(|| "pending".to_string()))
}

/// The columns a run table must have. A pre-S2-b table is migrated additively;
/// any other missing column is an incompatible schema and is refused.
const RUN_TABLE_COLUMNS: &[&str] = &[
    "run_id",
    "plan_fingerprint",
    "spec_hash",
    "source_identity_key",
    "destination_identity_key",
    "schema_hash",
    "masking_policy",
    "mask_key_id",
    "conflict_policy",
    "import_mode",
    "destination_digest",
    "rows_written",
    "status",
    "sequence_status",
    "finished_at",
];

/// Create the run table, or migrate an existing one additively.
///
/// `CREATE TABLE IF NOT EXISTS` does not add a column to an existing table, so
/// an S1 or S2-a run table would lack `sequence_status`. This runs under the
/// DDL advisory lock, so concurrent initializers are serialized, and it fails
/// closed on any other missing column instead of silently discarding state.
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
           sequence_status text NOT NULL DEFAULT 'pending',
           finished_at timestamptz NOT NULL DEFAULT now()
         )"
    ))
    .execute(&mut *session.connection())
    .await
    .map_err(connection_error)?;

    let existing: Vec<String> = sqlx::query_scalar(
        "SELECT a.attname::text
         FROM pg_attribute a
         JOIN pg_class c ON c.oid = a.attrelid
         JOIN pg_namespace n ON n.oid = c.relnamespace
         WHERE n.nspname = 'public' AND c.relname = 'dbpinball_safe_sync_runs'
           AND a.attnum > 0 AND NOT a.attisdropped",
    )
    .fetch_all(&mut *session.connection())
    .await
    .map_err(connection_error)?;

    if !existing.iter().any(|column| column == "sequence_status") {
        // Additive migration for an S1 or S2-a run table. Those milestones did
        // not import identity columns, so their historical runs need no
        // sequence movement; new runs still start `pending`. Both statements
        // run in one transaction so a partial failure cannot leave the unsafe
        // default in place.
        sqlx::query("BEGIN")
            .execute(&mut *session.connection())
            .await
            .map_err(connection_error)?;
        let migrated = async {
            sqlx::query(&format!(
                "ALTER TABLE {RUN_TABLE} ADD COLUMN sequence_status text NOT NULL DEFAULT 'not_required'"
            ))
            .execute(&mut *session.connection())
            .await
            .map_err(connection_error)?;
            sqlx::query(&format!(
                "ALTER TABLE {RUN_TABLE} ALTER COLUMN sequence_status SET DEFAULT 'pending'"
            ))
            .execute(&mut *session.connection())
            .await
            .map_err(connection_error)?;
            Ok::<(), CoreError>(())
        }
        .await;
        match migrated {
            Ok(()) => {
                sqlx::query("COMMIT")
                    .execute(&mut *session.connection())
                    .await
                    .map_err(connection_error)?;
            }
            Err(error) => {
                let _ = sqlx::query("ROLLBACK")
                    .execute(&mut *session.connection())
                    .await;
                return Err(error);
            }
        }
    }

    let missing: Vec<&str> = RUN_TABLE_COLUMNS
        .iter()
        .copied()
        .filter(|required| !existing.iter().any(|column| column == required))
        .filter(|required| *required != "sequence_status")
        .collect();
    if !missing.is_empty() {
        return Err(CoreError::with_detail(
            ErrorCode::Conflict,
            format!(
                "the run table has an incompatible schema; missing columns: {}",
                missing.join(", ")
            ),
            "run_table_incompatible",
        ));
    }

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
    let mut columns = Vec::new();
    let mut placeholders = Vec::new();
    let mut values = Vec::new();
    let mut overriding = false;

    for (index, column) in table.columns.iter().enumerate() {
        if table.generated.get(index).copied().unwrap_or(false) {
            // A generated column is recomputed by the destination.
            continue;
        }
        if table.identity_always.get(index).copied().unwrap_or(false) {
            overriding = true;
        }

        let cast = cast_for(&table.types[index])?;
        columns.push(quote_ident(column));
        placeholders.push(format!("CAST(${} AS {})", placeholders.len() + 1, cast));
        values.push(
            row.get(index)
                .and_then(MaskedValue::as_text)
                .map(str::to_string),
        );
    }

    if columns.is_empty() {
        return Ok(());
    }

    let overriding = if overriding {
        "OVERRIDING SYSTEM VALUE "
    } else {
        ""
    };

    let sql = format!(
        "INSERT INTO {}.{} ({}) {}VALUES ({})",
        quote_ident(&table.table.schema),
        quote_ident(&table.table.table),
        columns.join(", "),
        overriding,
        placeholders.join(", ")
    );

    let mut query = sqlx::query(&sql);
    for value in values {
        query = query.bind(value);
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
        "bigint" => Ok("bigint"),
        "integer" => Ok("integer"),
        "smallint" => Ok("smallint"),
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
    // A server error may echo a bound value, an identifier, or a constraint
    // detail. Only the SQLSTATE is propagated; the message is withheld.
    let code = error
        .as_database_error()
        .and_then(|database| database.code())
        .map(|code| code.to_string())
        .unwrap_or_else(|| "unknown".to_string());
    CoreError::with_detail(
        ErrorCode::Operation,
        format!("destination import failed (SQLSTATE {code}); the server message is withheld"),
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

    #[derive(Debug)]
    struct FakeDatabaseError {
        message: String,
        code: String,
    }

    impl std::fmt::Display for FakeDatabaseError {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str(&self.message)
        }
    }

    impl std::error::Error for FakeDatabaseError {}

    impl sqlx::error::DatabaseError for FakeDatabaseError {
        fn code(&self) -> Option<std::borrow::Cow<'_, str>> {
            Some(std::borrow::Cow::Borrowed(&self.code))
        }

        fn message(&self) -> &str {
            &self.message
        }

        fn kind(&self) -> sqlx::error::ErrorKind {
            sqlx::error::ErrorKind::Other
        }

        fn as_error(&self) -> &(dyn std::error::Error + Send + Sync + 'static) {
            self
        }

        fn as_error_mut(&mut self) -> &mut (dyn std::error::Error + Send + Sync + 'static) {
            self
        }

        fn into_error(self: Box<Self>) -> Box<dyn std::error::Error + Send + Sync + 'static> {
            self
        }
    }

    #[test]
    fn a_driver_error_never_leaks_the_server_message() {
        let canary = "CANARY-SECRET-VALUE";
        let error = sqlx::Error::Database(Box::new(FakeDatabaseError {
            message: format!("value {canary} rejected"),
            code: "22P02".into(),
        }));
        let core = connection_error(error);
        let rendered = format!("{:?} {} {:?}", core.code, core.message, core.detail);
        assert!(!rendered.contains(canary));
        assert!(core.message.contains("22P02"));
    }

    #[test]
    fn the_shared_database_error_never_leaks_the_server_message() {
        let canary = "CANARY-SECRET-VALUE";
        let error = sqlx::Error::Database(Box::new(FakeDatabaseError {
            message: format!("value {canary} rejected"),
            code: "42501".into(),
        }));
        let core = crate::safesync::database_error(
            ErrorCode::Connection,
            "source extraction",
            "source_error",
            error,
        );
        let rendered = format!("{:?} {} {:?}", core.code, core.message, core.detail);
        assert!(!rendered.contains(canary));
        assert!(core.message.contains("42501"));
    }

    #[test]
    fn a_sequence_lock_or_privilege_error_is_reconciliation_required() {
        for code in ["42501", "55P03", "57014"] {
            let error = sqlx::Error::Database(Box::new(FakeDatabaseError {
                message: "withheld".into(),
                code: code.into(),
            }));
            assert!(is_lock_or_privilege(&error), "{code} must be unproven");
        }
        let other = sqlx::Error::Database(Box::new(FakeDatabaseError {
            message: "withheld".into(),
            code: "42P01".into(),
        }));
        assert!(!is_lock_or_privilege(&other));
    }
}
