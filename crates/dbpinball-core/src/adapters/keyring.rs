use crate::domain::error::{CoreError, ErrorCode};
use crate::ports::credentials::{CredentialProvider, Secret};

/// The service name used for every entry. It must match the name the desktop
/// application has always used so existing keyring entries keep working.
const SERVICE_NAME: &str = "dbpinball";

/// Credentials stored in the operating system keyring.
pub struct KeyringCredentialProvider;

impl KeyringCredentialProvider {
    fn entry(connection_id: &str) -> Result<keyring::Entry, CoreError> {
        keyring::Entry::new(SERVICE_NAME, connection_id).map_err(|error| {
            CoreError::new(
                ErrorCode::Connection,
                format!("operating system keyring is unavailable: {error}"),
            )
        })
    }
}

impl CredentialProvider for KeyringCredentialProvider {
    fn get(&self, connection_id: &str) -> Result<Secret, CoreError> {
        let entry = Self::entry(connection_id)?;

        entry.get_password().map(Secret::new).map_err(|error| {
            CoreError::new(
                ErrorCode::Connection,
                format!("credential could not be read for this connection: {error}"),
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_credential_returns_a_connection_error() {
        let provider = KeyringCredentialProvider;
        let result = provider.get("dbpinball-test-nonexistent-connection-id");

        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, ErrorCode::Connection);
    }
}
