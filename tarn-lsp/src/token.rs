//! Shared scanning primitives for interpolation-token detection.
//!
//! L1.3 (hover) and L1.4 (completion) both need to reason about the
//! cursor's relationship to `{{ … }}` interpolation syntax inside a Tarn
//! YAML buffer. Hover asks "am I *inside* a full `{{ … }}` pair?" and
//! needs the whole token span so it can highlight it. Completion asks
//! "what identifier am I in the middle of typing right now?" and needs
//! the line prefix between the most recent unclosed `{{` and the cursor.
//!
//! The two questions have different answers, but they share:
//!
//!   * UTF-8 ↔ LSP `Position` conversion.
//!   * Line-start / line-end byte offset helpers.
//!   * A byte-substring scan for `{{` / `}}`.
//!
//! L1.4 promotes all of the above out of `hover.rs` into this module so
//! both consumers share one well-tested implementation. Keeping the
//! helpers behind a narrow public surface also stops future tickets from
//! reaching into each other's internals.
//!
//! Nothing in here touches the filesystem, the parser, or `tarn::*`. It
//! is LSP-types-only, which is why every helper is trivially
//! unit-testable.

use lsp_types::{Position, Range};

// Re-export the token classifier + its token enum under the "interpolation"
// naming so downstream modules (definition, references) can depend on a
// neutral name instead of one that bakes in the original L1.3 feature.
// The implementation still lives in [`crate::hover`] to keep the NAZ-292
// history intact — renaming the module would be strictly cleanup and is
// out of scope for NAZ-297, which only needs a clean public alias.
pub use crate::hover::{
    resolve_hover_token as resolve_interpolation_token, HoverToken as InterpolationToken,
    HoverTokenSpan as InterpolationTokenSpan,
};

/// Every classified `{{ ... }}` interpolation token in `source`, in document
/// order, with each token's LSP `Range` attached.
///
/// L2.2 (`textDocument/references`) needs a "scan the whole file" pass that
/// hover and definition never required: those features classify the *one*
/// token under the cursor, while references has to enumerate every use site
/// of a given env key or capture name. This helper gives the references
/// renderer a single source of truth — it scans `source` exactly once and
/// returns a `Vec` the renderer can filter on.
///
/// Behaviour:
///
///   * Multi-token lines are supported. The scanner advances past each
///     `}}` and continues looking for more `{{` after it on the same line.
///   * Multi-line tokens are supported (a `{{` followed by a newline and
///     then a `}}` on the next line still resolves into one token).
///   * Unclosed `{{` (a typo or an in-progress edit) terminates the scan
///     gracefully — every token already collected is still returned.
///   * The scanner does not look at YAML structure: tokens that appear
///     inside YAML comments are skipped, since interpolation in a `#`
///     comment never fires at runtime and including those use sites
///     would be misleading. Comment detection is line-local — a `#`
///     anywhere on a line marks the rest of that line as a comment.
///   * Empty token bodies (`{{ }}`) and tokens that classify into
///     `None` (raw text we don't recognise) are skipped.
pub fn scan_all_interpolations(source: &str) -> Vec<InterpolationTokenSpan> {
    let mut out = Vec::new();
    let bytes = source.as_bytes();
    let mut i = 0usize;
    while i + 1 < bytes.len() {
        // Skip YAML line comments. A `#` following whitespace (or at the
        // start of a line) starts a comment that runs to the next `\n`.
        // Interpolations inside that comment have no runtime effect, so
        // they should not appear as references.
        if bytes[i] == b'#' && (i == 0 || is_yaml_comment_start(bytes, i)) {
            // Fast-forward to the next newline.
            let rest = &bytes[i..];
            let advance = rest.iter().position(|&b| b == b'\n').unwrap_or(rest.len());
            i += advance;
            continue;
        }
        if bytes[i] == b'{' && bytes[i + 1] == b'{' {
            // Find the matching `}}`.
            let after_open = i + 2;
            let Some(rel_end) = find_subslice(&bytes[after_open..], b"}}") else {
                // Unclosed `{{`. Stop scanning so we don't claim the
                // rest of the file as a single token.
                break;
            };
            let content_start = after_open;
            let content_end = after_open + rel_end;
            let token_end = content_end + 2;
            let raw = &source[content_start..content_end];
            if let Some(token) = classify_interpolation_body(raw.trim()) {
                let start_pos = byte_offset_to_position(source, i);
                let end_pos = byte_offset_to_position(source, token_end);
                out.push(InterpolationTokenSpan {
                    token,
                    range: Range::new(start_pos, end_pos),
                });
            }
            i = token_end;
            continue;
        }
        i += 1;
    }
    out
}

/// Classify the trimmed body of a `{{ ... }}` interpolation.
///
/// Mirrors the logic of `crate::hover::classify_expression` but is private
/// to the scanner so future changes to either side stay isolated. We also
/// reject empty bodies up front — `{{ }}` is a typo, not a token.
fn classify_interpolation_body(raw: &str) -> Option<InterpolationToken> {
    if raw.is_empty() {
        return None;
    }
    if let Some(rest) = raw.strip_prefix("env.") {
        return Some(InterpolationToken::Env(rest.trim().to_owned()));
    }
    if raw == "env" {
        return Some(InterpolationToken::Env(String::new()));
    }
    if let Some(rest) = raw.strip_prefix("capture.") {
        return Some(InterpolationToken::Capture(rest.trim().to_owned()));
    }
    if raw == "capture" {
        return Some(InterpolationToken::Capture(String::new()));
    }
    if let Some(rest) = raw.strip_prefix('$') {
        let name = rest.split('(').next().unwrap_or("").trim();
        return Some(InterpolationToken::Builtin(name.to_owned()));
    }
    None
}

/// True when the `#` byte at `idx` is a YAML line-comment start. The
/// scanner only walks back to the previous newline (or the buffer start)
/// — the rule we enforce is "preceding character is whitespace, or `#`
/// is at column 0".
fn is_yaml_comment_start(bytes: &[u8], idx: usize) -> bool {
    if idx == 0 {
        return true;
    }
    let prev = bytes[idx - 1];
    prev == b' ' || prev == b'\t' || prev == b'\n'
}

/// Convert a 0-based LSP [`Position`] into a byte offset into `source`.
///
/// LSP addresses each `character` as a UTF-16 code unit. Tarn YAML is
/// overwhelmingly ASCII, but the helper walks characters defensively —
/// a cursor past the end of the line folds to the line's end rather
/// than overflowing the slice. Returns `None` only when the line index
/// is impossibly far past the document (beyond every existing
/// newline).
pub fn position_to_byte_offset(source: &str, position: Position) -> Option<usize> {
    let line_start = position_to_line_start(source, position.line as usize)?;
    let line_end = find_line_end(source, line_start);
    let line = &source[line_start..line_end];
    let char_count_limit = position.character as usize;
    let offset_in_line: usize = line
        .chars()
        .take(char_count_limit)
        .map(char::len_utf8)
        .sum();
    Some(line_start + offset_in_line.min(line.len()))
}

/// Convert a byte offset back into a [`Position`]. Used by classifiers
/// that scan raw bytes and need to report the answer in LSP coordinates.
pub fn byte_offset_to_position(source: &str, offset: usize) -> Position {
    let mut line = 0u32;
    let mut col = 0u32;
    let clamped = offset.min(source.len());
    for (i, ch) in source.char_indices() {
        if i >= clamped {
            break;
        }
        if ch == '\n' {
            line += 1;
            col = 0;
        } else {
            col += 1;
        }
    }
    Position::new(line, col)
}

/// Byte offset of the start of a 0-based line in `source`.
///
/// Returns `Some(source.len())` for lines past the document end so
/// callers get a well-defined empty slice instead of a `None` they have
/// to branch on.
pub fn position_to_line_start(source: &str, target_line: usize) -> Option<usize> {
    if target_line == 0 {
        return Some(0);
    }
    let mut newline_count = 0usize;
    for (i, b) in source.bytes().enumerate() {
        if b == b'\n' {
            newline_count += 1;
            if newline_count == target_line {
                return Some(i + 1);
            }
        }
    }
    Some(source.len())
}

/// Byte offset of the newline terminating the line that starts at
/// `line_start`. Returns `source.len()` when the line is the last one
/// and has no terminating `\n`.
pub fn find_line_end(source: &str, line_start: usize) -> usize {
    source[line_start..]
        .bytes()
        .position(|b| b == b'\n')
        .map(|rel| line_start + rel)
        .unwrap_or(source.len())
}

/// First occurrence of `needle` inside `haystack`, or `None`.
///
/// Used by the hover-token scanner to find `}}` after a `{{` without
/// pulling in `memchr` or paying for `str::find` overhead on every
/// call.
pub fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    (0..=haystack.len() - needle.len()).find(|&i| &haystack[i..i + needle.len()] == needle)
}

/// Is `s` a bare identifier — `[A-Za-z_][A-Za-z0-9_-]*`?
///
/// Used by both the hover schema-key classifier and the completion
/// schema-key classifier to reject lines whose "key" is clearly not a
/// Tarn field (numbers, sub-scripts, quoted strings, etc).
pub fn is_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Slice of the line that contains `position`, along with the byte
/// offset of that line's start. Returned as a tuple so callers that
/// want both the prefix slice (e.g. completion context detection) and
/// the absolute offset (e.g. diagnostics) share one pass.
///
/// The returned slice covers the full line — callers restrict it to
/// `line[..cursor_col_in_bytes]` when they only care about the prefix
/// up to the cursor.
pub fn line_at_position(source: &str, position: Position) -> Option<(usize, &str)> {
    let line_start = position_to_line_start(source, position.line as usize)?;
    let line_end = find_line_end(source, line_start);
    Some((line_start, &source[line_start..line_end]))
}

/// Byte offset within `line` of `position.character`, clamped to the
/// line length. Mirrors what `position_to_byte_offset` does but works
/// on a pre-computed line slice so completion can ask "how much of
/// this line is the prefix?" without walking the document twice.
pub fn column_to_line_byte_offset(line: &str, character: u32) -> usize {
    let take = character as usize;
    let bytes: usize = line.chars().take(take).map(char::len_utf8).sum();
    bytes.min(line.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- position <-> byte offset ----

    #[test]
    fn position_to_byte_offset_at_document_start_is_zero() {
        let src = "abc\n";
        assert_eq!(position_to_byte_offset(src, Position::new(0, 0)), Some(0));
    }

    #[test]
    fn position_to_byte_offset_mid_line() {
        let src = "abc\ndef\n";
        assert_eq!(position_to_byte_offset(src, Position::new(1, 2)), Some(6));
    }

    #[test]
    fn position_to_byte_offset_past_end_of_line_clamps_to_line_end() {
        let src = "abc\ndef\n";
        // line 0 is `abc`, length 3. Asking for column 10 should clamp.
        assert_eq!(position_to_byte_offset(src, Position::new(0, 10)), Some(3));
    }

    #[test]
    fn byte_offset_to_position_round_trips_ascii() {
        let src = "abc\ndef\n";
        let pos = byte_offset_to_position(src, 5);
        assert_eq!(pos, Position::new(1, 1));
    }

    // ---- line start/end ----

    #[test]
    fn position_to_line_start_first_line_is_zero() {
        assert_eq!(position_to_line_start("abc\ndef", 0), Some(0));
    }

    #[test]
    fn position_to_line_start_second_line() {
        assert_eq!(position_to_line_start("abc\ndef", 1), Some(4));
    }

    #[test]
    fn position_to_line_start_past_end_returns_source_len() {
        let src = "abc\n";
        assert_eq!(position_to_line_start(src, 50), Some(src.len()));
    }

    #[test]
    fn find_line_end_stops_at_newline() {
        let src = "abc\ndef\n";
        assert_eq!(find_line_end(src, 0), 3);
    }

    #[test]
    fn find_line_end_handles_missing_trailing_newline() {
        let src = "abc";
        assert_eq!(find_line_end(src, 0), 3);
    }

    // ---- scanning ----

    #[test]
    fn find_subslice_finds_match() {
        assert_eq!(find_subslice(b"hello world", b"world"), Some(6));
    }

    #[test]
    fn find_subslice_returns_none_when_missing() {
        assert_eq!(find_subslice(b"hello", b"z"), None);
    }

    #[test]
    fn find_subslice_empty_needle_returns_none() {
        assert_eq!(find_subslice(b"hello", b""), None);
    }

    // ---- identifier check ----

    #[test]
    fn is_identifier_accepts_alpha_underscore() {
        assert!(is_identifier("name"));
        assert!(is_identifier("_name"));
        assert!(is_identifier("name_2"));
        assert!(is_identifier("env-file"));
    }

    #[test]
    fn is_identifier_rejects_leading_digit() {
        assert!(!is_identifier("2fast"));
    }

    #[test]
    fn is_identifier_rejects_empty() {
        assert!(!is_identifier(""));
    }

    // ---- line_at_position + column_to_line_byte_offset ----

    #[test]
    fn line_at_position_returns_line_slice_and_start_offset() {
        let src = "abc\ndef\nghi";
        let (start, line) = line_at_position(src, Position::new(1, 0)).unwrap();
        assert_eq!(start, 4);
        assert_eq!(line, "def");
    }

    #[test]
    fn column_to_line_byte_offset_clamps_past_end() {
        assert_eq!(column_to_line_byte_offset("abc", 10), 3);
    }

    #[test]
    fn column_to_line_byte_offset_mid_line_is_char_byte_sum() {
        assert_eq!(column_to_line_byte_offset("abcdef", 3), 3);
    }

    // ---- scan_all_interpolations ----

    #[test]
    fn scan_all_interpolations_finds_single_env_token() {
        let src = "url: \"{{ env.base_url }}\"\n";
        let tokens = scan_all_interpolations(src);
        assert_eq!(tokens.len(), 1);
        assert!(matches!(tokens[0].token, InterpolationToken::Env(ref k) if k == "base_url"));
        assert_eq!(tokens[0].range.start.line, 0);
    }

    #[test]
    fn scan_all_interpolations_finds_multiple_tokens_on_one_line() {
        let src = "url: \"{{ env.base_url }}/items/{{ capture.id }}\"\n";
        let tokens = scan_all_interpolations(src);
        assert_eq!(tokens.len(), 2);
        match &tokens[0].token {
            InterpolationToken::Env(k) => assert_eq!(k, "base_url"),
            other => panic!("expected env token, got {other:?}"),
        }
        match &tokens[1].token {
            InterpolationToken::Capture(k) => assert_eq!(k, "id"),
            other => panic!("expected capture token, got {other:?}"),
        }
    }

    #[test]
    fn scan_all_interpolations_finds_tokens_across_multiple_lines() {
        let src = "a: \"{{ env.x }}\"\nb: \"{{ capture.y }}\"\n";
        let tokens = scan_all_interpolations(src);
        assert_eq!(tokens.len(), 2);
        assert_eq!(tokens[0].range.start.line, 0);
        assert_eq!(tokens[1].range.start.line, 1);
    }

    #[test]
    fn scan_all_interpolations_handles_unclosed_open_gracefully() {
        // Unclosed `{{` should not eat the rest of the file or panic.
        let src = "first: \"{{ env.a }}\"\nsecond: \"{{ env.b\nthird: ok\n";
        let tokens = scan_all_interpolations(src);
        assert_eq!(tokens.len(), 1);
        match &tokens[0].token {
            InterpolationToken::Env(k) => assert_eq!(k, "a"),
            other => panic!("expected first token to be env.a, got {other:?}"),
        }
    }

    #[test]
    fn scan_all_interpolations_skips_tokens_inside_yaml_line_comments() {
        let src = "# {{ env.commented }}\nurl: \"{{ env.real }}\"\n";
        let tokens = scan_all_interpolations(src);
        assert_eq!(tokens.len(), 1);
        match &tokens[0].token {
            InterpolationToken::Env(k) => assert_eq!(k, "real"),
            other => panic!("expected env.real, got {other:?}"),
        }
    }

    #[test]
    fn scan_all_interpolations_skips_tokens_in_trailing_comment() {
        let src = "url: \"{{ env.real }}\" # {{ env.commented }}\n";
        let tokens = scan_all_interpolations(src);
        assert_eq!(tokens.len(), 1);
        match &tokens[0].token {
            InterpolationToken::Env(k) => assert_eq!(k, "real"),
            other => panic!("expected env.real, got {other:?}"),
        }
    }

    #[test]
    fn scan_all_interpolations_returns_empty_for_plain_text() {
        let tokens = scan_all_interpolations("just some plain yaml text\nwith no tokens\n");
        assert!(tokens.is_empty());
    }

    #[test]
    fn scan_all_interpolations_skips_empty_token_body() {
        let tokens = scan_all_interpolations("url: \"{{ }}\"\n");
        assert!(tokens.is_empty());
    }

    #[test]
    fn scan_all_interpolations_classifies_builtin_token() {
        let tokens = scan_all_interpolations("id: \"{{ $uuid }}\"\n");
        assert_eq!(tokens.len(), 1);
        assert!(matches!(tokens[0].token, InterpolationToken::Builtin(ref n) if n == "uuid"));
    }
}
