use dbpinball_core::domain::error::CoreError;
use serde::Serialize;
use serde_json::json;

use crate::exit::exit_code;

/// Print a successful result.
///
/// With `--json`, stdout receives exactly one envelope object and nothing else.
/// Without it, the caller's human renderer writes to stdout.
pub fn print_success<T: Serialize>(json_output: bool, data: &T, human: impl FnOnce(&T)) {
    if json_output {
        let envelope = json!({ "ok": true, "data": data, "error": null });
        println!(
            "{}",
            serde_json::to_string(&envelope).expect("the envelope always serializes")
        );
    } else {
        human(data);
    }
}

/// Print an error and return the exit code for it.
pub fn print_error(error: &CoreError, json_output: bool) -> i32 {
    if json_output {
        let envelope = json!({
            "ok": false,
            "data": null,
            "error": { "code": error.code, "message": error.message }
        });
        println!(
            "{}",
            serde_json::to_string(&envelope).expect("the envelope always serializes")
        );
    } else {
        eprintln!("error: {}", error.message);
    }

    exit_code(error)
}
