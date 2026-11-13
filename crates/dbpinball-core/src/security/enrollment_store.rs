//! Persistence for enrolled destination fingerprints.
//!
//! The registry lives next to the connection store, in
//! `<config>/dbpinball/security/enrollments.json`. It contains identity keys and
//! categories only: no password, no token, no full connection string.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::domain::error::{CoreError, ErrorCode};
use crate::security::destination::EnrollmentRegistry;

const DIRECTORY: &str = "security";
const FILE_NAME: &str = "enrollments.json";
const VERSION: u32 = 1;

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct RegistryFile {
    version: u32,
    #[serde(default)]
    enrollments: EnrollmentRegistry,
}

/// A registry rooted at a configuration directory.
pub struct EnrollmentStore {
    root: PathBuf,
}

impl EnrollmentStore {
    pub fn new(config_dir: impl Into<PathBuf>) -> Self {
        Self {
            root: config_dir.into(),
        }
    }

    pub fn path(&self) -> PathBuf {
        self.root.join(DIRECTORY).join(FILE_NAME)
    }

    pub fn load(&self) -> Result<EnrollmentRegistry, CoreError> {
        let path = self.path();
        if !path.exists() {
            return Ok(EnrollmentRegistry::default());
        }

        let contents = fs::read_to_string(&path).map_err(|error| {
            CoreError::new(
                ErrorCode::Operation,
                format!("enrollment registry could not be read: {error}"),
            )
        })?;

        let file: RegistryFile = serde_json::from_str(&contents).map_err(|error| {
            CoreError::new(
                ErrorCode::Operation,
                format!("enrollment registry is not valid JSON: {error}"),
            )
        })?;

        if file.version != VERSION {
            return Err(CoreError::new(
                ErrorCode::Unsupported,
                format!(
                    "enrollment registry version {} is not supported (expected {VERSION})",
                    file.version
                ),
            ));
        }

        Ok(file.enrollments)
    }

    pub fn save(&self, registry: &EnrollmentRegistry) -> Result<(), CoreError> {
        let path = self.path();
        let directory = path
            .parent()
            .ok_or_else(|| CoreError::new(ErrorCode::Operation, "enrollment path has no parent"))?;

        fs::create_dir_all(directory).map_err(|error| {
            CoreError::new(
                ErrorCode::Operation,
                format!("enrollment directory could not be created: {error}"),
            )
        })?;

        let file = RegistryFile {
            version: VERSION,
            enrollments: registry.clone(),
        };

        let json = serde_json::to_vec_pretty(&file).map_err(|error| {
            CoreError::new(
                ErrorCode::Operation,
                format!("enrollment registry could not be serialized: {error}"),
            )
        })?;

        let temporary = directory.join(format!(
            ".enrollments-{}-{}.tmp",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or(0)
        ));

        {
            let mut handle = fs::File::create(&temporary).map_err(|error| {
                CoreError::new(
                    ErrorCode::Operation,
                    format!("enrollment registry could not be created: {error}"),
                )
            })?;
            handle.write_all(&json).map_err(|error| {
                let _ = fs::remove_file(&temporary);
                CoreError::new(
                    ErrorCode::Operation,
                    format!("enrollment registry could not be written: {error}"),
                )
            })?;
            handle.flush().map_err(|error| {
                let _ = fs::remove_file(&temporary);
                CoreError::new(
                    ErrorCode::Operation,
                    format!("enrollment registry could not be flushed: {error}"),
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
                format!("enrollment registry could not be finalized: {error}"),
            )
        })?;

        Ok(())
    }

    /// Remove an enrollment. Returns true when one existed.
    pub fn remove(&self, connection_id: &str) -> Result<bool, CoreError> {
        let mut registry = self.load()?;
        let removed = registry.remove(connection_id);
        if removed {
            self.save(&registry)?;
        }
        Ok(removed)
    }
}

/// Resolve the enrollment store for a configuration directory.
pub fn store_for(config_dir: &Path) -> EnrollmentStore {
    EnrollmentStore::new(config_dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::security::destination::{DestinationCategory, EnrolledFingerprint};

    fn fingerprint(key: &str) -> EnrolledFingerprint {
        EnrolledFingerprint {
            identity_key: key.into(),
            engine: "postgres".into(),
            database: "dev".into(),
            category: DestinationCategory::VerifiedDevelopment,
            enrolled_at_ms: 1,
            note: None,
        }
    }

    #[test]
    fn a_missing_registry_loads_empty() {
        let directory = tempfile::tempdir().unwrap();
        let store = EnrollmentStore::new(directory.path());
        assert!(store.load().unwrap().entries.is_empty());
    }

    #[test]
    fn an_enrollment_round_trips() {
        let directory = tempfile::tempdir().unwrap();
        let store = EnrollmentStore::new(directory.path());

        let mut registry = EnrollmentRegistry::default();
        registry.enroll("conn-1", fingerprint("key-1"));
        store.save(&registry).unwrap();

        let loaded = store.load().unwrap();
        assert_eq!(
            loaded.get("conn-1").unwrap().identity_key,
            "key-1".to_string()
        );
    }

    #[test]
    fn removal_is_persisted() {
        let directory = tempfile::tempdir().unwrap();
        let store = EnrollmentStore::new(directory.path());

        let mut registry = EnrollmentRegistry::default();
        registry.enroll("conn-1", fingerprint("key-1"));
        store.save(&registry).unwrap();

        assert!(store.remove("conn-1").unwrap());
        assert!(store.load().unwrap().get("conn-1").is_none());
    }

    #[test]
    fn the_registry_contains_no_secret() {
        let directory = tempfile::tempdir().unwrap();
        let store = EnrollmentStore::new(directory.path());

        let mut registry = EnrollmentRegistry::default();
        registry.enroll("conn-1", fingerprint("key-1"));
        store.save(&registry).unwrap();

        let contents = std::fs::read_to_string(store.path()).unwrap();
        assert!(!contents.to_ascii_lowercase().contains("password"));
    }
}
