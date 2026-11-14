//! The Safe Sync selection specification.
//!
//! The spec is the operator's input. It is validated into typed structs before
//! anything reaches SQL, and unknown fields are rejected so a typo cannot silently
//! remove a limit or a masking rule.

use serde::{Deserialize, Serialize};
use std::fmt;

use crate::domain::error::{CoreError, ErrorCode};

/// The only spec version this build understands.
pub const SPEC_VERSION: u32 = 1;

/// Default limits. Omitted limits take these values.
pub const DEFAULT_MAX_ROWS_PER_TABLE: u64 = 1_000;
pub const DEFAULT_MAX_TOTAL_ROWS: u64 = 20_000;
pub const DEFAULT_MAX_DEPTH: u8 = 3;
pub const DEFAULT_MAX_BYTES: u64 = 32 * 1024 * 1024;
pub const DEFAULT_TIMEOUT_MS: u64 = 60_000;
pub const DEFAULT_ACK_TTL_MS: u64 = 15 * 60 * 1_000;

/// A validated `schema.table` name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct QualifiedName {
    pub schema: String,
    pub table: String,
}

impl QualifiedName {
    /// Parse `schema.table` or a bare `table` (which defaults to `public`).
    pub fn parse(input: &str) -> Result<Self, CoreError> {
        let trimmed = input.trim();
        let mut parts = trimmed.split('.');
        let (schema, table) = match (parts.next(), parts.next(), parts.next()) {
            (Some(table), None, None) => ("public", table),
            (Some(schema), Some(table), None) => (schema, table),
            _ => {
                return Err(invalid_identifier(format!(
                    "'{input}' is not a schema.table name"
                )))
            }
        };

        if !is_identifier(schema) || !is_identifier(table) {
            return Err(invalid_identifier(format!(
                "'{input}' is not a valid identifier"
            )));
        }

        Ok(Self {
            schema: schema.to_string(),
            table: table.to_string(),
        })
    }
}

impl fmt::Display for QualifiedName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}", self.schema, self.table)
    }
}

/// The whole spec.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SafeSyncSpec {
    pub version: u32,
    pub source: String,
    pub destination: String,
    pub selection: Selection,
    #[serde(default)]
    pub limits: Limits,
    pub masking: MaskingSelection,
    #[serde(default)]
    pub execution: ExecutionOptions,
}

impl SafeSyncSpec {
    pub fn parse_yaml(text: &str) -> Result<Self, CoreError> {
        let spec: Self = serde_yaml::from_str(text).map_err(|error| {
            CoreError::with_detail(
                ErrorCode::Usage,
                format!("the spec could not be parsed: {error}"),
                "invalid_spec",
            )
        })?;
        spec.validate()?;
        Ok(spec)
    }

    pub fn parse_json(text: &str) -> Result<Self, CoreError> {
        let spec: Self = serde_json::from_str(text).map_err(|error| {
            CoreError::with_detail(
                ErrorCode::Usage,
                format!("the spec could not be parsed: {error}"),
                "invalid_spec",
            )
        })?;
        spec.validate()?;
        Ok(spec)
    }

    pub fn validate(&self) -> Result<(), CoreError> {
        if self.version != SPEC_VERSION {
            return Err(CoreError::with_detail(
                ErrorCode::Unsupported,
                format!("spec version {} is not supported", self.version),
                "unsupported_spec_version",
            ));
        }

        if self.source.trim().is_empty() || self.destination.trim().is_empty() {
            return Err(invalid_spec("source and destination are required"));
        }

        if self.source == self.destination {
            return Err(invalid_spec("source and destination must differ"));
        }

        if self.selection.roots.is_empty() {
            return Err(invalid_spec("selection.roots must not be empty"));
        }

        for root in &self.selection.roots {
            QualifiedName::parse(&root.table)
                .map_err(|_| invalid_spec(format!("'{}' is not a valid table name", root.table)))?;

            for filter in &root.filter {
                if !is_identifier(&filter.column) {
                    return Err(invalid_identifier(format!(
                        "'{}' is not a valid filter column",
                        filter.column
                    )));
                }
                match (&filter.op, &filter.value) {
                    (FilterOp::In, FilterValue::List(values)) => {
                        if values.is_empty() || values.len() > 100 {
                            return Err(invalid_spec(
                                "an `in` filter needs between 1 and 100 values",
                            ));
                        }
                        if values
                            .iter()
                            .any(|value| matches!(value, FilterValue::List(_)))
                        {
                            return Err(invalid_spec("an `in` filter cannot nest lists"));
                        }
                    }
                    (FilterOp::In, _) => {
                        return Err(invalid_spec("an `in` filter needs a list value"))
                    }
                    (_, FilterValue::List(_)) => {
                        return Err(invalid_spec("only an `in` filter may take a list value"))
                    }
                    (_, _) => {}
                }
            }
        }

        if self.masking.policy.trim().is_empty() {
            return Err(invalid_spec("masking.policy is required"));
        }

        if self.selection.relationships.depth > self.limits.max_depth {
            return Err(invalid_spec(
                "selection.relationships.depth exceeds limits.max_depth",
            ));
        }

        if self.limits.max_rows_per_table == 0
            || self.limits.max_total_rows == 0
            || self.limits.max_bytes == 0
            || self.limits.timeout_ms < 1_000
        {
            return Err(invalid_spec("the limits are outside their allowed range"));
        }

        if self.execution.acknowledgement_ttl_ms < 1_000 {
            return Err(invalid_spec(
                "execution.acknowledgement_ttl_ms must be at least 1000",
            ));
        }

        Ok(())
    }
}

/// What to select.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    pub roots: Vec<RootSelection>,
    #[serde(default)]
    pub relationships: RelationshipSpec,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RootSelection {
    pub table: String,
    #[serde(default)]
    pub filter: Vec<FilterPredicate>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FilterPredicate {
    pub column: String,
    pub op: FilterOp,
    pub value: FilterValue,
}

impl FilterPredicate {
    /// A canonical, injective rendering used for the keyed filter digest.
    pub fn canonical(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

/// Canonical, injective rendering of a whole filter list.
///
/// JSON is used instead of delimiter-joined fields: a value containing `|`,
/// `,` or a separator byte cannot make two different filters look identical.
pub fn canonical_filter(filter: &[FilterPredicate]) -> String {
    serde_json::to_string(filter).unwrap_or_default()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterOp {
    Eq,
    In,
    Gte,
    Lte,
}

impl FilterOp {
    pub fn as_str(self) -> &'static str {
        match self {
            FilterOp::Eq => "eq",
            FilterOp::In => "in",
            FilterOp::Gte => "gte",
            FilterOp::Lte => "lte",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FilterValue {
    Bool(bool),
    Int(i64),
    Text(String),
    List(Vec<FilterValue>),
}

impl FilterValue {
    pub fn canonical(&self) -> String {
        match self {
            FilterValue::Bool(value) => format!("b:{value}"),
            FilterValue::Int(value) => format!("i:{value}"),
            FilterValue::Text(value) => format!("t:{value}"),
            FilterValue::List(values) => {
                let inner: Vec<String> = values.iter().map(FilterValue::canonical).collect();
                format!("l:[{}]", inner.join(","))
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelationshipSpec {
    #[serde(default = "default_depth")]
    pub depth: u8,
    #[serde(default)]
    pub direction: TraversalDirection,
}

impl Default for RelationshipSpec {
    fn default() -> Self {
        Self {
            depth: default_depth(),
            direction: TraversalDirection::Parents,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TraversalDirection {
    #[default]
    Parents,
    Both,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    #[serde(default = "default_rows_per_table")]
    pub max_rows_per_table: u64,
    #[serde(default = "default_total_rows")]
    pub max_total_rows: u64,
    #[serde(default = "default_max_depth")]
    pub max_depth: u8,
    #[serde(default = "default_max_bytes")]
    pub max_bytes: u64,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_rows_per_table: DEFAULT_MAX_ROWS_PER_TABLE,
            max_total_rows: DEFAULT_MAX_TOTAL_ROWS,
            max_depth: DEFAULT_MAX_DEPTH,
            max_bytes: DEFAULT_MAX_BYTES,
            timeout_ms: DEFAULT_TIMEOUT_MS,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaskingSelection {
    pub policy: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionOptions {
    #[serde(default = "default_ack_ttl")]
    pub acknowledgement_ttl_ms: u64,
}

impl Default for ExecutionOptions {
    fn default() -> Self {
        Self {
            acknowledgement_ttl_ms: DEFAULT_ACK_TTL_MS,
        }
    }
}

fn default_depth() -> u8 {
    0
}

fn default_rows_per_table() -> u64 {
    DEFAULT_MAX_ROWS_PER_TABLE
}

fn default_total_rows() -> u64 {
    DEFAULT_MAX_TOTAL_ROWS
}

fn default_max_depth() -> u8 {
    DEFAULT_MAX_DEPTH
}

fn default_max_bytes() -> u64 {
    DEFAULT_MAX_BYTES
}

fn default_timeout_ms() -> u64 {
    DEFAULT_TIMEOUT_MS
}

fn default_ack_ttl() -> u64 {
    DEFAULT_ACK_TTL_MS
}

fn is_identifier(value: &str) -> bool {
    if value.is_empty() || value.len() > 63 {
        return false;
    }
    let mut chars = value.chars();
    let first = chars.next().unwrap_or(' ');
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
}

fn invalid_spec(message: impl Into<String>) -> CoreError {
    CoreError::with_detail(ErrorCode::Usage, message, "invalid_spec")
}

fn invalid_identifier(message: impl Into<String>) -> CoreError {
    CoreError::with_detail(ErrorCode::Usage, message, "invalid_identifier")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_spec() -> SafeSyncSpec {
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
                        value: FilterValue::Text("abc".into()),
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

    #[test]
    fn a_valid_spec_passes() {
        valid_spec().validate().unwrap();
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let error = SafeSyncSpec::parse_yaml("version: 1\nunexpected: true\n").unwrap_err();
        assert_eq!(error.detail.as_deref(), Some("invalid_spec"));
    }

    #[test]
    fn a_password_field_is_rejected() {
        let yaml = "version: 1\nsource: a\ndestination: b\npassword: secret\n";
        let error = SafeSyncSpec::parse_yaml(yaml).unwrap_err();
        assert_eq!(error.detail.as_deref(), Some("invalid_spec"));
    }

    #[test]
    fn an_unsupported_version_is_refused() {
        let mut spec = valid_spec();
        spec.version = 2;
        let error = spec.validate().unwrap_err();
        assert_eq!(error.detail.as_deref(), Some("unsupported_spec_version"));
    }

    #[test]
    fn an_empty_root_list_is_refused() {
        let mut spec = valid_spec();
        spec.selection.roots.clear();
        let error = spec.validate().unwrap_err();
        assert_eq!(error.detail.as_deref(), Some("invalid_spec"));
    }

    #[test]
    fn equal_source_and_destination_is_refused() {
        let mut spec = valid_spec();
        spec.destination = spec.source.clone();
        let error = spec.validate().unwrap_err();
        assert_eq!(error.detail.as_deref(), Some("invalid_spec"));
    }

    #[test]
    fn an_in_filter_over_100_values_is_refused() {
        let mut spec = valid_spec();
        spec.selection.roots[0].filter[0].op = FilterOp::In;
        spec.selection.roots[0].filter[0].value =
            FilterValue::List((0..101).map(FilterValue::Int).collect());
        let error = spec.validate().unwrap_err();
        assert_eq!(error.detail.as_deref(), Some("invalid_spec"));
    }

    #[test]
    fn an_in_filter_without_a_list_is_refused() {
        let mut spec = valid_spec();
        spec.selection.roots[0].filter[0].op = FilterOp::In;
        let error = spec.validate().unwrap_err();
        assert_eq!(error.detail.as_deref(), Some("invalid_spec"));
    }

    #[test]
    fn defaults_apply_when_limits_are_omitted() {
        let spec = SafeSyncSpec::parse_yaml(
            "version: 1\nsource: a\ndestination: b\nmasking:\n  policy: p\nselection:\n  roots:\n    - table: public.t\n",
        )
        .unwrap();
        assert_eq!(spec.limits.max_rows_per_table, DEFAULT_MAX_ROWS_PER_TABLE);
        assert_eq!(spec.limits.max_total_rows, DEFAULT_MAX_TOTAL_ROWS);
        assert_eq!(spec.limits.max_depth, DEFAULT_MAX_DEPTH);
        assert_eq!(spec.limits.max_bytes, DEFAULT_MAX_BYTES);
        assert_eq!(spec.limits.timeout_ms, DEFAULT_TIMEOUT_MS);
        assert_eq!(spec.execution.acknowledgement_ttl_ms, DEFAULT_ACK_TTL_MS);
        assert_eq!(spec.selection.relationships.depth, 0);
    }

    #[test]
    fn a_depth_above_the_limit_is_refused() {
        let mut spec = valid_spec();
        spec.selection.relationships.depth = 5;
        let error = spec.validate().unwrap_err();
        assert_eq!(error.detail.as_deref(), Some("invalid_spec"));
    }

    #[test]
    fn a_bad_identifier_is_refused() {
        let mut spec = valid_spec();
        spec.selection.roots[0].filter[0].column = "id; drop table".into();
        let error = spec.validate().unwrap_err();
        assert_eq!(error.detail.as_deref(), Some("invalid_identifier"));
    }

    #[test]
    fn a_qualified_name_parses_with_and_without_a_schema() {
        assert_eq!(QualifiedName::parse("accounts").unwrap().schema, "public");
        assert_eq!(
            QualifiedName::parse("app.accounts").unwrap().to_string(),
            "app.accounts"
        );
        assert!(QualifiedName::parse("a.b.c").is_err());
        assert!(QualifiedName::parse("bad-name").is_err());
    }

    #[test]
    fn the_filter_canonical_form_is_unambiguous() {
        let filter = FilterPredicate {
            column: "id".into(),
            op: FilterOp::Eq,
            value: FilterValue::Text("a:b".into()),
        };
        // JSON, so a value that contains a delimiter stays distinguishable.
        assert!(filter.canonical().contains("a:b"));
        assert!(filter.canonical().starts_with('{'));

        let list = FilterPredicate {
            column: "id".into(),
            op: FilterOp::In,
            value: FilterValue::List(vec![
                FilterValue::Text("a".into()),
                FilterValue::Text("b".into()),
            ]),
        };
        let smuggled = FilterPredicate {
            column: "id".into(),
            op: FilterOp::In,
            value: FilterValue::List(vec![FilterValue::Text("a,t:b".into())]),
        };
        assert_ne!(
            canonical_filter(std::slice::from_ref(&list)),
            canonical_filter(std::slice::from_ref(&smuggled))
        );

        let first = FilterPredicate {
            column: "id".into(),
            op: FilterOp::Eq,
            value: FilterValue::Text("a".into()),
        };
        let second = FilterPredicate {
            column: "id".into(),
            op: FilterOp::Eq,
            value: FilterValue::Text("b".into()),
        };
        let single = FilterPredicate {
            column: "id".into(),
            op: FilterOp::Eq,
            value: FilterValue::Text("a|{}".into()),
        };
        assert_ne!(
            canonical_filter(&[first, second]),
            canonical_filter(std::slice::from_ref(&single))
        );
    }
}
