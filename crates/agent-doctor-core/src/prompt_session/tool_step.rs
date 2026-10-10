//! One tool call as fixed fields, so the chat can draw it without parsing a status line.
//!
//! A step is sent when the tool starts and again when its result arrives. The second
//! send carries the same `id` and only the fields the result adds (status, output).

use serde::{Deserialize, Serialize};
use serde_json::Value;

const MAX_LINES: usize = 200;
const MAX_OUTPUT_CHARS: usize = 6_000;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolStep {
    pub id: String,
    /// `read` / `write` / `edit` / `terminal` / `search` / `other`. Empty on a result-only update.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// `running` / `done` / `failed`.
    pub status: String,
    /// The runtime's own one-line description of the step, when it gives one.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub path: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub command: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_start: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_end: Option<u64>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub additions: u64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub deletions: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub added_lines: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deleted_lines: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub output: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i64>,
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

fn key(name: &str) -> String {
    name.chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn text(obj: &serde_json::Map<String, Value>, keys: &[&str]) -> String {
    keys.iter()
        .find_map(|k| obj.get(*k).and_then(Value::as_str))
        .map(str::trim)
        .unwrap_or("")
        .to_string()
}

fn command_text(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(s)) => s.trim().to_string(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(" "),
        _ => String::new(),
    }
}

fn capped_lines(text: &str) -> Vec<String> {
    text.lines().take(MAX_LINES).map(str::to_string).collect()
}

fn line_count(text: &str) -> u64 {
    text.lines().count() as u64
}

fn cap_output(text: &str) -> String {
    let text = text.trim_end();
    if text.chars().count() <= MAX_OUTPUT_CHARS {
        return text.to_string();
    }
    let kept: String = text.chars().take(MAX_OUTPUT_CHARS).collect();
    format!("{kept}\n…")
}

/// Text of a tool result: a plain string, or Claude's list of `{type:"text"}` blocks.
pub(crate) fn tool_result_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(items) => items
            .iter()
            .filter_map(|item| match item {
                Value::String(s) => Some(s.clone()),
                Value::Object(map) => map.get("text").and_then(Value::as_str).map(str::to_string),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Object(map) => ["output", "stdout", "content", "text", "result"]
            .iter()
            .find_map(|k| map.get(*k))
            .map(tool_result_text)
            .unwrap_or_default(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// A step built from the tool name and its input, sent when the tool starts.
pub(crate) fn tool_step(id: &str, name: &str, input: &Value) -> ToolStep {
    let mut step = ToolStep {
        id: id.to_string(),
        name: name.to_string(),
        status: "running".into(),
        kind: "other".into(),
        ..ToolStep::default()
    };
    let parsed;
    let input = match input.as_str().map(str::trim) {
        Some(raw) if raw.starts_with('{') => {
            parsed = serde_json::from_str::<Value>(raw).unwrap_or(Value::Null);
            &parsed
        }
        _ => input,
    };
    let empty = serde_json::Map::new();
    let obj = input.as_object().unwrap_or(&empty);
    step.title = text(obj, &["description"]);
    step.path = text(
        obj,
        &["file_path", "path", "notebook_path", "file", "filePath"],
    );
    match key(name).as_str() {
        "read" | "readfile" | "view" | "viewfile" | "imageview" | "openfile" => {
            step.kind = "read".into();
            if let Some(start) = obj.get("offset").and_then(Value::as_u64).filter(|n| *n > 0) {
                let limit = obj.get("limit").and_then(Value::as_u64).unwrap_or(1).max(1);
                step.line_start = Some(start);
                step.line_end = Some(start + limit - 1);
            }
        }
        "write" | "writefile" | "createfile" | "addfile" => {
            step.kind = "write".into();
            let content = text(obj, &["content", "contents", "text"]);
            step.additions = line_count(&content);
            step.added_lines = capped_lines(&content);
        }
        "edit" | "multiedit" | "notebookedit" | "strreplace" | "searchreplace" | "applydiff"
        | "filechange" | "patch" => {
            step.kind = "edit".into();
            let pairs: Vec<(String, String)> = match obj.get("edits").and_then(Value::as_array) {
                Some(edits) => edits
                    .iter()
                    .filter_map(Value::as_object)
                    .map(|e| {
                        (
                            text(e, &["old_string", "oldText"]),
                            text(e, &["new_string", "newText"]),
                        )
                    })
                    .collect(),
                None => vec![(
                    text(obj, &["old_string", "oldText", "old_str"]),
                    text(obj, &["new_string", "newText", "new_str", "new_source"]),
                )],
            };
            for (old, new) in pairs {
                step.deletions += line_count(&old);
                step.additions += line_count(&new);
                step.deleted_lines.extend(capped_lines(&old));
                step.added_lines.extend(capped_lines(&new));
            }
            let patch = text(obj, &["patch", "diff"]);
            if !patch.is_empty() && step.additions == 0 && step.deletions == 0 {
                apply_unified_diff(&mut step, &patch, false);
            }
        }
        "applypatch" => {
            step.kind = "edit".into();
            let patch = text(obj, &["patch", "input", "diff"]);
            if step.path.is_empty() {
                step.path = patch
                    .lines()
                    .find_map(|l| {
                        l.strip_prefix("*** Update File: ")
                            .or_else(|| l.strip_prefix("*** Add File: "))
                    })
                    .unwrap_or("")
                    .trim()
                    .to_string();
            }
            apply_unified_diff(&mut step, &patch, false);
        }
        "grep" | "glob" | "search" | "searchfiles" | "rg" | "codebasesearch" | "websearch"
        | "webfetch" | "fetch" | "find" => {
            step.kind = "search".into();
            step.query = text(obj, &["pattern", "query", "url", "glob_pattern", "q"]);
        }
        "bash" | "shell" | "terminal" | "execcommand" | "command" | "commandexecution"
        | "runcommand" | "runterminalcmd" | "exec" => {
            step.command = command_text(obj.get("command").or_else(|| obj.get("cmd")));
            step.path.clear();
            classify_command(&mut step);
        }
        _ => {
            step.query = text(obj, &["pattern", "query", "url"]);
        }
    }
    step
}

/// A result-only update for a step that already started.
pub(crate) fn tool_step_result(id: &str, output: &str, failed: bool) -> ToolStep {
    let exit_code = output
        .trim_start()
        .strip_prefix("Exit code ")
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|n| n.parse::<i64>().ok());
    ToolStep {
        id: id.to_string(),
        status: if failed { "failed" } else { "done" }.into(),
        output: cap_output(output),
        exit_code,
        ..ToolStep::default()
    }
}

/// Codex app-server items. A file change touching several files becomes one step per file.
pub(crate) fn codex_tool_steps(item: &Value, item_type: &str, completed: bool) -> Vec<ToolStep> {
    let id = item.get("id").and_then(Value::as_str).unwrap_or("");
    let item_status = item.get("status").and_then(Value::as_str).unwrap_or("");
    let failed = matches!(item_status, "failed" | "declined");
    let status = if !completed {
        "running"
    } else if failed {
        "failed"
    } else {
        "done"
    };
    let base = ToolStep {
        id: id.to_string(),
        name: item_type.to_string(),
        status: status.into(),
        kind: "other".into(),
        ..ToolStep::default()
    };
    match key(item_type).as_str() {
        "commandexecution" => {
            let mut step = ToolStep {
                command: command_text(item.get("command")),
                output: cap_output(
                    item.get("aggregatedOutput")
                        .or_else(|| item.get("aggregated_output"))
                        .and_then(Value::as_str)
                        .unwrap_or(""),
                ),
                exit_code: item.get("exitCode").and_then(Value::as_i64),
                ..base
            };
            let action = item
                .get("commandActions")
                .or_else(|| item.get("command_actions"))
                .and_then(Value::as_array)
                .and_then(|actions| actions.first())
                .and_then(Value::as_object);
            match action.map(|a| (text(a, &["type"]), a)) {
                Some((kind, a)) if kind == "read" => {
                    step.kind = "read".into();
                    step.path = text(a, &["path", "name"]);
                }
                Some((kind, a)) if kind == "search" => {
                    step.kind = "search".into();
                    step.query = text(a, &["query", "path"]);
                }
                _ => classify_command(&mut step),
            }
            if step.kind == "other" {
                step.kind = "terminal".into();
            }
            vec![step]
        }
        "filechange" => {
            let changes = item
                .get("changes")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            if changes.is_empty() {
                return vec![ToolStep {
                    kind: "edit".into(),
                    ..base
                }];
            }
            let many = changes.len() > 1;
            changes
                .iter()
                .enumerate()
                .map(|(n, change)| {
                    let path = change
                        .get("path")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let change_kind = change
                        .pointer("/kind/type")
                        .or_else(|| change.get("kind"))
                        .and_then(Value::as_str)
                        .unwrap_or("update");
                    let mut step = ToolStep {
                        id: if many {
                            format!("{id}#{n}")
                        } else {
                            id.to_string()
                        },
                        kind: if change_kind == "add" {
                            "write"
                        } else {
                            "edit"
                        }
                        .into(),
                        path,
                        ..base.clone()
                    };
                    let diff = change.get("diff").and_then(Value::as_str).unwrap_or("");
                    apply_unified_diff(&mut step, diff, change_kind == "add");
                    step
                })
                .collect()
        }
        "websearch" => vec![ToolStep {
            kind: "search".into(),
            query: command_text(item.get("query")),
            ..base
        }],
        "imageview" => vec![ToolStep {
            kind: "read".into(),
            path: command_text(item.get("path")),
            ..base
        }],
        _ => {
            let server = item.get("server").and_then(Value::as_str).unwrap_or("");
            let tool = item.get("tool").and_then(Value::as_str).unwrap_or("");
            let mut step = match item.get("arguments") {
                Some(args) if !tool.is_empty() => tool_step(id, tool, args),
                _ => base.clone(),
            };
            step.status = base.status.clone();
            if step.name.is_empty() || !server.is_empty() {
                step.name = [server, tool]
                    .iter()
                    .filter(|s| !s.is_empty())
                    .copied()
                    .collect::<Vec<_>>()
                    .join("/");
            }
            if completed {
                step.output =
                    cap_output(&item.get("result").map(tool_result_text).unwrap_or_default());
            }
            vec![step]
        }
    }
}

fn apply_unified_diff(step: &mut ToolStep, diff: &str, whole_file_added: bool) {
    let has_hunks = diff.lines().any(|l| l.starts_with("@@"));
    if whole_file_added && !has_hunks {
        step.additions += line_count(diff);
        step.added_lines.extend(capped_lines(diff));
        return;
    }
    for line in diff.lines() {
        if line.starts_with("+++") || line.starts_with("---") || line.starts_with("***") {
            continue;
        }
        if let Some(rest) = line.strip_prefix('+') {
            step.additions += 1;
            if step.added_lines.len() < MAX_LINES {
                step.added_lines.push(rest.to_string());
            }
        } else if let Some(rest) = line.strip_prefix('-') {
            step.deletions += 1;
            if step.deleted_lines.len() < MAX_LINES {
                step.deleted_lines.push(rest.to_string());
            }
        }
    }
}

/// Split a shell line into words, keeping quoted text together and redirects as their own words.
fn shell_words(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut chars = line.chars().peekable();
    let flush = |current: &mut String, words: &mut Vec<String>| {
        if !current.is_empty() {
            words.push(std::mem::take(current));
        }
    };
    while let Some(ch) = chars.next() {
        if let Some(q) = quote {
            if ch == q {
                quote = None;
            } else {
                current.push(ch);
            }
            continue;
        }
        match ch {
            '\'' | '"' => quote = Some(ch),
            ' ' | '\t' => flush(&mut current, &mut words),
            '|' | ';' | '&' | '>' | '<' => {
                let fd_redirect =
                    ch == '>' && !current.is_empty() && current.chars().all(|c| c.is_ascii_digit());
                if !fd_redirect {
                    flush(&mut current, &mut words);
                }
                let mut op = std::mem::take(&mut current);
                op.push(ch);
                while let Some(&next) = chars.peek() {
                    if matches!(next, '|' | '&' | '>' | '<') {
                        op.push(next);
                        chars.next();
                    } else {
                        break;
                    }
                }
                words.push(op);
            }
            _ => current.push(ch),
        }
    }
    flush(&mut current, &mut words);
    words
}

fn file_word(word: &str) -> Option<String> {
    let word = word.trim();
    if word.is_empty() || word.starts_with('-') || word.starts_with('$') || word == "/dev/null" {
        return None;
    }
    let has_ext = word.rsplit_once('.').is_some_and(|(stem, ext)| {
        !stem.is_empty()
            && (1..=10).contains(&ext.len())
            && ext.chars().all(|c| c.is_ascii_alphanumeric())
            && ext.chars().any(|c| c.is_ascii_alphabetic())
    });
    (word.contains('/') || has_ext).then(|| word.to_string())
}

fn heredoc_lines(command: &str) -> Vec<String> {
    let mut lines = command.lines();
    let first = lines.next().unwrap_or("");
    let Some(pos) = first.find("<<") else {
        return Vec::new();
    };
    let tag: String = first[pos + 2..]
        .trim_start_matches('-')
        .trim()
        .trim_matches(|c| c == '\'' || c == '"')
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    if tag.is_empty() {
        return Vec::new();
    }
    lines
        .take_while(|line| line.trim() != tag)
        .take(MAX_LINES)
        .map(str::to_string)
        .collect()
}

/// Terminal commands that write, append to, read, or search name what they touch.
fn classify_command(step: &mut ToolStep) {
    step.kind = "terminal".into();
    let command = step.command.clone();
    let first = command.lines().next().unwrap_or("");
    let words = shell_words(first);
    let mut segments: Vec<&[String]> = Vec::new();
    let mut start = 0;
    for (i, w) in words.iter().enumerate() {
        if matches!(w.as_str(), "|" | "||" | "&&" | ";" | "&") {
            segments.push(&words[start..i]);
            start = i + 1;
        }
    }
    segments.push(&words[start..]);
    let segments: Vec<&[String]> = segments
        .into_iter()
        .filter(|s| !s.is_empty() && s[0] != "cd")
        .collect();

    let written = heredoc_lines(&command);
    for seg in &segments {
        let echoed: Vec<String> = match seg[0].as_str() {
            "echo" | "printf" => seg[1..]
                .iter()
                .take_while(|w| !w.starts_with('>'))
                .filter(|w| !w.starts_with('-'))
                .cloned()
                .collect::<Vec<_>>()
                .join(" ")
                .lines()
                .map(str::to_string)
                .collect(),
            _ => Vec::new(),
        };
        let added = if echoed.is_empty() {
            written.clone()
        } else {
            echoed
        };
        let mut found: Option<(&str, String)> = None;
        for (i, w) in seg.iter().enumerate() {
            let next = seg.get(i + 1).map(String::as_str).unwrap_or("");
            if w == ">>" {
                found = file_word(next).map(|p| ("edit", p));
            } else if w == ">" || w == ">|" {
                found = file_word(next).map(|p| ("write", p));
            }
            if found.is_some() {
                break;
            }
        }
        if found.is_none() {
            let cmd = seg[0].as_str();
            let args: Vec<&String> = seg[1..]
                .iter()
                .filter(|w| !w.starts_with('<') && !w.starts_with('>') && !w.contains('>'))
                .collect();
            let last_file = || args.iter().rev().find_map(|w| file_word(w));
            found = match cmd {
                "tee" => {
                    let append = seg[1..].iter().any(|w| w == "-a" || w == "--append");
                    args.iter()
                        .find_map(|w| file_word(w))
                        .map(|p| (if append { "edit" } else { "write" }, p))
                }
                "touch" => last_file().map(|p| ("write", p)),
                "sed" | "perl"
                    if seg[1..]
                        .iter()
                        .any(|w| w.starts_with('-') && !w.starts_with("--") && w.contains('i')) =>
                {
                    last_file().map(|p| ("edit", p))
                }
                _ => None,
            };
        }
        if let Some((kind, path)) = found {
            step.kind = kind.into();
            step.path = path;
            step.additions = added.len() as u64;
            step.added_lines = added;
            return;
        }
    }
    for seg in &segments {
        let cmd = seg[0].as_str();
        let has_redirect = seg.iter().any(|w| w.contains('>') || w.starts_with('<'));
        if matches!(
            cmd,
            "cat" | "bat" | "head" | "tail" | "less" | "more" | "nl" | "wc" | "stat"
        ) && !has_redirect
        {
            if let Some(path) = seg[1..].iter().rev().find_map(|w| file_word(w)) {
                step.kind = "read".into();
                step.path = path;
                return;
            }
        }
    }
    if let Some(seg) = segments.first() {
        if matches!(
            seg[0].as_str(),
            "curl" | "wget" | "rg" | "grep" | "egrep" | "find" | "fd" | "ag"
        ) {
            step.kind = "search".into();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn bash(command: &str) -> ToolStep {
        tool_step("t", "Bash", &json!({ "command": command }))
    }

    #[test]
    fn terminal_commands_name_the_file_they_touch() {
        let step = bash("cat > /tmp/test_demo.txt << 'EOF'\nhello\nworld\nEOF");
        assert_eq!(
            (step.kind.as_str(), step.path.as_str(), step.additions),
            ("write", "/tmp/test_demo.txt", 2)
        );
        assert_eq!(step.added_lines, vec!["hello", "world"]);

        let step = bash("echo \"third line\" >> /tmp/test_demo.txt");
        assert_eq!(
            (step.kind.as_str(), step.path.as_str()),
            ("edit", "/tmp/test_demo.txt")
        );
        assert_eq!(step.added_lines, vec!["third line"]);

        let step = bash("cd /repo && head -n 20 src/chat.ts | grep foo");
        assert_eq!(
            (step.kind.as_str(), step.path.as_str()),
            ("read", "src/chat.ts")
        );

        let step = bash("sed -i '' 's/a/b/' notes.md");
        assert_eq!(
            (step.kind.as_str(), step.path.as_str()),
            ("edit", "notes.md")
        );

        assert_eq!(bash("npm run build 2>&1 > /dev/null").kind, "terminal");
        assert_eq!(bash("ls -la /tmp 2>&1").kind, "terminal");
        assert_eq!(bash("curl -s https://example.com").kind, "search");
    }

    #[test]
    fn file_tools_carry_lines_and_changes() {
        let read = tool_step(
            "r",
            "Read",
            &json!({"file_path": "src/send.ts", "offset": 330, "limit": 80}),
        );
        assert_eq!(
            (read.kind.as_str(), read.line_start, read.line_end),
            ("read", Some(330), Some(409))
        );

        let edit = tool_step(
            "e",
            "Edit",
            &json!({"file_path": "a.ts", "old_string": "one\ntwo", "new_string": "one\nthree\nfour", "description": ""}),
        );
        assert_eq!(
            (edit.kind.as_str(), edit.additions, edit.deletions),
            ("edit", 3, 2)
        );

        let titled = tool_step(
            "b",
            "Bash",
            &json!({"command": "git status", "description": "查看改动"}),
        );
        assert_eq!(
            (titled.kind.as_str(), titled.title.as_str()),
            ("terminal", "查看改动")
        );
    }

    #[test]
    fn results_and_codex_items() {
        let result = tool_step_result("t", "Exit code 2\nboom", true);
        assert_eq!(
            (result.status.as_str(), result.exit_code),
            ("failed", Some(2))
        );

        let item = json!({
            "id": "c1", "type": "fileChange", "status": "completed",
            "changes": [
                {"path": "a.ts", "kind": {"type": "update"}, "diff": "@@ -1 +1,2 @@\n-x\n+y\n+z"},
                {"path": "b.ts", "kind": {"type": "add"}, "diff": "new\nfile"}
            ]
        });
        let steps = codex_tool_steps(&item, "fileChange", true);
        assert_eq!(steps.len(), 2);
        assert_eq!(
            (steps[0].id.as_str(), steps[0].additions, steps[0].deletions),
            ("c1#0", 2, 1)
        );
        assert_eq!((steps[1].kind.as_str(), steps[1].additions), ("write", 2));

        let cmd = json!({
            "id": "c2", "type": "commandExecution", "command": "cat README.md", "status": "completed",
            "aggregatedOutput": "hi", "exitCode": 0,
            "commandActions": [{"type": "read", "path": "README.md", "name": "README.md"}]
        });
        let step = &codex_tool_steps(&cmd, "commandExecution", true)[0];
        assert_eq!(
            (
                step.kind.as_str(),
                step.path.as_str(),
                step.output.as_str(),
                step.exit_code
            ),
            ("read", "README.md", "hi", Some(0))
        );
    }
}
