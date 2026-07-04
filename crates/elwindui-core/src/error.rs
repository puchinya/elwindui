/// See docs/elwindui_spec.md 付録S.4. WinUI3 has no equivalent generic error screen; this is a
/// minimal shape sized to that requirement, not a port of anything.
pub struct ErrorInfo {
    pub message: String,
    pub debug_details: Option<String>,
}

/// `debug_build` selects between the terse release-mode message and the fuller debug-mode one
/// (stack info etc., when present in `debug_details`).
pub fn default_error_message(err: &ErrorInfo, debug_build: bool) -> String {
    if debug_build {
        match &err.debug_details {
            Some(details) => format!("{}\n{}", err.message, details),
            None => err.message.clone(),
        }
    } else {
        err.message.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_build_hides_debug_details() {
        let err = ErrorInfo {
            message: "something went wrong".to_string(),
            debug_details: Some("stack trace here".to_string()),
        };
        assert_eq!(default_error_message(&err, false), "something went wrong");
    }

    #[test]
    fn debug_build_appends_debug_details() {
        let err = ErrorInfo {
            message: "something went wrong".to_string(),
            debug_details: Some("stack trace here".to_string()),
        };
        assert_eq!(
            default_error_message(&err, true),
            "something went wrong\nstack trace here"
        );
    }
}
