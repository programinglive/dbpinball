//! Thin Tauri adapter over the shared Core connection store.
//!
//! No validation, deduplication or persistence logic lives here: every rule is
//! in `dbpinball-core`, so the desktop and the CLI cannot drift. This module
//! only resolves the shared directory, calls Core, and maps errors to a safe
//! message that never contains a credential or a filesystem path.

use dbpinball_core::adapters::keyring::KeyringCredentialProvider;
use dbpinball_core::domain::connection::DbConnection;
use dbpinball_core::domain::error::{CoreError, ErrorCode};
use dbpinball_core::ports::credentials::{CredentialProvider, Secret};
use dbpinball_core::store::{default_config_dir, ConnectionStore, ImportReport, MigrationStatus};

fn store() -> Result<ConnectionStore, String> {
    let directory = default_config_dir().map_err(safe_error)?;
    Ok(ConnectionStore::new(directory))
}

/// Map a Core error to a user-facing message.
///
/// The Core message is deliberately not forwarded: it can contain a filesystem
/// path or an OS error string. The code is stable, so the UI can branch on it.
fn safe_error(error: CoreError) -> String {
    match error.code {
        ErrorCode::Conflict => {
            "The connection list changed in another window. Reload and try again.".to_string()
        }
        ErrorCode::Busy => "The connection store is busy. Try again in a moment.".to_string(),
        ErrorCode::NotFound => "That connection no longer exists.".to_string(),
        ErrorCode::Usage => error.message,
        ErrorCode::Connection => {
            "The operating system keyring is unavailable for this connection.".to_string()
        }
        ErrorCode::Unsupported => {
            "This connection store version is not supported by this build.".to_string()
        }
        ErrorCode::PolicyRefusal => error.message,
        ErrorCode::Operation => "The connection store could not be updated.".to_string(),
    }
}

pub fn list() -> Result<Vec<DbConnection>, String> {
    store()?.load().map_err(safe_error)
}

pub fn create(connection: DbConnection, password: Option<String>) -> Result<DbConnection, String> {
    let store = store()?;
    let keyring = KeyringCredentialProvider;
    let secret = password.filter(|value| !value.is_empty());

    // The duplicate-id check, the credential write and the metadata write are
    // one logical mutation under the store lock. A concurrent create cannot
    // slip the same id in between, so this can never overwrite (and then
    // delete) another profile's credential.
    store
        .create_with_credential(connection, |id| match &secret {
            Some(value) => keyring.set(id, &Secret::new(value.clone())),
            None => Ok(()),
        })
        .map_err(safe_error)
}

pub fn update(
    connection: DbConnection,
    password: Option<String>,
    expected_revision: Option<u64>,
) -> Result<(), String> {
    let store = store()?;
    let keyring = KeyringCredentialProvider;
    let secret = password.filter(|value| !value.is_empty());

    // The revision check, the credential write and the metadata write are one
    // logical mutation under the store lock, so a concurrent delete cannot
    // remove the profile between the check and the credential write.
    store
        .update_with_credential(connection, expected_revision, |id| match &secret {
            Some(value) => keyring.set(id, &Secret::new(value.clone())),
            None => Ok(()),
        })
        .map_err(safe_error)
}

pub fn delete(id: String, expected_revision: Option<u64>) -> Result<bool, String> {
    let store = store()?;

    // The revision check, the credential cleanup and the metadata write are one
    // logical mutation under the store lock. A stale revision fails before the
    // keyring is touched, so it can never delete the credential of a profile
    // that stays active.
    store
        .remove_with_revision_and_cleanup(&id, expected_revision, || {
            KeyringCredentialProvider.delete(&id)
        })
        .map_err(safe_error)
}

pub fn import(records: Vec<DbConnection>) -> Result<ImportReport, String> {
    store()?.import(records).map_err(safe_error)
}

pub fn migrate_local_storage(records: Vec<DbConnection>) -> Result<ImportReport, String> {
    store()?
        .import_with_provenance(records, "localStorage@v1")
        .map_err(safe_error)
}

pub fn migration_status() -> Result<MigrationStatus, String> {
    store()?.migration_status().map_err(safe_error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_error_never_leaks_a_credential() {
        let error = CoreError::new(
            ErrorCode::Connection,
            "keyring failed for postgres://user:hunter2@host/db",
        );

        let message = safe_error(error);
        assert!(!message.contains("hunter2"));
        assert!(!message.contains("postgres://"));
    }

    #[test]
    fn safe_error_does_not_leak_a_filesystem_path() {
        let error = CoreError::new(
            ErrorCode::Operation,
            "connection store could not be created: C:\\Users\\someone\\AppData",
        );

        let message = safe_error(error);
        assert!(!message.contains("AppData"));
    }
}
