//! PostgreSQL catalogue inspection for one table.
//!
//! Slice 1 inspects a single table. The same structures are what a later slice
//! will expand into a foreign-key graph.

use serde::Serialize;
use sqlx::Row;

use crate::domain::error::{CoreError, ErrorCode};
use crate::safesync::mask::{hex_lower, ColumnShape};
use crate::safesync::session::BoundSession;
use crate::safesync::spec::QualifiedName;

/// One column as the catalogue describes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnInfo {
    pub name: String,
    /// The base type name with no length modifier, e.g. `text` or `uuid`.
    pub type_name: String,
    pub nullable: bool,
    pub is_primary_key: bool,
    pub is_generated: bool,
    pub is_identity: bool,
    /// A declared length limit (`varchar(n)`), when present.
    pub max_len: Option<usize>,
}

/// One table and its identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TableInfo {
    pub schema: String,
    pub table: String,
    pub relkind: String,
    pub rls_active: bool,
    pub columns: Vec<ColumnInfo>,
    pub primary_key: Vec<String>,
    /// The columns that make a row unique. Equals the primary key when present.
    pub row_identity: Vec<String>,
    /// A stable hash of the table shape, used to detect schema drift.
    pub schema_hash: String,
}

impl TableInfo {
    pub fn name(&self) -> QualifiedName {
        QualifiedName {
            schema: self.schema.clone(),
            table: self.table.clone(),
        }
    }

    /// The shape the masking engine consumes.
    pub fn column_shapes(&self) -> Vec<ColumnShape> {
        self.columns
            .iter()
            .map(|column| ColumnShape {
                name: column.name.clone(),
                type_name: column.type_name.clone(),
                nullable: column.nullable,
                is_primary_key: column.is_primary_key,
                max_len: column.max_len,
            })
            .collect()
    }
}

/// A foreign key from `child` to `parent`, in column order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForeignKeyEdge {
    pub name: String,
    pub child: QualifiedName,
    pub parent: QualifiedName,
    pub child_columns: Vec<String>,
    pub parent_columns: Vec<String>,
    pub deferrable: bool,
    pub initially_deferred: bool,
    /// `s` (MATCH SIMPLE) or `f` (MATCH FULL).
    pub match_type: String,
}

/// The tables and edges discovered for one selection.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemaGraph {
    pub tables: Vec<TableInfo>,
    pub edges: Vec<ForeignKeyEdge>,
}

impl SchemaGraph {
    pub fn table(&self, name: &QualifiedName) -> Option<&TableInfo> {
        self.tables.iter().find(|table| table.name() == *name)
    }

    pub fn outgoing(&self, child: &QualifiedName) -> Vec<ForeignKeyEdge> {
        self.edges
            .iter()
            .filter(|edge| edge.child == *child)
            .cloned()
            .collect()
    }

    pub fn edges_between(&self, tables: &[QualifiedName]) -> Vec<ForeignKeyEdge> {
        self.edges
            .iter()
            .filter(|edge| tables.contains(&edge.child) && tables.contains(&edge.parent))
            .cloned()
            .collect()
    }
}

/// Discover the parent closure of `roots` to `depth` FK hops.
///
/// Only outgoing foreign keys of a discovered table are read, so the scan never
/// reaches beyond the authorised roots and their parents. A table reached twice
/// is inspected once.
pub async fn discover_parents(
    session: &mut BoundSession,
    roots: &[QualifiedName],
    depth: u8,
) -> Result<SchemaGraph, CoreError> {
    use std::collections::{BTreeMap, VecDeque};

    let mut tables: BTreeMap<QualifiedName, TableInfo> = BTreeMap::new();
    let mut edges: Vec<ForeignKeyEdge> = Vec::new();
    let mut seen_edges: std::collections::BTreeSet<(String, String, String)> =
        std::collections::BTreeSet::new();

    let mut frontier: VecDeque<(QualifiedName, u8)> =
        roots.iter().cloned().map(|name| (name, 0)).collect();

    while let Some((name, level)) = frontier.pop_front() {
        if !tables.contains_key(&name) {
            let info = inspect_table(session, &name).await?;
            tables.insert(name.clone(), info);
        }

        for edge in foreign_keys_of(session, &name).await? {
            let key = (
                edge.name.clone(),
                edge.child.to_string(),
                edge.parent.to_string(),
            );
            if seen_edges.insert(key) {
                edges.push(edge.clone());
            }
            // `depth` counts hops from a root. A root itself is level 0.
            if level < depth && !tables.contains_key(&edge.parent) {
                frontier.push_back((edge.parent.clone(), level + 1));
            }
        }
    }

    Ok(SchemaGraph {
        tables: tables.into_values().collect(),
        edges,
    })
}

async fn foreign_keys_of(
    session: &mut BoundSession,
    child: &QualifiedName,
) -> Result<Vec<ForeignKeyEdge>, CoreError> {
    let rows = sqlx::query(
        "SELECT con.conname::text,
                pn.nspname::text,
                pc.relname::text,
                con.condeferrable,
                con.condeferred,
                con.confmatchtype::text,
                (SELECT array_agg(ca.attname::text ORDER BY o.n)
                   FROM unnest(con.conkey) WITH ORDINALITY AS o(attnum, n)
                   JOIN pg_attribute ca
                     ON ca.attrelid = con.conrelid AND ca.attnum = o.attnum),
                (SELECT array_agg(pa.attname::text ORDER BY o.n)
                   FROM unnest(con.confkey) WITH ORDINALITY AS o(attnum, n)
                   JOIN pg_attribute pa
                     ON pa.attrelid = con.confrelid AND pa.attnum = o.attnum)
         FROM pg_constraint con
         JOIN pg_class cc ON cc.oid = con.conrelid
         JOIN pg_namespace cn ON cn.oid = cc.relnamespace
         JOIN pg_class pc ON pc.oid = con.confrelid
         JOIN pg_namespace pn ON pn.oid = pc.relnamespace
         WHERE con.contype = 'f' AND cn.nspname = $1 AND cc.relname = $2
         ORDER BY con.conname",
    )
    .bind(&child.schema)
    .bind(&child.table)
    .fetch_all(&mut *session.connection())
    .await
    .map_err(connection_error)?;

    let mut edges = Vec::with_capacity(rows.len());
    for row in rows {
        let name: String = row.try_get(0).unwrap_or_default();
        let parent_schema: String = row.try_get(1).unwrap_or_default();
        let parent_table: String = row.try_get(2).unwrap_or_default();
        let deferrable: bool = row.try_get(3).unwrap_or(false);
        let initially_deferred: bool = row.try_get(4).unwrap_or(false);
        let match_type: String = row.try_get(5).unwrap_or_else(|_| "s".to_string());
        let child_columns: Vec<String> = row.try_get(6).unwrap_or_default();
        let parent_columns: Vec<String> = row.try_get(7).unwrap_or_default();

        let parent = QualifiedName {
            schema: parent_schema,
            table: parent_table,
        };

        // Refuse a relationship we cannot reason about rather than guessing.
        if child_columns.is_empty() || parent_columns.len() != child_columns.len() {
            return Err(CoreError::with_detail(
                ErrorCode::Unsupported,
                format!("foreign key {name} has an unsupported column mapping"),
                "unsupported_foreign_key",
            ));
        }

        edges.push(ForeignKeyEdge {
            name,
            child: child.clone(),
            parent,
            child_columns,
            parent_columns,
            deferrable,
            initially_deferred,
            match_type,
        });
    }

    Ok(edges)
}

/// A stable structural fingerprint of the discovered graph.
///
/// It binds the constraint name, both sides, the ordered column mapping, the
/// match type and the deferrability state, so a changed relationship cannot
/// reuse an old plan. Names and structure, not catalog OIDs, which are not
/// stable across a dump and restore.
pub fn graph_fingerprint(graph: &SchemaGraph) -> String {
    let mut edges: Vec<&ForeignKeyEdge> = graph.edges.iter().collect();
    edges.sort_by(|left, right| {
        (left.name.as_str(), left.child.to_string())
            .cmp(&(right.name.as_str(), right.child.to_string()))
    });

    let document = serde_json::json!({
        "edges": edges
            .iter()
            .map(|edge| {
                serde_json::json!({
                    "name": edge.name,
                    "child": edge.child.to_string(),
                    "parent": edge.parent.to_string(),
                    "childColumns": edge.child_columns,
                    "parentColumns": edge.parent_columns,
                    "deferrable": edge.deferrable,
                    "initiallyDeferred": edge.initially_deferred,
                    "matchType": edge.match_type,
                })
            })
            .collect::<Vec<_>>(),
    });

    let canonical = serde_json::to_string(&document).unwrap_or_default();
    crate::safesync::mask::hex_lower(&crate::safesync::mask::digest(canonical.as_bytes()))
}

pub async fn inspect_table(
    session: &mut BoundSession,
    table: &QualifiedName,
) -> Result<TableInfo, CoreError> {
    let shape = sqlx::query(
        "SELECT c.relkind::text, c.relrowsecurity
         FROM pg_class c
         JOIN pg_namespace n ON n.oid = c.relnamespace
         WHERE n.nspname = $1 AND c.relname = $2",
    )
    .bind(&table.schema)
    .bind(&table.table)
    .fetch_optional(&mut *session.connection())
    .await
    .map_err(connection_error)?;

    let Some(shape) = shape else {
        return Err(CoreError::with_detail(
            ErrorCode::Usage,
            format!("{table} does not exist"),
            "invalid_identifier",
        ));
    };

    let relkind: String = shape.try_get(0).unwrap_or_default();
    let rls_active: bool = shape.try_get(1).unwrap_or(false);

    if relkind != "r" && relkind != "p" {
        return Err(CoreError::with_detail(
            ErrorCode::Unsupported,
            format!("{table} is not an ordinary or partitioned table"),
            "unsupported_relation",
        ));
    }

    let primary_key = primary_key_columns(session, table).await?;
    let row_identity = if primary_key.is_empty() {
        unique_columns(session, table).await?
    } else {
        primary_key.clone()
    };

    if row_identity.is_empty() {
        return Err(CoreError::with_detail(
            ErrorCode::Unsupported,
            format!("{table} has no primary key or usable unique key"),
            "no_row_identity",
        ));
    }

    let rows = sqlx::query(
        "SELECT a.attname::text,
                format_type(a.atttypid, a.atttypmod)::text,
                NOT a.attnotnull,
                (a.attgenerated <> ''),
                (a.attidentity <> ''),
                a.atttypmod
         FROM pg_attribute a
         JOIN pg_class c ON c.oid = a.attrelid
         JOIN pg_namespace n ON n.oid = c.relnamespace
         WHERE n.nspname = $1 AND c.relname = $2
           AND a.attnum > 0 AND NOT a.attisdropped
         ORDER BY a.attnum",
    )
    .bind(&table.schema)
    .bind(&table.table)
    .fetch_all(&mut *session.connection())
    .await
    .map_err(connection_error)?;

    let mut columns = Vec::with_capacity(rows.len());
    for row in rows {
        let raw_type: String = row.try_get(1).unwrap_or_default();
        let atttypmod: i32 = row.try_get(5).unwrap_or(-1);
        let (type_name, max_len) = split_type(&raw_type, atttypmod);
        let name: String = row.try_get(0).unwrap_or_default();

        columns.push(ColumnInfo {
            is_primary_key: primary_key.contains(&name),
            name,
            type_name,
            nullable: row.try_get(2).unwrap_or(true),
            is_generated: row.try_get(3).unwrap_or(false),
            is_identity: row.try_get(4).unwrap_or(false),
            max_len,
        });
    }

    // A nullable unique key is not a usable row identity: two NULLs would
    // collapse to one identity and a row could be dropped silently.
    for column in &row_identity {
        let nullable = columns
            .iter()
            .find(|candidate| &candidate.name == column)
            .map(|candidate| candidate.nullable)
            .unwrap_or(true);
        if nullable {
            return Err(CoreError::with_detail(
                ErrorCode::Unsupported,
                format!("{table} has a nullable row identity column {column}"),
                "nullable_row_identity",
            ));
        }
    }

    // CHECK constraints that could reject a masked value are part of the shape.
    let check_constraints: Vec<String> = sqlx::query_scalar(
        "SELECT pg_get_constraintdef(k.oid)::text
         FROM pg_constraint k
         JOIN pg_class c ON c.oid = k.conrelid
         JOIN pg_namespace n ON n.oid = c.relnamespace
         WHERE k.contype = 'c' AND n.nspname = $1 AND c.relname = $2
         ORDER BY k.conname",
    )
    .bind(&table.schema)
    .bind(&table.table)
    .fetch_all(&mut *session.connection())
    .await
    .map_err(connection_error)?;

    let schema_hash = schema_hash(
        &table.schema,
        &table.table,
        &relkind,
        &columns,
        &row_identity,
        &check_constraints,
    )?;

    Ok(TableInfo {
        schema: table.schema.clone(),
        table: table.table.clone(),
        relkind,
        rls_active,
        columns,
        primary_key,
        row_identity,
        schema_hash,
    })
}

async fn primary_key_columns(
    session: &mut BoundSession,
    table: &QualifiedName,
) -> Result<Vec<String>, CoreError> {
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT a.attname::text
         FROM pg_index i
         JOIN pg_class c ON c.oid = i.indrelid
         JOIN pg_namespace n ON n.oid = c.relnamespace
         JOIN pg_attribute a ON a.attrelid = c.oid AND a.attnum = ANY(i.indkey::int2[])
         WHERE n.nspname = $1 AND c.relname = $2 AND i.indisprimary
         ORDER BY array_position(i.indkey::int2[], a.attnum)",
    )
    .bind(&table.schema)
    .bind(&table.table)
    .fetch_all(&mut *session.connection())
    .await
    .map_err(connection_error)?;

    Ok(rows.into_iter().map(|(name,)| name).collect())
}

async fn unique_columns(
    session: &mut BoundSession,
    table: &QualifiedName,
) -> Result<Vec<String>, CoreError> {
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT a.attname::text
         FROM pg_index i
         JOIN pg_class c ON c.oid = i.indrelid
         JOIN pg_namespace n ON n.oid = c.relnamespace
         JOIN pg_attribute a ON a.attrelid = c.oid AND a.attnum = ANY(i.indkey::int2[])
         WHERE n.nspname = $1 AND c.relname = $2
           AND i.indisunique AND NOT i.indisprimary
           AND i.indpred IS NULL AND i.indexprs IS NULL
           AND i.indexrelid = (
             SELECT i2.indexrelid
             FROM pg_index i2
             JOIN pg_class c2 ON c2.oid = i2.indrelid
             JOIN pg_namespace n2 ON n2.oid = c2.relnamespace
             WHERE n2.nspname = $1 AND c2.relname = $2
               AND i2.indisunique AND NOT i2.indisprimary
               AND i2.indpred IS NULL AND i2.indexprs IS NULL
             ORDER BY i2.indexrelid
             LIMIT 1)
         ORDER BY array_position(i.indkey::int2[], a.attnum)",
    )
    .bind(&table.schema)
    .bind(&table.table)
    .fetch_all(&mut *session.connection())
    .await
    .map_err(connection_error)?;

    Ok(rows.into_iter().map(|(name,)| name).collect())
}

/// Split `character varying(64)` into `character varying` and `Some(64)`.
fn split_type(raw: &str, atttypmod: i32) -> (String, Option<usize>) {
    let base = raw.split('(').next().unwrap_or(raw).trim().to_string();
    let limit = if atttypmod > 4
        && (base == "character varying" || base == "character" || base == "bit varying")
    {
        Some((atttypmod - 4) as usize)
    } else {
        None
    };
    (base, limit)
}

fn schema_hash(
    schema: &str,
    table: &str,
    relkind: &str,
    columns: &[ColumnInfo],
    row_identity: &[String],
    check_constraints: &[String],
) -> Result<String, CoreError> {
    let document = serde_json::json!({
        "schema": schema,
        "table": table,
        "relkind": relkind,
        "columns": columns,
        "rowIdentity": row_identity,
        "checkConstraints": check_constraints,
    });
    let canonical = serde_json::to_string(&document).map_err(|error| {
        CoreError::new(
            ErrorCode::Operation,
            format!("schema hash could not be built: {error}"),
        )
    })?;
    Ok(hex_lower(&crate::safesync::mask::digest(
        canonical.as_bytes(),
    )))
}

fn connection_error(error: sqlx::Error) -> CoreError {
    CoreError::redacted(
        ErrorCode::Connection,
        format!("catalogue inspection failed: {error}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_varchar_length_is_extracted() {
        assert_eq!(
            split_type("character varying(64)", 68),
            ("character varying".to_string(), Some(64))
        );
        assert_eq!(split_type("text", -1), ("text".to_string(), None));
        assert_eq!(split_type("uuid", -1), ("uuid".to_string(), None));
    }

    #[test]
    fn the_schema_hash_is_stable_and_shape_sensitive() {
        let columns = vec![ColumnInfo {
            name: "id".into(),
            type_name: "uuid".into(),
            nullable: false,
            is_primary_key: true,
            is_generated: false,
            is_identity: false,
            max_len: None,
        }];
        let first = schema_hash("public", "accounts", "r", &columns, &["id".into()], &[]).unwrap();
        let second = schema_hash("public", "accounts", "r", &columns, &["id".into()], &[]).unwrap();
        assert_eq!(first, second);

        let mut changed = columns;
        changed[0].type_name = "text".into();
        let third = schema_hash("public", "accounts", "r", &changed, &["id".into()], &[]).unwrap();
        assert_ne!(first, third);
    }

    #[test]
    fn column_shapes_carry_the_masking_inputs() {
        let info = TableInfo {
            schema: "public".into(),
            table: "accounts".into(),
            relkind: "r".into(),
            rls_active: false,
            columns: vec![ColumnInfo {
                name: "email".into(),
                type_name: "text".into(),
                nullable: false,
                is_primary_key: false,
                is_generated: false,
                is_identity: false,
                max_len: None,
            }],
            primary_key: vec!["id".into()],
            row_identity: vec!["id".into()],
            schema_hash: "hash".into(),
        };
        let shapes = info.column_shapes();
        assert_eq!(shapes.len(), 1);
        assert_eq!(shapes[0].type_name, "text");
    }
}
