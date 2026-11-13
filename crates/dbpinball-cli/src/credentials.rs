use dbpinball_core::adapters::keyring::KeyringCredentialProvider;
use dbpinball_core::domain::error::{CoreError, ErrorCode};
use dbpinball_core::ports::credentials::{CredentialProvider, Secret};

use crate::cli::CredentialSource;

/// Reads a password from `DBPINBALL_PASSWORD`.
///
/// It is only constructed when the caller explicitly passes
/// `--credential-source env`. It cannot store or delete credentials.
pub struct EnvCredentialProvider;

impl CredentialProvider for EnvCredentialProvider {
    fn get(&self, _connection_id: &str) -> Result<Secret, CoreError> {
        let value = std::env::var("DBPINBALL_PASSWORD").map_err(|_| {
            CoreError::new(
                ErrorCode::Connection,
                "DBPINBALL_PASSWORD is not set for the environment credential source",
            )
        })?;

        Ok(Secret::new(value))
    }

    fn set(&self, _connection_id: &str, _secret: &Secret) -> Result<(), CoreError> {
        Err(CoreError::new(
            ErrorCode::Usage,
            "the environment credential source cannot store passwords",
        ))
    }

    fn delete(&self, _connection_id: &str) -> Result<(), CoreError> {
        Err(CoreError::new(
            ErrorCode::Usage,
            "the environment credential source cannot delete passwords",
        ))
    }
}

/// Build the credential provider selected on the command line.
pub fn provider(source: CredentialSource) -> Box<dyn CredentialProvider> {
    match source {
        CredentialSource::Keyring => Box::new(KeyringCredentialProvider),
        CredentialSource::Env => Box::new(EnvCredentialProvider),
    }
}
