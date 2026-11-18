use serde::{Deserialize, Serialize};

/// Optional SSH tunnel configuration for reaching a database behind a bastion.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SshTunnelConfig {
    #[serde(default)]
    pub enabled: bool,
    pub host: String,
    #[serde(default = "default_ssh_port")]
    pub port: u16,
    pub username: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub private_key_path: Option<String>,
    #[serde(default)]
    pub strict_host_key_checking: bool,
}

fn default_ssh_port() -> u16 {
    22
}

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
    /// User-provided metadata only. Never treated as verified evidence.
    #[serde(default)]
    pub target: TargetClass,
    /// Optional SSH tunnel configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_tunnel: Option<SshTunnelConfig>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DbEngine {
    Mysql,
    Postgres,
}

/// A user-provided hint about where a connection points.
///
/// This is metadata, not proof. It must never be used on its own to decide that
/// a database is safe to write to.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum TargetClass {
    Production,
    Staging,
    Local,
    #[default]
    Unknown,
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
        assert_eq!(connection.target, TargetClass::Unknown);
    }

    #[test]
    fn target_defaults_to_unknown_and_round_trips() {
        let connection = DbConnection {
            id: "abc".into(),
            name: "local".into(),
            engine: DbEngine::Postgres,
            host: "127.0.0.1".into(),
            port: 5432,
            database: "app".into(),
            username: "postgres".into(),
            use_ssl: false,
            target: TargetClass::Local,
            ssh_tunnel: None,
        };

        let serialized = serde_json::to_string(&connection).unwrap();
        assert!(serialized.contains("\"target\":\"local\""));
        assert!(!serialized.contains("sshTunnel"));

        let parsed: DbConnection = serde_json::from_str(&serialized).unwrap();
        assert_eq!(parsed.target, TargetClass::Local);
        assert_eq!(parsed.ssh_tunnel, None);
    }

    #[test]
    fn ssh_tunnel_defaults_and_round_trips() {
        let payload = r#"{
            "id": "tun-1",
            "name": "prod-via-bastion",
            "engine": "mysql",
            "host": "127.0.0.1",
            "port": 3306,
            "database": "erp",
            "username": "dbteam",
            "useSsl": false,
            "target": "production",
            "sshTunnel": {
                "enabled": true,
                "host": "34.101.87.166",
                "username": "dbtunnel",
                "privateKeyPath": "~/.ssh/tunnel_id_rsa"
            }
        }"#;

        let parsed: DbConnection = serde_json::from_str(payload).unwrap();
        let tunnel = parsed.ssh_tunnel.as_ref().expect("sshTunnel should parse");
        assert!(tunnel.enabled);
        assert_eq!(tunnel.port, 22);
        assert!(!tunnel.strict_host_key_checking);
        assert_eq!(
            tunnel.private_key_path.as_deref(),
            Some("~/.ssh/tunnel_id_rsa")
        );

        let serialized = serde_json::to_string(&parsed).unwrap();
        let round_tripped: DbConnection = serde_json::from_str(&serialized).unwrap();
        assert_eq!(round_tripped, parsed);
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
