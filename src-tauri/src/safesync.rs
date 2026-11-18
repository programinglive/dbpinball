//! Thin Tauri adapter over `dbpinball_core::safesync`.
//!
//! Desktop never makes independent authorization decisions or weaker security
//! checks; every plan, validation, and execution delegates directly to Core.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use dbpinball_core::adapters::keyring::KeyringCredentialProvider;
use dbpinball_core::domain::connection::DbConnection;
use dbpinball_core::domain::error::CoreError;
use dbpinball_core::safesync::service::{
    self, ExecuteRequest, RunReport, RunStatus, ValidationReport,
};
use dbpinball_core::safesync::{
    DryRunPlan, KeyringMaskKeyStore, Limits, SafeSyncSpec, SessionIdentity,
};
use dbpinball_core::security::{self, EnrollmentStore};
use dbpinball_core::store::default_config_dir;

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn safe_error(error: CoreError) -> String {
    match error.detail.as_deref() {
        Some(detail) => format!("[{detail}] {}", error.message),
        None => error.message,
    }
}

fn enrollment_store() -> Result<EnrollmentStore, String> {
    let directory = default_config_dir().map_err(safe_error)?;
    Ok(security::store_for(&directory))
}

fn audit_dir() -> Result<std::path::PathBuf, String> {
    let directory = default_config_dir().map_err(safe_error)?;
    Ok(directory.join("safe-sync-audit"))
}

pub async fn inspect(connection: DbConnection) -> Result<SessionIdentity, String> {
    service::inspect(&connection, &KeyringCredentialProvider, &Limits::default())
        .await
        .map_err(safe_error)
}

pub async fn plan(
    spec_yaml_or_json: String,
    source: DbConnection,
    destination: DbConnection,
    estimate: bool,
    replace: bool,
) -> Result<DryRunPlan, String> {
    let spec = parse_spec(&spec_yaml_or_json).map_err(safe_error)?;
    let enrollments = enrollment_store()?;
    service::plan(
        &spec,
        &source,
        &destination,
        &KeyringCredentialProvider,
        &enrollments,
        &KeyringMaskKeyStore,
        now_ms(),
        estimate,
        replace,
    )
    .await
    .map_err(safe_error)
}

pub async fn validate(
    plan: DryRunPlan,
    spec_yaml_or_json: String,
    source: DbConnection,
    destination: DbConnection,
) -> Result<ValidationReport, String> {
    let spec = parse_spec(&spec_yaml_or_json).map_err(safe_error)?;
    let enrollments = enrollment_store()?;
    service::validate(
        &plan,
        &spec,
        &source,
        &destination,
        &KeyringCredentialProvider,
        &enrollments,
        &KeyringMaskKeyStore,
        now_ms(),
    )
    .await
    .map_err(safe_error)
}

pub async fn execute(
    plan: DryRunPlan,
    spec_yaml_or_json: String,
    acknowledgement: String,
    replace: bool,
    source: DbConnection,
    destination: DbConnection,
) -> Result<RunReport, String> {
    let spec = parse_spec(&spec_yaml_or_json).map_err(safe_error)?;
    let enrollments = enrollment_store()?;
    let audit = audit_dir()?;
    service::execute(
        ExecuteRequest {
            plan: &plan,
            spec: &spec,
            acknowledgement: &acknowledgement,
            replace,
            source: &source,
            destination: &destination,
        },
        &KeyringCredentialProvider,
        &enrollments,
        &KeyringMaskKeyStore,
        Some(&audit),
        now_ms(),
        Arc::new(AtomicBool::new(false)),
    )
    .await
    .map_err(safe_error)
}

pub async fn status(run_id: String, destination: DbConnection) -> Result<RunStatus, String> {
    service::status(
        &run_id,
        &destination,
        &KeyringCredentialProvider,
        &Limits::default(),
    )
    .await
    .map_err(safe_error)
}

fn parse_spec(raw: &str) -> Result<SafeSyncSpec, CoreError> {
    let trimmed = raw.trim();
    if trimmed.starts_with('{') {
        SafeSyncSpec::parse_json(trimmed)
    } else {
        SafeSyncSpec::parse_yaml(trimmed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbpinball_core::domain::connection::{DbEngine, TargetClass};
    use dbpinball_core::safesync::key::{InMemoryMaskKeyStore, MaskKey};
    use dbpinball_core::safesync::mask::{Masker, MaskingPolicy};
    use dbpinball_core::safesync::plan::PlanIdentity;

    #[test]
    fn desktop_spec_parser_matches_core_yaml_and_json_contracts() {
        let yaml = "
version: 1
source: src
destination: dst
selection:
  roots:
    - table: mdb_src.orders
      filter:
        - column: id
          op: eq
          value: 1
  relationships:
    direction: parents
    depth: 1
masking:
  policy: development-default@2
";
        let parsed = parse_spec(yaml).unwrap();
        assert_eq!(parsed.source, "src");
        assert_eq!(parsed.selection.roots[0].table, "mdb_src.orders");

        let json = serde_json::to_string(&parsed).unwrap();
        let reparsed = parse_spec(&json).unwrap();
        assert_eq!(parsed.source, reparsed.source);
    }

    #[test]
    fn desktop_error_surfaces_detail_without_leaking_credentials() {
        let err = CoreError::with_detail(
            dbpinball_core::domain::error::ErrorCode::PolicyRefusal,
            "Safe Sync refused: destination is not enrolled",
            "destination_unenrolled",
        );
        let msg = safe_error(err);
        assert!(msg.contains("[destination_unenrolled]"));
        assert!(!msg.contains("password"));
    }

    #[test]
    fn desktop_and_cli_share_mariadb_plan_identity_engine_binding() {
        let session_id = SessionIdentity {
            engine: DbEngine::Mysql,
            identity_key: Some("mysql://h:3306/mdb_src#mariadb-fp:abc".into()),
            system_identifier: Some("mariadb-fp:abc".into()),
            database_oid: "mdb_src".into(),
            database_name: "mdb_src".into(),
            current_user: "ro@%".into(),
            backend_pid: 1,
            backend_start: String::new(),
            server_addr: "h".into(),
            server_port: 3306,
            transaction_read_only: true,
            host: "h".into(),
            port: 3306,
            server: dbpinball_core::security::ServerIdentity {
                engine: Some("mariadb".into()),
                ..Default::default()
            },
        };
        let plan_id = PlanIdentity::from_session(&session_id);
        assert_eq!(plan_id.engine, "mariadb");
        let _conn = DbConnection {
            id: "c1".into(),
            name: "c1".into(),
            engine: DbEngine::Mysql,
            host: "127.0.0.1".into(),
            port: 3306,
            database: "mdb_src".into(),
            username: "ro".into(),
            use_ssl: false,
            target: TargetClass::Production,
            ssh_tunnel: None,
        };
        let _store = InMemoryMaskKeyStore::with_key(MaskKey::generate().unwrap());
        let _masker =
            Masker::new(MaskingPolicy::builtin_v2(), MaskKey::generate().unwrap()).unwrap();
    }
}
