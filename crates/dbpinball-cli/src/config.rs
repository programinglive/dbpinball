use std::path::{Path, PathBuf};

use dbpinball_core::domain::error::{CoreError, ErrorCode};
use dbpinball_core::store::ConnectionStore;

/// Resolve the configuration directory.
///
/// Uses the operating system's per-user config directory unless the caller
/// supplied an override. The override exists for tests and advanced use.
pub fn resolve_config_dir(override_dir: Option<&Path>) -> Result<PathBuf, CoreError> {
    if let Some(path) = override_dir {
        return Ok(path.to_path_buf());
    }

    let base = dirs::config_dir().ok_or_else(|| {
        CoreError::new(
            ErrorCode::Operation,
            "the operating system did not provide a configuration directory",
        )
    })?;

    Ok(base.join("dbpinball"))
}

pub fn store(override_dir: Option<&Path>) -> Result<ConnectionStore, CoreError> {
    Ok(ConnectionStore::new(resolve_config_dir(override_dir)?))
}
