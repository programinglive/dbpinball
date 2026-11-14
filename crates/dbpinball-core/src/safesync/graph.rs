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

/// Inspect one table inside the caller's session.
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

    let schema_hash = schema_hash(
        &table.schema,
        &table.table,
        &relkind,
        &columns,
        &row_identity,
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
) -> Result<String, CoreError> {
    let document = serde_json::json!({
        "schema": schema,
        "table": table,
        "relkind": relkind,
        "columns": columns,
        "rowIdentity": row_identity,
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
        let first = schema_hash("public", "accounts", "r", &columns, &["id".into()]).unwrap();
        let second = schema_hash("public", "accounts", "r", &columns, &["id".into()]).unwrap();
        assert_eq!(first, second);

        let mut changed = columns;
        changed[0].type_name = "text".into();
        let third = schema_hash("public", "accounts", "r", &changed, &["id".into()]).unwrap();
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
