//! Destination identity, fingerprints, enrollment and freshness.
//!
//! THE RULE
//! --------
//! A user-declared label (`local`, `staging`) is a hint. It is never proof.
//! Authorization requires an identity that the server itself reported, matched
//! against a fingerprint the user explicitly enrolled, and evidence that has not
//! expired.
//!
//! WHAT THIS CANNOT DO
//! -------------------
//! A cloned database has the same schema but a different `system_identifier`.
//! A copied fingerprint therefore does not authorize the copy. This module
//! detects that; it cannot prevent a user from enrolling the copy deliberately.
//! Application policy also cannot replace database permissions: a least-privilege
//! role is still the only thing that stops a write at the engine.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::domain::connection::DbEngine;

/// How trustworthy a destination is, based on evidence rather than labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DestinationCategory {
    /// Independently verified as a disposable database.
    VerifiedDisposable,
    /// Independently verified as a development database.
    VerifiedDevelopment,
    /// Could not be classified. Never treated as development.
    Unknown,
    /// Evidence suggests production. Never an allowed write target.
    ProductionLike,
    /// Explicitly denied by the user or by policy.
    Denied,
}

impl DestinationCategory {
    /// Only verified development or disposable targets may receive writes.
    pub fn allows_write(self) -> bool {
        matches!(
            self,
            DestinationCategory::VerifiedDisposable | DestinationCategory::VerifiedDevelopment
        )
    }

    pub fn as_str(self) -> &'static str {
        match self {
            DestinationCategory::VerifiedDisposable => "verified_disposable",
            DestinationCategory::VerifiedDevelopment => "verified_development",
            DestinationCategory::Unknown => "unknown",
            DestinationCategory::ProductionLike => "production_like",
            DestinationCategory::Denied => "denied",
        }
    }
}

/// Facts a server reported about itself.
///
/// Every field is optional because engines and privileges differ. A missing
/// field is recorded as missing, never guessed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerIdentity {
    pub engine: Option<String>,
    pub version: Option<String>,
    /// PostgreSQL `system_identifier`, MySQL `server_uuid`.
    pub server_uuid: Option<String>,
    /// Database name the session is connected to.
    pub database: Option<String>,
    /// PostgreSQL `current_database()::regdatabase.oid` or MySQL schema.
    pub database_id: Option<String>,
    /// Current user as reported by the server.
    pub current_user: Option<String>,
    /// Whether the session is in a read-only transaction mode.
    pub read_only: Option<bool>,
    /// Whether the user can write to the target database.
    pub can_write: Option<bool>,
}

impl ServerIdentity {
    /// A stable identity key for comparisons.
    ///
    /// The key is built from the strongest evidence available. When no server
    /// identifier is present, the key is weaker and the caller must treat the
    /// identity as unverified.
    pub fn identity_key(&self, engine: DbEngine, host: &str, port: u16) -> Option<String> {
        let server = self
            .server_uuid
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())?;

        let database = self
            .database_id
            .as_deref()
            .or(self.database.as_deref())
            .map(str::trim)
            .filter(|value| !value.is_empty())?;

        Some(format!(
            "{}://{}:{}/{}#{}",
            engine_label(engine),
            host.trim().to_ascii_lowercase(),
            port,
            database.to_ascii_lowercase(),
            server.to_ascii_lowercase()
        ))
    }

    /// True when the server gave enough evidence to build an identity.
    pub fn is_identified(&self) -> bool {
        self.server_uuid
            .as_deref()
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false)
    }
}

fn engine_label(engine: DbEngine) -> &'static str {
    match engine {
        DbEngine::Mysql => "mysql",
        DbEngine::Postgres => "postgres",
    }
}

/// What was observed about a destination at one moment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DestinationIdentity {
    pub engine: DbEngine,
    pub host: String,
    pub port: u16,
    pub identity_key: Option<String>,
    pub server: ServerIdentity,
    /// Unix milliseconds when this observation was made.
    pub observed_at_ms: u64,
}

impl DestinationIdentity {
    pub fn observed_at(&self) -> u64 {
        self.observed_at_ms
    }
}

/// A fingerprint stored at enrollment time.
///
/// It intentionally contains no password, token, or full connection string.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnrolledFingerprint {
    pub identity_key: String,
    pub engine: String,
    pub database: String,
    pub category: DestinationCategory,
    /// Unix milliseconds when the user enrolled this destination.
    pub enrolled_at_ms: u64,
    /// Optional human note. Never used for a decision.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// How long a verification remains usable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FreshnessPolicy {
    pub max_age_ms: u64,
}

impl Default for FreshnessPolicy {
    fn default() -> Self {
        // Ten minutes: long enough for an interactive operation, short enough
        // that a destination change is unlikely to slip through unnoticed.
        Self {
            max_age_ms: 10 * 60 * 1000,
        }
    }
}

impl FreshnessPolicy {
    pub fn is_fresh(&self, observed_at_ms: u64, now_ms: u64) -> bool {
        if now_ms < observed_at_ms {
            // The caller samples its clock before the verification round-trip,
            // so the observation can legitimately be a few milliseconds later.
            // Treat a small skew as fresh; a large one is still stale.
            return observed_at_ms - now_ms <= 5_000;
        }

        now_ms - observed_at_ms <= self.max_age_ms
    }
}

/// Result of checking an observed destination against an enrollment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EnrollmentMatch {
    /// The identity matches the enrolled fingerprint and the evidence is fresh.
    Matches,
    /// No fingerprint was enrolled for this connection.
    NotEnrolled,
    /// A fingerprint exists but the server identity changed.
    IdentityChanged,
    /// The identity matches but the evidence is older than the freshness policy.
    Stale,
    /// The server did not provide enough evidence to compare.
    Unverifiable,
}

/// Compare an observation to an enrollment.
pub fn match_enrollment(
    identity: &DestinationIdentity,
    enrolled: Option<&EnrolledFingerprint>,
    freshness: FreshnessPolicy,
    now_ms: u64,
) -> EnrollmentMatch {
    let Some(enrolled) = enrolled else {
        return EnrollmentMatch::NotEnrolled;
    };

    let Some(observed_key) = identity.identity_key.as_deref() else {
        return EnrollmentMatch::Unverifiable;
    };

    if observed_key != enrolled.identity_key {
        return EnrollmentMatch::IdentityChanged;
    }

    if !freshness.is_fresh(identity.observed_at_ms, now_ms) {
        return EnrollmentMatch::Stale;
    }

    EnrollmentMatch::Matches
}

/// Infer a category from server evidence.
///
/// This is a *risk assessment helper*, not authorization. The result only
/// distinguishes an obviously protected destination from an unclassified one;
/// it never upgrades Unknown to Development.
pub fn assess_category(server: &ServerIdentity) -> DestinationCategory {
    if server.read_only == Some(true) {
        // A read-only server cannot be a write destination.
        return DestinationCategory::ProductionLike;
    }

    DestinationCategory::Unknown
}

/// A snapshot of every enrolled destination, keyed by connection id.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnrollmentRegistry {
    #[serde(default)]
    pub entries: BTreeMap<String, EnrolledFingerprint>,
}

impl EnrollmentRegistry {
    pub fn get(&self, connection_id: &str) -> Option<&EnrolledFingerprint> {
        self.entries.get(connection_id)
    }

    pub fn enroll(&mut self, connection_id: &str, fingerprint: EnrolledFingerprint) {
        self.entries.insert(connection_id.to_string(), fingerprint);
    }

    pub fn remove(&mut self, connection_id: &str) -> bool {
        self.entries.remove(connection_id).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(server_uuid: &str, database: &str, observed: u64) -> DestinationIdentity {
        let server = ServerIdentity {
            engine: Some("postgres".into()),
            version: Some("16".into()),
            server_uuid: Some(server_uuid.into()),
            database: Some(database.into()),
            database_id: Some(database.into()),
            current_user: Some("dev".into()),
            read_only: Some(false),
            can_write: Some(true),
        };

        DestinationIdentity {
            engine: DbEngine::Postgres,
            host: "127.0.0.1".into(),
            port: 5432,
            identity_key: server.identity_key(DbEngine::Postgres, "127.0.0.1", 5432),
            server,
            observed_at_ms: observed,
        }
    }

    fn fingerprint(key: &str) -> EnrolledFingerprint {
        EnrolledFingerprint {
            identity_key: key.into(),
            engine: "postgres".into(),
            database: "dev".into(),
            category: DestinationCategory::VerifiedDevelopment,
            enrolled_at_ms: 0,
            note: None,
        }
    }

    #[test]
    fn a_matching_fresh_enrollment_is_accepted() {
        let observed = identity("srv-1", "dev", 1_000);
        let key = observed.identity_key.clone().unwrap();
        let result = match_enrollment(
            &observed,
            Some(&fingerprint(&key)),
            FreshnessPolicy::default(),
            1_500,
        );
        assert_eq!(result, EnrollmentMatch::Matches);
    }

    #[test]
    fn a_changed_server_identity_is_detected() {
        let observed = identity("srv-2", "dev", 1_000);
        let old = identity("srv-1", "dev", 1_000);
        let key = old.identity_key.clone().unwrap();
        let result = match_enrollment(
            &observed,
            Some(&fingerprint(&key)),
            FreshnessPolicy::default(),
            1_500,
        );
        assert_eq!(result, EnrollmentMatch::IdentityChanged);
    }

    #[test]
    fn a_renamed_database_is_detected() {
        let observed = identity("srv-1", "other", 1_000);
        let old = identity("srv-1", "dev", 1_000);
        let key = old.identity_key.clone().unwrap();
        let result = match_enrollment(
            &observed,
            Some(&fingerprint(&key)),
            FreshnessPolicy::default(),
            1_500,
        );
        assert_eq!(result, EnrollmentMatch::IdentityChanged);
    }

    #[test]
    fn stale_evidence_is_not_accepted() {
        let observed = identity("srv-1", "dev", 1_000);
        let key = observed.identity_key.clone().unwrap();
        let result = match_enrollment(
            &observed,
            Some(&fingerprint(&key)),
            FreshnessPolicy { max_age_ms: 100 },
            5_000,
        );
        assert_eq!(result, EnrollmentMatch::Stale);
    }

    #[test]
    fn a_missing_enrollment_is_not_enrolled() {
        let observed = identity("srv-1", "dev", 1_000);
        let result = match_enrollment(&observed, None, FreshnessPolicy::default(), 1_500);
        assert_eq!(result, EnrollmentMatch::NotEnrolled);
    }

    #[test]
    fn an_unidentified_server_is_unverifiable() {
        let mut observed = identity("srv-1", "dev", 1_000);
        observed.identity_key = None;
        observed.server.server_uuid = None;
        let result = match_enrollment(
            &observed,
            Some(&fingerprint("anything")),
            FreshnessPolicy::default(),
            1_500,
        );
        assert_eq!(result, EnrollmentMatch::Unverifiable);
    }

    #[test]
    fn a_read_only_server_is_treated_as_protected() {
        let server = ServerIdentity {
            read_only: Some(true),
            ..ServerIdentity::default()
        };
        assert_eq!(
            assess_category(&server),
            DestinationCategory::ProductionLike
        );
    }

    #[test]
    fn unknown_is_never_upgraded_to_development() {
        let server = ServerIdentity::default();
        assert_eq!(assess_category(&server), DestinationCategory::Unknown);
        assert!(!DestinationCategory::Unknown.allows_write());
    }

    #[test]
    fn only_verified_categories_allow_writes() {
        assert!(DestinationCategory::VerifiedDevelopment.allows_write());
        assert!(DestinationCategory::VerifiedDisposable.allows_write());
        assert!(!DestinationCategory::ProductionLike.allows_write());
        assert!(!DestinationCategory::Denied.allows_write());
    }

    #[test]
    fn the_fingerprint_contains_no_secret() {
        let enrolled = fingerprint("postgres://127.0.0.1:5432/dev#srv-1");
        let json = serde_json::to_string(&enrolled).unwrap();
        assert!(!json.to_ascii_lowercase().contains("password"));
    }
}
