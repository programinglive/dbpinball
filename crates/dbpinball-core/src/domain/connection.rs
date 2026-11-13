use serde::{Deserialize, Serialize};

/// A saved database connection.
///
/// The serde shape is part of the desktop compatibility contract: the frontend
/// sends camelCase fields and the Tauri commands accept this exact structure.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserializes_frontend_payload() {
        let payload = r#"{
            "id": "abc",
            "name": "local",
            "engine": "postgres",
            "host": "127.0.0.1",
            "port": 5432,
            "database": "app",
            "username": "postgres",
            "useSsl": false
        }"#;

        let connection: DbConnection = serde_json::from_str(payload).expect("payload should parse");

        assert_eq!(connection.id, "abc");
        assert_eq!(connection.engine, DbEngine::Postgres);
        assert_eq!(connection.port, 5432);
        assert!(!connection.use_ssl);
    }

    #[test]
    fn engine_serializes_lowercase() {
        assert_eq!(
            serde_json::to_string(&DbEngine::Mysql).unwrap(),
            "\"mysql\""
        );
        assert_eq!(
            serde_json::to_string(&DbEngine::Postgres).unwrap(),
            "\"postgres\""
        );
    }
}
