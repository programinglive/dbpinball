use dbpinball_core::domain::error::{CoreError, ErrorCode};

/// Map a core error to the documented process exit code.
pub fn exit_code(error: &CoreError) -> i32 {
    match error.code {
        ErrorCode::Usage => 2,
        ErrorCode::Connection => 3,
        ErrorCode::PolicyRefusal => 4,
        ErrorCode::Operation | ErrorCode::Unsupported => 5,
        ErrorCode::NotFound => 7,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn error(code: ErrorCode) -> CoreError {
        CoreError::new(code, "test")
    }

    #[test]
    fn maps_documented_codes() {
        assert_eq!(exit_code(&error(ErrorCode::Usage)), 2);
        assert_eq!(exit_code(&error(ErrorCode::Connection)), 3);
        assert_eq!(exit_code(&error(ErrorCode::PolicyRefusal)), 4);
        assert_eq!(exit_code(&error(ErrorCode::Operation)), 5);
        assert_eq!(exit_code(&error(ErrorCode::Unsupported)), 5);
        assert_eq!(exit_code(&error(ErrorCode::NotFound)), 7);
    }
}
