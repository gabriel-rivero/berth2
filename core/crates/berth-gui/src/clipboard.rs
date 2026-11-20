//! OSC 52 clipboard reads: the reply the terminal sends after confirmation.
//!
//! A program asks with `OSC 52 ; selection ; ?` and the reply carries the
//! selection's text as base64. The window only builds that reply after the
//! user approves it; a denial is an empty payload.

/// A single read is capped; a larger selection is denied, not truncated.
pub const MAX_READ_BYTES: usize = 1024 * 1024;

/// Base64 with the standard alphabet and padding, as OSC 52 expects.
pub fn base64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied().unwrap_or(0);
        let third = chunk.get(2).copied().unwrap_or(0);
        out.push(ALPHABET[(first >> 2) as usize] as char);
        out.push(ALPHABET[(((first & 0x03) << 4) | (second >> 4)) as usize] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[(((second & 0x0f) << 2) | (third >> 6)) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[(third & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

/// The OSC 52 reply for a selection. `None` or an oversized text denies it.
pub fn response(selection: &str, text: Option<&str>) -> Vec<u8> {
    let selection: String = selection
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .take(16)
        .collect();
    let selection = if selection.is_empty() {
        "c".to_string()
    } else {
        selection
    };
    match text {
        Some(text) if !text.is_empty() && text.len() <= MAX_READ_BYTES => format!(
            "\x1b]52;{selection};{}\x1b\\",
            base64_encode(text.as_bytes())
        )
        .into_bytes(),
        _ => format!("\x1b]52;{selection};\x1b\\").into_bytes(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn given_known_bytes_then_base64_matches_the_standard_encoding() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"a"), "YQ==");
        assert_eq!(base64_encode(b"ab"), "YWI=");
        assert_eq!(base64_encode(b"abc"), "YWJj");
        assert_eq!(base64_encode(b"hello"), "aGVsbG8=");
    }

    #[test]
    fn given_an_approved_read_then_the_reply_carries_the_selection_and_text() {
        assert_eq!(
            response("c", Some("hello")),
            b"\x1b]52;c;aGVsbG8=\x1b\\".to_vec()
        );
    }

    #[test]
    fn given_a_denied_or_oversized_read_then_the_reply_is_empty() {
        assert_eq!(response("c", None), b"\x1b]52;c;\x1b\\".to_vec());
        assert_eq!(response("c", Some("")), b"\x1b]52;c;\x1b\\".to_vec());
        let oversized = "x".repeat(MAX_READ_BYTES + 1);
        assert_eq!(
            response("c", Some(&oversized)),
            b"\x1b]52;c;\x1b\\".to_vec()
        );
    }

    #[test]
    fn given_a_odd_selection_then_only_alphanumerics_reach_the_reply() {
        assert_eq!(
            response("c;rm -rf", Some("x")),
            b"\x1b]52;crmrf;eA==\x1b\\".to_vec()
        );
        assert_eq!(response(";", Some("x")), b"\x1b]52;c;eA==\x1b\\".to_vec());
    }
}
