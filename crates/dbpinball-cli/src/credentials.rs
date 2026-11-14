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
    fn get(&self, connection_id: &str) -> Result<Secret, CoreError> {
        // A per-connection variable makes multi-profile automation possible
        // without putting a password on the command line. It falls back to the
        // shared variable, so existing scripts keep working.
        let suffix: String = connection_id
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() {
                    character.to_ascii_uppercase()
                } else {
                    '_'
                }
            })
            .collect();
        let per_connection = format!("DBPINBALL_PASSWORD_{suffix}");

        if let Ok(value) = std::env::var(&per_connection) {
            return Ok(Secret::new(value));
        }

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
