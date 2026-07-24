//! Local job executors — Claude Code / Codex CLI + OpenClaw / Hermes hooks.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::doctor::run_doctor;
use crate::setup::evotown_agent_env_path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssignedJob {
    pub job_id: String,
    pub run_id: String,
    pub kind: String,
    pub title: String,
    pub message: String,
    pub payload: Value,
    pub refs: Value,
    pub runtime: String,
    pub cwd: String,
    pub timeout_sec: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobResult {
    pub status: String,
    pub exit_code: i32,
    pub result_summary: String,
    pub log_excerpt: String,
    pub runtime: String,
    pub signals: Value,
}

impl AssignedJob {
    pub fn from_assign_message(msg: &Value) -> Result<Self> {
        let job = msg
            .get("job")
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("job.assign missing job"))?;
        let payload = job.get("payload").cloned().unwrap_or(json!({}));
        let runtime = job
            .get("runtime")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .or_else(|| payload.get("runtime").and_then(|v| v.as_str()))
            .or_else(|| payload.get("runtime_hint").and_then(|v| v.as_str()))
            .unwrap_or("")
            .to_string();
        let timeout_sec = job
            .get("timeout_sec")
            .and_then(|v| v.as_u64())
            .or_else(|| payload.get("timeout_sec").and_then(|v| v.as_u64()))
            .unwrap_or(600)
            .clamp(30, 3600);
        Ok(Self {
            job_id: job
                .get("job_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            run_id: job
                .get("run_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            kind: job
                .get("kind")
                .and_then(|v| v.as_str())
                .unwrap_or("dispatch")
                .to_string(),
            title: job
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            message: job
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            payload,
            refs: job.get("refs").cloned().unwrap_or(json!({})),
            runtime,
            cwd: job
                .get("cwd")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            timeout_sec,
        })
    }

    pub fn prompt(&self) -> String {
        if self.title.is_empty() {
            self.message.clone()
        } else {
            format!("{}\n\n{}", self.title, self.message)
        }
    }
}

fn load_agent_env() -> std::collections::HashMap<String, String> {
    let mut values = std::collections::HashMap::new();
    if let Some(path) = evotown_agent_env_path() {
        if let Ok(raw) = std::fs::read_to_string(path) {
            for line in raw.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') || !line.contains('=') {
                    continue;
                }
                if let Some((k, v)) = line.split_once('=') {
                    values.insert(k.trim().to_string(), v.trim().trim_matches('"').to_string());
                }
            }
        }
    }
    for key in [
        "EVOTOWN_RUNTIME",
        "OPENCLAW_HOOK_URL",
        "OPENCLAW_HOOK_TOKEN",
        "OPENCLAW_AGENT_ID",
        "HERMES_HOOK_URL",
        "HERMES_HOOK_TOKEN",
        "EVOTOWN_HOOK_TOKEN",
        "AGENT_DOCTOR_CLAUDE_BIN",
        "AGENT_DOCTOR_CODEX_BIN",
        "AGENT_DOCTOR_JOB_CWD",
    ] {
        if let Ok(v) = std::env::var(key) {
            values.insert(key.to_string(), v);
        }
    }
    values
}

pub fn resolve_runtime(job: &AssignedJob) -> String {
    if !job.runtime.is_empty() {
        return normalize_runtime(&job.runtime);
    }
    let env = load_agent_env();
    if let Some(r) = env.get("EVOTOWN_RUNTIME") {
        return normalize_runtime(r);
    }
    // Prefer an installed coding agent, then openclaw/hermes
    let report = run_doctor();
    let installed: Vec<&str> = report
        .runtimes
        .iter()
        .filter(|r| r.installed)
        .map(|r| r.id.as_str())
        .collect();
    for preferred in ["claude-code", "codex", "openclaw", "hermes"] {
        if installed.contains(&preferred) {
            return preferred.to_string();
        }
    }
    "claude-code".to_string()
}

fn normalize_runtime(raw: &str) -> String {
    match raw.trim().to_lowercase().as_str() {
        "claude" | "claude_code" | "claude-code" | "claudecode" => "claude-code".into(),
        "codex" | "openai-codex" => "codex".into(),
        "openclaw" | "claw" => "openclaw".into(),
        "hermes" => "hermes".into(),
        other => other.to_string(),
    }
}

pub fn execute_job(job: &AssignedJob) -> JobResult {
    let runtime = resolve_runtime(job);
    eprintln!(
        "→ executing job_id={} runtime={} timeout={}s",
        job.job_id, runtime, job.timeout_sec
    );
    let result = match runtime.as_str() {
        "claude-code" => run_claude_cli(job),
        "codex" => run_codex_cli(job),
        "openclaw" => run_openclaw_hook(job),
        "hermes" => run_hermes_hook(job),
        other => Err(anyhow::anyhow!("unsupported runtime '{other}'")),
    };
    match result {
        Ok(mut ok) => {
            ok.runtime = runtime;
            ok
        }
        Err(err) => JobResult {
            status: "failed".into(),
            exit_code: 1,
            result_summary: format!("{err:#}"),
            log_excerpt: format!("{err:#}"),
            runtime,
            signals: json!({ "error": true }),
        },
    }
}

fn workdir(job: &AssignedJob) -> PathBuf {
    let env = load_agent_env();
    if !job.cwd.is_empty() {
        return PathBuf::from(&job.cwd);
    }
    if let Some(cwd) = env.get("AGENT_DOCTOR_JOB_CWD") {
        if !cwd.is_empty() {
            return PathBuf::from(cwd);
        }
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn run_claude_cli(job: &AssignedJob) -> Result<JobResult> {
    let env = load_agent_env();
    let bin = env
        .get("AGENT_DOCTOR_CLAUDE_BIN")
        .cloned()
        .unwrap_or_else(|| "claude".into());
    let cwd = workdir(job);
    let prompt = job.prompt();
    let mut cmd = Command::new(&bin);
    cmd.arg("-p")
        .arg(&prompt)
        .arg("--output-format")
        .arg("text")
        .current_dir(&cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Allow non-interactive automation when explicitly requested via payload
    if job
        .payload
        .get("dangerously_skip_permissions")
        .and_then(|v| v.as_bool())
        == Some(true)
    {
        cmd.arg("--dangerously-skip-permissions");
    }
    let output = run_command_with_timeout(cmd, job.timeout_sec)?;
    Ok(command_result(output, "claude-code"))
}

fn run_codex_cli(job: &AssignedJob) -> Result<JobResult> {
    let env = load_agent_env();
    let bin = env
        .get("AGENT_DOCTOR_CODEX_BIN")
        .cloned()
        .unwrap_or_else(|| "codex".into());
    let cwd = workdir(job);
    let prompt = job.prompt();
    // Prefer modern `codex exec`; fall back to positional prompt.
    let mut cmd = Command::new(&bin);
    cmd.arg("exec")
        .arg(&prompt)
        .current_dir(&cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if job.payload.get("full_auto").and_then(|v| v.as_bool()) == Some(true) {
        cmd.arg("--full-auto");
    }
    let output = match run_command_with_timeout(cmd, job.timeout_sec) {
        Ok(out) => out,
        Err(_) => {
            let mut cmd2 = Command::new(&bin);
            cmd2.arg(&prompt)
                .current_dir(&cwd)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            run_command_with_timeout(cmd2, job.timeout_sec)?
        }
    };
    Ok(command_result(output, "codex"))
}

fn command_result(output: std::process::Output, runtime: &str) -> JobResult {
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let combined = if stderr.trim().is_empty() {
        stdout.clone()
    } else {
        format!("{stdout}\n{stderr}")
    };
    let excerpt: String = combined.chars().take(6000).collect();
    let summary: String = combined
        .chars()
        .take(1500)
        .collect::<String>()
        .trim()
        .to_string();
    let ok = output.status.success();
    JobResult {
        status: if ok { "succeeded" } else { "failed" }.into(),
        exit_code: output.status.code().unwrap_or(1),
        result_summary: if summary.is_empty() {
            if ok {
                format!("{runtime} completed")
            } else {
                format!("{runtime} failed")
            }
        } else {
            summary
        },
        log_excerpt: excerpt,
        runtime: runtime.into(),
        signals: json!({ "runtime": runtime }),
    }
}

fn run_command_with_timeout(mut cmd: Command, timeout_sec: u64) -> Result<std::process::Output> {
    let child = cmd.spawn().context("failed to spawn runtime CLI")?;
    let pid = child.id();
    let (tx, rx) = std::sync::mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });
    match rx.recv_timeout(Duration::from_secs(timeout_sec)) {
        Ok(result) => result.context("wait_with_output failed"),
        Err(_) => {
            let _ = Command::new("kill").arg("-9").arg(pid.to_string()).status();
            bail!("runtime timed out after {timeout_sec}s (killed pid {pid})")
        }
    }
}

fn run_openclaw_hook(job: &AssignedJob) -> Result<JobResult> {
    let env = load_agent_env();
    let url = env
        .get("OPENCLAW_HOOK_URL")
        .cloned()
        .unwrap_or_else(|| "http://127.0.0.1:18789/hooks/agent".into());
    let token = env
        .get("OPENCLAW_HOOK_TOKEN")
        .cloned()
        .or_else(|| env.get("EVOTOWN_HOOK_TOKEN").cloned())
        .unwrap_or_default();
    if token.is_empty() {
        bail!("OPENCLAW_HOOK_TOKEN not set (must match OpenClaw hooks.token)");
    }
    let agent_id = env
        .get("OPENCLAW_AGENT_ID")
        .cloned()
        .unwrap_or_else(|| "main".into());
    let message = format!(
        "{}\n\n[evotown] job_id={} run_id={}\nWhen done, Agent Doctor will complete the job.",
        job.prompt(),
        job.job_id,
        job.run_id
    );
    let body = json!({
        "message": message,
        "name": "Evotown",
        "agentId": agent_id,
        "wakeMode": "now",
        "deliver": false,
        "timeoutSeconds": job.timeout_sec.min(600),
        "metadata": { "source": "agent-doctor", "job_id": job.job_id },
    });
    let (status, text) = http_post_json(&url, Some(&token), &body, job.timeout_sec)?;
    let ok = (200..300).contains(&status);
    Ok(JobResult {
        status: if ok { "succeeded" } else { "failed" }.into(),
        exit_code: if ok { 0 } else { 1 },
        result_summary: text.chars().take(1500).collect(),
        log_excerpt: text.chars().take(6000).collect(),
        runtime: "openclaw".into(),
        signals: json!({ "http_status": status }),
    })
}

fn run_hermes_hook(job: &AssignedJob) -> Result<JobResult> {
    let env = load_agent_env();
    let url = env
        .get("HERMES_HOOK_URL")
        .cloned()
        .unwrap_or_else(|| "http://127.0.0.1:18789/hooks/evotown".into());
    let token = env
        .get("HERMES_HOOK_TOKEN")
        .cloned()
        .or_else(|| env.get("EVOTOWN_HOOK_TOKEN").cloned());
    let body = json!({
        "message": job.prompt(),
        "job_id": job.job_id,
        "kind": job.kind,
        "refs": job.refs,
        "timeoutSeconds": job.timeout_sec.min(600),
    });
    let (status, text) = http_post_json(&url, token.as_deref(), &body, job.timeout_sec)?;
    let ok = (200..300).contains(&status);
    Ok(JobResult {
        status: if ok { "succeeded" } else { "failed" }.into(),
        exit_code: if ok { 0 } else { 1 },
        result_summary: text.chars().take(1500).collect(),
        log_excerpt: text.chars().take(6000).collect(),
        runtime: "hermes".into(),
        signals: json!({ "http_status": status }),
    })
}

fn http_post_json(
    url: &str,
    bearer: Option<&str>,
    body: &Value,
    timeout_sec: u64,
) -> Result<(u16, String)> {
    // Prefer ureq-like via std + tiny HTTP for no new deps: use reqwest blocking already in crate.
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(timeout_sec.clamp(5, 600)))
        .build()?;
    let mut req = client.post(url).json(body);
    if let Some(token) = bearer {
        if !token.is_empty() {
            req = req.bearer_auth(token);
        }
    }
    let resp = req.send().with_context(|| format!("POST {url}"))?;
    let status = resp.status().as_u16();
    let text = resp.text().unwrap_or_default();
    Ok((status, text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_assign_message() {
        let msg = json!({
            "type": "job.assign",
            "job": {
                "job_id": "job_1",
                "run_id": "job_1",
                "kind": "dispatch",
                "title": "Hi",
                "message": "Say hello",
                "payload": { "runtime": "claude-code" },
                "refs": {},
                "runtime": "claude-code",
                "cwd": "/tmp",
                "timeout_sec": 120
            }
        });
        let job = AssignedJob::from_assign_message(&msg).unwrap();
        assert_eq!(job.job_id, "job_1");
        assert_eq!(resolve_runtime(&job), "claude-code");
    }

    #[test]
    fn normalize_aliases() {
        assert_eq!(normalize_runtime("claude"), "claude-code");
        assert_eq!(normalize_runtime("OpenClaw"), "openclaw");
    }
}
