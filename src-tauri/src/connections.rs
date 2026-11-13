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

pub fn create(
    mut connection: DbConnection,
    password: Option<String>,
) -> Result<DbConnection, String> {
    let store = store()?;

    // Core owns id generation. Assigning here (rather than inside `create`)
    // means the rollback path knows the id if the profile write fails.
    if connection.id.trim().is_empty() {
        connection.id = ConnectionStore::new_id();
    }

    let keyring = KeyringCredentialProvider;
    let id = connection.id.clone();

    // Store the credential first so a profile never exists without one.
    let stored = match password {
        Some(value) if !value.is_empty() => {
            keyring.set(&id, &Secret::new(value)).map_err(safe_error)?;
            true
        }
        _ => false,
    };

    match store.create(connection) {
        Ok(created) => Ok(created),
        Err(error) => {
            if stored {
                let _ = keyring.delete(&id);
            }
            Err(safe_error(error))
        }
    }
}

pub fn update(
    connection: DbConnection,
    password: Option<String>,
    expected_revision: Option<u64>,
) -> Result<(), String> {
    let store = store()?;
    store
        .update(connection.clone(), expected_revision)
        .map_err(safe_error)?;

    if let Some(value) = password {
        if !value.is_empty() {
            KeyringCredentialProvider
                .set(&connection.id, &Secret::new(value))
                .map_err(safe_error)?;
        }
    }

    Ok(())
}

pub fn delete(id: String, expected_revision: Option<u64>) -> Result<bool, String> {
    let store = store()?;

    // Delete the credential first: a failure keeps the profile so it never
    // looks removed while a credential is still stored.
    KeyringCredentialProvider.delete(&id).map_err(safe_error)?;

    store
        .remove_with_revision(&id, expected_revision)
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
