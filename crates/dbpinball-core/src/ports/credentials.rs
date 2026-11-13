use crate::domain::error::CoreError;

/// A credential provider resolves database passwords for a connection id.
///
/// The desktop application implements this over the operating system keyring.
/// A future CLI may supply a different implementation; services never read a
/// secret from configuration or the environment directly.
pub trait CredentialProvider: Send + Sync {
    fn get(&self, connection_id: &str) -> Result<Secret, CoreError>;
    fn set(&self, connection_id: &str, secret: &Secret) -> Result<(), CoreError>;
    fn delete(&self, connection_id: &str) -> Result<(), CoreError>;
}

/// A secret value that never renders itself.
///
/// `Debug` is implemented manually so a password cannot reach a log line or an
/// error message through an accidental `{:?}`.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Borrow the secret. Call only when building the credential handoff.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Secret(***)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_debug_is_redacted() {
        let secret = Secret::new("hunter2");
        let rendered = format!("{:?}", secret);

        assert_eq!(rendered, "Secret(***)");
        assert!(!rendered.contains("hunter2"));
    }

    #[test]
    fn secret_exposes_value_only_when_asked() {
        let secret = Secret::new("hunter2");
        assert_eq!(secret.expose(), "hunter2");
    }
}
