use std::fs;
use std::io::Write;
use std::path::PathBuf;

use serde::Serialize;

use crate::domain::connection::{DbConnection, TargetClass};
use crate::domain::error::{CoreError, ErrorCode};
use crate::store::lock;
use crate::store::record::{
    ConflictRecord, ConnectionRecord, ImportReport, InvalidRecord, StoreFile, STORE_VERSION,
};

const FILE_NAME: &str = "connections.json";

/// A connection profile store rooted at a configuration directory.
///
/// The store takes the directory as a parameter so it stays independent of the
/// `dirs` crate and testable without touching the real user configuration.
///
/// Every mutation takes a cross-process lock, re-reads the file inside it, and
/// bumps a monotonic revision. A caller that supplies an expected revision and
/// finds a different one gets [`ErrorCode::Conflict`] instead of overwriting
/// another writer's change.
pub struct ConnectionStore {
    root: PathBuf,
}

/// A read-only snapshot of the store's migration state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationStatus {
    pub migrated_from: Option<String>,
    pub revision: u64,
    pub profiles: usize,
}

impl ConnectionStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// A collision-resistant profile id.
    ///
    /// Core owns id generation so the desktop and the CLI cannot produce
    /// different id shapes.
    pub fn new_id() -> String {
        uuid::Uuid::new_v4().to_string()
    }

    pub fn path(&self) -> PathBuf {
        self.root.join(FILE_NAME)
    }

    pub fn load_file(&self) -> Result<StoreFile, CoreError> {
        let path = self.path();

        if !path.exists() {
            return Ok(StoreFile::default());
        }

        let contents = fs::read_to_string(&path).map_err(|error| {
            CoreError::new(
                ErrorCode::Operation,
                format!("connection store could not be read: {error}"),
            )
        })?;

        let file: StoreFile = serde_json::from_str(&contents).map_err(|error| {
            CoreError::new(
                ErrorCode::Operation,
                format!("connection store is not valid JSON: {error}"),
            )
        })?;

        if file.version != STORE_VERSION {
            return Err(CoreError::new(
                ErrorCode::Unsupported,
                format!(
                    "connection store version {} is not supported (expected {})",
                    file.version, STORE_VERSION
                ),
            ));
        }

        Ok(file)
    }

    pub fn load(&self) -> Result<Vec<ConnectionRecord>, CoreError> {
        Ok(self.load_file()?.connections)
    }

    pub fn get(&self, id: &str) -> Result<Option<ConnectionRecord>, CoreError> {
        Ok(self.load()?.into_iter().find(|record| record.id == id))
    }

    pub fn migration_status(&self) -> Result<MigrationStatus, CoreError> {
        let file = self.load_file()?;
        Ok(MigrationStatus {
            migrated_from: file.migrated_from,
            revision: file.revision,
            profiles: file.connections.len(),
        })
    }

    /// Replace the whole profile list.
    pub fn save(&self, connections: &[ConnectionRecord]) -> Result<(), CoreError> {
        let _guard = lock::acquire(&self.root)?;
        let mut file = self.load_file()?;
        file.connections = connections.to_vec();
        self.write_locked(&mut file)
    }

    /// Write a file the caller already built, refusing a stale revision.
    pub fn save_file_if_revision(
        &self,
        file: &StoreFile,
        expected_revision: u64,
    ) -> Result<(), CoreError> {
        let _guard = lock::acquire(&self.root)?;
        let current = self.load_file()?;

        if current.revision != expected_revision {
            return Err(revision_conflict(current.revision, expected_revision));
        }

        let mut next = file.clone();
        next.revision = current.revision;
        self.write_locked(&mut next)
    }

    /// Add a profile, assigning a collision-resistant id when none is supplied.
    pub fn create(&self, mut connection: ConnectionRecord) -> Result<ConnectionRecord, CoreError> {
        if connection.id.trim().is_empty() {
            connection.id = uuid::Uuid::new_v4().to_string();
        }

        validate(&connection).map_err(|reason| CoreError::new(ErrorCode::Usage, reason))?;

        let _guard = lock::acquire(&self.root)?;
        let mut file = self.load_file()?;

        if file
            .connections
            .iter()
            .any(|existing| existing.id == connection.id)
        {
            return Err(CoreError::new(
                ErrorCode::Usage,
                "a connection with this id already exists",
            ));
        }

        file.connections.push(connection.clone());
        self.write_locked(&mut file)?;

        Ok(connection)
    }

    /// Add a profile with a caller-supplied id.
    pub fn add(&self, connection: ConnectionRecord) -> Result<(), CoreError> {
        let _guard = lock::acquire(&self.root)?;
        let mut file = self.load_file()?;

        if file
            .connections
            .iter()
            .any(|existing| existing.id == connection.id)
        {
            return Err(CoreError::new(
                ErrorCode::Usage,
                "a connection with this id already exists",
            ));
        }

        file.connections.push(connection);
        self.write_locked(&mut file)
    }

    /// Replace an existing profile, optionally refusing a stale revision.
    pub fn update(
        &self,
        connection: ConnectionRecord,
        expected_revision: Option<u64>,
    ) -> Result<(), CoreError> {
        validate(&connection).map_err(|reason| CoreError::new(ErrorCode::Usage, reason))?;

        let _guard = lock::acquire(&self.root)?;
        let mut file = self.load_file()?;

        if let Some(expected) = expected_revision {
            if file.revision != expected {
                return Err(revision_conflict(file.revision, expected));
            }
        }

        let position = file
            .connections
            .iter()
            .position(|existing| existing.id == connection.id)
            .ok_or_else(|| {
                CoreError::new(
                    ErrorCode::NotFound,
                    format!("connection '{}' was not found", connection.id),
                )
            })?;

        file.connections[position] = connection;
        self.write_locked(&mut file)
    }

    /// Remove a profile, optionally refusing a stale revision.
    pub fn remove_with_revision(
        &self,
        id: &str,
        expected_revision: Option<u64>,
    ) -> Result<bool, CoreError> {
        let _guard = lock::acquire(&self.root)?;
        let mut file = self.load_file()?;

        if let Some(expected) = expected_revision {
            if file.revision != expected {
                return Err(revision_conflict(file.revision, expected));
            }
        }

        let before = file.connections.len();
        file.connections.retain(|record| record.id != id);
        let removed = file.connections.len() != before;

        if removed {
            self.write_locked(&mut file)?;
        }

        Ok(removed)
    }

    pub fn remove(&self, id: &str) -> Result<bool, CoreError> {
        self.remove_with_revision(id, None)
    }

    /// Import profiles without changing provenance.
    pub fn import(&self, incoming: Vec<ConnectionRecord>) -> Result<ImportReport, CoreError> {
        self.import_inner(incoming, None)
    }

    /// Import profiles and record where they came from.
    ///
    /// The provenance marker is written only after the store write succeeds, so
    /// a failed migration never looks complete.
    pub fn import_with_provenance(
        &self,
        incoming: Vec<ConnectionRecord>,
        migrated_from: &str,
    ) -> Result<ImportReport, CoreError> {
        self.import_inner(incoming, Some(migrated_from))
    }

    fn import_inner(
        &self,
        incoming: Vec<ConnectionRecord>,
        migrated_from: Option<&str>,
    ) -> Result<ImportReport, CoreError> {
        let _guard = lock::acquire(&self.root)?;
        let mut file = self.load_file()?;
        let marker_missing = file.migrated_from.is_none();

        let mut report = ImportReport::empty();

        for (index, mut connection) in incoming.into_iter().enumerate() {
            if let Err(reason) = validate(&connection) {
                report.skipped_invalid.push(InvalidRecord { index, reason });
                continue;
            }

            // Imported profiles are never trusted as a target classification.
            connection.target = TargetClass::Unknown;

            if let Some(existing) = file
                .connections
                .iter()
                .find(|existing| existing.id == connection.id)
            {
                if same_identity(existing, &connection) {
                    report.skipped_duplicates += 1;
                } else {
                    report.skipped_conflicts.push(ConflictRecord {
                        index,
                        id: connection.id.clone(),
                        reason: "id already exists with different connection details".to_string(),
                    });
                }
                continue;
            }

            if file
                .connections
                .iter()
                .any(|existing| same_identity(existing, &connection))
            {
                report.skipped_duplicates += 1;
                continue;
            }

            file.connections.push(connection);
            report.imported += 1;
        }

        if report.imported > 0 || (marker_missing && migrated_from.is_some()) {
            if let Some(source) = migrated_from {
                if file.migrated_from.is_none() {
                    file.migrated_from = Some(source.to_string());
                }
            }
            self.write_locked(&mut file)?;
        }

        Ok(report)
    }

    /// Write the file atomically and bump the revision.
    ///
    /// The caller must already hold the lock.
    fn write_locked(&self, file: &mut StoreFile) -> Result<(), CoreError> {
        file.version = STORE_VERSION;
        file.revision = file.revision.saturating_add(1);
        self.write_atomic(file)
    }

    fn write_atomic(&self, file: &StoreFile) -> Result<(), CoreError> {
        fs::create_dir_all(&self.root).map_err(|error| {
            CoreError::new(
                ErrorCode::Operation,
                format!("configuration directory could not be created: {error}"),
            )
        })?;

        let path = self.path();
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let temporary = self
            .root
            .join(format!(".connections-{}-{unique}.tmp", std::process::id()));

        let json = serde_json::to_vec_pretty(file).map_err(|error| {
            CoreError::new(
                ErrorCode::Operation,
                format!("connection store could not be serialized: {error}"),
            )
        })?;

        {
            let mut handle = fs::File::create(&temporary).map_err(|error| {
                CoreError::new(
                    ErrorCode::Operation,
                    format!("connection store could not be created: {error}"),
                )
            })?;

            handle.write_all(&json).map_err(|error| {
                let _ = fs::remove_file(&temporary);
                CoreError::new(
                    ErrorCode::Operation,
                    format!("connection store could not be written: {error}"),
                )
            })?;

            handle.flush().map_err(|error| {
                let _ = fs::remove_file(&temporary);
                CoreError::new(
                    ErrorCode::Operation,
                    format!("connection store could not be flushed: {error}"),
                )
            })?;
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600));
        }

        fs::rename(&temporary, &path).map_err(|error| {
            let _ = fs::remove_file(&temporary);
            CoreError::new(
                ErrorCode::Operation,
                format!("connection store could not be finalized: {error}"),
            )
        })?;

        Ok(())
    }
}

fn revision_conflict(actual: u64, expected: u64) -> CoreError {
    CoreError::new(
        ErrorCode::Conflict,
        format!("the connection store changed since it was read (revision {actual}, expected {expected})"),
    )
}

fn validate(connection: &DbConnection) -> Result<(), String> {
    if connection.id.trim().is_empty() {
        return Err("id is empty".to_string());
    }
    if connection.name.trim().is_empty() {
        return Err("name is empty".to_string());
    }
    if connection.host.trim().is_empty() {
        return Err("host is empty".to_string());
    }
    if connection.database.trim().is_empty() {
        return Err("database is empty".to_string());
    }
    if connection.username.trim().is_empty() {
        return Err("username is empty".to_string());
    }
    if connection.port == 0 {
        return Err("port must be between 1 and 65535".to_string());
    }

    Ok(())
}

fn same_identity(left: &DbConnection, right: &DbConnection) -> bool {
    left.engine == right.engine
        && left.host == right.host
        && left.port == right.port
        && left.database == right.database
        && left.username == right.username
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::connection::{DbEngine, TargetClass};

    fn connection(id: &str, database: &str) -> DbConnection {
        DbConnection {
            id: id.to_string(),
            name: format!("profile-{id}"),
            engine: DbEngine::Postgres,
            host: "127.0.0.1".to_string(),
            port: 5432,
            database: database.to_string(),
            username: "postgres".to_string(),
            use_ssl: false,
            target: TargetClass::Local,
        }
    }

    #[test]
    fn missing_file_loads_as_empty_store() {
        let directory = tempfile::tempdir().unwrap();
        let store = ConnectionStore::new(directory.path());

        assert!(store.load().unwrap().is_empty());
        assert_eq!(store.load_file().unwrap().version, STORE_VERSION);
        assert_eq!(store.load_file().unwrap().revision, 0);
    }

    #[test]
    fn save_and_load_round_trip() {
        let directory = tempfile::tempdir().unwrap();
        let store = ConnectionStore::new(directory.path());

        store.save(&[connection("a", "app")]).unwrap();

        let loaded = store.load().unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, "a");
        assert_eq!(loaded[0].target, TargetClass::Local);
    }

    #[test]
    fn serialized_store_never_contains_a_password() {
        let directory = tempfile::tempdir().unwrap();
        let store = ConnectionStore::new(directory.path());

        store.save(&[connection("a", "app")]).unwrap();

        let contents = std::fs::read_to_string(store.path()).unwrap();
        assert!(!contents.contains("password"));
    }

    #[test]
    fn atomic_write_leaves_no_temporary_file() {
        let directory = tempfile::tempdir().unwrap();
        let store = ConnectionStore::new(directory.path());

        store.save(&[connection("a", "app")]).unwrap();

        let leftovers: Vec<_> = std::fs::read_dir(directory.path())
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();

        assert!(leftovers.is_empty());
    }

    #[test]
    fn add_and_remove_are_reflected() {
        let directory = tempfile::tempdir().unwrap();
        let store = ConnectionStore::new(directory.path());

        store.add(connection("a", "app")).unwrap();
        assert!(store.get("a").unwrap().is_some());
        assert!(store.remove("a").unwrap());
        assert!(store.get("a").unwrap().is_none());
    }

    #[test]
    fn duplicate_id_is_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let store = ConnectionStore::new(directory.path());

        store.add(connection("a", "app")).unwrap();
        let error = store.add(connection("a", "other")).unwrap_err();

        assert_eq!(error.code, ErrorCode::Usage);
    }

    #[test]
    fn create_assigns_a_uuid_when_the_id_is_empty() {
        let directory = tempfile::tempdir().unwrap();
        let store = ConnectionStore::new(directory.path());

        let mut incoming = connection("", "app");
        incoming.id = String::new();
        let created = store.create(incoming).unwrap();

        assert_eq!(created.id.len(), 36);
        assert!(store.get(&created.id).unwrap().is_some());
    }

    #[test]
    fn create_preserves_a_valid_legacy_id() {
        let directory = tempfile::tempdir().unwrap();
        let store = ConnectionStore::new(directory.path());

        let created = store
            .create(connection("conn_1700000000000", "app"))
            .unwrap();

        assert_eq!(created.id, "conn_1700000000000");
    }

    #[test]
    fn import_skips_invalid_and_duplicate_records() {
        let directory = tempfile::tempdir().unwrap();
        let store = ConnectionStore::new(directory.path());

        store.add(connection("existing", "existing_app")).unwrap();

        let report = store
            .import(vec![
                connection("new", "new_app"),
                DbConnection {
                    host: String::new(),
                    ..connection("broken", "new_app")
                },
                connection("duplicate", "new_app"),
            ])
            .unwrap();

        assert_eq!(report.imported, 1);
        assert_eq!(report.skipped_invalid.len(), 1);
        assert_eq!(report.skipped_invalid[0].index, 1);
        assert_eq!(report.skipped_duplicates, 1);
        assert!(report.skipped_conflicts.is_empty());
        assert_eq!(store.load().unwrap().len(), 2);
    }

    #[test]
    fn import_reports_a_conflicting_id_without_overwriting() {
        let directory = tempfile::tempdir().unwrap();
        let store = ConnectionStore::new(directory.path());

        store.add(connection("shared", "original")).unwrap();

        let report = store
            .import(vec![connection("shared", "different")])
            .unwrap();

        assert_eq!(report.imported, 0);
        assert_eq!(report.skipped_conflicts.len(), 1);
        assert_eq!(report.skipped_conflicts[0].id, "shared");
        assert_eq!(store.get("shared").unwrap().unwrap().database, "original");
    }

    #[test]
    fn import_sets_provenance_and_unknown_target() {
        let directory = tempfile::tempdir().unwrap();
        let store = ConnectionStore::new(directory.path());

        let report = store
            .import_with_provenance(vec![connection("legacy", "app")], "localStorage@v1")
            .unwrap();

        assert_eq!(report.imported, 1);
        let file = store.load_file().unwrap();
        assert_eq!(file.migrated_from.as_deref(), Some("localStorage@v1"));
        assert_eq!(file.connections[0].target, TargetClass::Unknown);
    }

    #[test]
    fn repeated_migration_is_idempotent() {
        let directory = tempfile::tempdir().unwrap();
        let store = ConnectionStore::new(directory.path());

        let first = store
            .import_with_provenance(vec![connection("legacy", "app")], "localStorage@v1")
            .unwrap();
        let second = store
            .import_with_provenance(vec![connection("legacy", "app")], "localStorage@v1")
            .unwrap();

        assert_eq!(first.imported, 1);
        assert_eq!(second.imported, 0);
        assert_eq!(second.skipped_duplicates, 1);
        assert_eq!(store.load().unwrap().len(), 1);
    }

    #[test]
    fn revision_increments_on_every_write() {
        let directory = tempfile::tempdir().unwrap();
        let store = ConnectionStore::new(directory.path());

        store.add(connection("a", "app")).unwrap();
        let first = store.load_file().unwrap().revision;
        store.add(connection("b", "app")).unwrap();
        let second = store.load_file().unwrap().revision;

        assert_eq!(second, first + 1);
    }

    #[test]
    fn a_stale_revision_is_a_conflict() {
        let directory = tempfile::tempdir().unwrap();
        let store = ConnectionStore::new(directory.path());

        store.add(connection("a", "app")).unwrap();
        let stale = store.load_file().unwrap().revision;
        store.add(connection("b", "app")).unwrap();

        let error = store
            .update(connection("a", "renamed"), Some(stale))
            .unwrap_err();

        assert_eq!(error.code, ErrorCode::Conflict);
    }

    #[test]
    fn migration_status_reports_provenance_and_revision() {
        let directory = tempfile::tempdir().unwrap();
        let store = ConnectionStore::new(directory.path());

        store
            .import_with_provenance(vec![connection("legacy", "app")], "localStorage@v1")
            .unwrap();

        let status = store.migration_status().unwrap();
        assert_eq!(status.migrated_from.as_deref(), Some("localStorage@v1"));
        assert_eq!(status.profiles, 1);
        assert!(status.revision >= 1);
    }

    #[test]
    fn invalid_json_is_an_operation_error() {
        let directory = tempfile::tempdir().unwrap();
        let store = ConnectionStore::new(directory.path());
        std::fs::write(store.path(), "{ not json").unwrap();

        let error = store.load().unwrap_err();
        assert_eq!(error.code, ErrorCode::Operation);
    }

    #[test]
    fn unsupported_version_is_refused() {
        let directory = tempfile::tempdir().unwrap();
        let store = ConnectionStore::new(directory.path());
        std::fs::write(store.path(), r#"{"version":99,"connections":[]}"#).unwrap();

        let error = store.load().unwrap_err();
        assert_eq!(error.code, ErrorCode::Unsupported);
    }

    #[test]
    fn migration_marker_is_preserved_on_save() {
        let directory = tempfile::tempdir().unwrap();
        let store = ConnectionStore::new(directory.path());

        store
            .save_file_if_revision(
                &StoreFile {
                    version: STORE_VERSION,
                    migrated_from: Some("localStorage@v1".to_string()),
                    revision: 0,
                    connections: vec![connection("a", "app")],
                },
                0,
            )
            .unwrap();

        store.add(connection("b", "other")).unwrap();

        let file = store.load_file().unwrap();
        assert_eq!(file.migrated_from.as_deref(), Some("localStorage@v1"));
    }
}
