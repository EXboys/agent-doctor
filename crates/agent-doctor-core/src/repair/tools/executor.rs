use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use super::diff::unified_diff;
use super::discover::{grep_allowed_files, list_allowed_directory};
use super::patch::patch_structured_file;
use super::policy::{allowed_paths_for_runtime, bash_command_allowed, path_is_allowed};
use crate::repair::mask::{is_env_file, mask_config_file, unmask_file_content, SecretVault};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepairToolKind {
    Read,
    ListDir,
    GrepFiles,
    WriteFile,
    SearchReplace,
    PatchConfig,
    Bash,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepairToolCall {
    pub kind: RepairToolKind,
    pub path: Option<String>,
    pub content: Option<String>,
    pub old_string: Option<String>,
    pub new_string: Option<String>,
    pub key_path: Option<String>,
    pub value: Option<String>,
    pub pattern: Option<String>,
    pub command: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepairToolResult {
    pub kind: RepairToolKind,
    pub success: bool,
    pub masked_output: String,
    pub applied: bool,
    pub preview_diff: Option<String>,
    pub error: Option<String>,
}

pub struct RepairToolExecutor {
    runtime_id: String,
    allowed_paths: Vec<PathBuf>,
    vault: SecretVault,
    apply_writes: bool,
}

impl RepairToolExecutor {
    pub fn new(runtime_id: &str, vault: SecretVault, apply_writes: bool) -> Self {
        Self {
            runtime_id: runtime_id.to_string(),
            allowed_paths: allowed_paths_for_runtime(runtime_id),
            vault,
            apply_writes,
        }
    }

    pub fn with_allowed_paths(
        runtime_id: &str,
        vault: SecretVault,
        apply_writes: bool,
        allowed_paths: Vec<PathBuf>,
    ) -> Self {
        Self {
            runtime_id: runtime_id.to_string(),
            allowed_paths,
            vault,
            apply_writes,
        }
    }

    pub fn vault(&self) -> &SecretVault {
        &self.vault
    }

    pub fn vault_mut(&mut self) -> &mut SecretVault {
        &mut self.vault
    }

    pub fn into_vault(self) -> SecretVault {
        self.vault
    }

    pub fn execute(&mut self, call: &RepairToolCall) -> Result<RepairToolResult> {
        match call.kind {
            RepairToolKind::Read => self.read_file(call.path.as_deref()),
            RepairToolKind::ListDir => self.list_dir(call.path.as_deref()),
            RepairToolKind::GrepFiles => {
                self.grep_files(call.path.as_deref(), call.pattern.as_deref())
            }
            RepairToolKind::WriteFile => {
                self.write_file(call.path.as_deref(), call.content.as_deref())
            }
            RepairToolKind::SearchReplace => self.search_replace(
                call.path.as_deref(),
                call.old_string.as_deref(),
                call.new_string.as_deref(),
            ),
            RepairToolKind::PatchConfig => self.patch_config(
                call.path.as_deref(),
                call.key_path.as_deref(),
                call.value.as_deref(),
            ),
            RepairToolKind::Bash => self.run_bash(call.command.as_deref()),
        }
    }

    fn read_file(&mut self, path: Option<&str>) -> Result<RepairToolResult> {
        let path = match self.resolve_path(path, RepairToolKind::Read) {
            Ok(path) => path,
            Err(result) => return Ok(result),
        };

        if !path.exists() {
            return Ok(ok(
                RepairToolKind::Read,
                false,
                None,
                format!("(file not found: {})", path.display()),
            ));
        }

        let raw = fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let mut snippet_vault = SecretVault::default();
        let masked = mask_config_file(&path, &raw, &mut snippet_vault);
        self.vault.absorb(snippet_vault);

        Ok(ok(
            RepairToolKind::Read,
            false,
            None,
            format_numbered_lines(&masked),
        ))
    }

    fn list_dir(&mut self, path: Option<&str>) -> Result<RepairToolResult> {
        if let Some(path) = path {
            let path = PathBuf::from(path);
            if !path_is_allowed(&path, &self.allowed_paths) {
                return Ok(failed(
                    RepairToolKind::ListDir,
                    false,
                    format!("path not allowed: {}", path.display()),
                ));
            }
        }

        match list_allowed_directory(path.map(Path::new), &self.allowed_paths) {
            Ok(output) => Ok(ok(RepairToolKind::ListDir, false, None, output)),
            Err(error) => Ok(failed(RepairToolKind::ListDir, false, error.to_string())),
        }
    }

    fn grep_files(
        &mut self,
        path: Option<&str>,
        pattern: Option<&str>,
    ) -> Result<RepairToolResult> {
        let Some(pattern) = pattern else {
            return Ok(failed(
                RepairToolKind::GrepFiles,
                false,
                "grep_files requires pattern".to_string(),
            ));
        };

        let path_buf = path.map(PathBuf::from);
        if let Some(ref path) = path_buf {
            if !path_is_allowed(path, &self.allowed_paths) {
                return Ok(failed(
                    RepairToolKind::GrepFiles,
                    false,
                    format!("path not allowed: {}", path.display()),
                ));
            }
        }

        match grep_allowed_files(
            pattern,
            path_buf.as_deref(),
            &self.allowed_paths,
            &mut self.vault,
        ) {
            Ok(output) => Ok(ok(RepairToolKind::GrepFiles, false, None, output)),
            Err(error) => Ok(failed(RepairToolKind::GrepFiles, false, error.to_string())),
        }
    }

    fn write_file(
        &mut self,
        path: Option<&str>,
        content: Option<&str>,
    ) -> Result<RepairToolResult> {
        let path = match self.resolve_path(path, RepairToolKind::WriteFile) {
            Ok(path) => path,
            Err(result) => return Ok(result),
        };
        let Some(content) = content else {
            return Ok(failed(
                RepairToolKind::WriteFile,
                false,
                "write_file requires content".to_string(),
            ));
        };

        let original = self.read_original(&path);
        let unmasked = unmask_file_content(&path, original.as_deref(), content, &self.vault);

        if !self.apply_writes {
            let before = original.unwrap_or_default();
            let diff = unified_diff(&path.display().to_string(), &before, &unmasked);
            return Ok(ok(
                RepairToolKind::WriteFile,
                false,
                Some(mask_diff(&diff, &path, &mut self.vault)),
                format!("preview: would write {} byte(s)", unmasked.len()),
            ));
        }

        self.write_text(&path, &unmasked)?;
        Ok(ok(
            RepairToolKind::WriteFile,
            true,
            None,
            format!("wrote {} byte(s) to {}", unmasked.len(), path.display()),
        ))
    }

    fn search_replace(
        &mut self,
        path: Option<&str>,
        old_string: Option<&str>,
        new_string: Option<&str>,
    ) -> Result<RepairToolResult> {
        let path = match self.resolve_path(path, RepairToolKind::SearchReplace) {
            Ok(path) => path,
            Err(result) => return Ok(result),
        };
        let Some(old_string) = old_string else {
            return Ok(failed(
                RepairToolKind::SearchReplace,
                false,
                "search_replace requires old_string".to_string(),
            ));
        };
        let new_string = new_string.unwrap_or("");

        if !path.exists() {
            return Ok(failed(
                RepairToolKind::SearchReplace,
                false,
                format!("file not found: {}", path.display()),
            ));
        }

        let original = self
            .read_original(&path)
            .context("search_replace requires readable file")?;
        let old_unmasked = self.vault.restore_tokens_in_text(old_string);
        let new_unmasked = self.vault.restore_tokens_in_text(new_string);

        let matches = count_occurrences(&original, &old_unmasked);
        if matches == 0 {
            return Ok(failed(
                RepairToolKind::SearchReplace,
                false,
                "old_string not found — copy exact text from read_file (without line numbers)"
                    .to_string(),
            ));
        }
        if matches > 1 {
            return Ok(failed(
                RepairToolKind::SearchReplace,
                false,
                format!("old_string matched {matches} times — include more surrounding context"),
            ));
        }

        let updated = original.replacen(&old_unmasked, &new_unmasked, 1);

        if !self.apply_writes {
            let diff = unified_diff(&path.display().to_string(), &original, &updated);
            return Ok(ok(
                RepairToolKind::SearchReplace,
                false,
                Some(mask_diff(&diff, &path, &mut self.vault)),
                "preview: search_replace ready".to_string(),
            ));
        }

        self.write_text(&path, &updated)?;
        Ok(ok(
            RepairToolKind::SearchReplace,
            true,
            None,
            format!("replaced 1 occurrence in {}", path.display()),
        ))
    }

    fn patch_config(
        &mut self,
        path: Option<&str>,
        key_path: Option<&str>,
        value: Option<&str>,
    ) -> Result<RepairToolResult> {
        let path = match self.resolve_path(path, RepairToolKind::PatchConfig) {
            Ok(path) => path,
            Err(result) => return Ok(result),
        };
        let Some(key_path) = key_path else {
            return Ok(failed(
                RepairToolKind::PatchConfig,
                false,
                "patch_config requires key_path".to_string(),
            ));
        };
        let Some(value) = value else {
            return Ok(failed(
                RepairToolKind::PatchConfig,
                false,
                "patch_config requires value".to_string(),
            ));
        };

        if !path.exists() {
            return Ok(failed(
                RepairToolKind::PatchConfig,
                false,
                format!("file not found: {}", path.display()),
            ));
        }

        let original = self
            .read_original(&path)
            .context("patch_config requires readable file")?;
        let unmasked_value = self.vault.restore_tokens_in_text(value);
        let updated = patch_structured_file(&path, key_path, &unmasked_value)?;

        if !self.apply_writes {
            let diff = unified_diff(&path.display().to_string(), &original, &updated);
            return Ok(ok(
                RepairToolKind::PatchConfig,
                false,
                Some(mask_diff(&diff, &path, &mut self.vault)),
                format!("preview: would set {key_path}"),
            ));
        }

        self.write_text(&path, &updated)?;
        Ok(ok(
            RepairToolKind::PatchConfig,
            true,
            None,
            format!("patched {key_path} in {}", path.display()),
        ))
    }

    fn run_bash(&mut self, command: Option<&str>) -> Result<RepairToolResult> {
        let Some(command) = command else {
            return Ok(failed(
                RepairToolKind::Bash,
                false,
                "bash requires command".to_string(),
            ));
        };

        let command = self.vault.restore_tokens_in_text(command);
        if !bash_command_allowed(&self.runtime_id, &command) {
            return Ok(failed(
                RepairToolKind::Bash,
                false,
                "command not on repair allowlist".to_string(),
            ));
        }

        if !self.apply_writes {
            return Ok(ok(
                RepairToolKind::Bash,
                false,
                None,
                format!("preview: would run `{command}`"),
            ));
        }

        let output = run_shell(&command)?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !output.status.success() {
            let detail = if stderr.trim().is_empty() {
                stdout.trim().to_string()
            } else {
                stderr.trim().to_string()
            };
            return Ok(failed(
                RepairToolKind::Bash,
                true,
                if detail.is_empty() {
                    format!("command failed: {:?}", output.status.code())
                } else {
                    detail
                },
            ));
        }

        let combined = if stderr.trim().is_empty() {
            stdout.trim().to_string()
        } else {
            format!("{}\n{}", stdout.trim(), stderr.trim())
        };
        let mut snippet_vault = SecretVault::default();
        let masked = snippet_vault.mask_text(&combined);
        self.vault.absorb(snippet_vault);

        Ok(ok(RepairToolKind::Bash, true, None, masked))
    }

    fn resolve_path(
        &self,
        path: Option<&str>,
        kind: RepairToolKind,
    ) -> std::result::Result<PathBuf, RepairToolResult> {
        let Some(path) = path else {
            return Err(failed(kind, false, "path is required".to_string()));
        };
        let path = PathBuf::from(path);
        if !path_is_allowed(&path, &self.allowed_paths) {
            return Err(failed(
                kind,
                false,
                format!("path not allowed: {}", path.display()),
            ));
        }
        Ok(path)
    }

    fn read_original(&self, path: &Path) -> Option<String> {
        if path.exists() {
            fs::read_to_string(path).ok()
        } else {
            None
        }
    }

    fn write_text(&mut self, path: &Path, content: &str) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        fs::write(path, content).with_context(|| format!("failed to write {}", path.display()))?;

        if is_env_file(path) {
            let mut refresh_vault = SecretVault::default();
            let _ = mask_config_file(path, content, &mut refresh_vault);
            self.vault.absorb(refresh_vault);
        }
        Ok(())
    }
}

fn format_numbered_lines(masked: &str) -> String {
    masked
        .lines()
        .enumerate()
        .map(|(index, line)| format!("{:>6}|{line}", index + 1))
        .collect::<Vec<_>>()
        .join("\n")
}

fn count_occurrences(haystack: &str, needle: &str) -> usize {
    if needle.is_empty() {
        return 0;
    }
    haystack.match_indices(needle).count()
}

fn mask_diff(diff: &str, path: &Path, vault: &mut SecretVault) -> String {
    if diff.is_empty() {
        return diff.to_string();
    }
    mask_config_file(path, diff, vault)
}

fn run_shell(command_line: &str) -> Result<std::process::Output> {
    use std::time::Duration;
    let timeout = Duration::from_secs(120);
    #[cfg(unix)]
    let output = crate::exec::run_output("bash", &["-c", command_line], timeout)
        .map_err(|err| anyhow::anyhow!("{err}"))
        .context("failed to start bash")?;

    #[cfg(windows)]
    let output = crate::exec::run_output("cmd", &["/C", command_line], timeout)
        .map_err(|err| anyhow::anyhow!("{err}"))
        .context("failed to start cmd")?;

    Ok(output)
}

fn ok(
    kind: RepairToolKind,
    applied: bool,
    preview_diff: Option<String>,
    masked_output: String,
) -> RepairToolResult {
    RepairToolResult {
        kind,
        success: true,
        masked_output,
        applied,
        preview_diff,
        error: None,
    }
}

fn failed(kind: RepairToolKind, applied: bool, error: String) -> RepairToolResult {
    RepairToolResult {
        kind,
        success: false,
        masked_output: String::new(),
        applied,
        preview_diff: None,
        error: Some(error),
    }
}

pub fn parse_tool_call(name: &str, arguments: &str) -> Result<RepairToolCall> {
    let value: serde_json::Value =
        serde_json::from_str(arguments).context("tool arguments must be JSON object")?;
    let path = value
        .get("path")
        .and_then(|item| item.as_str())
        .map(str::to_string);
    let content = value
        .get("content")
        .and_then(|item| item.as_str())
        .map(str::to_string);
    let old_string = value
        .get("old_string")
        .and_then(|item| item.as_str())
        .map(str::to_string);
    let new_string = value
        .get("new_string")
        .and_then(|item| item.as_str())
        .map(str::to_string);
    let key_path = value
        .get("key_path")
        .and_then(|item| item.as_str())
        .map(str::to_string);
    let value_field = value
        .get("value")
        .and_then(|item| item.as_str())
        .map(str::to_string);
    let command = value
        .get("command")
        .and_then(|item| item.as_str())
        .map(str::to_string);
    let pattern = value
        .get("pattern")
        .and_then(|item| item.as_str())
        .map(str::to_string);

    let kind = match name {
        "read_file" => RepairToolKind::Read,
        "list_dir" => RepairToolKind::ListDir,
        "grep_files" => RepairToolKind::GrepFiles,
        "write_file" | "edit_file" => RepairToolKind::WriteFile,
        "search_replace" => RepairToolKind::SearchReplace,
        "patch_config" => RepairToolKind::PatchConfig,
        "bash" => RepairToolKind::Bash,
        other => bail!("unknown tool: {other}"),
    };

    Ok(RepairToolCall {
        kind,
        path,
        content,
        old_string,
        new_string,
        key_path,
        value: value_field,
        pattern,
        command,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn executor(dir: &TempDir, apply: bool) -> RepairToolExecutor {
        RepairToolExecutor::with_allowed_paths(
            "hermes",
            SecretVault::default(),
            apply,
            vec![dir.path().to_path_buf()],
        )
    }

    #[test]
    fn read_masks_secrets_and_write_restores_them() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join(".env");
        let mut file = std::fs::File::create(&path).expect("create");
        writeln!(file, "DEEPSEEK_API_KEY=sk-test-secret-key").expect("write");

        let mut exec = executor(&dir, true);
        let read = exec
            .execute(&RepairToolCall {
                kind: RepairToolKind::Read,
                path: Some(path.display().to_string()),
                content: None,
                old_string: None,
                new_string: None,
                key_path: None,
                value: None,
                pattern: None,
                command: None,
            })
            .expect("read");
        assert!(read.success);
        assert!(read.masked_output.contains("{{SECRET:"));
        assert!(read.masked_output.contains("|DEEPSEEK_API_KEY="));

        let body = read
            .masked_output
            .lines()
            .map(|line| line.split_once('|').map(|(_, rest)| rest).unwrap_or(line))
            .collect::<Vec<_>>()
            .join("\n");
        let write = exec
            .execute(&RepairToolCall {
                kind: RepairToolKind::WriteFile,
                path: Some(path.display().to_string()),
                content: Some(body),
                old_string: None,
                new_string: None,
                key_path: None,
                value: None,
                pattern: None,
                command: None,
            })
            .expect("write");
        assert!(write.success);
        assert!(write.applied);

        let on_disk = std::fs::read_to_string(&path).expect("read back");
        assert!(on_disk.contains("sk-test-secret-key"));
    }

    #[test]
    fn search_replace_requires_unique_match() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("config.txt");
        std::fs::write(&path, "foo\nfoo\n").expect("write");

        let mut exec = executor(&dir, true);
        let fail = exec
            .execute(&RepairToolCall {
                kind: RepairToolKind::SearchReplace,
                path: Some(path.display().to_string()),
                content: None,
                old_string: Some("foo".to_string()),
                new_string: Some("bar".to_string()),
                key_path: None,
                value: None,
                pattern: None,
                command: None,
            })
            .expect("replace");
        assert!(!fail.success);
        assert!(fail.error.unwrap().contains("matched 2 times"));
    }

    #[test]
    fn search_replace_preview_includes_diff() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("config.txt");
        std::fs::write(&path, "provider: openai\n").expect("write");

        let mut exec = executor(&dir, false);
        let preview = exec
            .execute(&RepairToolCall {
                kind: RepairToolKind::SearchReplace,
                path: Some(path.display().to_string()),
                content: None,
                old_string: Some("openai".to_string()),
                new_string: Some("deepseek".to_string()),
                key_path: None,
                value: None,
                pattern: None,
                command: None,
            })
            .expect("preview");
        assert!(preview.success);
        assert!(!preview.applied);
        assert!(preview
            .preview_diff
            .unwrap()
            .contains("+provider: deepseek"));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "provider: openai\n"
        );
    }

    #[test]
    fn write_preview_does_not_write() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("notes.txt");
        let mut exec = executor(&dir, false);
        let result = exec
            .execute(&RepairToolCall {
                kind: RepairToolKind::WriteFile,
                path: Some(path.display().to_string()),
                content: Some("hello".to_string()),
                old_string: None,
                new_string: None,
                key_path: None,
                value: None,
                pattern: None,
                command: None,
            })
            .expect("write preview");
        assert!(result.success);
        assert!(!result.applied);
        assert!(!path.exists());
    }

    #[test]
    fn list_dir_shows_children() {
        let dir = TempDir::new().expect("tempdir");
        std::fs::write(dir.path().join("config.yaml"), "ok").expect("write");
        let mut exec = executor(&dir, false);
        let result = exec
            .execute(&RepairToolCall {
                kind: RepairToolKind::ListDir,
                path: Some(dir.path().display().to_string()),
                content: None,
                old_string: None,
                new_string: None,
                key_path: None,
                value: None,
                pattern: None,
                command: None,
            })
            .expect("list");
        assert!(result.success);
        assert!(result.masked_output.contains("config.yaml"));
    }

    #[test]
    fn grep_files_returns_masked_hits() {
        let dir = TempDir::new().expect("tempdir");
        std::fs::write(dir.path().join("config.yaml"), "provider: openai\n").expect("write");
        let mut exec = executor(&dir, false);
        let result = exec
            .execute(&RepairToolCall {
                kind: RepairToolKind::GrepFiles,
                path: Some(dir.path().display().to_string()),
                content: None,
                old_string: None,
                new_string: None,
                key_path: None,
                value: None,
                pattern: Some("provider".to_string()),
                command: None,
            })
            .expect("grep");
        assert!(result.success);
        assert!(result.masked_output.contains("config.yaml"));
        assert!(result.masked_output.contains("provider"));
    }
}
