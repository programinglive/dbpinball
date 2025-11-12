use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DbConnection {
    pub id: String,
    pub name: String,
    pub engine: DbEngine,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
    pub use_ssl: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum DbEngine {
    Mysql,
    Postgres,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BackupOptions {
    pub structure: bool,
    pub data: bool,
    pub include_views: bool,
    pub include_routines: bool,
    pub include_triggers: bool,
    pub bulk_insert: bool,
    pub ignore_definer: bool,
    pub compress: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RestoreOptions {
    pub drop_existing: bool,
    pub stop_on_error: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CopyOptions {
    pub structure: bool,
    pub data: bool,
    pub drop_in_target: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Option<String>>>,
}
