//! Safe Sync: bounded, masked, verifiable production-to-local PostgreSQL sync.
//!
//! Slice 1 implements the security spine only. The module is split so each part
//! can be reviewed and tested on its own:
//!
//! - [`spec`] validates the operator's selection specification;
//! - [`mask`] applies a fail-closed, keyed, deterministic masking policy;
//! - [`key`] owns the masking key and never exposes it;
//! - [`session`] binds an identity to the one connection that runs the work;
//! - [`graph`] inspects a table's catalogue shape;
//! - [`planner`] bounds and extracts a selection;
//! - [`plan`] produces the reviewable, value-free plan and its fingerprint;
//! - [`import`] writes masked rows in one destination transaction;
//! - [`audit`] records counts and identities, never values;
//! - [`service`] combines them with the M3 security policy.

pub mod audit;
pub mod graph;
pub mod import;
pub mod key;
pub mod mask;
pub mod plan;
pub mod planner;
pub mod service;
pub mod session;
pub mod spec;

pub use audit::{AuditRecord, TableAudit};
pub use graph::{
    discover_parents, inspect_table, ColumnInfo, ForeignKeyEdge, SchemaGraph, TableInfo,
};
pub use import::{ConflictPolicy, ImportReport, TableImport};
pub use key::{key_id_for, InMemoryMaskKeyStore, KeyringMaskKeyStore, MaskKey, MaskKeyStore};
pub use mask::{
    base32_lower, hex_lower, normalize_identity, ColumnClass, ColumnCoverage, ColumnRule,
    ColumnShape, CoverageReport, KeyDomain, MaskInput, MaskStrategy, MaskedValue, Masker,
    MaskingPolicy, EMAIL_DOMAIN, FILTER_DOMAIN, MASK_DOMAIN,
};
pub use plan::{DryRunPlan, PlanIdentity, PLAN_VERSION};
#[cfg(any(test, feature = "integration"))]
pub use planner::extract_table;
pub use planner::{quote_ident, LimitCheck, RootPlan, SelectionResult, TableRows};
pub use service::{
    execute, inspect, masker_for, plan as build_plan, resolve_policy, status, validate,
    ExecuteRequest, RunReport, RunStatus, ValidationReport,
};
pub use session::{BoundSession, SessionIdentity, SessionRole};
pub use spec::{
    ExecutionOptions, FilterOp, FilterPredicate, FilterValue, Limits, MaskingSelection,
    QualifiedName, RelationshipSpec, RootSelection, SafeSyncSpec, Selection, TraversalDirection,
    SPEC_VERSION,
};

/// Convert a driver error into a typed error that exposes only the SQLSTATE.
///
/// A server message can echo a bound value, an identifier, a constraint detail
/// or a credential. The message is therefore withheld everywhere, not only on
/// the destination path.
pub(crate) fn database_error(
    code: crate::domain::error::ErrorCode,
    context: &str,
    detail: &'static str,
    error: sqlx::Error,
) -> crate::domain::error::CoreError {
    let sqlstate = error
        .as_database_error()
        .and_then(|database| database.code())
        .map(|code| code.to_string())
        .unwrap_or_else(|| "unknown".to_string());
    crate::domain::error::CoreError::with_detail(
        code,
        format!("{context} failed (SQLSTATE {sqlstate}); the server message is withheld"),
        detail,
    )
}
