//! Transactional MariaDB InnoDB destination import, GET_LOCK concurrency,
//! and post-commit AUTO_INCREMENT reconciliation (MDB-5 & MDB-6).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use sha2::{Digest, Sha256};
use sqlx::Row;

use crate::domain::connection::DbEngine;
use crate::domain::error::{CoreError, ErrorCode};
use crate::safesync::import::{ConflictPolicy, ImportReport, TableImport};
use crate::safesync::mariadb_graph::inspect_mariadb_table_conn;
use crate::safesync::mask::{hex_lower, MaskedValue};
use crate::safesync::plan::DryRunPlan;
use crate::safesync::planner::{quote_ident_for, TableRows};
use crate::safesync::service::RunStatus;
use crate::safesync::session::BoundSession;
use crate::safesync::spec::QualifiedName;

const RUN_TABLE_NAME: &str = "dbpinball_safe_sync_runs";
const RUN_TABLE_LOCK_NAME: &str = "dbpinball_run_table_init";

fn q_mdb(ident: &str) -> String {
    quote_ident_for(DbEngine::Mysql, ident)
}

fn plan_lock_name(fingerprint: &str) -> String {
    let prefix: String = fingerprint.chars().take(32).collect();
    format!("dbpinball_sync_{prefix}")
}

async fn acquire_get_lock(session: &mut BoundSession, name: &str) -> Result<(), CoreError> {
    let conn = session.mysql_connection()?;
    let acquired: Option<i32> = sqlx::query_scalar("SELECT GET_LOCK(?, 5)")
        .bind(name)
        .fetch_one(&mut *conn)
        .await
        .map_err(connection_error)?;
    if acquired != Some(1) {
        return Err(CoreError::with_detail(
            ErrorCode::Conflict,
            "another Safe Sync run holds the MariaDB destination lock",
            "concurrent_run_in_progress",
        ));
    }
    Ok(())
}

async fn release_get_lock(session: &mut BoundSession, name: &str) {
    if let Ok(conn) = session.mysql_connection() {
        let _ = sqlx::query("SELECT RELEASE_LOCK(?)")
            .bind(name)
            .fetch_optional(&mut *conn)
            .await;
    }
}

/// Create the MariaDB run journal table outside any data transaction, under a
/// short session-scoped `GET_LOCK`.
async fn setup_run_table(session: &mut BoundSession, dst_db: &str) -> Result<(), CoreError> {
    acquire_get_lock(session, RUN_TABLE_LOCK_NAME).await?;
    let ddl = format!(
        "CREATE TABLE IF NOT EXISTS {}.{} (
            run_id VARCHAR(128) PRIMARY KEY,
            plan_fingerprint VARCHAR(128) NOT NULL,
            status VARCHAR(32) NOT NULL,
            rows_written BIGINT NOT NULL,
            destination_digest VARCHAR(128) NOT NULL,
            sequence_status VARCHAR(32) NOT NULL,
            started_at TIMESTAMP(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6),
            finished_at TIMESTAMP(6) NULL
        ) ENGINE=InnoDB",
        q_mdb(dst_db),
        q_mdb(RUN_TABLE_NAME),
    );
    let res = sqlx::query(&ddl)
        .execute(&mut *session.mysql_connection()?)
        .await
        .map_err(connection_error);
    release_get_lock(session, RUN_TABLE_LOCK_NAME).await;
    res.map(|_| ())
}

/// Import masked rows into a MariaDB InnoDB destination inside one atomic
/// transaction, guarded by a session-scoped `GET_LOCK`.
pub async fn run(
    session: &mut BoundSession,
    plan: &DryRunPlan,
    tables: &[TableRows],
    policy: ConflictPolicy,
    cancel: Arc<AtomicBool>,
) -> Result<ImportReport, CoreError> {
    let dst_db = session.identity().database_name.clone();
    setup_run_table(session, &dst_db).await?;

    let lock_name = plan_lock_name(&plan.plan_fingerprint);
    acquire_get_lock(session, &lock_name).await?;

    let tx_res = run_with_lock(session, &dst_db, plan, tables, policy, &cancel).await;
    release_get_lock(session, &lock_name).await;
    tx_res
}

async fn run_with_lock(
    session: &mut BoundSession,
    dst_db: &str,
    plan: &DryRunPlan,
    tables: &[TableRows],
    policy: ConflictPolicy,
    cancel: &Arc<AtomicBool>,
) -> Result<ImportReport, CoreError> {
    session.begin_destination_transaction().await?;
    match run_inner(session, dst_db, plan, tables, policy, cancel).await {
        Ok(report) => {
            sqlx::query("COMMIT")
                .execute(&mut *session.mysql_connection()?)
                .await
                .map_err(connection_error)?;
            Ok(report)
        }
        Err(error) => {
            let _ = sqlx::query("ROLLBACK")
                .execute(&mut *session.mysql_connection()?)
                .await;
            Err(error)
        }
    }
}

async fn run_inner(
    session: &mut BoundSession,
    dst_db: &str,
    plan: &DryRunPlan,
    tables: &[TableRows],
    policy: ConflictPolicy,
    cancel: &Arc<AtomicBool>,
) -> Result<ImportReport, CoreError> {
    if session.identity().identity_key != plan.destination.identity_key {
        return Err(CoreError::with_detail(
            ErrorCode::Conflict,
            "the destination identity changed since the plan was made",
            "destination_changed",
        ));
    }

    // Preflight every planned table on the destination: InnoDB, BASE TABLE,
    // zero triggers, and no schema drift.
    for planned in &plan.tables {
        let target = QualifiedName {
            schema: dst_db.to_string(),
            table: planned.table.clone(),
        };
        let live = inspect_mariadb_table_conn(session.mysql_connection()?, &target).await?;
        if live.schema_hash != planned.schema_hash {
            return Err(CoreError::with_detail(
                ErrorCode::Conflict,
                format!(
                    "the schema of {}.{} changed since the plan was made",
                    dst_db, planned.table
                ),
                "schema_drift",
            ));
        }
    }

    if cancel.load(Ordering::Relaxed) {
        return Err(cancelled());
    }

    let (current_digest, current_rows) = destination_digest(session, dst_db, tables).await?;

    if let Some((stored_digest, stored_rows, run_id)) =
        completed_run(session, dst_db, &plan.plan_fingerprint).await?
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
                        schema: dst_db.to_string(),
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

    // Use DELETE FROM in reverse topological order instead of TRUNCATE TABLE,
    // because TRUNCATE causes an implicit DDL COMMIT in MariaDB.
    if policy == ConflictPolicy::Replace {
        for table in tables.iter().rev() {
            let stmt = format!(
                "DELETE FROM {}.{}",
                q_mdb(dst_db),
                q_mdb(&table.table.table)
            );
            sqlx::query(&stmt)
                .execute(&mut *session.mysql_connection()?)
                .await
                .map_err(connection_error)?;
        }
    }

    let mut written_tables = Vec::with_capacity(tables.len());
    let mut rows_written = 0u64;

    for table in tables {
        if cancel.load(Ordering::Relaxed) {
            return Err(cancelled());
        }
        let count = insert_table(session, dst_db, table).await?;
        rows_written = rows_written.saturating_add(count);
        written_tables.push(TableImport {
            schema: dst_db.to_string(),
            table: table.table.table.clone(),
            rows: count,
        });
    }

    let (final_digest, final_rows) = destination_digest(session, dst_db, tables).await?;

    let insert_run = format!(
        "INSERT INTO {}.{} (run_id, plan_fingerprint, status, rows_written, destination_digest, sequence_status, finished_at) \
         VALUES (?, ?, 'completed', ?, ?, 'pending', CURRENT_TIMESTAMP(6)) \
         ON DUPLICATE KEY UPDATE \
           status = 'completed', \
           rows_written = VALUES(rows_written), \
           destination_digest = VALUES(destination_digest), \
           sequence_status = 'pending', \
           finished_at = CURRENT_TIMESTAMP(6)",
        q_mdb(dst_db),
        q_mdb(RUN_TABLE_NAME),
    );
    sqlx::query(&insert_run)
        .bind(&plan.run_id)
        .bind(&plan.plan_fingerprint)
        .bind(final_rows as i64)
        .bind(&final_digest)
        .execute(&mut *session.mysql_connection()?)
        .await
        .map_err(connection_error)?;

    Ok(ImportReport {
        run_id: plan.run_id.clone(),
        already_completed: false,
        rows_written,
        destination_digest: final_digest,
        tables: written_tables,
    })
}

async fn insert_table(
    session: &mut BoundSession,
    dst_db: &str,
    table: &TableRows,
) -> Result<u64, CoreError> {
    if table.rows.is_empty() {
        return Ok(0);
    }

    let mut column_indices = Vec::new();
    let mut columns = Vec::new();
    let mut placeholders = Vec::new();
    let mut col_types = Vec::new();

    for (index, column) in table.columns.iter().enumerate() {
        if table.generated.get(index).copied().unwrap_or(false) {
            continue;
        }
        let type_name = table.types.get(index).map(String::as_str).ok_or_else(|| {
            CoreError::with_detail(
                ErrorCode::Operation,
                "table metadata is missing column types",
                "missing_column_type",
            )
        })?;
        let cast = mariadb_cast_for(type_name)?;
        column_indices.push(index);
        columns.push(q_mdb(column));
        placeholders.push(cast.to_string());
        col_types.push(type_name.to_ascii_lowercase());
    }

    let sql = format!(
        "INSERT INTO {}.{} ({}) VALUES ({})",
        q_mdb(dst_db),
        q_mdb(&table.table.table),
        columns.join(", "),
        placeholders.join(", "),
    );

    let conn = session.mysql_connection()?;
    for row in &table.rows {
        let mut query = sqlx::query(&sql);
        for (pos, &col_idx) in column_indices.iter().enumerate() {
            let val = match row.get(col_idx) {
                Some(MaskedValue::Text(s)) => Some(normalize_mariadb_value(&col_types[pos], s)),
                Some(MaskedValue::Uuid(u)) => Some(u.clone()),
                Some(MaskedValue::Null) | None => None,
            };
            query = query.bind(val);
        }
        query.execute(&mut *conn).await.map_err(connection_error)?;
    }

    Ok(table.rows.len() as u64)
}

fn normalize_mariadb_value(type_name: &str, value: &str) -> String {
    match type_name {
        "datetime" | "timestamp" => value.strip_suffix('Z').unwrap_or(value).replace('T', " "),
        "boolean" | "bool" => match value.to_ascii_lowercase().as_str() {
            "true" | "t" | "1" => "1".to_string(),
            "false" | "f" | "0" => "0".to_string(),
            _ => value.to_string(),
        },
        _ => value.to_string(),
    }
}

fn mariadb_cast_for(type_name: &str) -> Result<&'static str, CoreError> {
    match type_name.to_ascii_lowercase().as_str() {
        "char" | "varchar" | "text" | "tinytext" | "mediumtext" | "longtext" | "uuid" => Ok("?"),
        "tinyint" | "smallint" | "mediumint" | "int" | "integer" | "bigint" | "boolean"
        | "bool" => Ok("CAST(? AS SIGNED)"),
        "decimal" | "numeric" => Ok("CAST(? AS DECIMAL(65, 30))"),
        "date" => Ok("CAST(? AS DATE)"),
        "datetime" | "timestamp" => Ok("CAST(? AS DATETIME(6))"),
        other => Err(CoreError::with_detail(
            ErrorCode::Unsupported,
            format!("MariaDB column type '{other}' is not supported in Safe Sync"),
            "unsupported_type",
        )),
    }
}

async fn destination_digest(
    session: &mut BoundSession,
    dst_db: &str,
    tables: &[TableRows],
) -> Result<(String, u64), CoreError> {
    let mut hasher = Sha256::new();
    hasher.update(b"dbpinball/safesync/mariadb-destination/v1\n");
    let mut total_rows = 0u64;

    let conn = session.mysql_connection()?;
    for table in tables {
        hasher.update(table.table.table.as_bytes());
        hasher.update(b"\n");

        let cols = table
            .columns
            .iter()
            .map(|c| format!("CAST({} AS CHAR CHARACTER SET utf8mb4)", q_mdb(c)))
            .collect::<Vec<_>>()
            .join(", ");
        let order_by = if table.row_identity.is_empty() {
            table
                .columns
                .iter()
                .map(|c| q_mdb(c))
                .collect::<Vec<_>>()
                .join(", ")
        } else {
            table
                .row_identity
                .iter()
                .map(|c| q_mdb(c))
                .collect::<Vec<_>>()
                .join(", ")
        };

        let sql = format!(
            "SELECT {} FROM {}.{} ORDER BY {}",
            cols,
            q_mdb(dst_db),
            q_mdb(&table.table.table),
            order_by,
        );
        let rows = sqlx::query(&sql)
            .fetch_all(&mut *conn)
            .await
            .map_err(connection_error)?;

        total_rows = total_rows.saturating_add(rows.len() as u64);
        for row in rows {
            for idx in 0..table.columns.len() {
                let val: Option<String> = row
                    .try_get::<Option<String>, _>(idx)
                    .ok()
                    .flatten()
                    .or_else(|| {
                        row.try_get::<Option<Vec<u8>>, _>(idx)
                            .ok()
                            .flatten()
                            .map(|b| String::from_utf8_lossy(&b).into_owned())
                    });
                match val {
                    Some(s) => {
                        hasher.update(b"V:");
                        hasher.update(s.len().to_string().as_bytes());
                        hasher.update(b":");
                        hasher.update(s.as_bytes());
                    }
                    None => hasher.update(b"N"),
                }
                hasher.update(b"|");
            }
            hasher.update(b"\n");
        }
    }

    Ok((
        format!("sha256:{}", hex_lower(&hasher.finalize())),
        total_rows,
    ))
}

async fn completed_run(
    session: &mut BoundSession,
    dst_db: &str,
    plan_fingerprint: &str,
) -> Result<Option<(String, u64, String)>, CoreError> {
    let sql = format!(
        "SELECT destination_digest, rows_written, run_id \
         FROM {}.{} \
         WHERE plan_fingerprint = ? AND status = 'completed' \
         ORDER BY finished_at DESC \
         LIMIT 1",
        q_mdb(dst_db),
        q_mdb(RUN_TABLE_NAME),
    );
    let row = sqlx::query(&sql)
        .bind(plan_fingerprint)
        .fetch_optional(&mut *session.mysql_connection()?)
        .await
        .map_err(connection_error)?;

    let Some(row) = row else {
        return Ok(None);
    };
    let digest: String = row.try_get(0).unwrap_or_default();
    let rows: i64 = row.try_get(1).unwrap_or(0);
    let run_id: String = row.try_get(2).unwrap_or_default();
    Ok(Some((digest, rows.max(0) as u64, run_id)))
}

/// Reconcile `AUTO_INCREMENT` counters on MariaDB destination tables **after**
/// the data transaction commits (because `ALTER TABLE ... AUTO_INCREMENT = N`
/// triggers an implicit DDL commit in MariaDB).
pub async fn reconcile_sequences(
    session: &mut BoundSession,
    tables: &[TableRows],
    run_id: &str,
) -> Result<String, CoreError> {
    let dst_db = session.identity().database_name.clone();
    let lock_name = format!(
        "dbpinball_ai_{}",
        &run_id.chars().take(32).collect::<String>()
    );
    acquire_get_lock(session, &lock_name).await?;

    let result = reconcile_sequences_inner(session, &dst_db, tables, run_id).await;
    release_get_lock(session, &lock_name).await;
    result
}

async fn reconcile_sequences_inner(
    session: &mut BoundSession,
    dst_db: &str,
    tables: &[TableRows],
    run_id: &str,
) -> Result<String, CoreError> {
    let mut required = false;
    let conn = session.mysql_connection()?;

    for table in tables {
        let Some((idx, col)) = table
            .columns
            .iter()
            .enumerate()
            .find(|(i, _)| table.identity.get(*i).copied().unwrap_or(false))
        else {
            continue;
        };
        let _ = idx;
        required = true;

        let max_sql = format!(
            "SELECT CAST(COALESCE(MAX({}), 0) AS SIGNED) FROM {}.{}",
            q_mdb(col),
            q_mdb(dst_db),
            q_mdb(&table.table.table),
        );
        let max_val: i64 = sqlx::query_scalar(&max_sql)
            .fetch_one(&mut *conn)
            .await
            .map_err(connection_error)?;

        if max_val <= 0 {
            continue;
        }
        let next_id = max_val.checked_add(1).ok_or_else(|| {
            CoreError::with_detail(
                ErrorCode::Operation,
                "AUTO_INCREMENT counter overflowed i64",
                "unsafe_auto_increment",
            )
        })? as u64;

        let cur_ai: Option<u64> = sqlx::query_scalar(
            "SELECT CAST(AUTO_INCREMENT AS UNSIGNED) \
             FROM information_schema.TABLES \
             WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ?",
        )
        .bind(dst_db)
        .bind(&table.table.table)
        .fetch_optional(&mut *conn)
        .await
        .map_err(connection_error)?
        .flatten();

        if cur_ai.unwrap_or(0) <= max_val as u64 {
            let alter_sql = format!(
                "ALTER TABLE {}.{} AUTO_INCREMENT = {}",
                q_mdb(dst_db),
                q_mdb(&table.table.table),
                next_id,
            );
            sqlx::query(&alter_sql)
                .execute(&mut *conn)
                .await
                .map_err(connection_error)?;
        }
    }

    let status = if required {
        "completed"
    } else {
        "not_required"
    };
    let update_sql = format!(
        "UPDATE {}.{} SET sequence_status = ? WHERE run_id = ?",
        q_mdb(dst_db),
        q_mdb(RUN_TABLE_NAME),
    );
    sqlx::query(&update_sql)
        .bind(status)
        .bind(run_id)
        .execute(&mut *conn)
        .await
        .map_err(connection_error)?;

    Ok(status.to_string())
}

/// Read back persisted sequence reconciliation status for a MariaDB run.
pub async fn persisted_sequence_status(
    session: &mut BoundSession,
    run_id: &str,
) -> Result<String, CoreError> {
    let dst_db = session.identity().database_name.clone();
    let sql = format!(
        "SELECT sequence_status FROM {}.{} WHERE run_id = ? LIMIT 1",
        q_mdb(&dst_db),
        q_mdb(RUN_TABLE_NAME),
    );
    let status: Option<String> = sqlx::query_scalar(&sql)
        .bind(run_id)
        .fetch_optional(&mut *session.mysql_connection()?)
        .await
        .map_err(connection_error)?;
    Ok(status.unwrap_or_else(|| "pending".to_string()))
}

/// Read persisted run status for `safe-sync status` on MariaDB.
pub async fn read_run_status(
    session: &mut BoundSession,
    run_id: &str,
) -> Result<RunStatus, CoreError> {
    let dst_db = session.identity().database_name.clone();
    let sql = format!(
        "SELECT run_id, plan_fingerprint, status, rows_written, destination_digest, sequence_status, \
                COALESCE(CAST(finished_at AS CHAR CHARACTER SET utf8mb4), '') \
         FROM {}.{} WHERE run_id = ? LIMIT 1",
        q_mdb(&dst_db),
        q_mdb(RUN_TABLE_NAME),
    );
    let row = sqlx::query(&sql)
        .bind(run_id)
        .fetch_optional(&mut *session.mysql_connection()?)
        .await
        .map_err(connection_error)?;

    let Some(row) = row else {
        return Err(CoreError::with_detail(
            ErrorCode::NotFound,
            format!("Safe Sync run '{run_id}' was not found"),
            "run_not_found",
        ));
    };

    Ok(RunStatus {
        run_id: row.try_get(0).unwrap_or_default(),
        plan_fingerprint: row.try_get(1).unwrap_or_default(),
        status: row.try_get(2).unwrap_or_default(),
        rows_written: row.try_get::<i64, _>(3).unwrap_or(0).max(0) as u64,
        destination_digest: row.try_get(4).unwrap_or_default(),
        sequence_status: row.try_get(5).unwrap_or_default(),
        finished_at: row.try_get(6).unwrap_or_default(),
    })
}

fn cancelled() -> CoreError {
    CoreError::with_detail(ErrorCode::Operation, "the run was cancelled", "cancelled")
}

fn connection_error(error: sqlx::Error) -> CoreError {
    let code = error
        .as_database_error()
        .and_then(|database| database.code())
        .map(|code| code.to_string())
        .unwrap_or_else(|| "unknown".to_string());
    CoreError::with_detail(
        ErrorCode::Operation,
        format!(
            "MariaDB destination import failed (SQLSTATE {code}); the server message is withheld"
        ),
        "destination_import_failed",
    )
}
