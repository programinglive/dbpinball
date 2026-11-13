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

    fn set(&self, connection_id: &str, secret: &Secret) -> Result<(), CoreError> {
        let entry = Self::entry(connection_id)?;

        entry
            .set_password(secret.expose())
            .map_err(|error| credential_write_error(&error))
    }

    fn delete(&self, connection_id: &str) -> Result<(), CoreError> {
        let entry = Self::entry(connection_id)?;

        match entry.delete_password() {
            Ok(()) => Ok(()),
            // A missing entry is not a failure: the caller is deleting the
            // profile and the desired end state is "no credential stored".
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(credential_write_error(&error)),
        }
    }
}

fn credential_write_error(error: &keyring::Error) -> CoreError {
    CoreError::new(
        ErrorCode::Operation,
        format!("credential could not be stored: {error}"),
    )
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
