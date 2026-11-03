//! Truncated-JSON recovery for write tools.
//!
//! Ported from SkillLite's `parse_truncated_json_for_file_tools`: when the
//! model's output budget cuts a `write_file` / `search_replace` / `patch_config`
//! argument mid-string, salvage the fields we can still read instead of losing
//! the whole call. The caller must warn the model that content may be truncated.

use serde_json::{json, Value};

/// Write tools whose arguments are worth recovering from a cut-off JSON blob.
pub fn is_recoverable_write_tool(tool_name: &str) -> bool {
    matches!(tool_name, "write_file" | "search_replace" | "patch_config")
}

/// Best-effort extraction of string fields from truncated tool-call JSON.
/// Returns `None` when even the `path` cannot be recovered.
pub fn recover_truncated_arguments(tool_name: &str, arguments: &str) -> Option<Value> {
    let recovered = match tool_name {
        "write_file" => {
            let path = extract_string_field(arguments, &["path", "file_path"])?;
            let content = extract_trailing_string_field(arguments, "content")?;
            json!({ "path": path, "content": content })
        }
        "search_replace" => {
            let path = extract_string_field(arguments, &["path", "file_path"])?;
            let old_string = extract_string_field(arguments, &["old_string"])?;
            let new_string =
                extract_trailing_string_field(arguments, "new_string").unwrap_or_default();
            json!({ "path": path, "old_string": old_string, "new_string": new_string })
        }
        "patch_config" => {
            let path = extract_string_field(arguments, &["path", "file_path"])?;
            let key_path = extract_string_field(arguments, &["key_path"])?;
            let value = extract_trailing_string_field(arguments, "value").unwrap_or_default();
            json!({ "path": path, "key_path": key_path, "value": value })
        }
        _ => return None,
    };
    Some(recovered)
}

/// Complete escaped-string values for the first field name that parses whole.
fn extract_string_field(arguments: &str, names: &[&str]) -> Option<String> {
    for name in names {
        let pattern = format!("\"{name}\"");
        if let Some(value) = complete_string_after(&pattern, arguments) {
            return Some(value);
        }
    }
    None
}

/// The last field of a truncated call: take everything after `"name": "` up to
/// a closing quote (if present) and unescape it.
fn extract_trailing_string_field(arguments: &str, name: &str) -> Option<String> {
    let pattern = format!("\"{name}\"");
    let start = arguments.find(&pattern)?;
    let after_key = &arguments[start + pattern.len()..];
    let after_colon = after_key.trim_start().strip_prefix(':')?;
    let after_quote = after_colon.trim_start().strip_prefix('"')?;
    let end = after_quote.rfind('"');
    let raw = match end {
        Some(index) => &after_quote[..index],
        None => after_quote,
    };
    Some(unescape_json_string(raw))
}

fn complete_string_after(marker: &str, haystack: &str) -> Option<String> {
    let start = haystack.find(marker)?;
    let after_key = &haystack[start + marker.len()..];
    let after_colon = after_key.trim_start().strip_prefix(':')?;
    let after_quote = after_colon.trim_start().strip_prefix('"')?;
    let end = find_closing_quote(after_quote)?;
    Some(unescape_json_string(&after_quote[..end]))
}

/// Index of the closing quote of a complete JSON string body (`\"` skipped).
fn find_closing_quote(body: &str) -> Option<usize> {
    let bytes = body.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
            b'"' => return Some(index),
            _ => index += 1,
        }
    }
    None
}

fn unescape_json_string(raw: &str) -> String {
    let escaped = format!("\"{raw}\"");
    if let Ok(value) = serde_json::from_str::<Value>(&escaped) {
        if let Some(text) = value.as_str() {
            return text.to_string();
        }
    }
    // Manual fallback: a string cut by the output limit may hold raw newlines
    // (invalid JSON) next to normal escapes; unescape the common ones.
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(current) = chars.next() {
        if current == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some('/') => out.push('/'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(current);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_json_needs_no_recovery() {
        // The normal path parses fine; recovery only runs when serde fails.
        assert!(serde_json::from_str::<Value>(r#"{"path": "/a"}"#).is_ok());
    }

    #[test]
    fn recovers_write_file_with_cut_content() {
        let raw = r##"{"path": "/tmp/new.md", "content": "# Title\nline1
line2"##;
        let recovered = recover_truncated_arguments("write_file", raw).unwrap();
        assert_eq!(recovered["path"], "/tmp/new.md");
        assert_eq!(recovered["content"], "# Title\nline1\nline2");
    }

    #[test]
    fn recovers_search_replace_with_cut_new_string() {
        let raw = r#"{"path": "/a.yaml", "old_string": "port: 1", "new_string": "port: 2\nextra"#;
        let recovered = recover_truncated_arguments("search_replace", raw).unwrap();
        assert_eq!(recovered["old_string"], "port: 1");
        assert_eq!(recovered["new_string"], "port: 2\nextra");
    }

    #[test]
    fn recovers_patch_config() {
        let raw = r#"{"path": "/c.toml", "key_path": "model.name", "value": "gpt"#;
        let recovered = recover_truncated_arguments("patch_config", raw).unwrap();
        assert_eq!(recovered["key_path"], "model.name");
        assert_eq!(recovered["value"], "gpt");
    }

    #[test]
    fn refuses_when_path_is_missing_or_incomplete() {
        assert!(recover_truncated_arguments("write_file", r#"{"content": "x""#).is_none());
        assert!(recover_truncated_arguments("write_file", r#"{"path": "/a"#).is_none());
        assert!(!is_recoverable_write_tool("read_file"));
    }
}
