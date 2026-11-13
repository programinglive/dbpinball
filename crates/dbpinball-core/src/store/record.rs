use serde::{Deserialize, Serialize};

use crate::domain::connection::DbConnection;

/// A stored connection profile. Aliased so the record can gain fields later
/// without changing the store API.
pub type ConnectionRecord = DbConnection;

/// The only store format M2 understands.
pub const STORE_VERSION: u32 = 1;

/// The on-disk shape of `connections.json`.
///
/// No password field exists. A test asserts the key is absent.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StoreFile {
    pub version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub migrated_from: Option<String>,
    /// Monotonic counter bumped on every successful write.
    ///
    /// A writer that read revision N and finds N+1 refuses with a conflict
    /// instead of overwriting the other writer's change. This catches a lost
    /// update even if the cross-process lock is bypassed.
    #[serde(default)]
    pub revision: u64,
    #[serde(default)]
    pub connections: Vec<ConnectionRecord>,
}

impl Default for StoreFile {
    fn default() -> Self {
        Self {
            version: STORE_VERSION,
            migrated_from: None,
            revision: 0,
            connections: Vec::new(),
        }
    }
}

/// Result of importing a JSON profile list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    pub imported: usize,
    pub skipped_invalid: Vec<InvalidRecord>,
    pub skipped_duplicates: usize,
    /// Records whose id already exists with different connection details.
    ///
    /// They are never overwritten: the shared record wins and the import is
    /// reported so the user can decide what to do.
    pub skipped_conflicts: Vec<ConflictRecord>,
}

impl ImportReport {
    pub fn empty() -> Self {
        Self {
            imported: 0,
            skipped_invalid: Vec::new(),
            skipped_duplicates: 0,
            skipped_conflicts: Vec::new(),
        }
    }
}

/// A record the importer refused to persist, with the reason only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InvalidRecord {
    pub index: usize,
    pub reason: String,
}

/// A record whose id collides with a different existing profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictRecord {
    pub index: usize,
    pub id: String,
    pub reason: String,
}
