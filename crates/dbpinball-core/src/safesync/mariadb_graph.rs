//! MariaDB `information_schema` metadata and parent-directed foreign-key graph
//! inspection (MDB-2).
//!
//! Reads catalogue metadata only — never extracts table rows. Enforces InnoDB
//! `BASE TABLE` requirements, composite foreign-key ordering, zero-trigger
//! safety, and fail-closed refusal of non-deferrable cycles.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::{MySqlConnection, MySqlPool, Row};

use crate::domain::error::{CoreError, ErrorCode};
use crate::safesync::database_error;
use crate::safesync::graph::{ColumnInfo, ForeignKeyEdge, SchemaGraph, TableInfo};
use crate::safesync::mask::hex_lower;
use crate::safesync::spec::QualifiedName;

/// A trigger attached to a MariaDB table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MariaDbTrigger {
    pub name: String,
    pub event: String,
    pub timing: String,
}

async fn acquire_conn(
    pool: &MySqlPool,
) -> Result<sqlx::pool::PoolConnection<sqlx::MySql>, CoreError> {
    pool.acquire().await.map_err(|err| {
        database_error(
            ErrorCode::Connection,
            "MariaDB connection pool",
            "connect_failed",
            err,
        )
    })
}

/// Inspect triggers on a MariaDB table via `information_schema.TRIGGERS`.
pub async fn inspect_mariadb_triggers(
    pool: &MySqlPool,
    table: &QualifiedName,
) -> Result<Vec<MariaDbTrigger>, CoreError> {
    let mut conn = acquire_conn(pool).await?;
    inspect_mariadb_triggers_conn(&mut conn, table).await
}

/// Inspect triggers on a MariaDB table using an existing pinned connection.
pub async fn inspect_mariadb_triggers_conn(
    conn: &mut MySqlConnection,
    table: &QualifiedName,
) -> Result<Vec<MariaDbTrigger>, CoreError> {
    let rows = sqlx::query(
        "SELECT TRIGGER_NAME, EVENT_MANIPULATION, ACTION_TIMING \
         FROM information_schema.TRIGGERS \
         WHERE EVENT_OBJECT_SCHEMA = ? AND EVENT_OBJECT_TABLE = ? \
         ORDER BY TRIGGER_NAME",
    )
    .bind(&table.schema)
    .bind(&table.table)
    .fetch_all(&mut *conn)
    .await
    .map_err(|err| {
        database_error(
            ErrorCode::Connection,
            "MariaDB trigger inspection",
            "metadata_query_failed",
            err,
        )
    })?;

    Ok(rows
        .into_iter()
        .map(|row| MariaDbTrigger {
            name: decode_string(&row, 0),
            event: decode_string(&row, 1),
            timing: decode_string(&row, 2),
        })
        .collect())
}

/// Inspect one MariaDB table's metadata, enforcing `BASE TABLE`, `InnoDB`,
/// and zero triggers.
pub async fn inspect_mariadb_table(
    pool: &MySqlPool,
    table: &QualifiedName,
) -> Result<TableInfo, CoreError> {
    let mut conn = acquire_conn(pool).await?;
    inspect_mariadb_table_conn(&mut conn, table).await
}

/// Inspect one MariaDB table's metadata using an existing pinned connection.
pub async fn inspect_mariadb_table_conn(
    conn: &mut MySqlConnection,
    table: &QualifiedName,
) -> Result<TableInfo, CoreError> {
    let table_row = sqlx::query(
        "SELECT TABLE_TYPE, COALESCE(ENGINE, '') \
         FROM information_schema.TABLES \
         WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ?",
    )
    .bind(&table.schema)
    .bind(&table.table)
    .fetch_optional(&mut *conn)
    .await
    .map_err(|err| {
        database_error(
            ErrorCode::Connection,
            "MariaDB table inspection",
            "metadata_query_failed",
            err,
        )
    })?;

    let Some(table_row) = table_row else {
        return Err(CoreError::with_detail(
            ErrorCode::NotFound,
            format!("table {}.{} does not exist", table.schema, table.table),
            "table_not_found",
        ));
    };

    let table_type = decode_string(&table_row, 0);
    let engine = decode_string(&table_row, 1);

    if !table_type.eq_ignore_ascii_case("BASE TABLE") {
        return Err(CoreError::with_detail(
            ErrorCode::Unsupported,
            format!(
                "table {}.{} has unsupported type '{table_type}' (BASE TABLE required)",
                table.schema, table.table
            ),
            "unsupported_table_type",
        ));
    }

    if !engine.eq_ignore_ascii_case("InnoDB") {
        return Err(CoreError::with_detail(
            ErrorCode::Unsupported,
            format!(
                "table {}.{} uses unsupported engine '{engine}' (InnoDB required)",
                table.schema, table.table
            ),
            "unsupported_storage_engine",
        ));
    }

    let triggers = inspect_mariadb_triggers_conn(conn, table).await?;
    if !triggers.is_empty() {
        return Err(CoreError::with_detail(
            ErrorCode::Unsupported,
            format!(
                "table {}.{} has {} trigger(s); triggers are refused in Safe Sync",
                table.schema,
                table.table,
                triggers.len()
            ),
            "triggers_not_supported",
        ));
    }

    let primary_key = inspect_primary_key(conn, table).await?;
    let pk_set: BTreeSet<&str> = primary_key.iter().map(String::as_str).collect();

    let col_rows = sqlx::query(
        "SELECT COLUMN_NAME, \
                DATA_TYPE, \
                IS_NULLABLE, \
                EXTRA, \
                CHARACTER_MAXIMUM_LENGTH, \
                COALESCE(GENERATION_EXPRESSION, '') \
         FROM information_schema.COLUMNS \
         WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? \
         ORDER BY ORDINAL_POSITION",
    )
    .bind(&table.schema)
    .bind(&table.table)
    .fetch_all(&mut *conn)
    .await
    .map_err(|err| {
        database_error(
            ErrorCode::Connection,
            "MariaDB column inspection",
            "metadata_query_failed",
            err,
        )
    })?;

    if col_rows.is_empty() {
        return Err(CoreError::with_detail(
            ErrorCode::PolicyRefusal,
            format!(
                "table {}.{} returned no column metadata (insufficient privileges)",
                table.schema, table.table
            ),
            "metadata_permission_denied",
        ));
    }

    let mut columns = Vec::with_capacity(col_rows.len());
    for row in col_rows {
        let name = decode_string(&row, 0);
        let type_name = decode_string(&row, 1).to_ascii_lowercase();
        let is_nullable = decode_string(&row, 2).eq_ignore_ascii_case("YES");
        let extra = decode_string(&row, 3).to_ascii_lowercase();
        let max_len = decode_optional_usize(&row, 4);
        let gen_expr = decode_string(&row, 5);

        let is_generated = extra.contains("generated");
        let is_identity = extra.contains("auto_increment");
        let generated_expression = if is_generated && !gen_expr.trim().is_empty() {
            Some(gen_expr.trim().to_string())
        } else {
            None
        };

        columns.push(ColumnInfo {
            is_primary_key: pk_set.contains(name.as_str()),
            name,
            type_name,
            nullable: is_nullable,
            is_generated,
            is_identity,
            identity_always: false,
            generated_expression,
            max_len,
        });
    }

    let row_identity = if !primary_key.is_empty() {
        primary_key.clone()
    } else {
        inspect_unique_not_null_key(conn, table, &columns).await?
    };

    let schema_hash = compute_mariadb_schema_hash(table, &columns, &primary_key, &row_identity);

    Ok(TableInfo {
        schema: table.schema.clone(),
        table: table.table.clone(),
        relkind: "r".to_string(),
        rls_active: false,
        columns,
        primary_key,
        row_identity,
        schema_hash,
    })
}

/// Inspect outgoing foreign-key constraints for a MariaDB table in exact
/// `ORDINAL_POSITION` order.
pub async fn inspect_mariadb_foreign_keys(
    pool: &MySqlPool,
    table: &QualifiedName,
) -> Result<Vec<ForeignKeyEdge>, CoreError> {
    let mut conn = acquire_conn(pool).await?;
    inspect_mariadb_foreign_keys_conn(&mut conn, table).await
}

/// Inspect outgoing foreign-key constraints using an existing pinned connection.
pub async fn inspect_mariadb_foreign_keys_conn(
    conn: &mut MySqlConnection,
    table: &QualifiedName,
) -> Result<Vec<ForeignKeyEdge>, CoreError> {
    let rows = sqlx::query(
        "SELECT kcu.CONSTRAINT_NAME, \
                kcu.COLUMN_NAME, \
                kcu.REFERENCED_TABLE_SCHEMA, \
                kcu.REFERENCED_TABLE_NAME, \
                kcu.REFERENCED_COLUMN_NAME \
         FROM information_schema.KEY_COLUMN_USAGE kcu \
         INNER JOIN information_schema.REFERENTIAL_CONSTRAINTS rc \
           ON rc.CONSTRAINT_SCHEMA = kcu.CONSTRAINT_SCHEMA \
          AND rc.CONSTRAINT_NAME = kcu.CONSTRAINT_NAME \
          AND rc.TABLE_NAME = kcu.TABLE_NAME \
         WHERE kcu.TABLE_SCHEMA = ? \
           AND kcu.TABLE_NAME = ? \
           AND kcu.REFERENCED_TABLE_NAME IS NOT NULL \
         ORDER BY kcu.CONSTRAINT_NAME, kcu.ORDINAL_POSITION",
    )
    .bind(&table.schema)
    .bind(&table.table)
    .fetch_all(&mut *conn)
    .await
    .map_err(|err| {
        database_error(
            ErrorCode::Connection,
            "MariaDB foreign-key inspection",
            "metadata_query_failed",
            err,
        )
    })?;

    struct EdgeAccum {
        parent_schema: String,
        parent_table: String,
        child_columns: Vec<String>,
        parent_columns: Vec<String>,
    }

    let mut grouped: BTreeMap<String, EdgeAccum> = BTreeMap::new();
    for row in rows {
        let name = decode_string(&row, 0);
        let child_col = decode_string(&row, 1);
        let parent_schema = decode_string(&row, 2);
        let parent_table = decode_string(&row, 3);
        let parent_col = decode_string(&row, 4);

        if parent_schema.trim().is_empty()
            || parent_table.trim().is_empty()
            || parent_col.trim().is_empty()
        {
            return Err(CoreError::with_detail(
                ErrorCode::Operation,
                format!(
                    "foreign key '{name}' on {}.{} is missing parent metadata",
                    table.schema, table.table
                ),
                "missing_parent_metadata",
            ));
        }

        let entry = grouped.entry(name).or_insert_with(|| EdgeAccum {
            parent_schema,
            parent_table,
            child_columns: Vec::new(),
            parent_columns: Vec::new(),
        });
        entry.child_columns.push(child_col);
        entry.parent_columns.push(parent_col);
    }

    Ok(grouped
        .into_iter()
        .map(|(name, acc)| ForeignKeyEdge {
            name,
            child: table.clone(),
            parent: QualifiedName {
                schema: acc.parent_schema,
                table: acc.parent_table,
            },
            child_columns: acc.child_columns,
            parent_columns: acc.parent_columns,
            deferrable: false,
            initially_deferred: false,
            match_type: "s".to_string(),
        })
        .collect())
}

/// Discover the parent-directed InnoDB foreign-key graph from `roots` up to
/// `max_depth`. Fails closed on missing parent metadata, non-InnoDB tables,
/// triggers, or any foreign-key cycle/self-reference (MariaDB has no
/// deferrable foreign keys).
pub async fn discover_mariadb_parents(
    pool: &MySqlPool,
    roots: &[QualifiedName],
    max_depth: u8,
) -> Result<SchemaGraph, CoreError> {
    let mut conn = acquire_conn(pool).await?;
    discover_mariadb_parents_conn(&mut conn, roots, max_depth).await
}

/// Discover the parent-directed InnoDB foreign-key graph using an existing
/// pinned connection.
pub async fn discover_mariadb_parents_conn(
    conn: &mut MySqlConnection,
    roots: &[QualifiedName],
    max_depth: u8,
) -> Result<SchemaGraph, CoreError> {
    if roots.is_empty() {
        return Err(CoreError::with_detail(
            ErrorCode::Usage,
            "at least one root table is required",
            "invalid_spec",
        ));
    }

    let mut visited: BTreeSet<QualifiedName> = BTreeSet::new();
    let mut tables: Vec<TableInfo> = Vec::new();
    let mut edges: Vec<ForeignKeyEdge> = Vec::new();
    let mut queue: VecDeque<(QualifiedName, u8)> = VecDeque::new();

    for root in roots {
        if visited.insert(root.clone()) {
            queue.push_back((root.clone(), 0));
        }
    }

    while let Some((current, depth)) = queue.pop_front() {
        let info = inspect_mariadb_table_conn(conn, &current).await?;
        tables.push(info);

        if depth >= max_depth {
            continue;
        }

        let fks = inspect_mariadb_foreign_keys_conn(conn, &current).await?;
        for edge in fks {
            if edge.child == edge.parent {
                return Err(CoreError::with_detail(
                    ErrorCode::Unsupported,
                    format!(
                        "self-referential foreign key '{}' on {}.{} is not deferrable in MariaDB",
                        edge.name, edge.child.schema, edge.child.table
                    ),
                    "cyclic_fk_not_supported",
                ));
            }
            if visited.insert(edge.parent.clone()) {
                queue.push_back((edge.parent.clone(), depth + 1));
            }
            edges.push(edge);
        }
    }

    detect_mariadb_cycles(&edges)?;

    Ok(SchemaGraph { tables, edges })
}

fn detect_mariadb_cycles(edges: &[ForeignKeyEdge]) -> Result<(), CoreError> {
    let mut adj: BTreeMap<&QualifiedName, Vec<&QualifiedName>> = BTreeMap::new();
    for edge in edges {
        adj.entry(&edge.child).or_default().push(&edge.parent);
    }

    // 0 = unvisited, 1 = visiting, 2 = done
    let mut state: BTreeMap<&QualifiedName, u8> = BTreeMap::new();
    for node in adj.keys().copied() {
        if state.get(node).copied().unwrap_or(0) == 0 && has_cycle(node, &adj, &mut state) {
            return Err(CoreError::with_detail(
                ErrorCode::Unsupported,
                "foreign-key cycle detected; MariaDB InnoDB does not support deferrable foreign keys",
                "cyclic_fk_not_supported",
            ));
        }
    }
    Ok(())
}

fn has_cycle<'a>(
    node: &'a QualifiedName,
    adj: &BTreeMap<&'a QualifiedName, Vec<&'a QualifiedName>>,
    state: &mut BTreeMap<&'a QualifiedName, u8>,
) -> bool {
    state.insert(node, 1);
    if let Some(neighbors) = adj.get(node) {
        for &next in neighbors {
            match state.get(next).copied().unwrap_or(0) {
                1 => return true,
                0 if has_cycle(next, adj, state) => return true,
                _ => {}
            }
        }
    }
    state.insert(node, 2);
    false
}

async fn inspect_primary_key(
    conn: &mut MySqlConnection,
    table: &QualifiedName,
) -> Result<Vec<String>, CoreError> {
    let rows = sqlx::query(
        "SELECT COLUMN_NAME \
         FROM information_schema.STATISTICS \
         WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? AND INDEX_NAME = 'PRIMARY' \
         ORDER BY SEQ_IN_INDEX",
    )
    .bind(&table.schema)
    .bind(&table.table)
    .fetch_all(&mut *conn)
    .await
    .map_err(|err| {
        database_error(
            ErrorCode::Connection,
            "MariaDB primary key inspection",
            "metadata_query_failed",
            err,
        )
    })?;

    Ok(rows.into_iter().map(|r| decode_string(&r, 0)).collect())
}

async fn inspect_unique_not_null_key(
    conn: &mut MySqlConnection,
    table: &QualifiedName,
    columns: &[ColumnInfo],
) -> Result<Vec<String>, CoreError> {
    let rows = sqlx::query(
        "SELECT INDEX_NAME, COLUMN_NAME \
         FROM information_schema.STATISTICS \
         WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? AND NON_UNIQUE = 0 AND INDEX_NAME <> 'PRIMARY' \
         ORDER BY INDEX_NAME, SEQ_IN_INDEX",
    )
    .bind(&table.schema)
    .bind(&table.table)
    .fetch_all(&mut *conn)
    .await
    .map_err(|err| {
        database_error(
            ErrorCode::Connection,
            "MariaDB unique key inspection",
            "metadata_query_failed",
            err,
        )
    })?;

    let mut indexes: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for row in rows {
        indexes
            .entry(decode_string(&row, 0))
            .or_default()
            .push(decode_string(&row, 1));
    }

    for (_idx_name, cols) in indexes {
        let all_not_null = cols.iter().all(|col| {
            columns
                .iter()
                .find(|c| c.name == *col)
                .map(|c| !c.nullable)
                .unwrap_or(false)
        });
        if all_not_null && !cols.is_empty() {
            return Ok(cols);
        }
    }
    Ok(Vec::new())
}

fn compute_mariadb_schema_hash(
    table: &QualifiedName,
    columns: &[ColumnInfo],
    primary_key: &[String],
    row_identity: &[String],
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"dbpinball/mariadb-schema/v1|");
    // Omit database/schema name so identical table shapes across differently named source/destination databases match.
    hasher.update(table.table.as_bytes());
    hasher.update(b"|engine=innodb|pk=");
    hasher.update(primary_key.join(",").as_bytes());
    hasher.update(b"|id=");
    hasher.update(row_identity.join(",").as_bytes());

    for col in columns {
        hasher.update(b"|col=");
        hasher.update(col.name.as_bytes());
        hasher.update(b":");
        hasher.update(col.type_name.as_bytes());
        hasher.update(if col.nullable { ":null" } else { ":notnull" }.as_bytes());
        hasher.update(if col.is_identity { ":autoinc" } else { "" }.as_bytes());
        hasher.update(if col.is_generated { ":gen" } else { "" }.as_bytes());
        if let Some(expr) = &col.generated_expression {
            hasher.update(b":");
            hasher.update(expr.as_bytes());
        }
        if let Some(len) = col.max_len {
            hasher.update(format!(":len={len}").as_bytes());
        }
    }

    format!("sha256:{}", hex_lower(&hasher.finalize()))
}

fn decode_string(row: &sqlx::mysql::MySqlRow, idx: usize) -> String {
    row.try_get::<String, _>(idx)
        .or_else(|_| {
            row.try_get::<Vec<u8>, _>(idx)
                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        })
        .unwrap_or_default()
}

fn decode_optional_usize(row: &sqlx::mysql::MySqlRow, idx: usize) -> Option<usize> {
    row.try_get::<Option<u64>, _>(idx)
        .ok()
        .flatten()
        .or_else(|| {
            row.try_get::<Option<i64>, _>(idx)
                .ok()
                .flatten()
                .filter(|&v| v >= 0)
                .map(|v| v as u64)
        })
        .and_then(|v| usize::try_from(v).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(s: &str) -> QualifiedName {
        QualifiedName::parse(s).unwrap()
    }

    #[test]
    fn schema_hash_is_deterministic_and_detects_drift() {
        let cols_a = vec![ColumnInfo {
            name: "id".into(),
            type_name: "bigint".into(),
            nullable: false,
            is_primary_key: true,
            is_generated: false,
            is_identity: true,
            identity_always: false,
            generated_expression: None,
            max_len: None,
        }];
        let mut cols_b = cols_a.clone();
        cols_b[0].type_name = "int".into();

        let h1 =
            compute_mariadb_schema_hash(&q("app.orders"), &cols_a, &["id".into()], &["id".into()]);
        let h2 =
            compute_mariadb_schema_hash(&q("app.orders"), &cols_a, &["id".into()], &["id".into()]);
        let h3 =
            compute_mariadb_schema_hash(&q("app.orders"), &cols_b, &["id".into()], &["id".into()]);
        assert_eq!(h1, h2);
        assert_ne!(h1, h3);
    }

    #[test]
    fn detect_mariadb_cycles_refuses_two_node_cycle_and_allows_dag() {
        let dag = vec![
            ForeignKeyEdge {
                name: "fk_order_customer".into(),
                child: q("app.orders"),
                parent: q("app.customers"),
                child_columns: vec!["customer_id".into()],
                parent_columns: vec!["id".into()],
                deferrable: false,
                initially_deferred: false,
                match_type: "s".into(),
            },
            ForeignKeyEdge {
                name: "fk_item_order".into(),
                child: q("app.items"),
                parent: q("app.orders"),
                child_columns: vec!["order_id".into()],
                parent_columns: vec!["id".into()],
                deferrable: false,
                initially_deferred: false,
                match_type: "s".into(),
            },
        ];
        assert!(detect_mariadb_cycles(&dag).is_ok());

        let cycle = vec![
            ForeignKeyEdge {
                name: "fk_a_b".into(),
                child: q("app.a"),
                parent: q("app.b"),
                child_columns: vec!["b_id".into()],
                parent_columns: vec!["id".into()],
                deferrable: false,
                initially_deferred: false,
                match_type: "s".into(),
            },
            ForeignKeyEdge {
                name: "fk_b_a".into(),
                child: q("app.b"),
                parent: q("app.a"),
                child_columns: vec!["a_id".into()],
                parent_columns: vec!["id".into()],
                deferrable: false,
                initially_deferred: false,
                match_type: "s".into(),
            },
        ];
        let err = detect_mariadb_cycles(&cycle).unwrap_err();
        assert_eq!(err.code, ErrorCode::Unsupported);
        assert_eq!(err.detail.as_deref(), Some("cyclic_fk_not_supported"));
    }
}
