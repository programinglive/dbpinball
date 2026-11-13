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
    #[serde(default)]
    pub connections: Vec<ConnectionRecord>,
}

impl Default for StoreFile {
    fn default() -> Self {
        Self {
            version: STORE_VERSION,
            migrated_from: None,
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
}

/// A record the importer refused to persist, with the reason only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InvalidRecord {
    pub index: usize,
    pub reason: String,
}
