use std::fs;
use std::io::Write;
use std::path::PathBuf;

use crate::domain::connection::DbConnection;
use crate::domain::error::{CoreError, ErrorCode};
use crate::store::record::{
    ConnectionRecord, ImportReport, InvalidRecord, StoreFile, STORE_VERSION,
};

const FILE_NAME: &str = "connections.json";

/// A connection profile store rooted at a configuration directory.
///
/// The store takes the directory as a parameter so it stays independent of the
/// `dirs` crate and testable without touching the real user configuration.
pub struct ConnectionStore {
    root: PathBuf,
}

impl ConnectionStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
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

    pub fn save(&self, connections: &[ConnectionRecord]) -> Result<(), CoreError> {
        let mut file = self.load_file()?;
        file.connections = connections.to_vec();
        self.save_file(&file)
    }

    pub fn save_file(&self, file: &StoreFile) -> Result<(), CoreError> {
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

    pub fn get(&self, id: &str) -> Result<Option<ConnectionRecord>, CoreError> {
        Ok(self.load()?.into_iter().find(|record| record.id == id))
    }

    pub fn add(&self, connection: ConnectionRecord) -> Result<(), CoreError> {
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
        self.save_file(&file)
    }

    pub fn remove(&self, id: &str) -> Result<bool, CoreError> {
        let mut file = self.load_file()?;
        let before = file.connections.len();
        file.connections.retain(|record| record.id != id);

        let removed = file.connections.len() != before;

        if removed {
            self.save_file(&file)?;
        }

        Ok(removed)
    }

    pub fn import(&self, incoming: Vec<ConnectionRecord>) -> Result<ImportReport, CoreError> {
        let mut file = self.load_file()?;
        let mut skipped_invalid = Vec::new();
        let mut skipped_duplicates = 0;
        let mut imported = 0;

        for (index, connection) in incoming.into_iter().enumerate() {
            if let Err(reason) = validate(&connection) {
                skipped_invalid.push(InvalidRecord { index, reason });
                continue;
            }

            if file
                .connections
                .iter()
                .any(|existing| same_identity(existing, &connection))
            {
                skipped_duplicates += 1;
                continue;
            }

            file.connections.push(connection);
            imported += 1;
        }

        if imported > 0 {
            self.save_file(&file)?;
        }

        Ok(ImportReport {
            imported,
            skipped_invalid,
            skipped_duplicates,
        })
    }
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
        assert_eq!(store.load().unwrap().len(), 2);
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
            .save_file(&StoreFile {
                version: STORE_VERSION,
                migrated_from: Some("localStorage@v1".to_string()),
                connections: vec![connection("a", "app")],
            })
            .unwrap();

        store.add(connection("b", "other")).unwrap();

        let file = store.load_file().unwrap();
        assert_eq!(file.migrated_from.as_deref(), Some("localStorage@v1"));
    }
}
