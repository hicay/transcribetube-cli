//! Data goes to stdout, everything else to stderr, so pipes stay clean.
use std::io::Write;

use serde_json::Value;

use crate::api::ApiError;

pub fn out(text: &str) {
    let stdout = std::io::stdout();
    let mut lock = stdout.lock();
    // A closed pipe (`tube ... | head`) is not an error worth a panic.
    let _ = lock.write_all(text.as_bytes());
    if !text.ends_with('\n') {
        let _ = lock.write_all(b"\n");
    }
    let _ = lock.flush();
}

pub fn json(value: &Value) {
    out(&pretty(value));
}

pub fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

/// Progress and status: stderr only, suppressed in `--json` mode.
pub fn info(text: &str, quiet: bool) {
    if !quiet {
        eprintln!("{text}");
    }
}

/// Prints the error and returns the exit code for it.
pub fn fail(err: &ApiError) -> i32 {
    eprintln!("error: {}", err.message);
    exit_code_for(err.status)
}

/// Script-friendly exit codes (see README).
pub fn exit_code_for(status: Option<u16>) -> i32 {
    match status {
        Some(401) | Some(403) => 2,
        Some(402) => 3,
        Some(422) => 4,
        Some(404) => 5,
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_follow_the_readme() {
        assert_eq!(exit_code_for(Some(401)), 2);
        assert_eq!(exit_code_for(Some(403)), 2);
        assert_eq!(exit_code_for(Some(402)), 3);
        assert_eq!(exit_code_for(Some(422)), 4);
        assert_eq!(exit_code_for(Some(404)), 5);
        assert_eq!(exit_code_for(Some(500)), 1);
        assert_eq!(exit_code_for(None), 1);
    }
}
