//! Bounded, parameterized selection for one table.
//!
//! Slice 1 has no foreign-key traversal. The planner still refuses a non-zero
//! depth explicitly rather than silently ignoring it.

use serde::Serialize;
use sqlx::Row;

use crate::domain::error::{CoreError, ErrorCode};
use crate::safesync::graph::{inspect_table, TableInfo};
use crate::safesync::mask::{MaskInput, MaskedValue, Masker};
use crate::safesync::session::BoundSession;
use crate::safesync::spec::{
    FilterOp, FilterPredicate, FilterValue, Limits, QualifiedName, SafeSyncSpec, TraversalDirection,
};

/// One root selection with its keyed digest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RootPlan {
    pub schema: String,
    pub table: String,
    pub filter_columns: Vec<String>,
    pub filter_ops: Vec<String>,
    pub filter_digest: String,
}

/// One limit that was evaluated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LimitCheck {
    pub limit: String,
    pub observed: u64,
    pub ok: bool,
}

/// The result of planning, before it becomes a reviewable plan document.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectionPlan {
    pub tables: Vec<TableInfo>,
    pub roots: Vec<RootPlan>,
    pub checks: Vec<LimitCheck>,
    pub max_depth_used: u8,
    pub direction: String,
    pub complete: bool,
}

/// Rows after masking. Plaintext never leaves the extractor.
#[derive(Debug, Clone)]
pub struct TableRows {
    pub table: QualifiedName,
    pub columns: Vec<String>,
    /// Base type names parallel to `columns`, from the catalogue.
    pub types: Vec<String>,
    /// The columns that identify a row, in order.
    pub row_identity: Vec<String>,
    pub rows: Vec<Vec<MaskedValue>>,
    pub total_bytes: u64,
}

impl TableRows {
    pub fn row_count(&self) -> u64 {
        self.rows.len() as u64
    }
}

/// Inspect the selected tables and validate the selection.
pub async fn build(
    session: &mut BoundSession,
    spec: &SafeSyncSpec,
    masker: &Masker,
) -> Result<SelectionPlan, CoreError> {
    if spec.selection.relationships.depth > 0 {
        return Err(CoreError::with_detail(
            ErrorCode::Unsupported,
            "foreign-key traversal is not implemented in this slice",
            "unsupported_traversal",
        ));
    }

    if spec.selection.relationships.direction != TraversalDirection::Parents {
        return Err(CoreError::with_detail(
            ErrorCode::Unsupported,
            "child traversal is not implemented in this slice",
            "unsupported_traversal",
        ));
    }

    let mut tables: Vec<TableInfo> = Vec::new();
    let mut roots = Vec::new();

    for root in &spec.selection.roots {
        let name = QualifiedName::parse(&root.table)?;

        if !tables.iter().any(|table| table.name() == name) {
            let info = inspect_table(session, &name).await?;
            masker.coverage(&name.schema, &name.table, &info.column_shapes())?;
            tables.push(info);
        }

        let canonical = root
            .filter
            .iter()
            .map(FilterPredicate::canonical)
            .collect::<Vec<_>>()
            .join("|");

        roots.push(RootPlan {
            schema: name.schema.clone(),
            table: name.table.clone(),
            filter_columns: root.filter.iter().map(|f| f.column.clone()).collect(),
            filter_ops: root
                .filter
                .iter()
                .map(|f| f.op.as_str().to_string())
                .collect(),
            filter_digest: masker.filter_digest(&name.schema, &name.table, &canonical),
        });
    }

    Ok(SelectionPlan {
        tables,
        roots,
        checks: vec![LimitCheck {
            limit: "max_depth".to_string(),
            observed: 0,
            ok: true,
        }],
        max_depth_used: 0,
        direction: "parents".to_string(),
        complete: true,
    })
}

/// Read one table's selected rows and mask them in memory.
///
/// The SQL is built only from catalogue-validated identifiers, quoted with
/// `quote_ident`; values are always bound parameters.
pub async fn extract_table(
    session: &mut BoundSession,
    table: &TableInfo,
    root: &crate::safesync::spec::RootSelection,
    masker: &Masker,
    limits: &Limits,
) -> Result<TableRows, CoreError> {
    let name = table.name();
    let column_list = table
        .columns
        .iter()
        .map(|column| format!("{}::text", quote_ident(&column.name)))
        .collect::<Vec<_>>()
        .join(", ");

    let (where_clause, binds) = build_filter(table, root)?;

    let sql = format!(
        "SELECT {} FROM {}.{} {} LIMIT {}",
        column_list,
        quote_ident(&name.schema),
        quote_ident(&name.table),
        where_clause,
        limits.max_rows_per_table.saturating_add(1)
    );

    let mut query = sqlx::query(&sql);
    for value in &binds {
        query = query.bind(value);
    }

    session.begin_read_only().await?;
    let fetched = query.fetch_all(&mut *session.connection()).await;
    let _ = session.rollback().await;
    let rows = fetched.map_err(connection_error)?;

    if rows.len() as u64 > limits.max_rows_per_table {
        return Err(CoreError::with_detail(
            ErrorCode::Operation,
            format!(
                "{name} returned more than {} rows",
                limits.max_rows_per_table
            ),
            "limit_exceeded",
        ));
    }

    let mut masked_rows = Vec::with_capacity(rows.len());
    let mut identities = Vec::with_capacity(rows.len());
    let mut total_bytes: u64 = 0;

    for row in &rows {
        let identity = identity_of(table, row);
        let mut values = Vec::with_capacity(table.columns.len());
        let mut identity_parts = Vec::new();

        for (index, column) in table.columns.iter().enumerate() {
            let value: Option<String> = row.try_get(index).unwrap_or(None);
            let input = value.as_deref().unwrap_or("");

            let rule = masker
                .policy()
                .rule_for(&table.schema, &table.table, &column.name)
                .ok_or_else(|| {
                    CoreError::with_detail(
                        ErrorCode::Operation,
                        format!("column {} has no masking rule", column.name),
                        "unmapped_column",
                    )
                })?;

            let masked = masker.mask_value(rule, &identity, MaskInput::Text(input))?;

            if table.row_identity.contains(&column.name) {
                let rendered = masked.as_text().unwrap_or("").to_string();
                identity_parts.push(format!("{}={}", column.name, rendered));
            }

            if let Some(text) = masked.as_text() {
                total_bytes = total_bytes.saturating_add(text.len() as u64);
            }

            values.push(masked);
        }

        identities.push(identity_parts.join(","));
        masked_rows.push(values);
    }

    masker.ensure_unique(&identities)?;

    if total_bytes > limits.max_bytes {
        return Err(CoreError::with_detail(
            ErrorCode::Operation,
            format!("{name} exceeded the byte limit"),
            "limit_exceeded",
        ));
    }

    Ok(TableRows {
        table: name,
        columns: table.columns.iter().map(|c| c.name.clone()).collect(),
        types: table.columns.iter().map(|c| c.type_name.clone()).collect(),
        row_identity: table.row_identity.clone(),
        rows: masked_rows,
        total_bytes,
    })
}

/// A stable identity string from the raw row, used as the masking material.
fn identity_of(table: &TableInfo, row: &sqlx::postgres::PgRow) -> String {
    let mut parts = Vec::new();
    for column in &table.columns {
        if !table.row_identity.contains(&column.name) {
            continue;
        }
        let position = table
            .columns
            .iter()
            .position(|candidate| candidate.name == column.name)
            .unwrap_or(0);
        let value: Option<String> = row.try_get(position).unwrap_or(None);
        let value = crate::safesync::mask::normalize_identity(value.as_deref().unwrap_or(""));
        parts.push(format!("{}={}", column.name, value));
    }
    parts.join(",")
}

fn build_filter(
    table: &TableInfo,
    root: &crate::safesync::spec::RootSelection,
) -> Result<(String, Vec<String>), CoreError> {
    if root.filter.is_empty() {
        return Err(CoreError::with_detail(
            ErrorCode::Usage,
            "Slice 1 requires an explicit root filter",
            "invalid_spec",
        ));
    }

    let mut clauses = Vec::new();
    let mut binds = Vec::new();

    for predicate in &root.filter {
        if !table
            .columns
            .iter()
            .any(|column| column.name == predicate.column)
        {
            return Err(CoreError::with_detail(
                ErrorCode::Usage,
                format!("filter column {} does not exist", predicate.column),
                "invalid_identifier",
            ));
        }

        let column = quote_ident(&predicate.column);

        match predicate.op {
            FilterOp::Eq => {
                binds.push(render_value(&predicate.value)?);
                clauses.push(format!("{column}::text = ${}", binds.len()));
            }
            FilterOp::In => {
                let FilterValue::List(values) = &predicate.value else {
                    return Err(invalid_filter("an `in` filter needs a list"));
                };
                let mut placeholders = Vec::with_capacity(values.len());
                for value in values {
                    binds.push(render_value(value)?);
                    placeholders.push(format!("${}", binds.len()));
                }
                clauses.push(format!("{column}::text IN ({})", placeholders.join(", ")));
            }
            FilterOp::Gte | FilterOp::Lte => {
                return Err(CoreError::with_detail(
                    ErrorCode::Unsupported,
                    "range filters are not implemented in this slice",
                    "unsupported_filter",
                ))
            }
        }
    }

    Ok((format!("WHERE {}", clauses.join(" AND ")), binds))
}

fn render_value(value: &FilterValue) -> Result<String, CoreError> {
    match value {
        FilterValue::Bool(value) => Ok(value.to_string()),
        FilterValue::Int(value) => Ok(value.to_string()),
        FilterValue::Text(value) => Ok(value.clone()),
        FilterValue::List(_) => Err(invalid_filter("a nested list is not a value")),
    }
}

fn invalid_filter(message: &str) -> CoreError {
    CoreError::with_detail(ErrorCode::Usage, message, "invalid_spec")
}

/// Quote an identifier. The catalogue already validated it, but quoting keeps a
/// future caller from reintroducing string concatenation.
pub fn quote_ident(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

fn connection_error(error: sqlx::Error) -> CoreError {
    CoreError::redacted(
        ErrorCode::Connection,
        format!("source extraction failed: {error}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_are_quoted_and_escaped() {
        assert_eq!(quote_ident("accounts"), "\"accounts\"");
        assert_eq!(quote_ident("we\"ird"), "\"we\"\"ird\"");
    }

    #[test]
    fn a_value_is_rendered_not_interpolated() {
        assert_eq!(
            render_value(&FilterValue::Text("a'b".into())).unwrap(),
            "a'b"
        );
        assert_eq!(render_value(&FilterValue::Int(7)).unwrap(), "7");
        assert_eq!(render_value(&FilterValue::Bool(true)).unwrap(), "true");
    }

    #[test]
    fn a_nested_list_is_not_a_value() {
        let error = render_value(&FilterValue::List(vec![FilterValue::Int(1)])).unwrap_err();
        assert_eq!(error.detail.as_deref(), Some("invalid_spec"));
    }
}
