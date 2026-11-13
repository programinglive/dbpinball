use std::path::{Path, PathBuf};

use dbpinball_core::domain::error::CoreError;
use dbpinball_core::store::{default_config_dir, ConnectionStore};

/// Resolve the configuration directory.
///
/// The default comes from Core so the desktop and the CLI cannot drift into two
/// different stores. The override exists for tests and advanced use.
pub fn resolve_config_dir(override_dir: Option<&Path>) -> Result<PathBuf, CoreError> {
    if let Some(path) = override_dir {
        return Ok(path.to_path_buf());
    }

    default_config_dir()
}

pub fn store(override_dir: Option<&Path>) -> Result<ConnectionStore, CoreError> {
    Ok(ConnectionStore::new(resolve_config_dir(override_dir)?))
}
