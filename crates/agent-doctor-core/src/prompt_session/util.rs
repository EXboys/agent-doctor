use std::process::Child;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use super::{PromptSessionStatus, MAX_CAPTURE_CHARS, SUMMARY_CHARS};

pub(crate) fn push_capped(acc: &Arc<Mutex<String>>, line: &str) {
    if let Ok(mut guard) = acc.lock() {
        if guard.len() >= MAX_CAPTURE_CHARS {
            return;
        }
        let remain = MAX_CAPTURE_CHARS.saturating_sub(guard.len());
        if line.len() <= remain {
            if !guard.is_empty() {
                guard.push('\n');
            }
            guard.push_str(line);
        } else {
            guard.push_str(&line[..remain]);
        }
    }
}

pub(crate) fn combine_output(stdout: &str, stderr: &str) -> String {
    match (stdout.is_empty(), stderr.is_empty()) {
        (true, true) => String::new(),
        (false, true) => stdout.to_string(),
        (true, false) => stderr.to_string(),
        (false, false) => format!("{stdout}\n{stderr}"),
    }
}

pub(crate) fn summarize(combined: &str, status: &PromptSessionStatus, runtime: &str) -> String {
    let trimmed = combined.trim();
    if trimmed.is_empty() {
        return format!("{runtime} ask ended ({status:?}) with no captured output");
    }
    if trimmed.chars().count() <= SUMMARY_CHARS {
        return trimmed.to_string();
    }
    let truncated: String = trimmed.chars().take(SUMMARY_CHARS).collect();
    format!("{truncated}…")
}

pub(crate) fn force_stop_child(child: &mut Child, pid: u32) {
    #[cfg(unix)]
    {
        let _ = std::process::Command::new("pkill")
            .args(["-P", &pid.to_string()])
            .status();
        let _ = std::process::Command::new("kill")
            .args(["-9", &pid.to_string()])
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

pub(crate) fn join_reader(handle: thread::JoinHandle<()>, budget: Duration) {
    let start = Instant::now();
    while start.elapsed() < budget {
        if handle.is_finished() {
            let _ = handle.join();
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
}

pub(crate) fn is_runtime_stderr_noise(line: &str) -> bool {
    let t = line.trim();
    if t.is_empty() {
        return true;
    }
    let lower = t.to_ascii_lowercase();
    [
        "openai codex",
        "reading additional input from stdin",
        "workdir:",
        "approval:",
        "model:",
        "provider:",
        "session id:",
        "-------",
        "user",
        "assistant",
    ]
    .iter()
    .any(|p| lower.starts_with(p) || lower == *p)
}

/// Map Codex app-server / policy jargon into short Chinese UI copy.
pub(crate) fn humanize_runtime_error(raw: &str) -> String {
    let t = raw.trim();
    if t.is_empty() {
        return t.to_string();
    }
    let lower = t.to_ascii_lowercase();

    if lower.contains("apply_patch") && (lower.contains("hunk") || lower.contains("verification")) {
        return "写文件补丁格式不正确：每一段必须以 `*** Add File: 路径` / `*** Update File: 路径` / `*** Delete File: 路径` 开头，不能把文件内容写在标题行。模型应修正补丁后重试。".into();
    }
    if lower.contains("unlesstrusted") || lower.contains("unless trusted") {
        return "当前审批策略为「不信任除非已信任」(UnlessTrusted)，不能使用 require_escalated 提升权限。请改用普通命令，或关闭 elevated/全自动后重试。".into();
    }
    if lower.contains("require_escalated") || lower.contains("escalated permissions") {
        return "模型请求了提升权限，但当前策略不允许。请用普通工具操作，或在聊天里通过「允许 / 拒绝」确认。".into();
    }
    if lower.contains("unknown variant") && lower.contains("approval") {
        return format!("审批策略参数不被本地 Codex 接受。详情：{t}");
    }
    if lower.contains("unknown variant")
        && (lower.contains("sandbox") || lower.contains("workspace"))
    {
        return format!("沙箱参数与本地 Codex 版本不匹配。详情：{t}");
    }
    if lower.contains("invalid request") {
        return format!("Codex 请求无效（多为协议字段与 CLI 版本不一致）。详情：{t}");
    }
    t.to_string()
}

#[cfg(test)]
pub(crate) static TEST_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn humanizes_apply_patch_hunk_error() {
        let msg = humanize_runtime_error(
            "apply_patch verification failed: invalid hunk at line 3, 'hello from codex' is not a valid hunk header",
        );
        assert!(msg.contains("补丁") || msg.contains("Add File"));
    }

    #[test]
    fn humanizes_unless_trusted_escalate() {
        let msg = humanize_runtime_error(
            "approval policy is UnlessTrusted; reject command — you cannot ask for escalated permissions",
        );
        assert!(msg.contains("UnlessTrusted") || msg.contains("提升权限"));
        assert!(!msg.starts_with("approval policy is"));
    }

    #[test]
    fn humanizes_unknown_sandbox_variant() {
        let msg = humanize_runtime_error(
            "Invalid request: unknown variant `workspaceWrite`, expected one of `read-only`",
        );
        assert!(msg.contains("沙箱") || msg.contains("Codex"));
    }
}