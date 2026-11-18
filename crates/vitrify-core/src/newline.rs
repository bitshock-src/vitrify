//! Universal-newline line splitting.
//!
//! LF, CR, and CRLF each terminate a line; joins use LF — the
//! universal-newline rule over the spec-file input class. Every line split
//! on the hashing/masking/parsing path goes through here.

/// Split `text` into lines, treating `\n`, `\r`, and `\r\n` each as one
/// terminator. A trailing terminator does not produce an empty final line.
pub fn split_universal(text: &str) -> Vec<&str> {
    let bytes = text.as_bytes();
    let mut lines = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\n' => {
                lines.push(&text[start..i]);
                i += 1;
                start = i;
            }
            b'\r' => {
                lines.push(&text[start..i]);
                i += 1;
                if i < bytes.len() && bytes[i] == b'\n' {
                    i += 1;
                }
                start = i;
            }
            _ => i += 1,
        }
    }
    if start < bytes.len() {
        lines.push(&text[start..]);
    }
    lines
}

/// Join lines with LF (no trailing newline).
pub fn join_lf(lines: &[&str]) -> String {
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_universal_newlines() {
        assert_eq!(split_universal(""), Vec::<&str>::new());
        assert_eq!(split_universal("a"), vec!["a"]);
        assert_eq!(split_universal("a\n"), vec!["a"]);
        assert_eq!(split_universal("a\nb"), vec!["a", "b"]);
        assert_eq!(split_universal("a\r\nb\rc\nd"), vec!["a", "b", "c", "d"]);
        assert_eq!(split_universal("\n\n"), vec!["", ""]);
        assert_eq!(split_universal("a\r\r\nb"), vec!["a", "", "b"]);
    }

    #[test]
    fn joins_with_lf() {
        assert_eq!(join_lf(&["a", "b"]), "a\nb");
        assert_eq!(join_lf(&[]), "");
    }
}
