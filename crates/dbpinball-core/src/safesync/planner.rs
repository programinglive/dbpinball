//! Bounded, parameterized parent traversal.
//!
//! The selection is a closure: each selected child pulls the parent rows its
//! foreign keys reference, up to a declared depth. Every value is a bound
//! parameter and every identifier is catalogue-validated and quoted. A selection
//! that cannot be completed is marked incomplete and refused; it is never
//! imported as if it were complete.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde::Serialize;
use sqlx::Row;

use crate::domain::error::{CoreError, ErrorCode};
use crate::safesync::graph::{discover_parents, ForeignKeyEdge, SchemaGraph, TableInfo};
use crate::safesync::mask::{ColumnRule, MaskInput, MaskStrategy, MaskedValue, Masker};
use crate::safesync::session::BoundSession;
use crate::safesync::spec::{
    FilterOp, FilterValue, Limits, QualifiedName, SafeSyncSpec, TraversalDirection,
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

/// Rows after masking. Plaintext never leaves the extractor.
#[derive(Debug, Clone)]
pub struct TableRows {
    pub table: QualifiedName,
    pub columns: Vec<String>,
    /// Base type names parallel to `columns`, from the catalogue.
    pub types: Vec<String>,
    /// The columns that identify a row, in order.
    pub row_identity: Vec<String>,
    pub schema_hash: String,
    pub rls_active: bool,
    /// True where the column is GENERATED (computed by the destination).
    pub generated: Vec<bool>,
    /// True where the column is any identity column.
    pub identity: Vec<bool>,
    /// True where the column is GENERATED ALWAYS AS IDENTITY.
    pub identity_always: Vec<bool>,
    pub rows: Vec<Vec<MaskedValue>>,
    pub total_bytes: u64,
}

impl TableRows {
    pub fn row_count(&self) -> u64 {
        self.rows.len() as u64
    }
}

/// The result of a selection.
#[derive(Debug, Clone)]
pub struct SelectionResult {
    pub graph: SchemaGraph,
    /// Masked rows, already in import order.
    pub rows: Vec<TableRows>,
    pub roots: Vec<RootPlan>,
    pub order: Vec<QualifiedName>,
    /// Cycles whose every internal edge is deferrable.
    pub deferred: Vec<Vec<QualifiedName>>,
    pub checks: Vec<LimitCheck>,
    pub max_depth_used: u8,
    pub direction: String,
    pub edges: usize,
    pub complete: bool,
    /// False for a metadata-only plan, which reads no source rows.
    pub read_rows: bool,
}

/// A raw row, before masking.
type RawRow = Vec<Option<String>>;

/// A pending parent fetch: parent table, parent key columns, tuple, level.
type ParentRequest = (QualifiedName, Vec<String>, Vec<Option<String>>, u8);

/// Select the bounded parent closure, mask it, and order the import.
pub async fn build(
    session: &mut BoundSession,
    spec: &SafeSyncSpec,
    masker: &Masker,
    read_rows: bool,
) -> Result<SelectionResult, CoreError> {
    if spec.selection.relationships.direction != TraversalDirection::Parents {
        return Err(CoreError::with_detail(
            ErrorCode::Unsupported,
            "child traversal is not implemented in this slice",
            "unsupported_traversal",
        ));
    }

    let depth = spec.selection.relationships.depth;
    let limits = &spec.limits;

    let root_names = spec
        .selection
        .roots
        .iter()
        .map(|root| QualifiedName::parse(&root.table))
        .collect::<Result<Vec<_>, _>>()?;

    // A non-zero depth needs row values to follow foreign keys; a metadata-only
    // plan is only possible for a flat selection.
    let effective_read = read_rows || depth > 0;
    let graph =
        discover_parents(session, &root_names, if effective_read { depth } else { 0 }).await?;

    // The source role must be safe for every discovered table, not just the
    // roots: a parent the role owns would bypass RLS on it.
    let discovered: Vec<QualifiedName> = graph.tables.iter().map(|table| table.name()).collect();
    session.assert_source_role(&discovered).await?;

    // Validate every discovered column's rule up front so the read path and the
    // metadata path agree on what a valid policy is.
    for info in &graph.tables {
        masker.coverage(&info.schema, &info.table, &info.column_shapes(&graph))?;
    }

    if !effective_read {
        return metadata_only(spec, &graph, masker);
    }

    let mut selected: BTreeMap<QualifiedName, Vec<RawRow>> = BTreeMap::new();
    let mut seen: BTreeMap<QualifiedName, BTreeSet<String>> = BTreeMap::new();
    let mut roots = Vec::new();
    let mut checks: Vec<LimitCheck> = Vec::new();
    let mut complete = true;
    let mut max_depth_used = 0u8;

    // Roots.
    for root in &spec.selection.roots {
        let name = QualifiedName::parse(&root.table)?;
        let info = graph
            .table(&name)
            .ok_or_else(|| missing_table(&name))?
            .clone();
        let raw = fetch_root(session, &info, root, limits).await?;
        if raw.len() as u64 > limits.max_rows_per_table {
            return Err(CoreError::with_detail(
                ErrorCode::Operation,
                format!(
                    "{name} returned more than {} rows",
                    limits.max_rows_per_table
                ),
                "limit_exceeded",
            ));
        }
        insert_rows(&info, raw, &mut selected, &mut seen);

        // The limit is cumulative across roots on the same table.
        let accumulated = selected
            .get(&name)
            .map(|rows| rows.len() as u64)
            .unwrap_or(0);
        if accumulated > limits.max_rows_per_table {
            return Err(CoreError::with_detail(
                ErrorCode::Operation,
                format!(
                    "{name} exceeded max_rows_per_table ({})",
                    limits.max_rows_per_table
                ),
                "limit_exceeded",
            ));
        }

        let canonical = crate::safesync::spec::canonical_filter(&root.filter);
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

    // Parent worklist. A request is (parent, parent columns, tuple, level).
    let mut queue: VecDeque<ParentRequest> = VecDeque::new();
    let mut requested: BTreeSet<ParentRequestKey> = BTreeSet::new();

    let mut truncated = false;
    for name in &root_names {
        if seed_parents(
            &graph,
            name,
            &selected,
            0,
            depth,
            &mut queue,
            &mut requested,
        ) {
            truncated = true;
        }
    }

    while let Some((parent, parent_columns, tuple, level)) = queue.pop_front() {
        // A NULL component means PostgreSQL does not enforce the reference, so
        // there is no parent to pull.
        if tuple.iter().any(Option::is_none) {
            continue;
        }

        let Some(info) = graph.table(&parent).cloned() else {
            complete = false;
            checks.push(LimitCheck {
                limit: "missing_parent".into(),
                observed: 0,
                ok: false,
            });
            continue;
        };

        let raw = fetch_by_key(session, &info, &parent_columns, &tuple, limits).await?;
        if raw.is_empty() {
            // A referenced parent that the restricted role cannot see is an
            // incomplete closure, not a silently missing row.
            complete = false;
            checks.push(LimitCheck {
                limit: "missing_parent".into(),
                observed: 0,
                ok: false,
            });
            continue;
        }
        if raw.len() as u64 > 1 {
            // A parent key fetch must be unique; more than one row means the
            // relationship is not the unique key we assumed.
            complete = false;
            checks.push(LimitCheck {
                limit: "ambiguous_parent".into(),
                observed: raw.len() as u64,
                ok: false,
            });
            continue;
        }

        max_depth_used = max_depth_used.max(level);
        insert_rows(&info, raw, &mut selected, &mut seen);
        let parent_rows = selected
            .get(&parent)
            .map(|rows| rows.len() as u64)
            .unwrap_or(0);
        if parent_rows > limits.max_rows_per_table {
            return Err(CoreError::with_detail(
                ErrorCode::Operation,
                format!(
                    "{parent} exceeded max_rows_per_table ({})",
                    limits.max_rows_per_table
                ),
                "limit_exceeded",
            ));
        }
        if seed_parents(
            &graph,
            &parent,
            &selected,
            level,
            depth,
            &mut queue,
            &mut requested,
        ) {
            truncated = true;
        }
    }

    if truncated {
        complete = false;
        checks.push(LimitCheck {
            limit: "closure".into(),
            observed: u64::from(depth),
            ok: false,
        });
    }

    // Global bounds.
    let total_rows: u64 = selected.values().map(|rows| rows.len() as u64).sum();
    let total_bytes: u64 = selected
        .values()
        .flat_map(|rows| rows.iter())
        .flat_map(|row| row.iter())
        .map(|value| value.as_deref().unwrap_or("").len() as u64)
        .sum();

    if total_rows > limits.max_total_rows {
        return Err(CoreError::with_detail(
            ErrorCode::Operation,
            "the selection exceeded max_total_rows",
            "limit_exceeded",
        ));
    }
    if total_bytes > limits.max_bytes {
        return Err(CoreError::with_detail(
            ErrorCode::Operation,
            "the selection exceeded max_bytes",
            "limit_exceeded",
        ));
    }

    // Mask, in a deterministic table order.
    let mut masked: Vec<TableRows> = Vec::with_capacity(selected.len());
    for (name, raw_rows) in &selected {
        let info = graph.table(name).ok_or_else(|| missing_table(name))?;
        masked.push(mask_table(info, raw_rows, masker, &graph)?);
    }

    // Import order, cycles and deferral.
    let names: Vec<QualifiedName> = selected.keys().cloned().collect();
    let edges = graph.edges_between(&names);
    let ordering = order_tables(&names, &edges)?;
    checks.push(LimitCheck {
        limit: "max_depth".into(),
        observed: u64::from(max_depth_used),
        ok: max_depth_used <= depth,
    });

    if !complete {
        checks.push(LimitCheck {
            limit: "closure".into(),
            observed: 0,
            ok: false,
        });
    }

    let mut ordered_rows = Vec::with_capacity(masked.len());
    for name in &ordering.order {
        if let Some(rows) = masked.iter().find(|rows| &rows.table == name) {
            ordered_rows.push(rows.clone());
        }
    }

    Ok(SelectionResult {
        rows: ordered_rows,
        graph,
        roots,
        order: ordering.order,
        deferred: ordering.deferred,
        checks,
        max_depth_used,
        direction: "parents".to_string(),
        edges: edges.len(),
        complete: complete && ordering.complete,
        read_rows: true,
    })
}

/// Inspect and validate without reading any row. Only valid for depth zero.
fn metadata_only(
    spec: &SafeSyncSpec,
    graph: &SchemaGraph,
    masker: &Masker,
) -> Result<SelectionResult, CoreError> {
    let mut roots = Vec::new();
    for root in &spec.selection.roots {
        let name = QualifiedName::parse(&root.table)?;
        let info = graph.table(&name).ok_or_else(|| missing_table(&name))?;
        masker.coverage(&name.schema, &name.table, &info.column_shapes(graph))?;
        let canonical = crate::safesync::spec::canonical_filter(&root.filter);
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

    let names: Vec<QualifiedName> = graph.tables.iter().map(|table| table.name()).collect();
    let edges = graph.edges_between(&names);
    let ordering = order_tables(&names, &edges)?;

    let rows = graph
        .tables
        .iter()
        .map(|info| TableRows {
            table: info.name(),
            columns: info.columns.iter().map(|c| c.name.clone()).collect(),
            types: info.columns.iter().map(|c| c.type_name.clone()).collect(),
            row_identity: info.row_identity.clone(),
            schema_hash: info.schema_hash.clone(),
            rls_active: info.rls_active,
            generated: info.columns.iter().map(|c| c.is_generated).collect(),
            identity: info.columns.iter().map(|c| c.is_identity).collect(),
            identity_always: info.columns.iter().map(|c| c.identity_always).collect(),
            rows: Vec::new(),
            total_bytes: 0,
        })
        .collect();

    Ok(SelectionResult {
        graph: graph.clone(),
        rows,
        roots,
        order: ordering.order,
        deferred: ordering.deferred,
        checks: {
            let mut checks = vec![LimitCheck {
                limit: "max_depth".into(),
                observed: 0,
                ok: true,
            }];
            if !graph.edges.is_empty() {
                checks.push(LimitCheck {
                    limit: "closure".into(),
                    observed: 0,
                    ok: false,
                });
            }
            checks
        },
        max_depth_used: 0,
        direction: "parents".to_string(),
        edges: edges.len(),
        // A depth-0 metadata plan for a table with a foreign key is truncated:
        // the referenced parent is not selected.
        complete: graph.edges.is_empty(),
        read_rows: false,
    })
}

#[derive(Debug)]
struct Ordering {
    order: Vec<QualifiedName>,
    deferred: Vec<Vec<QualifiedName>>,
    complete: bool,
}

/// Parents before children (Kahn). A remaining cycle is deferred only when every
/// edge inside it is deferrable.
fn order_tables(tables: &[QualifiedName], edges: &[ForeignKeyEdge]) -> Result<Ordering, CoreError> {
    let mut indegree: BTreeMap<QualifiedName, usize> =
        tables.iter().map(|name| (name.clone(), 0usize)).collect();
    let mut children: BTreeMap<QualifiedName, Vec<QualifiedName>> = BTreeMap::new();

    for edge in edges {
        // A self-reference is a cycle for ordering purposes: row order inside a
        // table is not guaranteed, so it needs deferral too.
        *indegree.entry(edge.child.clone()).or_insert(0) += 1;
        children
            .entry(edge.parent.clone())
            .or_default()
            .push(edge.child.clone());
    }

    let mut ready: BTreeSet<QualifiedName> = indegree
        .iter()
        .filter(|(_, degree)| **degree == 0)
        .map(|(name, _)| name.clone())
        .collect();

    let mut order = Vec::with_capacity(tables.len());
    while let Some(name) = ready.iter().next().cloned() {
        ready.remove(&name);
        order.push(name.clone());
        if let Some(list) = children.get(&name) {
            for child in list {
                if let Some(degree) = indegree.get_mut(child) {
                    *degree = degree.saturating_sub(1);
                    if *degree == 0 {
                        ready.insert(child.clone());
                    }
                }
            }
        }
    }

    if order.len() == tables.len() {
        return Ok(Ordering {
            order,
            deferred: Vec::new(),
            complete: true,
        });
    }

    let remaining: Vec<QualifiedName> = tables
        .iter()
        .filter(|name| !order.contains(name))
        .cloned()
        .collect();

    let cycle_edges: Vec<&ForeignKeyEdge> = edges
        .iter()
        .filter(|edge| remaining.contains(&edge.child) && remaining.contains(&edge.parent))
        .collect();

    if cycle_edges.iter().all(|edge| edge.deferrable) {
        let mut deferred = remaining.clone();
        deferred.sort();
        order.extend(deferred.iter().cloned());
        Ok(Ordering {
            order,
            deferred: vec![deferred],
            complete: true,
        })
    } else {
        Err(CoreError::with_detail(
            ErrorCode::Unsupported,
            "the selected tables contain a foreign-key cycle whose constraints are not deferrable",
            "cyclic_fk_requires_deferrable",
        ))
    }
}

/// A structurally typed deduplication key for a pending parent fetch.
///
/// A typed key is used instead of a joined string so a separator byte inside a
/// value cannot make two distinct requests look identical. `Option<String>`
/// also keeps NULL distinct from the empty string.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ParentRequestKey {
    parent: QualifiedName,
    parent_columns: Vec<String>,
    tuple: Vec<Option<String>>,
}

fn seed_parents(
    graph: &SchemaGraph,
    child: &QualifiedName,
    selected: &BTreeMap<QualifiedName, Vec<RawRow>>,
    level: u8,
    depth: u8,
    queue: &mut VecDeque<ParentRequest>,
    requested: &mut BTreeSet<ParentRequestKey>,
) -> bool {
    let mut truncated = false;
    let Some(info) = graph.table(child) else {
        return false;
    };
    let Some(rows) = selected.get(child) else {
        return false;
    };

    for edge in graph.outgoing(child) {
        for row in rows {
            let Ok(tuple) = key_tuple(info, row, &edge.child_columns) else {
                continue;
            };
            // A NULL component means PostgreSQL does not enforce the reference
            // under MATCH SIMPLE. Under MATCH FULL a partial NULL is invalid, so
            // it must not be treated as a satisfied closure.
            let nulls = tuple.iter().filter(|value| value.is_none()).count();
            if nulls > 0 {
                if edge.match_type == "f" && nulls < tuple.len() {
                    truncated = true;
                }
                continue;
            }
            let key = ParentRequestKey {
                parent: edge.parent.clone(),
                parent_columns: edge.parent_columns.clone(),
                tuple: tuple.clone(),
            };
            if level >= depth {
                // A real reference that the depth cannot follow. It is only a
                // truncation if the parent row is neither already selected nor
                // already queued by a sibling at a shallower level.
                let present = graph
                    .table(&edge.parent)
                    .map(|info| {
                        parent_present(selected, &edge.parent, info, &edge.parent_columns, &tuple)
                    })
                    .unwrap_or(false);
                if !present && !requested.contains(&key) {
                    truncated = true;
                }
                continue;
            }
            if requested.insert(key) {
                queue.push_back((
                    edge.parent.clone(),
                    edge.parent_columns.clone(),
                    tuple,
                    level + 1,
                ));
            }
        }
    }

    truncated
}

fn key_tuple(
    info: &TableInfo,
    row: &RawRow,
    columns: &[String],
) -> Result<Vec<Option<String>>, CoreError> {
    let mut tuple = Vec::with_capacity(columns.len());
    for column in columns {
        let index = info
            .columns
            .iter()
            .position(|candidate| &candidate.name == column)
            .ok_or_else(|| {
                CoreError::with_detail(
                    ErrorCode::Operation,
                    format!("key column {column} is missing from the selected row"),
                    "invalid_key_tuple",
                )
            })?;
        tuple.push(row.get(index).cloned().flatten());
    }
    Ok(tuple)
}

/// The columns whose values form the key tuple for this rule.
///
/// On the referenced table that is the key itself. On a referencing table it is
/// the foreign-key child columns, which may be named differently. This is what
/// makes a parent key and its child foreign key derive the same output.
fn key_columns_for(info: &TableInfo, rule: &ColumnRule, graph: &SchemaGraph) -> Vec<String> {
    let Some(domain) = rule.key_domain.as_ref() else {
        return vec![rule.column.clone()];
    };
    let Ok(parent) = QualifiedName::parse(&domain.table) else {
        return vec![rule.column.clone()];
    };

    let child = info.name();
    for edge in graph.edges.iter().filter(|edge| {
        edge.child == child && edge.parent == parent && edge.parent_columns == domain.columns
    }) {
        if edge.child_columns.contains(&rule.column) {
            return edge.child_columns.clone();
        }
    }

    if parent == child {
        return domain.columns.clone();
    }

    vec![rule.column.clone()]
}

fn edge_for<'a>(
    info: &TableInfo,
    rule: &ColumnRule,
    graph: &'a SchemaGraph,
) -> Option<&'a ForeignKeyEdge> {
    let domain = rule.key_domain.as_ref()?;
    let parent = QualifiedName::parse(&domain.table).ok()?;
    graph.edges.iter().find(|edge| {
        edge.child == info.name()
            && edge.parent == parent
            && edge.parent_columns == domain.columns
            && edge.child_columns.contains(&rule.column)
    })
}

/// True when the referenced parent row is already in the selection.
fn parent_present(
    selected: &BTreeMap<QualifiedName, Vec<RawRow>>,
    parent: &QualifiedName,
    parent_info: &TableInfo,
    parent_columns: &[String],
    tuple: &[Option<String>],
) -> bool {
    let Some(rows) = selected.get(parent) else {
        return false;
    };
    rows.iter().any(|row| {
        parent_columns.iter().enumerate().all(|(index, column)| {
            parent_info
                .columns
                .iter()
                .position(|candidate| &candidate.name == column)
                .map(|position| {
                    row.get(position).cloned().flatten() == tuple.get(index).cloned().flatten()
                })
                .unwrap_or(false)
        })
    })
}

fn insert_rows(
    info: &TableInfo,
    rows: Vec<RawRow>,
    selected: &mut BTreeMap<QualifiedName, Vec<RawRow>>,
    seen: &mut BTreeMap<QualifiedName, BTreeSet<String>>,
) {
    let name = info.name();
    let entry = selected.entry(name.clone()).or_default();
    let identities = seen.entry(name).or_default();
    for row in rows {
        let identity = raw_identity(info, &row);
        if identities.insert(identity) {
            entry.push(row);
        }
    }
}

/// The row identity from the declared key columns, length-prefixed so the
/// encoding is unambiguous.
fn raw_identity(info: &TableInfo, row: &RawRow) -> String {
    let mut parts = Vec::new();
    for column in &info.row_identity {
        if let Some(index) = info
            .columns
            .iter()
            .position(|candidate| &candidate.name == column)
        {
            let value = row.get(index).and_then(|v| v.as_deref()).unwrap_or("");
            parts.push(format!("{}:{}", value.len(), value));
        }
    }
    if parts.is_empty() {
        // The catalogue guarantees a row identity; this only keeps the function
        // total if a future caller violates that.
        for value in row {
            let value = value.as_deref().unwrap_or("");
            parts.push(format!("{}:{}", value.len(), value));
        }
    }
    parts.join("\u{1f}")
}

async fn fetch_root(
    session: &mut BoundSession,
    info: &TableInfo,
    root: &crate::safesync::spec::RootSelection,
    limits: &Limits,
) -> Result<Vec<RawRow>, CoreError> {
    let (where_clause, binds) = build_filter(info, root)?;
    fetch(session, info, &where_clause, &binds, limits).await
}

async fn fetch_by_key(
    session: &mut BoundSession,
    info: &TableInfo,
    columns: &[String],
    tuple: &[Option<String>],
    limits: &Limits,
) -> Result<Vec<RawRow>, CoreError> {
    let mut clauses = Vec::with_capacity(columns.len());
    let mut binds = Vec::with_capacity(columns.len());
    for (index, column) in columns.iter().enumerate() {
        if !info
            .columns
            .iter()
            .any(|candidate| &candidate.name == column)
        {
            return Err(CoreError::with_detail(
                ErrorCode::Usage,
                format!("parent key column {column} does not exist"),
                "invalid_identifier",
            ));
        }
        clauses.push(format!("{}::text = ${}", quote_ident(column), index + 1));
        binds.push(tuple[index].clone().unwrap_or_default());
    }

    let where_clause = format!("WHERE {}", clauses.join(" AND "));
    fetch(session, info, &where_clause, &binds, limits).await
}

async fn fetch(
    session: &mut BoundSession,
    info: &TableInfo,
    where_clause: &str,
    binds: &[String],
    limits: &Limits,
) -> Result<Vec<RawRow>, CoreError> {
    let column_list = info
        .columns
        .iter()
        .map(|column| format!("{}::text", quote_ident(&column.name)))
        .collect::<Vec<_>>()
        .join(", ");

    let sql = format!(
        "SELECT {} FROM {}.{} {} LIMIT {}",
        column_list,
        quote_ident(&info.schema),
        quote_ident(&info.table),
        where_clause,
        limits.max_rows_per_table.saturating_add(1)
    );

    let mut query = sqlx::query(&sql);
    for value in binds {
        query = query.bind(value);
    }

    session.begin_read_only().await?;
    let fetched = query.fetch_all(&mut *session.connection()).await;
    let _ = session.rollback().await;
    let rows = fetched.map_err(connection_error)?;

    let mut output = Vec::with_capacity(rows.len());
    for row in &rows {
        let mut values = Vec::with_capacity(info.columns.len());
        for index in 0..info.columns.len() {
            let value: Option<String> = row.try_get(index).unwrap_or(None);
            values.push(value);
        }
        output.push(values);
    }
    Ok(output)
}

fn mask_table(
    info: &TableInfo,
    raw_rows: &[RawRow],
    masker: &Masker,
    graph: &SchemaGraph,
) -> Result<TableRows, CoreError> {
    let mut rows = Vec::with_capacity(raw_rows.len());
    let mut total_bytes = 0u64;
    let mut identities: Vec<String> = Vec::with_capacity(raw_rows.len());

    for raw in raw_rows {
        let identity = identity_string(info, raw);
        let mut values = Vec::with_capacity(info.columns.len());

        for (index, column) in info.columns.iter().enumerate() {
            let rule = masker
                .policy()
                .rule_for(&info.schema, &info.table, &column.name)
                .ok_or_else(|| {
                    CoreError::with_detail(
                        ErrorCode::Operation,
                        format!("column {} has no masking rule", column.name),
                        "unmapped_column",
                    )
                })?;

            let masked = if rule.strategy == MaskStrategy::DeterministicKey {
                let domain = rule.key_domain.as_ref().ok_or_else(|| {
                    CoreError::with_detail(
                        ErrorCode::Operation,
                        format!("key column {} has no key domain", column.name),
                        "unmapped_key_domain",
                    )
                })?;
                let tuple_columns = key_columns_for(info, rule, graph);
                if tuple_columns.get(rule.key_position) != Some(&column.name) {
                    return Err(CoreError::with_detail(
                        ErrorCode::Operation,
                        format!(
                            "key position {} does not point at column {} in the resolved tuple",
                            rule.key_position, column.name
                        ),
                        "invalid_key_position",
                    ));
                }
                let tuple = key_tuple(info, raw, &tuple_columns)?;
                let _ = domain;

                // MATCH FULL forbids a partially NULL key; refuse rather than
                // inventing a parent dependency or a synthetic key.
                if let Some(edge) = edge_for(info, rule, graph) {
                    if edge.match_type == "f" {
                        let nulls = tuple.iter().filter(|value| value.is_none()).count();
                        if nulls > 0 && nulls < tuple.len() {
                            return Err(CoreError::with_detail(
                                ErrorCode::Unsupported,
                                format!(
                                    "column {} has a partially NULL MATCH FULL key",
                                    column.name
                                ),
                                "match_full_partial_null",
                            ));
                        }
                    }
                }

                masker.mask_key(rule, &tuple)?
            } else {
                let input = match raw.get(index).and_then(|value| value.as_deref()) {
                    Some(value) => MaskInput::Text(value),
                    None => MaskInput::Null,
                };
                masker.mask_value(rule, &identity, input)?
            };

            if let Some(text) = masked.as_text() {
                total_bytes = total_bytes.saturating_add(text.len() as u64);
            }
            values.push(masked);
        }

        // The masked row identity must be unique.
        let mut identity_parts = Vec::new();
        for (index, column) in info.columns.iter().enumerate() {
            if info.row_identity.contains(&column.name) {
                // A NULL identity is refused earlier, but never conflate it
                // with an empty string if it ever reaches here.
                let rendered = match &values[index] {
                    MaskedValue::Null => "\u{0}null".to_string(),
                    other => other.as_text().unwrap_or("").to_string(),
                };
                identity_parts.push(format!(
                    "{}={}",
                    column.name,
                    crate::safesync::mask::normalize_identity(&rendered)
                ));
            }
        }
        identities.push(identity_parts.join(","));
        rows.push(values);
    }

    masker.ensure_unique(&identities)?;

    Ok(TableRows {
        table: info.name(),
        columns: info.columns.iter().map(|c| c.name.clone()).collect(),
        types: info.columns.iter().map(|c| c.type_name.clone()).collect(),
        row_identity: info.row_identity.clone(),
        schema_hash: info.schema_hash.clone(),
        rls_active: info.rls_active,
        generated: info.columns.iter().map(|c| c.is_generated).collect(),
        identity: info.columns.iter().map(|c| c.is_identity).collect(),
        identity_always: info.columns.iter().map(|c| c.identity_always).collect(),
        rows,
        total_bytes,
    })
}

fn identity_string(info: &TableInfo, raw: &RawRow) -> String {
    let mut parts = Vec::new();
    for column in &info.columns {
        if !info.row_identity.contains(&column.name) {
            continue;
        }
        let Some(index) = info.columns.iter().position(|c| c.name == column.name) else {
            continue;
        };
        let value = raw.get(index).and_then(|v| v.as_deref()).unwrap_or("");
        parts.push(format!(
            "{}={}",
            column.name,
            crate::safesync::mask::normalize_identity(value)
        ));
    }
    parts.join(",")
}

fn build_filter(
    info: &TableInfo,
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
        if !info
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

/// Extract and mask one table with no traversal. Kept as a stable helper for
/// callers and tests that only need a single table.
/// Extract and mask one table with no traversal.
///
/// This is a test-facing helper only. It is compiled out of production builds,
/// refuses a table that has foreign keys (an empty graph would hide real
/// dependencies), and enforces the row bound.
#[cfg(any(test, feature = "integration"))]
pub async fn extract_table(
    session: &mut BoundSession,
    info: &TableInfo,
    root: &crate::safesync::spec::RootSelection,
    masker: &Masker,
    limits: &Limits,
) -> Result<TableRows, CoreError> {
    let name = info.name();
    let graph = discover_parents(session, std::slice::from_ref(&name), 0).await?;
    if !graph.edges.is_empty() {
        return Err(CoreError::with_detail(
            ErrorCode::Unsupported,
            format!("{name} has foreign keys; use the bounded planner"),
            "unguarded_extraction_refused",
        ));
    }

    let raw = fetch_root(session, info, root, limits).await?;
    if raw.len() as u64 > limits.max_rows_per_table {
        return Err(CoreError::with_detail(
            ErrorCode::Operation,
            format!("{name} exceeded max_rows_per_table"),
            "limit_exceeded",
        ));
    }
    mask_table(info, &raw, masker, &graph)
}

fn missing_table(name: &QualifiedName) -> CoreError {
    CoreError::with_detail(
        ErrorCode::Usage,
        format!("{name} was not discovered in the catalogue"),
        "invalid_identifier",
    )
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
    crate::safesync::database_error(
        ErrorCode::Connection,
        "source extraction",
        "source_error",
        error,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::safesync::graph::ForeignKeyEdge;

    fn name(value: &str) -> QualifiedName {
        QualifiedName::parse(value).unwrap()
    }

    #[test]
    fn the_dedup_key_cannot_collapse_delimiter_values() {
        let build = |region: &str, code: &str| ParentRequestKey {
            parent: name("public.shipments"),
            parent_columns: vec!["region".into(), "code".into()],
            tuple: vec![Some(region.into()), Some(code.into())],
        };

        // The old joined marker produced the same string for both.
        assert_ne!(build("a", "b\u{1f}c"), build("a\u{1f}b", "c"));
        assert_ne!(build("a", "b\u{1e}c"), build("a\u{1e}b", "c"));
        assert_ne!(build("a", "b\u{1fe}c"), build("a\u{1f}", "b\u{1e}c"));
        assert_eq!(build("a", "b"), build("a", "b"));
    }

    #[test]
    fn the_dedup_key_keeps_null_and_empty_distinct() {
        let key = |value: Option<&str>| ParentRequestKey {
            parent: name("public.parent"),
            parent_columns: vec!["id".into()],
            tuple: vec![value.map(str::to_string)],
        };
        assert_ne!(key(None), key(Some("")));
    }

    #[test]
    fn the_dedup_key_binds_parent_and_column_order() {
        let first = ParentRequestKey {
            parent: name("public.shipments"),
            parent_columns: vec!["region".into(), "code".into()],
            tuple: vec![Some("a".into()), Some("b".into())],
        };
        let other_table = ParentRequestKey {
            parent: name("public.other"),
            ..first.clone()
        };
        let other_order = ParentRequestKey {
            parent_columns: vec!["code".into(), "region".into()],
            tuple: vec![Some("b".into()), Some("a".into())],
            ..first.clone()
        };
        assert_ne!(first, other_table);
        assert_ne!(first, other_order);
    }

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
    fn a_parent_is_ordered_before_its_child() {
        let tables = vec![name("public.orders"), name("public.customers")];
        let edges = vec![ForeignKeyEdge {
            name: "orders_customer_fk".into(),
            child: name("public.orders"),
            parent: name("public.customers"),
            child_columns: vec!["customer_code".into()],
            parent_columns: vec!["code".into()],
            deferrable: false,
            initially_deferred: false,
            match_type: "s".into(),
        }];
        let ordering = order_tables(&tables, &edges).unwrap();
        assert_eq!(
            ordering.order,
            vec![name("public.customers"), name("public.orders")]
        );
        assert!(ordering.deferred.is_empty());
    }

    #[test]
    fn a_deferrable_cycle_is_deferred() {
        let tables = vec![name("public.a"), name("public.b")];
        let edges = vec![
            ForeignKeyEdge {
                name: "a_b".into(),
                child: name("public.a"),
                parent: name("public.b"),
                child_columns: vec!["b_id".into()],
                parent_columns: vec!["id".into()],
                deferrable: true,
                initially_deferred: false,
                match_type: "s".into(),
            },
            ForeignKeyEdge {
                name: "b_a".into(),
                child: name("public.b"),
                parent: name("public.a"),
                child_columns: vec!["a_id".into()],
                parent_columns: vec!["id".into()],
                deferrable: true,
                initially_deferred: false,
                match_type: "s".into(),
            },
        ];
        let ordering = order_tables(&tables, &edges).unwrap();
        assert_eq!(ordering.deferred.len(), 1);
        assert_eq!(ordering.order.len(), 2);
    }

    #[test]
    fn a_non_deferrable_cycle_is_refused() {
        let tables = vec![name("public.a"), name("public.b")];
        let edges = vec![
            ForeignKeyEdge {
                name: "a_b".into(),
                child: name("public.a"),
                parent: name("public.b"),
                child_columns: vec!["b_id".into()],
                parent_columns: vec!["id".into()],
                deferrable: true,
                initially_deferred: false,
                match_type: "s".into(),
            },
            ForeignKeyEdge {
                name: "b_a".into(),
                child: name("public.b"),
                parent: name("public.a"),
                child_columns: vec!["a_id".into()],
                parent_columns: vec!["id".into()],
                deferrable: false,
                initially_deferred: false,
                match_type: "s".into(),
            },
        ];
        let error = order_tables(&tables, &edges).unwrap_err();
        assert_eq!(
            error.detail.as_deref(),
            Some("cyclic_fk_requires_deferrable")
        );
    }

    #[test]
    fn a_self_reference_is_treated_as_a_cycle() {
        let tables = vec![name("public.employees")];
        let edges = vec![ForeignKeyEdge {
            name: "employees_manager_fk".into(),
            child: name("public.employees"),
            parent: name("public.employees"),
            child_columns: vec!["manager_id".into()],
            parent_columns: vec!["id".into()],
            deferrable: true,
            initially_deferred: false,
            match_type: "s".into(),
        }];
        let ordering = order_tables(&tables, &edges).unwrap();
        assert_eq!(ordering.deferred.len(), 1);
    }
}
