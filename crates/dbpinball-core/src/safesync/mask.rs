//! Fail-closed deterministic masking.
//!
//! THE RULE
//! --------
//! Every selected column needs an explicit disposition. There is no permissive
//! "copy unknown columns unchanged" default: a column without a rule is refused.
//!
//! WHAT THIS IS NOT
//! ----------------
//! Deterministic masking is not anonymization. A keyed HMAC of a low-entropy
//! value can be re-linked by an attacker who holds the key or a candidate list.
//! The output is de-identified test data, and the documentation says so.

use std::collections::BTreeSet;

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use unicode_normalization::UnicodeNormalization;

use crate::domain::error::{CoreError, ErrorCode};
use crate::safesync::key::MaskKey;

type HmacSha256 = Hmac<Sha256>;

/// The domain for column transformations. Changing it changes every output.
pub const MASK_DOMAIN: &str = "dbpinball/safesync/v1";
/// The domain for the keyed filter digest.
pub const FILTER_DOMAIN: &str = "dbpinball/safesync/filter/v1";
/// The reserved domain for synthetic email addresses.
pub const EMAIL_DOMAIN: &str = "example.invalid";

/// How one column is transformed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaskStrategy {
    Keep,
    Redact,
    Null,
    DeterministicPseudonym,
    DeterministicEmail,
    DeterministicPhone,
    DeterministicKey,
    DateShift,
    AmountJitter,
    Fixed,
}

impl MaskStrategy {
    pub fn as_str(self) -> &'static str {
        match self {
            MaskStrategy::Keep => "keep",
            MaskStrategy::Redact => "redact",
            MaskStrategy::Null => "null",
            MaskStrategy::DeterministicPseudonym => "deterministic_pseudonym",
            MaskStrategy::DeterministicEmail => "deterministic_email",
            MaskStrategy::DeterministicPhone => "deterministic_phone",
            MaskStrategy::DeterministicKey => "deterministic_key",
            MaskStrategy::DateShift => "date_shift",
            MaskStrategy::AmountJitter => "amount_jitter",
            MaskStrategy::Fixed => "fixed",
        }
    }

    /// True when the strategy transforms the value.
    fn is_transform(self) -> bool {
        !matches!(self, MaskStrategy::Keep)
    }
}

/// A column's role in the plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ColumnClass {
    PrimaryKey,
    ForeignKey,
    Masked,
    Keep,
    Unknown,
}

/// What the catalogue says about one column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnShape {
    pub name: String,
    pub type_name: String,
    pub nullable: bool,
    pub is_primary_key: bool,
    /// An explicit destination length limit, when the policy supplies one.
    pub max_len: Option<usize>,
}

/// The referenced key a `deterministic_key` column belongs to.
///
/// The domain is the referenced table and its ordered key columns, so a parent
/// primary key and every child foreign key that references it derive the same
/// masked value from the same tuple.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyDomain {
    /// The referenced table, as `schema.table`.
    pub table: String,
    /// The ordered key columns on the referenced table.
    pub columns: Vec<String>,
}

/// One explicit policy rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ColumnRule {
    pub schema: String,
    pub table: String,
    pub column: String,
    pub strategy: MaskStrategy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_len: Option<usize>,
    /// Set for `deterministic_key`; names the referenced key domain.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_domain: Option<KeyDomain>,
    /// The column's zero-based position within the key domain tuple.
    #[serde(default)]
    pub key_position: usize,
}

/// A versioned set of rules.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaskingPolicy {
    pub name: String,
    pub version: u32,
    pub rules: Vec<ColumnRule>,
    /// Hash of the rules, filled by `hash()`.
    #[serde(skip)]
    pub hash: String,
}

impl MaskingPolicy {
    /// `development-default@1`. Small on purpose: it covers Slice 1 and refuses
    /// everything it does not name.
    pub fn builtin_default() -> Self {
        let rule = |column: &str, strategy: MaskStrategy| ColumnRule {
            schema: "public".to_string(),
            table: "accounts".to_string(),
            column: column.to_string(),
            strategy,
            max_len: None,
            key_domain: None,
            key_position: 0,
        };

        let mut policy = Self {
            name: "development-default".to_string(),
            version: 1,
            rules: vec![
                rule("id", MaskStrategy::Keep),
                rule("email", MaskStrategy::DeterministicEmail),
                rule("full_name", MaskStrategy::DeterministicPseudonym),
            ],
            hash: String::new(),
        };
        policy.hash = policy.compute_hash();
        policy
    }

    /// `development-default@2`. Adds referential key masking for the S2 demo
    /// schema. Every table it does not name is still refused.
    pub fn builtin_v2() -> Self {
        let rule = |schema: &str, table: &str, column: &str, strategy: MaskStrategy| ColumnRule {
            schema: schema.to_string(),
            table: table.to_string(),
            column: column.to_string(),
            strategy,
            max_len: None,
            key_domain: None,
            key_position: 0,
        };
        let key =
            |table: &str, column: &str, domain_table: &str, columns: &[&str], position: usize| {
                ColumnRule {
                    schema: "public".to_string(),
                    table: table.to_string(),
                    column: column.to_string(),
                    strategy: MaskStrategy::DeterministicKey,
                    max_len: None,
                    key_domain: Some(KeyDomain {
                        table: domain_table.to_string(),
                        columns: columns.iter().map(|value| value.to_string()).collect(),
                    }),
                    key_position: position,
                }
            };

        let rules = vec![
            // Slice 1 fixture.
            rule("public", "accounts", "id", MaskStrategy::Keep),
            rule(
                "public",
                "accounts",
                "email",
                MaskStrategy::DeterministicEmail,
            ),
            rule(
                "public",
                "accounts",
                "full_name",
                MaskStrategy::DeterministicPseudonym,
            ),
            // Single-column natural key referenced by orders.
            key("customers", "code", "public.customers", &["code"], 0),
            rule(
                "public",
                "customers",
                "email",
                MaskStrategy::DeterministicEmail,
            ),
            rule(
                "public",
                "customers",
                "name",
                MaskStrategy::DeterministicPseudonym,
            ),
            rule("public", "orders", "id", MaskStrategy::Keep),
            key("orders", "customer_code", "public.customers", &["code"], 0),
            rule(
                "public",
                "orders",
                "note",
                MaskStrategy::DeterministicPseudonym,
            ),
            // Composite key referenced by shipment_events.
            key(
                "shipments",
                "region",
                "public.shipments",
                &["region", "code"],
                0,
            ),
            key(
                "shipments",
                "code",
                "public.shipments",
                &["region", "code"],
                1,
            ),
            rule(
                "public",
                "shipments",
                "label",
                MaskStrategy::DeterministicPseudonym,
            ),
            key(
                "shipment_events",
                "region",
                "public.shipments",
                &["region", "code"],
                0,
            ),
            key(
                "shipment_events",
                "code",
                "public.shipments",
                &["region", "code"],
                1,
            ),
            rule(
                "public",
                "shipment_events",
                "detail",
                MaskStrategy::DeterministicPseudonym,
            ),
            // Self-reference.
            key("employees", "id", "public.employees", &["id"], 0),
            key("employees", "manager_id", "public.employees", &["id"], 0),
            rule(
                "public",
                "employees",
                "name",
                MaskStrategy::DeterministicPseudonym,
            ),
            // Deferrable cycle fixture.
            key("cycle_a", "id", "public.cycle_a", &["id"], 0),
            key("cycle_a", "b_id", "public.cycle_b", &["id"], 0),
            key("cycle_b", "id", "public.cycle_b", &["id"], 0),
            key("cycle_b", "a_id", "public.cycle_a", &["id"], 0),
            // Non-deferrable cycle fixture.
            key("cycle_x", "id", "public.cycle_x", &["id"], 0),
            key("cycle_x", "y_id", "public.cycle_y", &["id"], 0),
            key("cycle_y", "id", "public.cycle_y", &["id"], 0),
            key("cycle_y", "x_id", "public.cycle_x", &["id"], 0),
            // Row-level-security fixture.
            key("rls_parents", "id", "public.rls_parents", &["id"], 0),
            rule(
                "public",
                "rls_parents",
                "name",
                MaskStrategy::DeterministicPseudonym,
            ),
            key("rls_children", "id", "public.rls_children", &["id"], 0),
            key(
                "rls_children",
                "parent_id",
                "public.rls_parents",
                &["id"],
                0,
            ),
        ];

        let mut policy = Self {
            name: "development-default".to_string(),
            version: 2,
            rules,
            hash: String::new(),
        };
        policy.hash = policy.compute_hash();
        policy
    }

    /// Load a custom policy from YAML.
    pub fn parse_yaml(text: &str) -> Result<Self, CoreError> {
        let mut policy: MaskingPolicy = serde_yaml::from_str(text).map_err(|error| {
            CoreError::with_detail(
                ErrorCode::Usage,
                format!("masking policy could not be parsed: {error}"),
                "mask_policy_invalid",
            )
        })?;
        policy.hash = policy.compute_hash();
        policy.validate()?;
        Ok(policy)
    }

    pub fn identifier(&self) -> String {
        format!("{}@{}", self.name, self.version)
    }

    pub fn rule_for(&self, schema: &str, table: &str, column: &str) -> Option<&ColumnRule> {
        self.rules
            .iter()
            .find(|rule| rule.schema == schema && rule.table == table && rule.column == column)
    }

    /// Stable hash of the rules, so a plan binds the exact policy.
    pub fn compute_hash(&self) -> String {
        let canonical =
            serde_json::to_string(&(&self.name, self.version, &self.rules)).unwrap_or_default();
        hex_lower(&digest(canonical.as_bytes()))
    }

    fn validate(&self) -> Result<(), CoreError> {
        if self.name.trim().is_empty() {
            return Err(invalid_policy("the policy name is empty"));
        }
        if self.version == 0 {
            return Err(invalid_policy("the policy version must be at least 1"));
        }

        let mut seen = BTreeSet::new();
        for rule in &self.rules {
            if !is_identifier(&rule.schema)
                || !is_identifier(&rule.table)
                || !is_identifier(&rule.column)
            {
                return Err(invalid_policy("a rule names an invalid identifier"));
            }
            if !seen.insert((&rule.schema, &rule.table, &rule.column)) {
                return Err(invalid_policy("a column has more than one rule"));
            }
            if rule.strategy == MaskStrategy::Keep && forbidden_keep(&rule.column) {
                return Err(invalid_policy(
                    "a sensitive-looking column cannot be kept unchanged",
                ));
            }
        }

        Ok(())
    }
}

/// What the masker will do to one column.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ColumnCoverage {
    pub column: String,
    pub class: ColumnClass,
    pub strategy: MaskStrategy,
}

/// The full coverage of a table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CoverageReport {
    pub columns: Vec<ColumnCoverage>,
    pub masked: usize,
    pub kept: usize,
}

/// A typed input value.
#[derive(Debug, Clone, Copy)]
pub enum MaskInput<'a> {
    Text(&'a str),
    Uuid(&'a str),
}

/// A typed output value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MaskedValue {
    Text(String),
    Uuid(String),
    Null,
}

impl MaskedValue {
    pub fn as_text(&self) -> Option<&str> {
        match self {
            MaskedValue::Text(value) | MaskedValue::Uuid(value) => Some(value),
            MaskedValue::Null => None,
        }
    }
}

/// Applies a policy to values.
#[derive(Debug)]
pub struct Masker {
    policy: MaskingPolicy,
    key: MaskKey,
}

impl Masker {
    pub fn new(policy: MaskingPolicy, key: MaskKey) -> Result<Self, CoreError> {
        policy.validate()?;
        Ok(Self { policy, key })
    }

    pub fn from_builtin(key: MaskKey) -> Result<Self, CoreError> {
        Self::new(MaskingPolicy::builtin_default(), key)
    }

    pub fn policy(&self) -> &MaskingPolicy {
        &self.policy
    }

    pub fn key_id(&self) -> &str {
        self.key.id()
    }

    /// Every column must have a rule, and the rule must be safe for the type.
    pub fn coverage(
        &self,
        schema: &str,
        table: &str,
        columns: &[ColumnShape],
    ) -> Result<CoverageReport, CoreError> {
        let mut report = CoverageReport {
            columns: Vec::with_capacity(columns.len()),
            masked: 0,
            kept: 0,
        };

        for column in columns {
            let rule = self
                .policy
                .rule_for(schema, table, &column.name)
                .ok_or_else(|| {
                    CoreError::with_detail(
                        ErrorCode::Operation,
                        format!(
                            "column {}.{}.{} has no masking rule",
                            schema, table, column.name
                        ),
                        "unmapped_column",
                    )
                })?;

            validate_rule_for_type(rule, column)?;

            let class = if column.is_primary_key && rule.strategy == MaskStrategy::Keep {
                ColumnClass::PrimaryKey
            } else if rule.strategy == MaskStrategy::Keep {
                ColumnClass::Keep
            } else {
                ColumnClass::Masked
            };

            if rule.strategy.is_transform() {
                report.masked += 1;
            } else {
                report.kept += 1;
            }

            report.columns.push(ColumnCoverage {
                column: column.name.clone(),
                class,
                strategy: rule.strategy,
            });
        }

        Ok(report)
    }

    /// Transform one value. The identity is the declared row identity and is
    /// what makes the output deterministic.
    pub fn mask_value(
        &self,
        rule: &ColumnRule,
        identity: &str,
        input: MaskInput<'_>,
    ) -> Result<MaskedValue, CoreError> {
        let output = match rule.strategy {
            MaskStrategy::Keep => match input {
                MaskInput::Text(value) => MaskedValue::Text(value.to_string()),
                MaskInput::Uuid(value) => MaskedValue::Uuid(value.to_string()),
            },
            MaskStrategy::Redact => match input {
                MaskInput::Text(_) => MaskedValue::Text("REDACTED".to_string()),
                MaskInput::Uuid(_) => {
                    return Err(unsupported_type(
                        "redact is not implemented for this column type",
                    ))
                }
            },
            MaskStrategy::Null => MaskedValue::Null,
            MaskStrategy::DeterministicEmail => {
                let MaskInput::Text(value) = input else {
                    return Err(unsupported_type("deterministic_email needs a text column"));
                };
                let _ = value;
                let token = self.token(rule, identity);
                let local = &token[..16];
                let output = format!("{local}@{EMAIL_DOMAIN}");
                fit_text(&output, rule.max_len, "deterministic_email")?
            }
            MaskStrategy::DeterministicPseudonym => {
                let MaskInput::Text(_) = input else {
                    return Err(unsupported_type(
                        "deterministic_pseudonym needs a text column",
                    ));
                };
                let token = self.token(rule, identity);
                let output = format!("Anon {}", &token[..10]);
                fit_text(&output, rule.max_len, "deterministic_pseudonym")?
            }
            MaskStrategy::DeterministicKey => {
                return Err(CoreError::with_detail(
                    ErrorCode::Unsupported,
                    "a key column must be masked through mask_key, not mask_value",
                    "unsupported_strategy",
                ))
            }
            MaskStrategy::DeterministicPhone
            | MaskStrategy::DateShift
            | MaskStrategy::AmountJitter
            | MaskStrategy::Fixed => {
                return Err(CoreError::with_detail(
                    ErrorCode::Unsupported,
                    format!(
                        "masking strategy '{}' is not implemented in this slice",
                        rule.strategy.as_str()
                    ),
                    "unsupported_strategy",
                ))
            }
        };

        Ok(output)
    }

    /// Mask one component of a key tuple.
    ///
    /// The tuple is the whole referenced key, not just this column. A parent
    /// primary key and a child foreign key that carry the same tuple therefore
    /// produce the same per-column output, so the join survives masking.
    ///
    /// PostgreSQL treats a composite foreign key with any NULL component as not
    /// present, so a nullable tuple is masked to NULL rather than to a value
    /// that could accidentally match a real parent.
    pub fn mask_key(
        &self,
        rule: &ColumnRule,
        tuple: &[Option<String>],
    ) -> Result<MaskedValue, CoreError> {
        let Some(domain) = &rule.key_domain else {
            return Err(CoreError::with_detail(
                ErrorCode::Operation,
                format!("key column {} has no key domain", rule.column),
                "unmapped_key_domain",
            ));
        };

        if tuple.len() != domain.columns.len() {
            return Err(CoreError::with_detail(
                ErrorCode::Operation,
                format!(
                    "key column {} expects {} tuple components",
                    rule.column,
                    domain.columns.len()
                ),
                "invalid_key_tuple",
            ));
        }

        if rule.key_position >= domain.columns.len() {
            return Err(CoreError::with_detail(
                ErrorCode::Operation,
                format!(
                    "key column {} has position {} outside a {}-column domain",
                    rule.column,
                    rule.key_position,
                    domain.columns.len()
                ),
                "invalid_key_position",
            ));
        }

        // Preserve PostgreSQL NULL semantics per component: a NULL component
        // stays NULL, and the non-NULL components are hashed with a NULL marker
        // so they cannot match a real parent key.
        if tuple
            .get(rule.key_position)
            .and_then(|value| value.as_ref())
            .is_none()
        {
            return Ok(MaskedValue::Null);
        }

        // Versioned canonical key domain: the referenced table, the ordered
        // referenced columns and the position are all bound, so two different
        // key spaces with the same raw values cannot correlate.
        let mut material = String::new();
        material.push_str("key-domain:v2");
        material.push('\u{1f}');
        material.push_str(&domain.table);
        material.push('\u{1f}');
        material.push_str(&domain.columns.join("\u{1e}"));
        material.push('\u{1f}');
        material.push_str(&rule.key_position.to_string());
        for value in tuple {
            material.push('\u{1f}');
            match value {
                Some(value) => {
                    let value = normalize_identity(value);
                    material.push_str(&format!("v{}:{}", value.len(), value));
                }
                None => material.push('n'),
            }
        }

        let bytes = self.key.hmac(MASK_DOMAIN, material.as_bytes());
        let token = base32_lower(&bytes);
        fit_text(&token, rule.max_len, "deterministic_key")
    }

    /// The keyed filter digest, so a plan can bind a filter without storing it.
    pub fn filter_digest(&self, schema: &str, table: &str, canonical_filter: &str) -> String {
        let material = format!("{schema}\u{1f}{table}\u{1f}{canonical_filter}");
        let bytes = self.key.hmac(FILTER_DOMAIN, material.as_bytes());
        format!("hmac-sha256:{}", hex_lower(&bytes))
    }

    /// Refuse duplicate masked values on a unique key.
    pub fn ensure_unique(&self, values: &[String]) -> Result<(), CoreError> {
        let mut seen = BTreeSet::new();
        for value in values {
            if !seen.insert(value) {
                return Err(CoreError::with_detail(
                    ErrorCode::Operation,
                    "two masked values collided on a unique key",
                    "mask_collision",
                ));
            }
        }
        Ok(())
    }

    fn token(&self, rule: &ColumnRule, identity: &str) -> String {
        let identity = normalize_identity(identity);
        let material = format!(
            "{}\u{1f}{}\u{1f}{}\u{1f}{}",
            rule.schema, rule.table, rule.column, identity
        );
        let bytes = self.key.hmac(MASK_DOMAIN, material.as_bytes());
        base32_lower(&bytes)
    }
}

/// NFC-normalise before hashing, so two visually identical inputs agree.
pub fn normalize_identity(input: &str) -> String {
    input.nfc().collect()
}

fn validate_rule_for_type(rule: &ColumnRule, column: &ColumnShape) -> Result<(), CoreError> {
    let type_name = column.type_name.to_ascii_lowercase();

    if is_risky_type(&type_name) && rule.strategy != MaskStrategy::Null {
        return Err(unsupported_type(format!(
            "column {} has a type that cannot be masked safely without an explicit null rule",
            column.name
        )));
    }

    match rule.strategy {
        MaskStrategy::Keep => {
            if !is_keepable_type(&type_name) {
                return Err(unsupported_type(format!(
                    "column {} cannot be kept unchanged",
                    column.name
                )));
            }
        }
        MaskStrategy::DeterministicEmail
        | MaskStrategy::DeterministicPseudonym
        | MaskStrategy::DeterministicKey
        | MaskStrategy::Redact => {
            if !is_text_type(&type_name) {
                return Err(unsupported_type(format!(
                    "column {} is not a text column",
                    column.name
                )));
            }
        }
        MaskStrategy::Null => {
            if !column.nullable {
                return Err(unsupported_type(format!(
                    "column {} is not nullable, so it cannot be nulled",
                    column.name
                )));
            }
        }
        MaskStrategy::DeterministicPhone
        | MaskStrategy::DateShift
        | MaskStrategy::AmountJitter
        | MaskStrategy::Fixed => {
            return Err(CoreError::with_detail(
                ErrorCode::Unsupported,
                format!(
                    "masking strategy '{}' is not implemented in this slice",
                    rule.strategy.as_str()
                ),
                "unsupported_strategy",
            ))
        }
    }

    Ok(())
}

fn fit_text(value: &str, max_len: Option<usize>, strategy: &str) -> Result<MaskedValue, CoreError> {
    match max_len {
        None => Ok(MaskedValue::Text(value.to_string())),
        Some(limit) if limit >= value.chars().count() => Ok(MaskedValue::Text(value.to_string())),
        Some(limit) => {
            // A pseudonym or a key may be truncated deterministically; an email
            // must keep its domain.
            if (strategy == "deterministic_pseudonym" || strategy == "deterministic_key")
                && limit > 0
            {
                Ok(MaskedValue::Text(value.chars().take(limit).collect()))
            } else {
                Err(CoreError::with_detail(
                    ErrorCode::Operation,
                    format!("the destination column is too short for {strategy} output"),
                    "mask_length_incompatible",
                ))
            }
        }
    }
}

pub fn is_text_type(type_name: &str) -> bool {
    matches!(
        type_name,
        "text" | "character varying" | "varchar" | "citext" | "name"
    )
}

pub fn is_uuid_type(type_name: &str) -> bool {
    type_name == "uuid"
}

pub fn is_keepable_type(type_name: &str) -> bool {
    is_text_type(type_name) || is_uuid_type(type_name)
}

pub fn is_risky_type(type_name: &str) -> bool {
    type_name.ends_with("[]")
        || matches!(
            type_name,
            "json" | "jsonb" | "bytea" | "xml" | "tsvector" | "tsquery"
        )
}

/// A sensitive-looking column name may never be kept unchanged.
fn forbidden_keep(column: &str) -> bool {
    let column = column.to_ascii_lowercase();
    [
        "password",
        "passwd",
        "password_hash",
        "secret",
        "token",
        "api_key",
        "apikey",
        "ssn",
        "national_id",
        "credit_card",
    ]
    .iter()
    .any(|needle| column.contains(needle))
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

fn invalid_policy(message: &str) -> CoreError {
    CoreError::with_detail(ErrorCode::Usage, message, "mask_policy_invalid")
}

fn unsupported_type(message: impl Into<String>) -> CoreError {
    CoreError::with_detail(ErrorCode::Unsupported, message, "unsupported_type")
}

/// HMAC-SHA256 over `domain || 0x1F || material`.
pub fn hmac_sha256(key: &[u8; 32], domain: &str, material: &[u8]) -> [u8; 32] {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts a 32-byte key");
    mac.update(domain.as_bytes());
    mac.update(&[0x1f]);
    mac.update(material);
    let bytes = mac.finalize().into_bytes();
    let mut output = [0u8; 32];
    output.copy_from_slice(&bytes);
    output
}

/// Plain SHA-256, for non-secret digests such as a schema hash.
pub fn digest(input: &[u8]) -> [u8; 32] {
    use sha2::Digest;
    let mut hasher = Sha256::new();
    hasher.update(input);
    let bytes = hasher.finalize();
    let mut output = [0u8; 32];
    output.copy_from_slice(&bytes);
    output
}

pub fn hex_lower(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push_str(&format!("{byte:02x}"));
    }
    output
}

/// RFC 4648 base32, lowercase, no padding.
pub fn base32_lower(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";
    let mut output = String::with_capacity(bytes.len() * 8 / 5 + 1);
    let mut buffer: u32 = 0;
    let mut bits: u32 = 0;

    for byte in bytes {
        buffer = (buffer << 8) | u32::from(*byte);
        bits += 8;
        while bits >= 5 {
            let index = ((buffer >> (bits - 5)) & 0x1f) as usize;
            output.push(ALPHABET[index] as char);
            bits -= 5;
        }
    }

    if bits > 0 {
        let index = ((buffer << (5 - bits)) & 0x1f) as usize;
        output.push(ALPHABET[index] as char);
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::safesync::key::MaskKeyStore;

    fn key() -> MaskKey {
        MaskKey::from_bytes("k-test", [7u8; 32])
    }

    fn masker() -> Masker {
        Masker::from_builtin(key()).unwrap()
    }

    fn accounts_columns() -> Vec<ColumnShape> {
        vec![
            ColumnShape {
                name: "id".into(),
                type_name: "uuid".into(),
                nullable: false,
                is_primary_key: true,
                max_len: None,
            },
            ColumnShape {
                name: "email".into(),
                type_name: "text".into(),
                nullable: false,
                is_primary_key: false,
                max_len: None,
            },
            ColumnShape {
                name: "full_name".into(),
                type_name: "text".into(),
                nullable: false,
                is_primary_key: false,
                max_len: None,
            },
        ]
    }

    fn rule(strategy: MaskStrategy) -> ColumnRule {
        ColumnRule {
            schema: "public".into(),
            table: "accounts".into(),
            column: "email".into(),
            strategy,
            max_len: None,
            key_domain: None,
            key_position: 0,
        }
    }

    #[test]
    fn same_input_same_key_same_output() {
        let masker = masker();
        let rule = rule(MaskStrategy::DeterministicEmail);
        let first = masker
            .mask_value(&rule, "id=1", MaskInput::Text("a@b.com"))
            .unwrap();
        let second = masker
            .mask_value(&rule, "id=1", MaskInput::Text("a@b.com"))
            .unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn a_different_key_produces_a_different_output() {
        let first = Masker::from_builtin(MaskKey::from_bytes("k1", [1u8; 32])).unwrap();
        let second = Masker::from_builtin(MaskKey::from_bytes("k2", [2u8; 32])).unwrap();
        let rule = rule(MaskStrategy::DeterministicEmail);
        let a = first
            .mask_value(&rule, "id=1", MaskInput::Text("a@b.com"))
            .unwrap();
        let b = second
            .mask_value(&rule, "id=1", MaskInput::Text("a@b.com"))
            .unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn different_fields_are_domain_separated() {
        let masker = masker();
        let identity = "id=1";
        let material_a = format!("public\u{1f}accounts\u{1f}email\u{1f}{identity}");
        let material_b = format!("public\u{1f}accounts\u{1f}full_name\u{1f}{identity}");
        let a = masker.key_hmac_for_test(MASK_DOMAIN, material_a.as_bytes());
        let b = masker.key_hmac_for_test(MASK_DOMAIN, material_b.as_bytes());
        assert_ne!(a, b);
    }

    #[test]
    fn identity_is_nfc_normalised() {
        let decomposed = "Jose\u{0301}";
        let composed = "Jos\u{00e9}";
        assert_eq!(normalize_identity(decomposed), normalize_identity(composed));
    }

    #[test]
    fn a_synthetic_email_uses_example_invalid() {
        let masker = masker();
        let value = masker
            .mask_value(
                &rule(MaskStrategy::DeterministicEmail),
                "id=1",
                MaskInput::Text("a@b.com"),
            )
            .unwrap();
        let text = value.as_text().unwrap();
        assert!(text.ends_with("@example.invalid"));
        assert_eq!(text.split('@').next().unwrap().len(), 16);
        assert!(text
            .split('@')
            .next()
            .unwrap()
            .chars()
            .all(|c| "abcdefghijklmnopqrstuvwxyz234567".contains(c)));
    }

    #[test]
    fn a_pseudonym_is_deterministic_and_anon_prefixed() {
        let masker = masker();
        let rule = rule(MaskStrategy::DeterministicPseudonym);
        let first = masker
            .mask_value(&rule, "id=1", MaskInput::Text("Alice Example"))
            .unwrap();
        let second = masker
            .mask_value(&rule, "id=2", MaskInput::Text("Alice Example"))
            .unwrap();
        let text = first.as_text().unwrap();
        assert!(text.starts_with("Anon "));
        assert_eq!(text.len(), 15);
        // Determinism is over the identity, not the value, so two identities
        // with the same name differ.
        assert_ne!(first, second);
    }

    #[test]
    fn the_uuid_id_is_kept_only_by_explicit_rule() {
        let masker = masker();
        let rule = rule(MaskStrategy::Keep);
        let value = masker
            .mask_value(
                &rule,
                "id=1",
                MaskInput::Uuid("0b9f4c3e-0000-0000-0000-000000000001"),
            )
            .unwrap();
        assert_eq!(
            value,
            MaskedValue::Uuid("0b9f4c3e-0000-0000-0000-000000000001".into())
        );
    }

    #[test]
    fn coverage_refuses_an_unmapped_column() {
        let masker = masker();
        let mut columns = accounts_columns();
        columns.push(ColumnShape {
            name: "phone".into(),
            type_name: "text".into(),
            nullable: true,
            is_primary_key: false,
            max_len: None,
        });
        let error = masker.coverage("public", "accounts", &columns).unwrap_err();
        assert_eq!(error.detail.as_deref(), Some("unmapped_column"));
    }

    #[test]
    fn coverage_covers_the_three_approved_columns() {
        let report = masker()
            .coverage("public", "accounts", &accounts_columns())
            .unwrap();
        assert_eq!(report.columns.len(), 3);
        assert_eq!(report.masked, 2);
        assert_eq!(report.kept, 1);
        assert_eq!(report.columns[0].class, ColumnClass::PrimaryKey);
    }

    #[test]
    fn coverage_refuses_a_jsonb_column_without_a_rule() {
        let masker = masker();
        let columns = vec![ColumnShape {
            name: "meta".into(),
            type_name: "jsonb".into(),
            nullable: true,
            is_primary_key: false,
            max_len: None,
        }];
        let error = masker.coverage("public", "accounts", &columns).unwrap_err();
        assert_eq!(error.detail.as_deref(), Some("unmapped_column"));
    }

    #[test]
    fn a_keep_rule_on_a_jsonb_column_is_refused() {
        let mut policy = MaskingPolicy::builtin_default();
        policy.rules.push(ColumnRule {
            schema: "public".into(),
            table: "accounts".into(),
            column: "meta".into(),
            strategy: MaskStrategy::Keep,
            max_len: None,
            key_domain: None,
            key_position: 0,
        });
        let masker = Masker::new(policy, key()).unwrap();
        let columns = vec![ColumnShape {
            name: "meta".into(),
            type_name: "jsonb".into(),
            nullable: true,
            is_primary_key: false,
            max_len: None,
        }];
        let error = masker.coverage("public", "accounts", &columns).unwrap_err();
        assert_eq!(error.detail.as_deref(), Some("unsupported_type"));
    }

    #[test]
    fn a_keep_rule_on_a_password_hash_is_rejected_at_load() {
        let policy = MaskingPolicy {
            name: "bad".into(),
            version: 1,
            rules: vec![ColumnRule {
                schema: "public".into(),
                table: "users".into(),
                column: "password_hash".into(),
                strategy: MaskStrategy::Keep,
                max_len: None,
                key_domain: None,
                key_position: 0,
            }],
            hash: String::new(),
        };
        let error = Masker::new(policy, key()).unwrap_err();
        assert_eq!(error.detail.as_deref(), Some("mask_policy_invalid"));
    }

    #[test]
    fn a_column_shorter_than_the_output_is_refused() {
        let masker = masker();
        let mut rule = rule(MaskStrategy::DeterministicEmail);
        rule.max_len = Some(10);
        let error = masker
            .mask_value(&rule, "id=1", MaskInput::Text("a@b.com"))
            .unwrap_err();
        assert_eq!(error.detail.as_deref(), Some("mask_length_incompatible"));
    }

    #[test]
    fn a_pseudonym_is_truncated_deterministically_when_max_len_allows() {
        let masker = masker();
        let mut rule = rule(MaskStrategy::DeterministicPseudonym);
        rule.max_len = Some(6);
        let value = masker
            .mask_value(&rule, "id=1", MaskInput::Text("Alice"))
            .unwrap();
        assert_eq!(value.as_text().unwrap().chars().count(), 6);
    }

    #[test]
    fn a_collision_is_refused() {
        let masker = masker();
        let error = masker
            .ensure_unique(&["a".into(), "b".into(), "a".into()])
            .unwrap_err();
        assert_eq!(error.detail.as_deref(), Some("mask_collision"));
    }

    #[test]
    fn distinct_values_pass_the_uniqueness_check() {
        masker()
            .ensure_unique(&["a".into(), "b".into(), "c".into()])
            .unwrap();
    }

    #[test]
    fn a_missing_key_refuses_a_masker() {
        let store = crate::safesync::key::InMemoryMaskKeyStore::default();
        let error = store.load().unwrap_err();
        assert_eq!(error.detail.as_deref(), Some("mask_key_missing"));
    }

    #[test]
    fn an_invalid_policy_is_refused() {
        let error = MaskingPolicy::parse_yaml("name: x\nversion: 1\nrules: [oops").unwrap_err();
        assert_eq!(error.detail.as_deref(), Some("mask_policy_invalid"));
    }

    #[test]
    fn a_key_and_its_reference_mask_to_the_same_value() {
        let masker = masker();
        let domain = KeyDomain {
            table: "public.customers".into(),
            columns: vec!["code".into()],
        };
        let parent = ColumnRule {
            schema: "public".into(),
            table: "customers".into(),
            column: "code".into(),
            strategy: MaskStrategy::DeterministicKey,
            max_len: None,
            key_domain: Some(domain.clone()),
            key_position: 0,
        };
        let child = ColumnRule {
            schema: "public".into(),
            table: "orders".into(),
            column: "customer_code".into(),
            strategy: MaskStrategy::DeterministicKey,
            max_len: None,
            key_domain: Some(domain),
            key_position: 0,
        };

        let left = masker.mask_key(&parent, &[Some("C-1".into())]).unwrap();
        let right = masker.mask_key(&child, &[Some("C-1".into())]).unwrap();
        assert_eq!(left, right);
        assert_ne!(left.as_text().unwrap(), "C-1");
    }

    #[test]
    fn unrelated_key_domains_stay_separated() {
        let masker = masker();
        let rule = |domain_table: &str| ColumnRule {
            schema: "public".into(),
            table: "t".into(),
            column: "c".into(),
            strategy: MaskStrategy::DeterministicKey,
            max_len: None,
            key_domain: Some(KeyDomain {
                table: domain_table.into(),
                columns: vec!["c".into()],
            }),
            key_position: 0,
        };

        let left = masker
            .mask_key(&rule("public.a"), &[Some("1".into())])
            .unwrap();
        let right = masker
            .mask_key(&rule("public.b"), &[Some("1".into())])
            .unwrap();
        assert_ne!(left, right);
    }

    #[test]
    fn a_composite_key_tuple_is_position_separated() {
        let masker = masker();
        let rule = |position: usize| ColumnRule {
            schema: "public".into(),
            table: "t".into(),
            column: "c".into(),
            strategy: MaskStrategy::DeterministicKey,
            max_len: None,
            key_domain: Some(KeyDomain {
                table: "public.parent".into(),
                columns: vec!["a".into(), "b".into()],
            }),
            key_position: position,
        };

        let tuple = [Some("x".into()), Some("y".into())];
        let first = masker.mask_key(&rule(0), &tuple).unwrap();
        let second = masker.mask_key(&rule(1), &tuple).unwrap();
        assert_ne!(first, second);
        assert_eq!(first, masker.mask_key(&rule(0), &tuple).unwrap());
    }

    #[test]
    fn a_fully_null_key_masks_to_null() {
        let masker = masker();
        let rule = ColumnRule {
            schema: "public".into(),
            table: "t".into(),
            column: "c".into(),
            strategy: MaskStrategy::DeterministicKey,
            max_len: None,
            key_domain: Some(KeyDomain {
                table: "public.parent".into(),
                columns: vec!["a".into(), "b".into()],
            }),
            key_position: 0,
        };
        let value = masker.mask_key(&rule, &[None, None]).unwrap();
        assert_eq!(value, MaskedValue::Null);
    }

    #[test]
    fn a_partially_null_key_component_stays_distinct() {
        let masker = masker();
        let rule = |position: usize| ColumnRule {
            schema: "public".into(),
            table: "t".into(),
            column: "c".into(),
            strategy: MaskStrategy::DeterministicKey,
            max_len: None,
            key_domain: Some(KeyDomain {
                table: "public.parent".into(),
                columns: vec!["a".into(), "b".into()],
            }),
            key_position: position,
        };
        let tuple = [Some("x".into()), None];
        // The non-NULL component stays non-NULL.
        let first = masker.mask_key(&rule(0), &tuple).unwrap();
        assert!(first.as_text().is_some());
        // The NULL component stays NULL.
        assert_eq!(
            masker.mask_key(&rule(1), &tuple).unwrap(),
            MaskedValue::Null
        );
        // The partial tuple cannot match the real (x, "") tuple.
        let empty = masker
            .mask_key(&rule(0), &[Some("x".into()), Some("".into())])
            .unwrap();
        assert_ne!(first, empty);
    }

    #[test]
    fn different_column_orders_are_separated() {
        let masker = masker();
        let rule = |columns: &[&str]| ColumnRule {
            schema: "public".into(),
            table: "t".into(),
            column: "c".into(),
            strategy: MaskStrategy::DeterministicKey,
            max_len: None,
            key_domain: Some(KeyDomain {
                table: "public.parent".into(),
                columns: columns.iter().map(|value| value.to_string()).collect(),
            }),
            key_position: 0,
        };

        let left = masker
            .mask_key(&rule(&["a", "b"]), &[Some("x".into()), Some("y".into())])
            .unwrap();
        let right = masker
            .mask_key(&rule(&["b", "a"]), &[Some("x".into()), Some("y".into())])
            .unwrap();
        assert_ne!(left, right);
    }

    #[test]
    fn an_out_of_range_key_position_is_refused() {
        let masker = masker();
        let rule = ColumnRule {
            schema: "public".into(),
            table: "t".into(),
            column: "c".into(),
            strategy: MaskStrategy::DeterministicKey,
            max_len: None,
            key_domain: Some(KeyDomain {
                table: "public.parent".into(),
                columns: vec!["a".into()],
            }),
            key_position: 3,
        };
        let error = masker.mask_key(&rule, &[Some("x".into())]).unwrap_err();
        assert_eq!(error.detail.as_deref(), Some("invalid_key_position"));
    }

    #[test]
    fn a_missing_key_domain_is_refused() {
        let masker = masker();
        let rule = ColumnRule {
            schema: "public".into(),
            table: "t".into(),
            column: "c".into(),
            strategy: MaskStrategy::DeterministicKey,
            max_len: None,
            key_domain: None,
            key_position: 0,
        };
        let error = masker.mask_key(&rule, &[Some("x".into())]).unwrap_err();
        assert_eq!(error.detail.as_deref(), Some("unmapped_key_domain"));
    }

    #[test]
    fn an_unimplemented_strategy_is_refused() {
        let masker = masker();
        let mut policy_rule = rule(MaskStrategy::DateShift);
        policy_rule.column = "email".into();
        let error = masker
            .mask_value(&policy_rule, "id=1", MaskInput::Text("2020-01-01"))
            .unwrap_err();
        assert_eq!(error.detail.as_deref(), Some("unsupported_strategy"));
    }

    #[test]
    fn the_filter_digest_is_keyed_and_stable() {
        let masker = masker();
        let first = masker.filter_digest("public", "accounts", "id=1");
        let second = masker.filter_digest("public", "accounts", "id=1");
        assert_eq!(first, second);
        assert!(first.starts_with("hmac-sha256:"));
        assert!(!first.contains("id=1"));
    }

    #[test]
    fn a_different_key_changes_the_filter_digest() {
        let first = Masker::from_builtin(MaskKey::from_bytes("k1", [1u8; 32])).unwrap();
        let second = Masker::from_builtin(MaskKey::from_bytes("k2", [2u8; 32])).unwrap();
        assert_ne!(
            first.filter_digest("public", "accounts", "id=1"),
            second.filter_digest("public", "accounts", "id=1")
        );
    }

    #[test]
    fn the_policy_hash_changes_with_a_rule() {
        let first = MaskingPolicy::builtin_default();
        let mut second = MaskingPolicy::builtin_default();
        second.rules[1].strategy = MaskStrategy::Redact;
        assert_ne!(first.compute_hash(), second.compute_hash());
    }

    #[test]
    fn base32_is_lowercase_and_padding_free() {
        let encoded = base32_lower(&[0u8, 0u8, 0u8, 0u8, 0u8]);
        assert_eq!(encoded, "aaaaaaaa");
    }

    impl Masker {
        fn key_hmac_for_test(&self, domain: &str, material: &[u8]) -> String {
            hex_lower(&self.key.hmac(domain, material))
        }
    }
}
