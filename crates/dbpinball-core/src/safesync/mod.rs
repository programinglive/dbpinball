//! Safe Sync: bounded, masked, verifiable production-to-local PostgreSQL sync.
//!
//! Slice 1 implements the security spine only. The module is split so each part
//! can be reviewed and tested on its own:
//!
//! - [`spec`] validates the operator's selection specification;
//! - [`mask`] applies a fail-closed, keyed, deterministic masking policy;
//! - [`key`] owns the masking key and never exposes it.
//!
//! Later slices add bound sessions, extraction, the plan and the import.

pub mod key;
pub mod mask;
pub mod spec;

pub use key::{key_id_for, InMemoryMaskKeyStore, KeyringMaskKeyStore, MaskKey, MaskKeyStore};
pub use mask::{
    base32_lower, hex_lower, normalize_identity, ColumnClass, ColumnCoverage, ColumnRule,
    ColumnShape, CoverageReport, MaskInput, MaskStrategy, MaskedValue, Masker, MaskingPolicy,
    EMAIL_DOMAIN, FILTER_DOMAIN, MASK_DOMAIN,
};
pub use spec::{
    ExecutionOptions, FilterOp, FilterPredicate, FilterValue, Limits, MaskingSelection,
    QualifiedName, RelationshipSpec, RootSelection, SafeSyncSpec, Selection, TraversalDirection,
    SPEC_VERSION,
};
