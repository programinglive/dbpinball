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

/// Print a result whose command did not fully succeed but whose committed work
/// must still be reported.
///
/// This is the one envelope that is `ok: false` and still carries `data`: the
/// rows are committed facts, and hiding them would be less accurate than
/// reporting both the committed data and the step that did not complete.
pub fn print_incomplete<T: Serialize>(
    json_output: bool,
    data: &T,
    code: &str,
    message: &str,
    detail: &str,
    human: impl FnOnce(&T),
) {
    if json_output {
        let envelope = json!({
            "ok": false,
            "data": data,
            "error": {
                "code": code,
                "message": message,
                "detail": detail
            }
        });
        println!(
            "{}",
            serde_json::to_string(&envelope).expect("the envelope always serializes")
        );
    } else {
        human(data);
        eprintln!("error: {message}");
    }
}

/// Print an error and return the exit code for it.
pub fn print_error(error: &CoreError, json_output: bool) -> i32 {
    if json_output {
        let envelope = json!({
            "ok": false,
            "data": null,
            "error": {
                "code": error.code,
                "message": error.message,
                "detail": error.detail
            }
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
