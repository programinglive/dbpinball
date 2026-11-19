//! Full database copy (schema + data) from a read-only source session
//! to an enrolled local/dev destination.
//!
//! Uses the exact same `BoundSession` and M3 security policy as Safe Sync
//! subset execution, so production targets and SSH-tunneled destinations are
//! always refused.

use futures_util::TryStreamExt;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use std::time::Instant;

use crate::domain::connection::{DbConnection, DbEngine};
use crate::domain::error::{CoreError, ErrorCode};
use crate::ports::credentials::CredentialProvider;
use crate::safesync::database_error;
use crate::safesync::planner::quote_ident_for;
use crate::safesync::service::{assert_same_engine, authorize};
use crate::safesync::session::{BoundSession, SessionRole};
use crate::safesync::spec::Limits;
use crate::security::EnrollmentStore;

const BATCH_SIZE: usize = 1000;
const MAX_BATCH_BYTES: usize = 2 * 1024 * 1024;

/// Per-table result reported after a full copy run.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FullCopyTableResult {
    pub table: String,
    pub rows_copied: u64,
}

/// Live progress snapshot emitted during [`full_copy_with_progress`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FullCopyProgress {
    pub phase: String,
    pub current_table: String,
    pub tables_done: usize,
    pub total_tables: usize,
    pub rows_copied: u64,
    pub percent: u8,
}

/// Summary report returned by [`full_copy`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FullCopyReport {
    pub engine: DbEngine,
    pub source_database: String,
    pub destination_database: String,
    pub tables_copied: usize,
    pub rows_copied: u64,
    pub replaced: bool,
    pub duration_ms: u64,
    pub tables: Vec<FullCopyTableResult>,
}

/// Copy all base tables (schema and rows) from `source` to `destination`.
///
/// Enforces:
/// - `source` and `destination` must share the same [`DbEngine`];
/// - `destination` must not use an SSH tunnel;
/// - `destination` must pass M3 `SafeSyncExecute` authorization (enrolled
///   `verified_development`, non-production, distinct from `source`).
pub async fn full_copy(
    source: &DbConnection,
    destination: &DbConnection,
    credentials: &dyn CredentialProvider,
    enrollments: &EnrollmentStore,
    now_ms: u64,
    replace: bool,
) -> Result<FullCopyReport, CoreError> {
    full_copy_with_progress(
        source,
        destination,
        credentials,
        enrollments,
        now_ms,
        replace,
        |_| {},
    )
    .await
}

/// Copy all base tables (schema and rows) from `source` to `destination`,
/// invoking `on_progress` as tables and batches are copied.
pub async fn full_copy_with_progress<F>(
    source: &DbConnection,
    destination: &DbConnection,
    credentials: &dyn CredentialProvider,
    enrollments: &EnrollmentStore,
    now_ms: u64,
    replace: bool,
    mut on_progress: F,
) -> Result<FullCopyReport, CoreError>
where
    F: FnMut(FullCopyProgress) + Send,
{
    assert_same_engine(source, destination)?;

    if destination.ssh_tunnel.as_ref().is_some_and(|t| t.enabled) {
        return Err(CoreError::with_detail(
            ErrorCode::PolicyRefusal,
            "Full Copy destination cannot use an SSH tunnel; only local destinations are allowed",
            "destination_ssh_tunnel_forbidden",
        ));
    }

    let limits = Limits::default();
    let mut source_session =
        BoundSession::open(SessionRole::Source, source, credentials, &limits).await?;
    let mut destination_session =
        BoundSession::open(SessionRole::Destination, destination, credentials, &limits).await?;

    let registry = enrollments.load()?;
    let enrollment = registry.get(&destination.id);

    authorize(
        &source_session,
        &destination_session,
        destination,
        enrollment,
        now_ms,
    )?;

    let source_database = source_session.identity().database_name.clone();
    let destination_database = destination_session.identity().database_name.clone();
    let started = Instant::now();

    let tables = match source.engine {
        DbEngine::Mysql => {
            if let Ok(src) = source_session.mysql_connection() {
                let _ = sqlx::query("SET SESSION max_statement_time = 0")
                    .execute(&mut *src)
                    .await;
            }
            copy_mysql(
                &mut source_session,
                &mut destination_session,
                replace,
                &mut on_progress,
            )
            .await?
        }
        DbEngine::Postgres => {
            if let Ok(src) = source_session.pg_connection() {
                let _ = sqlx::query("SET statement_timeout = 0")
                    .execute(&mut *src)
                    .await;
            }
            copy_postgres(
                &mut source_session,
                &mut destination_session,
                replace,
                &mut on_progress,
            )
            .await?
        }
    };

    let rows_copied = tables.iter().map(|t| t.rows_copied).sum();
    let tables_copied = tables.len();

    on_progress(FullCopyProgress {
        phase: "completed".to_string(),
        current_table: String::new(),
        tables_done: tables_copied,
        total_tables: tables_copied,
        rows_copied,
        percent: 100,
    });

    let _ = source_session.rollback().await;

    Ok(FullCopyReport {
        engine: source.engine,
        source_database,
        destination_database,
        tables_copied,
        rows_copied,
        replaced: replace,
        duration_ms: started.elapsed().as_millis() as u64,
        tables,
    })
}

fn q_mysql(ident: &str) -> String {
    quote_ident_for(DbEngine::Mysql, ident)
}

fn q_pg(ident: &str) -> String {
    quote_ident_for(DbEngine::Postgres, ident)
}

fn escape_mysql_str(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('\'');
    for ch in value.chars() {
        match ch {
            '\0' => out.push_str("\\0"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            '"' => out.push_str("\\\""),
            '\x1a' => out.push_str("\\Z"),
            other => out.push(other),
        }
    }
    out.push('\'');
    out
}

fn escape_pg_str(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('\'');
    for ch in value.chars() {
        if ch == '\'' {
            out.push_str("''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MysqlColumnEncoding {
    Text,
    Binary,
    Bit,
}

fn mysql_column_encoding(data_type: &str) -> MysqlColumnEncoding {
    let lower = data_type.to_ascii_lowercase();
    if lower == "bit" {
        return MysqlColumnEncoding::Bit;
    }
    if matches!(
        lower.as_str(),
        "binary"
            | "varbinary"
            | "tinyblob"
            | "blob"
            | "mediumblob"
            | "longblob"
            | "geometry"
            | "point"
            | "linestring"
            | "polygon"
            | "multipoint"
            | "multilinestring"
            | "multipolygon"
            | "geometrycollection"
    ) {
        return MysqlColumnEncoding::Binary;
    }
    MysqlColumnEncoding::Text
}

fn format_mysql_literal(encoding: MysqlColumnEncoding, val: Option<&str>) -> String {
    match (encoding, val) {
        (_, None) => "NULL".to_string(),
        (MysqlColumnEncoding::Bit, Some(s)) => format!("b'{s}'"),
        (MysqlColumnEncoding::Binary, Some(s)) => {
            if s.len() % 2 == 1 {
                format!("X'0{s}'")
            } else {
                format!("X'{s}'")
            }
        }
        (MysqlColumnEncoding::Text, Some(s)) => escape_mysql_str(s),
    }
}

struct MysqlColumn {
    name: String,
    encoding: MysqlColumnEncoding,
}

async fn copy_mysql<F>(
    source_session: &mut BoundSession,
    destination_session: &mut BoundSession,
    replace: bool,
    on_progress: &mut F,
) -> Result<Vec<FullCopyTableResult>, CoreError>
where
    F: FnMut(FullCopyProgress) + Send,
{
    let table_names: Vec<String> = {
        let conn = source_session.mysql_connection()?;
        sqlx::query_scalar(
            "SELECT TABLE_NAME FROM information_schema.TABLES \
             WHERE TABLE_SCHEMA = DATABASE() AND TABLE_TYPE = 'BASE TABLE' \
             ORDER BY TABLE_NAME",
        )
        .fetch_all(conn)
        .await
        .map_err(|e| {
            database_error(
                ErrorCode::Operation,
                "list MySQL tables",
                "full_copy_list_failed",
                e,
            )
        })?
    };

    let total_tables = table_names.len();
    let mut total_rows: u64 = 0;
    on_progress(FullCopyProgress {
        phase: "discovering".to_string(),
        current_table: String::new(),
        tables_done: 0,
        total_tables,
        rows_copied: 0,
        percent: 0,
    });

    {
        let dst = destination_session.mysql_connection()?;
        sqlx::query("SET FOREIGN_KEY_CHECKS = 0")
            .execute(&mut *dst)
            .await
            .map_err(|e| {
                database_error(
                    ErrorCode::Operation,
                    "disable FK checks",
                    "full_copy_fk_failed",
                    e,
                )
            })?;
        sqlx::query("SET SQL_MODE = 'NO_AUTO_VALUE_ON_ZERO'")
            .execute(&mut *dst)
            .await
            .map_err(|e| {
                database_error(
                    ErrorCode::Operation,
                    "set SQL_MODE",
                    "full_copy_mode_failed",
                    e,
                )
            })?;
        sqlx::query("SET autocommit = 0")
            .execute(&mut *dst)
            .await
            .map_err(|e| {
                database_error(
                    ErrorCode::Operation,
                    "disable autocommit",
                    "full_copy_autocommit_failed",
                    e,
                )
            })?;
    }

    let mut results = Vec::with_capacity(table_names.len());

    for (table_idx, table) in table_names.into_iter().enumerate() {
        let q_table = q_mysql(&table);
        let percent = if total_tables == 0 {
            0
        } else {
            ((table_idx * 100) / total_tables).min(99) as u8
        };
        on_progress(FullCopyProgress {
            phase: "copying".to_string(),
            current_table: table.clone(),
            tables_done: table_idx,
            total_tables,
            rows_copied: total_rows,
            percent,
        });
        let create_sql: String = {
            let src = source_session.mysql_connection()?;
            let row = sqlx::query(&format!("SHOW CREATE TABLE {q_table}"))
                .fetch_one(src)
                .await
                .map_err(|e| {
                    database_error(
                        ErrorCode::Operation,
                        &format!("SHOW CREATE TABLE {table}"),
                        "full_copy_ddl_failed",
                        e,
                    )
                })?;
            row.try_get::<String, _>(1).map_err(|e| {
                database_error(
                    ErrorCode::Operation,
                    &format!("read CREATE TABLE for {table}"),
                    "full_copy_ddl_failed",
                    e,
                )
            })?
        };

        {
            let dst = destination_session.mysql_connection()?;
            if replace {
                sqlx::query(&format!("DROP TABLE IF EXISTS {q_table}"))
                    .execute(&mut *dst)
                    .await
                    .map_err(|e| {
                        database_error(
                            ErrorCode::Operation,
                            &format!("DROP TABLE {table}"),
                            "full_copy_drop_failed",
                            e,
                        )
                    })?;
                sqlx::query(&create_sql)
                    .execute(&mut *dst)
                    .await
                    .map_err(|e| {
                        database_error(
                            ErrorCode::Operation,
                            &format!("CREATE TABLE {table}"),
                            "full_copy_create_failed",
                            e,
                        )
                    })?;
            } else {
                let create_if_not_exists =
                    if let Some(rest) = create_sql.strip_prefix("CREATE TABLE ") {
                        format!("CREATE TABLE IF NOT EXISTS {rest}")
                    } else {
                        create_sql.clone()
                    };
                sqlx::query(&create_if_not_exists)
                    .execute(&mut *dst)
                    .await
                    .map_err(|e| {
                        database_error(
                            ErrorCode::Operation,
                            &format!("CREATE TABLE IF NOT EXISTS {table}"),
                            "full_copy_create_failed",
                            e,
                        )
                    })?;
            }
        }

        let columns: Vec<MysqlColumn> = {
            let src = source_session.mysql_connection()?;
            let rows = sqlx::query(
                "SELECT COLUMN_NAME, DATA_TYPE, EXTRA \
                 FROM information_schema.COLUMNS \
                 WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = ? \
                 ORDER BY ORDINAL_POSITION",
            )
            .bind(&table)
            .fetch_all(src)
            .await
            .map_err(|e| {
                database_error(
                    ErrorCode::Operation,
                    &format!("list columns for {table}"),
                    "full_copy_columns_failed",
                    e,
                )
            })?;

            rows.into_iter()
                .filter_map(|row| {
                    let name: String = row.try_get(0).ok()?;
                    let data_type: String = row.try_get(1).ok()?;
                    let extra: String = row.try_get::<String, _>(2).unwrap_or_default();
                    if extra.to_ascii_uppercase().contains("GENERATED") {
                        return None;
                    }
                    Some(MysqlColumn {
                        name,
                        encoding: mysql_column_encoding(&data_type),
                    })
                })
                .collect()
        };

        if columns.is_empty() {
            results.push(FullCopyTableResult {
                table,
                rows_copied: 0,
            });
            continue;
        }

        let select_exprs: Vec<String> = columns
            .iter()
            .map(|c| {
                let qc = q_mysql(&c.name);
                match c.encoding {
                    MysqlColumnEncoding::Bit => format!("BIN({qc})"),
                    MysqlColumnEncoding::Binary => format!("HEX({qc})"),
                    MysqlColumnEncoding::Text => {
                        format!("CAST({qc} AS CHAR CHARACTER SET utf8mb4)")
                    }
                }
            })
            .collect();
        let insert_cols: Vec<String> = columns.iter().map(|c| q_mysql(&c.name)).collect();
        let insert_prefix = if replace {
            format!("INSERT INTO {q_table} ({}) VALUES ", insert_cols.join(", "))
        } else {
            format!(
                "INSERT IGNORE INTO {q_table} ({}) VALUES ",
                insert_cols.join(", ")
            )
        };

        let mut table_rows: u64 = 0;
        let select_sql = format!("SELECT {} FROM {q_table}", select_exprs.join(", "));
        let mut value_tuples: Vec<String> = Vec::with_capacity(BATCH_SIZE);
        let mut batch_bytes: usize = 0;

        let src = source_session.mysql_connection()?;
        let mut stream = sqlx::query(&select_sql).fetch(src);

        while let Some(row) = stream.try_next().await.map_err(|e| {
            database_error(
                ErrorCode::Operation,
                &format!("select rows from {table}"),
                "full_copy_select_failed",
                e,
            )
        })? {
            let mut vals = Vec::with_capacity(columns.len());
            for (idx, col) in columns.iter().enumerate() {
                let val: Option<String> = row.try_get(idx).map_err(|e| {
                    database_error(
                        ErrorCode::Operation,
                        &format!("decode column {} in {table}", col.name),
                        "full_copy_decode_failed",
                        e,
                    )
                })?;
                vals.push(format_mysql_literal(col.encoding, val.as_deref()));
            }
            let tuple = format!("({})", vals.join(", "));
            batch_bytes += tuple.len() + 2;
            value_tuples.push(tuple);

            if value_tuples.len() >= BATCH_SIZE || batch_bytes >= MAX_BATCH_BYTES {
                let batch_len = value_tuples.len() as u64;
                let insert_sql = format!("{insert_prefix}{}", value_tuples.join(", "));
                let dst = destination_session.mysql_connection()?;
                sqlx::query(&insert_sql)
                    .execute(&mut *dst)
                    .await
                    .map_err(|e| {
                        database_error(
                            ErrorCode::Operation,
                            &format!("insert rows into {table}"),
                            "full_copy_insert_failed",
                            e,
                        )
                    })?;
                value_tuples.clear();
                batch_bytes = 0;
                table_rows += batch_len;
                total_rows += batch_len;
                on_progress(FullCopyProgress {
                    phase: "copying".to_string(),
                    current_table: table.clone(),
                    tables_done: table_idx,
                    total_tables,
                    rows_copied: total_rows,
                    percent,
                });
            }
        }
        drop(stream);

        if !value_tuples.is_empty() {
            let batch_len = value_tuples.len() as u64;
            let insert_sql = format!("{insert_prefix}{}", value_tuples.join(", "));
            let dst = destination_session.mysql_connection()?;
            sqlx::query(&insert_sql)
                .execute(&mut *dst)
                .await
                .map_err(|e| {
                    database_error(
                        ErrorCode::Operation,
                        &format!("insert rows into {table}"),
                        "full_copy_insert_failed",
                        e,
                    )
                })?;
            table_rows += batch_len;
            total_rows += batch_len;
            on_progress(FullCopyProgress {
                phase: "copying".to_string(),
                current_table: table.clone(),
                tables_done: table_idx,
                total_tables,
                rows_copied: total_rows,
                percent,
            });
        }

        {
            let dst = destination_session.mysql_connection()?;
            sqlx::query("COMMIT")
                .execute(&mut *dst)
                .await
                .map_err(|e| {
                    database_error(
                        ErrorCode::Operation,
                        &format!("commit {table}"),
                        "full_copy_commit_failed",
                        e,
                    )
                })?;
        }

        results.push(FullCopyTableResult {
            table,
            rows_copied: table_rows,
        });
    }

    {
        let dst = destination_session.mysql_connection()?;
        let _ = sqlx::query("SET autocommit = 1").execute(&mut *dst).await;
        let _ = sqlx::query("SET FOREIGN_KEY_CHECKS = 1")
            .execute(&mut *dst)
            .await;
    }

    Ok(results)
}

struct PgColumn {
    name: String,
    type_name: String,
    is_not_null: bool,
    is_bytea: bool,
}

async fn copy_postgres<F>(
    source_session: &mut BoundSession,
    destination_session: &mut BoundSession,
    replace: bool,
    on_progress: &mut F,
) -> Result<Vec<FullCopyTableResult>, CoreError>
where
    F: FnMut(FullCopyProgress) + Send,
{
    let tables: Vec<(String, String)> = {
        let src = source_session.pg_connection()?;
        let rows = sqlx::query(
            "SELECT table_schema, table_name \
             FROM information_schema.tables \
             WHERE table_schema NOT IN ('pg_catalog', 'information_schema') \
               AND table_type = 'BASE TABLE' \
             ORDER BY table_schema, table_name",
        )
        .fetch_all(src)
        .await
        .map_err(|e| {
            database_error(
                ErrorCode::Operation,
                "list PostgreSQL tables",
                "full_copy_list_failed",
                e,
            )
        })?;

        rows.into_iter()
            .filter_map(|r| Some((r.try_get(0).ok()?, r.try_get(1).ok()?)))
            .collect()
    };

    let total_tables = tables.len();
    let mut total_rows: u64 = 0;
    on_progress(FullCopyProgress {
        phase: "discovering".to_string(),
        current_table: String::new(),
        tables_done: 0,
        total_tables,
        rows_copied: 0,
        percent: 0,
    });

    let mut results = Vec::with_capacity(total_tables);

    for (table_idx, (schema, table)) in tables.into_iter().enumerate() {
        let full_name = format!("{}.{}", q_pg(&schema), q_pg(&table));
        let display_name = format!("{schema}.{table}");
        let percent = if total_tables == 0 {
            0
        } else {
            ((table_idx * 100) / total_tables).min(99) as u8
        };
        on_progress(FullCopyProgress {
            phase: "copying".to_string(),
            current_table: display_name.clone(),
            tables_done: table_idx,
            total_tables,
            rows_copied: total_rows,
            percent,
        });

        let columns: Vec<PgColumn> = {
            let src = source_session.pg_connection()?;
            let rows = sqlx::query(
                "SELECT a.attname, \
                        pg_catalog.format_type(a.atttypid, a.atttypmod) AS formatted_type, \
                        a.attnotnull \
                 FROM pg_catalog.pg_attribute a \
                 JOIN pg_catalog.pg_class c ON c.oid = a.attrelid \
                 JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
                 WHERE n.nspname = $1 AND c.relname = $2 \
                   AND a.attnum > 0 AND NOT a.attisdropped \
                   AND a.attgenerated = '' \
                 ORDER BY a.attnum",
            )
            .bind(&schema)
            .bind(&table)
            .fetch_all(src)
            .await
            .map_err(|e| {
                database_error(
                    ErrorCode::Operation,
                    &format!("list columns for {display_name}"),
                    "full_copy_columns_failed",
                    e,
                )
            })?;

            rows.into_iter()
                .filter_map(|r| {
                    let name: String = r.try_get(0).ok()?;
                    let type_name: String = r.try_get(1).ok()?;
                    let is_not_null: bool = r.try_get(2).unwrap_or(false);
                    let is_bytea = type_name.eq_ignore_ascii_case("bytea");
                    Some(PgColumn {
                        name,
                        type_name,
                        is_not_null,
                        is_bytea,
                    })
                })
                .collect()
        };

        if columns.is_empty() {
            results.push(FullCopyTableResult {
                table: display_name,
                rows_copied: 0,
            });
            continue;
        }

        let col_defs: Vec<String> = columns
            .iter()
            .map(|c| {
                let nn = if c.is_not_null { " NOT NULL" } else { "" };
                format!("{} {}{}", q_pg(&c.name), c.type_name, nn)
            })
            .collect();

        {
            let dst = destination_session.pg_connection()?;
            sqlx::query(&format!("CREATE SCHEMA IF NOT EXISTS {}", q_pg(&schema)))
                .execute(&mut *dst)
                .await
                .map_err(|e| {
                    database_error(
                        ErrorCode::Operation,
                        &format!("CREATE SCHEMA {schema}"),
                        "full_copy_schema_failed",
                        e,
                    )
                })?;

            if replace {
                sqlx::query(&format!("DROP TABLE IF EXISTS {full_name} CASCADE"))
                    .execute(&mut *dst)
                    .await
                    .map_err(|e| {
                        database_error(
                            ErrorCode::Operation,
                            &format!("DROP TABLE {display_name}"),
                            "full_copy_drop_failed",
                            e,
                        )
                    })?;
            }

            sqlx::query(&format!(
                "CREATE TABLE IF NOT EXISTS {full_name} ({})",
                col_defs.join(", ")
            ))
            .execute(&mut *dst)
            .await
            .map_err(|e| {
                database_error(
                    ErrorCode::Operation,
                    &format!("CREATE TABLE {display_name}"),
                    "full_copy_create_failed",
                    e,
                )
            })?;
        }

        let select_exprs: Vec<String> = columns
            .iter()
            .map(|c| {
                let qc = q_pg(&c.name);
                if c.is_bytea {
                    format!("encode({qc}, 'hex')")
                } else {
                    format!("{qc}::text")
                }
            })
            .collect();
        let insert_cols: Vec<String> = columns.iter().map(|c| q_pg(&c.name)).collect();
        let insert_prefix = format!(
            "INSERT INTO {full_name} ({}) VALUES ",
            insert_cols.join(", ")
        );

        let mut table_rows: u64 = 0;
        let select_sql = format!("SELECT {} FROM {full_name}", select_exprs.join(", "));
        let mut value_tuples: Vec<String> = Vec::with_capacity(BATCH_SIZE);
        let mut batch_bytes: usize = 0;

        let src = source_session.pg_connection()?;
        let mut stream = sqlx::query(&select_sql).fetch(src);

        while let Some(row) = stream.try_next().await.map_err(|e| {
            database_error(
                ErrorCode::Operation,
                &format!("select rows from {display_name}"),
                "full_copy_select_failed",
                e,
            )
        })? {
            let mut vals = Vec::with_capacity(columns.len());
            for (idx, col) in columns.iter().enumerate() {
                let val: Option<String> = row.try_get(idx).map_err(|e| {
                    database_error(
                        ErrorCode::Operation,
                        &format!("decode column {} in {display_name}", col.name),
                        "full_copy_decode_failed",
                        e,
                    )
                })?;
                match val {
                    None => vals.push("NULL".to_string()),
                    Some(s) if col.is_bytea => vals.push(format!("decode('{}', 'hex')", s)),
                    Some(s) => vals.push(format!("{}::{}", escape_pg_str(&s), col.type_name)),
                }
            }
            let tuple = format!("({})", vals.join(", "));
            batch_bytes += tuple.len() + 2;
            value_tuples.push(tuple);

            if value_tuples.len() >= BATCH_SIZE || batch_bytes >= MAX_BATCH_BYTES {
                let batch_len = value_tuples.len() as u64;
                let insert_sql = format!("{insert_prefix}{}", value_tuples.join(", "));
                let dst = destination_session.pg_connection()?;
                sqlx::query(&insert_sql)
                    .execute(&mut *dst)
                    .await
                    .map_err(|e| {
                        database_error(
                            ErrorCode::Operation,
                            &format!("insert rows into {display_name}"),
                            "full_copy_insert_failed",
                            e,
                        )
                    })?;
                value_tuples.clear();
                batch_bytes = 0;
                table_rows += batch_len;
                total_rows += batch_len;
                on_progress(FullCopyProgress {
                    phase: "copying".to_string(),
                    current_table: display_name.clone(),
                    tables_done: table_idx,
                    total_tables,
                    rows_copied: total_rows,
                    percent,
                });
            }
        }
        drop(stream);

        if !value_tuples.is_empty() {
            let batch_len = value_tuples.len() as u64;
            let insert_sql = format!("{insert_prefix}{}", value_tuples.join(", "));
            let dst = destination_session.pg_connection()?;
            sqlx::query(&insert_sql)
                .execute(&mut *dst)
                .await
                .map_err(|e| {
                    database_error(
                        ErrorCode::Operation,
                        &format!("insert rows into {display_name}"),
                        "full_copy_insert_failed",
                        e,
                    )
                })?;
            table_rows += batch_len;
            total_rows += batch_len;
            on_progress(FullCopyProgress {
                phase: "copying".to_string(),
                current_table: display_name.clone(),
                tables_done: table_idx,
                total_tables,
                rows_copied: total_rows,
                percent,
            });
        }

        results.push(FullCopyTableResult {
            table: display_name,
            rows_copied: table_rows,
        });
    }

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mysql_bit_and_binary_literals_are_valid_sql() {
        assert_eq!(mysql_column_encoding("bit"), MysqlColumnEncoding::Bit);
        assert_eq!(mysql_column_encoding("blob"), MysqlColumnEncoding::Binary);
        assert_eq!(mysql_column_encoding("varchar"), MysqlColumnEncoding::Text);

        assert_eq!(
            format_mysql_literal(MysqlColumnEncoding::Bit, Some("0")),
            "b'0'"
        );
        assert_eq!(
            format_mysql_literal(MysqlColumnEncoding::Bit, Some("1")),
            "b'1'"
        );
        assert_eq!(
            format_mysql_literal(MysqlColumnEncoding::Binary, Some("f")),
            "X'0f'"
        );
        assert_eq!(
            format_mysql_literal(MysqlColumnEncoding::Binary, Some("abcd")),
            "X'abcd'"
        );
        assert_eq!(
            format_mysql_literal(MysqlColumnEncoding::Text, Some("a'b")),
            "'a\\'b'"
        );
        assert_eq!(format_mysql_literal(MysqlColumnEncoding::Bit, None), "NULL");
    }
}
