use std::path::PathBuf;

use crate::domain::error::{CoreError, ErrorCode};

/// The per-user application directory shared by the desktop and the CLI.
pub const APP_DIR: &str = "dbpinball";

/// Resolve the shared configuration directory.
///
/// Both the desktop and the CLI call this, so they cannot drift into two
/// different stores. `dirs::config_dir()` is `%APPDATA%` on Windows,
/// `~/Library/Application Support` on macOS and `~/.config` on Linux.
pub fn default_config_dir() -> Result<PathBuf, CoreError> {
    let base = dirs::config_dir().ok_or_else(|| {
        CoreError::new(
            ErrorCode::Operation,
            "the operating system did not provide a configuration directory",
        )
    })?;

    Ok(base.join(APP_DIR))
}
