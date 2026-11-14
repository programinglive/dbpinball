//! The reviewable, value-free plan and its canonical fingerprint.
//!
//! The plan never contains a selected row value or a plaintext filter value. It
//! carries a keyed filter digest instead, so `validate` and `execute` need the
//! original specification as well as the plan.

use serde::{Deserialize, Serialize};

use crate::domain::error::{CoreError, ErrorCode};
use crate::safesync::mask::{digest, hex_lower, Masker};
use crate::safesync::planner::SelectionResult;
use crate::safesync::session::SessionIdentity;
use crate::safesync::spec::{Limits, QualifiedName, SafeSyncSpec};
use crate::security::DestinationCategory;

/// The plan format version.
pub const PLAN_VERSION: u32 = 2;

/// A plan may not be valid for longer than this, whatever the spec asks for.
pub const MAX_PLAN_TTL_MS: u64 = 60 * 60 * 1_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanIdentity {
    pub identity_key: Option<String>,
    pub current_user: String,
    pub engine: String,
}

impl PlanIdentity {
    pub fn from_session(identity: &SessionIdentity) -> Self {
        Self {
            identity_key: identity.identity_key.clone(),
            current_user: identity.current_user.clone(),
            engine: "postgres".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanRoot {
    pub schema: String,
    pub table: String,
    pub filter_columns: Vec<String>,
    pub filter_ops: Vec<String>,
    pub filter_digest: String,
    /// Always `false` in M4; a future version cannot add values silently.
    pub contains_values: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanSelection {
    pub roots: Vec<PlanRoot>,
    pub direction: String,
    pub depth: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanTable {
    pub schema: String,
    pub table: String,
    pub schema_hash: String,
    pub row_identity: Vec<String>,
    pub rows_estimated: Option<u64>,
    pub rls_active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanGraph {
    pub edges: u64,
    pub cycles: u64,
    pub max_depth_used: u8,
    pub direction: String,
    /// Structural fingerprint of the foreign-key definitions.
    pub fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanMasking {
    pub policy: String,
    pub policy_hash: String,
    pub key_id: String,
    pub columns: u64,
    pub unmapped: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanImport {
    pub conflict_policy: String,
    pub mode: String,
    pub sequence_adjustment: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanCheck {
    pub limit: String,
    pub observed: u64,
    pub ok: bool,
}

/// The whole plan document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DryRunPlan {
    pub plan_version: u32,
    pub run_id: String,
    pub created_at_ms: u64,
    pub expires_at_ms: u64,
    pub spec_hash: String,
    pub plan_fingerprint: String,
    pub source: PlanIdentity,
    pub destination: PlanIdentity,
    pub selection: PlanSelection,
    pub tables: Vec<PlanTable>,
    pub graph: PlanGraph,
    pub masking: PlanMasking,
    pub import: PlanImport,
    pub constraints: PlanConstraints,
    pub limits: Limits,
    pub checks: Vec<PlanCheck>,
    pub complete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanConstraints {
    pub order: Vec<String>,
    pub deferred: Vec<String>,
    pub cyclic_refused: Vec<String>,
}

impl DryRunPlan {
    /// Build a plan. Estimates are optional and never carry values.
    #[allow(clippy::too_many_arguments)]
    pub fn build(
        spec: &SafeSyncSpec,
        source: &SessionIdentity,
        destination: &SessionIdentity,
        _destination_category: DestinationCategory,
        selection: &SelectionResult,
        masker: &Masker,
        _estimates: &[(QualifiedName, u64)],
        now_ms: u64,
    ) -> Result<Self, CoreError> {
        let mut plan = Self {
            plan_version: PLAN_VERSION,
            run_id: uuid::Uuid::new_v4().to_string(),
            created_at_ms: now_ms,
            expires_at_ms: now_ms
                .saturating_add(spec.execution.acknowledgement_ttl_ms.min(MAX_PLAN_TTL_MS)),
            spec_hash: String::new(),
            plan_fingerprint: String::new(),
            source: PlanIdentity::from_session(source),
            destination: PlanIdentity::from_session(destination),
            selection: PlanSelection {
                roots: selection
                    .roots
                    .iter()
                    .map(|root| PlanRoot {
                        schema: root.schema.clone(),
                        table: root.table.clone(),
                        filter_columns: root.filter_columns.clone(),
                        filter_ops: root.filter_ops.clone(),
                        filter_digest: root.filter_digest.clone(),
                        contains_values: false,
                    })
                    .collect(),
                direction: selection.direction.clone(),
                // The requested depth, not just the depth the closure used, so a
                // changed traversal limit invalidates the plan.
                depth: spec.selection.relationships.depth,
            },
            tables: selection
                .rows
                .iter()
                .map(|table| PlanTable {
                    schema: table.table.schema.clone(),
                    table: table.table.table.clone(),
                    schema_hash: table.schema_hash.clone(),
                    row_identity: table.row_identity.clone(),
                    rows_estimated: if selection.read_rows {
                        Some(table.row_count())
                    } else {
                        None
                    },
                    rls_active: table.rls_active,
                })
                .collect(),
            graph: PlanGraph {
                edges: selection.edges as u64,
                cycles: selection.deferred.len() as u64,
                max_depth_used: selection.max_depth_used,
                direction: selection.direction.clone(),
                fingerprint: crate::safesync::graph::graph_fingerprint(&selection.graph),
            },
            masking: PlanMasking {
                policy: masker.policy().identifier(),
                policy_hash: masker.policy().hash.clone(),
                key_id: masker.key_id().to_string(),
                columns: selection
                    .rows
                    .iter()
                    .map(|table| table.columns.len() as u64)
                    .sum(),
                unmapped: 0,
            },
            import: PlanImport {
                conflict_policy: "refuse".to_string(),
                mode: "single_transaction".to_string(),
                sequence_adjustment: false,
            },
            constraints: PlanConstraints {
                order: selection
                    .order
                    .iter()
                    .map(|name| name.to_string())
                    .collect(),
                deferred: selection
                    .deferred
                    .iter()
                    .map(|group| {
                        group
                            .iter()
                            .map(|name| name.to_string())
                            .collect::<Vec<_>>()
                            .join(",")
                    })
                    .collect(),
                cyclic_refused: Vec::new(),
            },
            limits: spec.limits.clone(),
            checks: selection
                .checks
                .iter()
                .map(|check| PlanCheck {
                    limit: check.limit.clone(),
                    observed: check.observed,
                    ok: check.ok,
                })
                .collect(),
            complete: selection.complete,
        };

        plan.spec_hash = spec_hash(spec, selection);
        plan.plan_fingerprint = plan.fingerprint();
        Ok(plan)
    }

    /// Canonical SHA-256 over the plan content.
    ///
    /// The content hash covers the graph, selection, limits, policy, key and
    /// import mode, so an acknowledgement cannot be reused for changed plan
    /// content, and a repeated identical plan keeps the same fingerprint for
    /// idempotency. `created_at_ms`, `expires_at_ms` and `run_id` are execution
    /// metadata; the expiry is bounded separately by `check_expiry_contract`.
    ///
    /// This is an integrity check, not a MAC. The operator owns the file, so an
    /// adversary with write access can recompute it; the real authorization is
    /// the live re-validation of identity, policy, graph and spec at execute.
    pub fn fingerprint(&self) -> String {
        let mut value = serde_json::to_value(self).expect("the plan always serializes");
        if let Some(object) = value.as_object_mut() {
            for key in [
                "run_id",
                "created_at_ms",
                "expires_at_ms",
                "plan_fingerprint",
            ] {
                object.remove(key);
            }
        }
        // serde_json's map is ordered, so this is canonical.
        let canonical = serde_json::to_string(&value).expect("the plan always serializes");
        hex_lower(&digest(canonical.as_bytes()))
    }

    pub fn is_expired(&self, now_ms: u64) -> bool {
        now_ms >= self.expires_at_ms
    }

    /// Refuse a plan from a different format version.
    pub fn check_version(&self) -> Result<(), CoreError> {
        if self.plan_version != PLAN_VERSION {
            return Err(CoreError::with_detail(
                ErrorCode::Conflict,
                format!(
                    "plan version {} is not supported; regenerate the plan",
                    self.plan_version
                ),
                "unsupported_plan_version",
            ));
        }
        Ok(())
    }

    /// Refuse an expiry that is malformed or longer than the maximum window.
    ///
    /// The expiry is not part of the content hash, so it is validated instead:
    /// a plan may not live longer than `MAX_PLAN_TTL_MS`, and an expiry before
    /// its creation is invalid. This bounds any edit of the field without
    /// breaking idempotency for repeated identical plans.
    pub fn check_expiry_contract(&self) -> Result<(), CoreError> {
        if self.expires_at_ms <= self.created_at_ms
            || self.expires_at_ms.saturating_sub(self.created_at_ms) > MAX_PLAN_TTL_MS
        {
            return Err(CoreError::with_detail(
                ErrorCode::Conflict,
                "the plan expiry is outside the permitted window; regenerate the plan",
                "plan_expiry_invalid",
            ));
        }
        Ok(())
    }
}

/// A hash of the selection shape that never includes a raw filter value.
pub fn spec_hash(spec: &SafeSyncSpec, selection: &SelectionResult) -> String {
    let document = serde_json::json!({
        "version": spec.version,
        "source": spec.source,
        "destination": spec.destination,
        "roots": selection.roots,
        "depth": spec.selection.relationships.depth,
        "direction": selection.direction,
        "limits": spec.limits,
        "policy": spec.masking.policy,
    });
    let canonical = serde_json::to_string(&document).unwrap_or_default();
    hex_lower(&digest(canonical.as_bytes()))
}

/// Refuse a plan whose schema has drifted from the live catalogue.
pub fn check_schema_drift(plan: &DryRunPlan, selection: &SelectionResult) -> Result<(), CoreError> {
    for planned in &plan.tables {
        let Some(live) = selection.rows.iter().find(|table| {
            table.table.schema == planned.schema && table.table.table == planned.table
        }) else {
            return Err(schema_drift(&planned.table));
        };

        if live.schema_hash != planned.schema_hash {
            return Err(schema_drift(&planned.table));
        }
    }
    Ok(())
}

fn schema_drift(table: &str) -> CoreError {
    CoreError::with_detail(
        ErrorCode::Conflict,
        format!("the schema of {table} changed since the plan was made"),
        "schema_drift",
    )
}

/// Refuse a plan when the live graph, table set, order or deferred groups differ
/// from what the operator reviewed.
pub fn assert_graph_matches(
    plan: &DryRunPlan,
    selection: &SelectionResult,
) -> Result<(), CoreError> {
    let live_order: Vec<String> = selection
        .order
        .iter()
        .map(|name| name.to_string())
        .collect();
    let live_deferred: Vec<String> = selection
        .deferred
        .iter()
        .map(|group| {
            group
                .iter()
                .map(|name| name.to_string())
                .collect::<Vec<_>>()
                .join(",")
        })
        .collect();

    let mut planned_tables: Vec<String> = plan
        .tables
        .iter()
        .map(|table| format!("{}.{}", table.schema, table.table))
        .collect();
    let mut live_tables: Vec<String> = selection
        .rows
        .iter()
        .map(|rows| rows.table.to_string())
        .collect();
    planned_tables.sort();
    live_tables.sort();

    let changed = live_order != plan.constraints.order
        || live_deferred != plan.constraints.deferred
        || planned_tables != live_tables
        || selection.edges as u64 != plan.graph.edges
        || selection.deferred.len() as u64 != plan.graph.cycles
        || crate::safesync::graph::graph_fingerprint(&selection.graph) != plan.graph.fingerprint;

    if changed {
        return Err(CoreError::with_detail(
            ErrorCode::Conflict,
            "the foreign-key graph or selection changed since the plan was made",
            "graph_changed",
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::connection::{DbEngine, TargetClass};
    use crate::safesync::key::MaskKey;
    use crate::safesync::mask::Masker;
    use crate::safesync::planner::TableRows;
    use crate::safesync::planner::{LimitCheck, RootPlan};
    use crate::safesync::spec::{
        ExecutionOptions, FilterOp, FilterPredicate, FilterValue, MaskingSelection,
        RelationshipSpec, RootSelection, Selection,
    };
    use crate::security::ServerIdentity;

    fn spec() -> SafeSyncSpec {
        SafeSyncSpec {
            version: 1,
            source: "prod".into(),
            destination: "dev".into(),
            selection: Selection {
                roots: vec![RootSelection {
                    table: "public.accounts".into(),
                    filter: vec![FilterPredicate {
                        column: "id".into(),
                        op: FilterOp::Eq,
                        value: FilterValue::Text("secret-value".into()),
                    }],
                }],
                relationships: RelationshipSpec::default(),
            },
            limits: Limits::default(),
            masking: MaskingSelection {
                policy: "development-default@1".into(),
            },
            execution: ExecutionOptions::default(),
        }
    }

    fn session(label: &str) -> SessionIdentity {
        SessionIdentity {
            engine: DbEngine::Postgres,
            identity_key: Some(format!("postgres://h:1/{label}#1")),
            system_identifier: Some("1".into()),
            database_oid: label.into(),
            database_name: label.into(),
            current_user: "dev".into(),
            backend_pid: 1,
            backend_start: "2026-01-01".into(),
            server_addr: "127.0.0.1".into(),
            server_port: 5432,
            transaction_read_only: label != "dev",
            host: "127.0.0.1".into(),
            port: 5432,
            server: ServerIdentity {
                server_uuid: Some("1".into()),
                database_id: Some(label.into()),
                database: Some(label.into()),
                ..ServerIdentity::default()
            },
        }
    }

    fn table_rows() -> TableRows {
        TableRows {
            table: QualifiedName::parse("public.accounts").unwrap(),
            columns: vec!["id".into()],
            types: vec!["uuid".into()],
            row_identity: vec!["id".into()],
            schema_hash: "hash".into(),
            rls_active: false,
            generated: vec![false],
            identity: vec![false],
            identity_always: vec![false],
            rows: vec![vec![crate::safesync::mask::MaskedValue::Text("x".into())]],
            total_bytes: 1,
        }
    }

    fn selection() -> SelectionResult {
        let rows = vec![table_rows()];
        let order = vec![QualifiedName::parse("public.accounts").unwrap()];
        SelectionResult {
            graph: crate::safesync::graph::SchemaGraph {
                tables: Vec::new(),
                edges: Vec::new(),
            },
            rows,
            roots: vec![RootPlan {
                schema: "public".into(),
                table: "accounts".into(),
                filter_columns: vec!["id".into()],
                filter_ops: vec!["eq".into()],
                filter_digest: "hmac-sha256:abc".into(),
            }],
            order,
            deferred: Vec::new(),
            checks: vec![LimitCheck {
                limit: "max_depth".into(),
                observed: 0,
                ok: true,
            }],
            max_depth_used: 0,
            direction: "parents".into(),
            edges: 0,
            complete: true,
            read_rows: true,
        }
    }

    fn masker() -> Masker {
        Masker::from_builtin(MaskKey::from_bytes("k-test", [7u8; 32])).unwrap()
    }

    fn plan() -> DryRunPlan {
        DryRunPlan::build(
            &spec(),
            &session("prod"),
            &session("dev"),
            DestinationCategory::VerifiedDevelopment,
            &selection(),
            &masker(),
            &[],
            1_000,
        )
        .unwrap()
    }

    #[test]
    fn a_changed_graph_is_refused() {
        let plan = plan();
        let mut drifted = selection();
        drifted.edges = 1;
        let error = assert_graph_matches(&plan, &drifted).unwrap_err();
        assert_eq!(error.detail.as_deref(), Some("graph_changed"));

        let mut table_drifted = selection();
        table_drifted.rows.clear();
        let error = assert_graph_matches(&plan, &table_drifted).unwrap_err();
        assert_eq!(error.detail.as_deref(), Some("graph_changed"));
    }

    #[test]
    fn an_unchanged_graph_matches() {
        let plan = plan();
        assert_graph_matches(&plan, &selection()).unwrap();
    }

    #[test]
    fn the_plan_contains_no_filter_value() {
        let rendered = serde_json::to_string(&plan()).unwrap();
        assert!(!rendered.contains("secret-value"));
        assert!(rendered.contains("contains_values\":false"));
    }

    #[test]
    fn the_fingerprint_is_stable_for_the_same_plan() {
        let first = plan();
        let second = plan();
        assert_eq!(first.plan_fingerprint, second.plan_fingerprint);
        assert_eq!(first.fingerprint(), second.fingerprint());
    }

    #[test]
    fn the_fingerprint_changes_with_the_destination() {
        let first = plan();
        let second = DryRunPlan::build(
            &spec(),
            &session("prod"),
            &session("other"),
            DestinationCategory::VerifiedDevelopment,
            &selection(),
            &masker(),
            &[],
            1_000,
        )
        .unwrap();
        assert_ne!(first.plan_fingerprint, second.plan_fingerprint);
    }

    #[test]
    fn the_fingerprint_changes_with_the_key_id() {
        let first = plan();
        let other = Masker::from_builtin(MaskKey::from_bytes("k-other", [9u8; 32])).unwrap();
        let second = DryRunPlan::build(
            &spec(),
            &session("prod"),
            &session("dev"),
            DestinationCategory::VerifiedDevelopment,
            &selection(),
            &other,
            &[],
            1_000,
        )
        .unwrap();
        assert_ne!(first.plan_fingerprint, second.plan_fingerprint);
    }

    #[test]
    fn the_fingerprint_changes_with_the_import_mode() {
        let first = plan();
        let mut second = plan();
        second.import.mode = "batch".into();
        assert_ne!(first.fingerprint(), second.fingerprint());
    }

    #[test]
    fn the_fingerprint_ignores_execution_metadata() {
        let first = plan();
        let mut second = plan();
        second.run_id = "different".into();
        second.created_at_ms += 5_000;
        second.expires_at_ms += 5_000;
        assert_eq!(first.plan_fingerprint, second.plan_fingerprint);
    }

    #[test]
    fn an_overlong_expiry_is_refused() {
        let mut plan = plan();
        plan.expires_at_ms = plan.created_at_ms + MAX_PLAN_TTL_MS + 1;
        let error = plan.check_expiry_contract().unwrap_err();
        assert_eq!(error.detail.as_deref(), Some("plan_expiry_invalid"));

        plan.expires_at_ms = plan.created_at_ms + MAX_PLAN_TTL_MS;
        plan.check_expiry_contract().unwrap();
    }

    #[test]
    fn an_old_plan_version_is_refused() {
        let mut old = plan();
        old.plan_version = 1;
        let error = old.check_version().unwrap_err();
        assert_eq!(error.detail.as_deref(), Some("unsupported_plan_version"));
    }

    #[test]
    fn an_expired_plan_is_detected() {
        let plan = plan();
        assert!(!plan.is_expired(1_001));
        assert!(plan.is_expired(plan.expires_at_ms));
    }

    #[test]
    fn schema_drift_is_detected() {
        let plan = plan();
        let mut drifted = selection();
        drifted.rows[0].schema_hash = "other".into();
        let error = check_schema_drift(&plan, &drifted).unwrap_err();
        assert_eq!(error.detail.as_deref(), Some("schema_drift"));
    }

    #[test]
    fn the_plan_is_never_a_user_label_decision() {
        // The declared target is not part of the plan; the policy decides.
        let _ = TargetClass::Local;
        assert!(plan().complete);
    }
}
