//! Reading the frontmatter of the vault's Markdown files.
//!
//! Voice notes, Scribbles and Kanban cards all store one value per line
//! between an opening `---` line and a closing `---` line. The closing
//! delimiter is therefore a *line* that is exactly `---`, never the first
//! `---` anywhere in the file: a title, URL or AI summary containing `---`
//! used to end the frontmatter mid-value, which dropped a Scribble from the
//! index, stripped a Voice Note's metadata into its body, and cost a card its
//! status.

/// Splits a vault Markdown file into `(frontmatter, body)`.
///
/// Returns `None` unless the file opens with a `---` line (after an optional
/// byte-order mark and leading blank lines) and has a closing `---` line. The
/// body is everything after the closing line, untrimmed.
pub(crate) fn split(raw: &str) -> Option<(&str, &str)> {
    let raw = raw.trim_start_matches('\u{feff}').trim_start();
    let after_open = raw.strip_prefix("---")?;
    let newline = after_open.find('\n')?;
    if !after_open[..newline].trim().is_empty() {
        return None;
    }

    let frontmatter_start = &after_open[newline + 1..];
    let mut offset = 0;
    loop {
        let remainder = &frontmatter_start[offset..];
        let (line, next) = match remainder.find('\n') {
            Some(end) => (&remainder[..end], Some(end + 1)),
            None => (remainder, None),
        };
        if line.trim() == "---" {
            let body_start = next.map_or(frontmatter_start.len(), |n| offset + n);
            return Some((&frontmatter_start[..offset], &frontmatter_start[body_start..]));
        }
        offset += next?;
    }
}

/// Reads a string value the note writer emitted.
///
/// Current files hold a JSON string. Older files hold Rust's `Debug`
/// rendering, which agrees with JSON on `\n`, `\t`, `\\` and `\"` but writes
/// combining marks as `\u{94d}` — invalid JSON, which is why a Hindi raw
/// transcript used to come back with the escapes as literal text and then be
/// saved that way for good.
pub(crate) fn decode_string(value: &str) -> String {
    let value = value.trim();
    if let Ok(decoded) = serde_json::from_str::<String>(value) {
        return decoded;
    }
    match value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) {
        Some(inner) => unescape_debug(inner),
        None => value.to_string(),
    }
}

/// Reads `Some(<string>)` or `None`, the shape the note writer uses for
/// optional strings.
pub(crate) fn decode_optional_string(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value == "None" {
        return None;
    }
    let inner = value
        .strip_prefix("Some(")
        .and_then(|v| v.strip_suffix(')'))
        .unwrap_or(value);
    Some(decode_string(inner))
}

/// Reads a list of strings: a JSON array now, a `Debug`-rendered `Vec` in
/// older files, or a bare comma-separated list in a hand-edited one.
pub(crate) fn decode_string_list(value: &str) -> Vec<String> {
    let value = value.trim();
    if let Ok(list) = serde_json::from_str::<Vec<String>>(value) {
        return list.into_iter().filter(|s| !s.is_empty()).collect();
    }
    let inner = value
        .strip_prefix('[')
        .and_then(|v| v.strip_suffix(']'))
        .unwrap_or(value);
    if !inner.contains('"') {
        return inner
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
    }

    let mut items = Vec::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c != '"' {
            continue;
        }
        let mut raw = String::new();
        let mut escaped = false;
        for ch in chars.by_ref() {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                break;
            }
            raw.push(ch);
        }
        let item = unescape_debug(&raw);
        if !item.is_empty() {
            items.push(item);
        }
    }
    items
}

/// Undoes the escapes Rust's `Debug` for `str` produces.
fn unescape_debug(inner: &str) -> String {
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('0') => out.push('\0'),
            Some('\\') => out.push('\\'),
            Some('"') => out.push('"'),
            Some('\'') => out.push('\''),
            Some('u') if chars.peek() == Some(&'{') => {
                chars.next();
                let hex: String = chars.by_ref().take_while(|&h| h != '}').collect();
                match u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                    Some(decoded) => out.push(decoded),
                    None => {
                        out.push_str("\\u{");
                        out.push_str(&hex);
                        out.push('}');
                    }
                }
            }
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_at_the_closing_line_not_the_first_triple_dash() {
        let raw = "---\ntitle: \"Standup --- Monday\"\nsummary: \"a --- b\"\n---\n\nBody\n---\nmore body";
        let (front, body) = split(raw).unwrap();
        assert_eq!(front, "title: \"Standup --- Monday\"\nsummary: \"a --- b\"\n");
        assert_eq!(body, "\nBody\n---\nmore body");
    }

    #[test]
    fn accepts_crlf_bom_and_leading_blank_lines() {
        let raw = "\u{feff}\r\n---\r\nid: \"x\"\r\n---\r\nbody";
        let (front, body) = split(raw).unwrap();
        assert_eq!(front, "id: \"x\"\r\n");
        assert_eq!(body, "body");
    }

    #[test]
    fn handles_empty_frontmatter_and_empty_body() {
        assert_eq!(split("---\n---\n"), Some(("", "")));
        assert_eq!(split("---\nid: 1\n---"), Some(("id: 1\n", "")));
    }

    #[test]
    fn rejects_files_without_both_delimiter_lines() {
        assert_eq!(split("no frontmatter here"), None);
        assert_eq!(split("---\nid: 1\nnever closed"), None);
        assert_eq!(split("--- title\nid: 1\n---\n"), None);
        assert_eq!(split("text\n---\nid: 1\n---\n"), None);
    }

    #[test]
    fn decodes_json_and_legacy_debug_strings() {
        // What `format!("{:?}", "क्या\nहै")` wrote: the virama escaped Rust-style.
        assert_eq!(decode_string("\"क\\u{94d}या\\nहै\""), "क्या\nहै");
        assert_eq!(decode_string(&serde_json::to_string("क्या\nहै").unwrap()), "क्या\nहै");
        assert_eq!(decode_string("\"it\\'s \\\"quoted\\\"\""), "it's \"quoted\"");
        assert_eq!(decode_string("\"C:\\\\Users\\\\a.wav\""), "C:\\Users\\a.wav");
        assert_eq!(decode_string("bare"), "bare");
    }

    #[test]
    fn decodes_optional_strings() {
        assert_eq!(decode_optional_string("None"), None);
        assert_eq!(decode_optional_string(""), None);
        assert_eq!(decode_optional_string("Some(\"clean\")"), Some("clean".to_string()));
        assert_eq!(
            decode_optional_string("Some(\"ok (really)\")"),
            Some("ok (really)".to_string())
        );
        assert_eq!(
            decode_optional_string("Some(\"क\\u{94d}या\")"),
            Some("क्या".to_string())
        );
    }

    #[test]
    fn decodes_string_lists_in_every_shape_written() {
        assert_eq!(decode_string_list("[\"a\",\"b, c\"]"), vec!["a", "b, c"]);
        assert_eq!(decode_string_list("[\"a\", \"क\\u{94d}\"]"), vec!["a", "क्"]);
        assert_eq!(decode_string_list("[\"say \\\"hi\\\"\", \"x\"]"), vec!["say \"hi\"", "x"]);
        assert_eq!(decode_string_list("[a, b]"), vec!["a", "b"]);
        assert!(decode_string_list("[]").is_empty());
    }
}
