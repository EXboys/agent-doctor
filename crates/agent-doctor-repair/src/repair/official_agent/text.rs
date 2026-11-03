//! Truncation profiles for tool results sent back to the model.
//!
//! Ported from SkillLite's `extensions/builtin/mod.rs` result-processing
//! profiles: `read_file` keeps verbatim content under a large cap (never
//! summarized), everything else gets a tight head+tail budget so one huge
//! tool result cannot crowd out the conversation.

/// `read_file` only: large dedicated cap, head+tail, never LLM-summarized.
pub const READ_FILE_RESULT_MAX_CHARS: usize = 786_432;
/// Every other tool: tight cap with head+tail fallback.
pub const TOOL_RESULT_MAX_CHARS: usize = 12_000;

/// Head+tail truncation with an explicit char budget (head 2/3, tail 1/3).
pub fn head_tail(content: &str, max_chars: usize) -> String {
    let len = content.chars().count();
    if len <= max_chars {
        return content.to_string();
    }
    let head_size = max_chars.min(len);
    let tail_size = (max_chars / 3).min(len);
    let head: String = content.chars().take(head_size).collect();
    let tail_start = len - tail_size;
    let tail: String = content.chars().skip(tail_start).collect();
    format!("{head}\n\n... [content truncated: {len} chars total, showing head+tail] ...\n\n{tail}")
}

/// Profile applied by tool name; mirrors SkillLite's `result_processing_profile_for`.
pub fn process_tool_result(tool_name: &str, content: &str) -> String {
    if tool_name == "read_file" {
        head_tail(content, READ_FILE_RESULT_MAX_CHARS)
    } else {
        head_tail(content, TOOL_RESULT_MAX_CHARS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_content_passes_through() {
        assert_eq!(head_tail("hello", 10), "hello");
    }

    #[test]
    fn oversized_content_keeps_head_and_tail() {
        let content = format!("{}MIDDLE{}", "a".repeat(500), "b".repeat(500));
        let out = head_tail(&content, 200);
        assert!(out.contains("[content truncated: 1006 chars total"));
        assert!(out.starts_with("aaaa"));
        assert!(out.ends_with("bbbb"));
        assert!(!out.contains("MIDDLE"), "middle must be dropped");
    }

    #[test]
    fn read_file_profile_uses_the_large_cap() {
        let huge = "x".repeat(TOOL_RESULT_MAX_CHARS * 3);
        assert_eq!(
            process_tool_result("read_file", &huge).chars().count(),
            huge.chars().count(),
            "read_file stays verbatim under its own cap"
        );
        let squeezed = process_tool_result("grep_files", &huge);
        assert!(squeezed.chars().count() < huge.chars().count());
    }
}
