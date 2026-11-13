use serde::Serialize;
use thiserror::Error;

/// Stable, machine-readable error categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    Usage,
    Connection,
    Operation,
    NotFound,
    Unsupported,
}

/// The error type returned by every public core service.
#[derive(Debug, Clone, Serialize, Error)]
#[error("{message}")]
pub struct CoreError {
    pub code: ErrorCode,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl CoreError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            detail: None,
        }
    }

    pub fn with_detail(
        code: ErrorCode,
        message: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            code,
            message: message.into(),
            detail: Some(detail.into()),
        }
    }

    /// Build an error whose message has connection credentials removed.
    pub fn redacted(code: ErrorCode, message: impl Into<String>) -> Self {
        Self::new(code, redact(&message.into()))
    }
}

/// Remove `user:password@` from any URL-shaped text.
///
/// Connection strings are never formatted into an error on purpose, but a
/// database driver may echo one, so every message passes through here.
pub fn redact(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut rest = input;

    while let Some(scheme_end) = rest.find("://") {
        let (before, after) = rest.split_at(scheme_end + 3);
        output.push_str(before);

        let authority_end = after
            .find(|c: char| c == '/' || c.is_whitespace())
            .unwrap_or(after.len());
        let (authority, tail) = after.split_at(authority_end);

        match authority.rfind('@') {
            Some(at) => {
                let (userinfo, host) = authority.split_at(at);
                if userinfo.contains(':') {
                    output.push_str("***:***");
                } else {
                    output.push_str("***");
                }
                output.push_str(host);
            }
            None => output.push_str(authority),
        }

        rest = tail;
    }

    output.push_str(rest);
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redaction_removes_password() {
        let redacted = redact("postgresql://app:super-secret@127.0.0.1:5432/app");
        assert_eq!(redacted, "postgresql://***:***@127.0.0.1:5432/app");
        assert!(!redacted.contains("super-secret"));
    }

    #[test]
    fn redaction_handles_at_sign_in_password() {
        let redacted = redact("mysql://root:p@ss@127.0.0.1:3306/app");
        assert_eq!(redacted, "mysql://***:***@127.0.0.1:3306/app");
    }

    #[test]
    fn redaction_leaves_plain_text_untouched() {
        assert_eq!(redact("connection refused"), "connection refused");
    }

    #[test]
    fn redacted_error_never_contains_the_secret() {
        let error = CoreError::redacted(
            ErrorCode::Connection,
            "failed to connect to mysql://root:hunter2@localhost:3306/app",
        );

        assert!(!error.message.contains("hunter2"));
        assert!(error.message.contains("***:***"));
    }
}
