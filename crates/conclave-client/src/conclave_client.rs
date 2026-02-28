pub mod api;
#[cfg(feature = "command")]
pub mod command;
pub mod config;
pub mod duration;
pub mod error;
pub mod mls;
pub mod operations;
pub mod state;
pub mod store;

/// Strip ASCII control characters from a string to prevent terminal escape
/// injection and notification rendering issues. Preserves printable ASCII,
/// newlines, and all valid Unicode above U+007F.
pub fn sanitize_control_chars(input: &str) -> String {
    input
        .chars()
        .filter(|c| {
            let code = *c as u32;
            if code == 0x0A {
                return true;
            }
            if code <= 0x1F || code == 0x7F {
                return false;
            }
            true
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_control_chars() {
        assert_eq!(sanitize_control_chars("hello world"), "hello world");
        assert_eq!(sanitize_control_chars("line1\nline2"), "line1\nline2");
        assert_eq!(sanitize_control_chars("has\x00null"), "hasnull");
        assert_eq!(sanitize_control_chars("bel\x07here"), "belhere");
        assert_eq!(sanitize_control_chars("back\x08space"), "backspace");
        assert_eq!(sanitize_control_chars("esc\x1b[2Jone"), "esc[2Jone");
        assert_eq!(sanitize_control_chars("del\x7fete"), "delete");
        assert_eq!(sanitize_control_chars("\x01\x02\x03\x1f"), "");
    }

    #[test]
    fn test_sanitize_control_chars_preserves_unicode() {
        assert_eq!(sanitize_control_chars("hello 世界"), "hello 世界");
        assert_eq!(sanitize_control_chars("emoji 🎉🔒"), "emoji 🎉🔒");
        assert_eq!(sanitize_control_chars("café résumé"), "café résumé");
        assert_eq!(
            sanitize_control_chars("mixed \x1b[8m秘密\x00test"),
            "mixed [8m秘密test"
        );
    }
}
