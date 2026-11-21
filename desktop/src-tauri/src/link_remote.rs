//! Treat a linked LAN/SSH computer just like a passwordless remote VPS:
//! the agent runs on THIS computer and executes remote commands over the
//! already-trusted link (`--remote-exec` / `agent-doctor remote exec`),
//! exactly how it uses `ssh <vps> '<cmd>'` on a VPS.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;

use agent_doctor_core::exec::shell_quote;
use agent_doctor_core::remote::link_methods;
use agent_doctor_core::{
    register_link_method, LinkClient, LinkMethodOutcome, LinkResponse, LinkRpcContext,
    LINK_DAEMON_PORT,
};
use agent_doctor_prompt::{
    run_prompt_session_with_cancel, PromptSessionCancel, PromptSessionControl, PromptSessionEvent,
    PromptSessionOptions, PromptSessionReport,
};
use anyhow::Context;

use crate::link_session::LinkSessionState;

#[derive(Clone, Default)]
pub struct LinkedPromptIndex {
    by_client: Arc<Mutex<HashMap<String, String>>>,
    by_backend: Arc<Mutex<HashMap<String, String>>>,
}

impl LinkedPromptIndex {
    pub fn host_for_client(&self, client_run_id: &str) -> Option<String> {
        self.by_client
            .lock()
            .ok()
            .and_then(|g| g.get(client_run_id).cloned())
    }

    pub fn host_for_backend(&self, session_id: &str) -> Option<String> {
        let client = self
            .by_backend
            .lock()
            .ok()
            .and_then(|g| g.get(session_id).cloned())?;
        self.host_for_client(&client)
    }

    pub fn is_remote_client(&self, client_run_id: &str) -> bool {
        self.host_for_client(client_run_id).is_some()
    }
}

/// `hostId::projectName` → (hostId, projectName).
pub fn split_linked_workspace(name: Option<&str>) -> Option<(String, String)> {
    let raw = name?.trim();
    let (host, project) = raw.split_once("::")?;
    let host = host.trim();
    let project = project.trim();
    if host.is_empty() || project.is_empty() {
        return None;
    }
    Some((host.to_string(), project.to_string()))
}

/// Register `proxy_exec` on this computer's loopback daemon so local CLI tools
/// (`--remote-exec <host> <cmd>` or `agent-doctor remote exec <host> <cmd>`)
/// reuse the desktop app's passwordless LAN / SSH link session.
pub fn register_proxy_exec_method(link_sessions: LinkSessionState) {
    register_link_method(
        link_methods::PROXY_EXEC,
        Arc::new(move |ctx: LinkRpcContext| {
            let sessions = link_sessions.clone();
            thread::Builder::new()
                .name("ad-proxy-exec".into())
                .spawn(move || {
                    let outcome = (|| -> anyhow::Result<serde_json::Value> {
                        let host_id = ctx
                            .request
                            .params
                            .get("host_id")
                            .and_then(|v| v.as_str())
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .context("缺少远程电脑标识")?;
                        let script = ctx
                            .request
                            .params
                            .get("script")
                            .and_then(|v| v.as_str())
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .context("缺少要执行的命令")?;
                        let cwd = ctx
                            .request
                            .params
                            .get("cwd")
                            .and_then(|v| v.as_str())
                            .map(str::trim)
                            .filter(|s| !s.is_empty());
                        let mut client = sessions
                            .open_fresh_client(host_id)
                            .map_err(|e| anyhow::anyhow!("{e}"))?;
                        let res = client.exec(script, cwd)?;
                        Ok(serde_json::to_value(res)?)
                    })();
                    let response = match outcome {
                        Ok(result) => LinkResponse {
                            id: ctx.request.id,
                            ok: true,
                            result: Some(result),
                            error: None,
                        },
                        Err(err) => LinkResponse {
                            id: ctx.request.id,
                            ok: false,
                            result: None,
                            error: Some(format!("{err:#}")),
                        },
                    };
                    let _ = ctx.socket.send_response(&response);
                })
                .ok();
            LinkMethodOutcome::Deferred
        }),
    );
}

/// Handle `<app-exe> --remote-exec <host_id> <script...>` before Tauri starts,
/// behaving identically to `ssh <host> '<script>'`.
pub fn maybe_run_remote_exec_cli() -> bool {
    let args: Vec<String> = std::env::args().collect();
    let Some(pos) = args.iter().position(|a| a == "--remote-exec") else {
        return false;
    };
    let code = run_remote_exec_from_args(&args[pos + 1..]);
    std::process::exit(code);
}

fn run_remote_exec_from_args(rest: &[String]) -> i32 {
    use std::io::Write;
    let Some((host_id, tail)) = rest.split_first() else {
        eprintln!("usage: --remote-exec <host_id> [--cwd <dir>] <command...>");
        return 2;
    };
    let (cwd, cmd_parts) = if tail.len() >= 2 && tail[0] == "--cwd" {
        (Some(tail[1].as_str()), &tail[2..])
    } else {
        (None, tail)
    };
    let script = cmd_parts.join(" ");
    if script.trim().is_empty() {
        eprintln!("remote command must not be empty");
        return 2;
    }
    let mut client = match LinkClient::connect_local(LINK_DAEMON_PORT) {
        Ok(c) => c,
        Err(err) => {
            eprintln!("{err:#}");
            return 1;
        }
    };
    match client.proxy_exec(host_id.trim(), &script, cwd) {
        Ok(res) => {
            if !res.stdout.is_empty() {
                let _ = std::io::stdout().write_all(res.stdout.as_bytes());
                let _ = std::io::stdout().flush();
            }
            if !res.stderr.is_empty() {
                let _ = std::io::stderr().write_all(res.stderr.as_bytes());
                let _ = std::io::stderr().flush();
            }
            res.status
        }
        Err(err) => {
            eprintln!("{err:#}");
            1
        }
    }
}

fn remote_exec_command_prefix(host_id: &str) -> String {
    #[cfg(unix)]
    if let Ok(exe) = std::env::current_exe() {
        if exe.is_file() {
            return format!(
                "{} --remote-exec {}",
                shell_quote(&exe.to_string_lossy()),
                shell_quote(host_id)
            );
        }
    }
    if let Ok(cli) = agent_doctor_core::workspace::resolve_agent_doctor_binary() {
        let stable =
            agent_doctor_core::workspace::ensure_stable_agent_doctor_cli(&cli).unwrap_or(cli);
        return format!(
            "{} remote exec {}",
            shell_quote(&stable.to_string_lossy()),
            shell_quote(host_id)
        );
    }
    format!("agent-doctor remote exec {}", shell_quote(host_id))
}

fn local_scratch_dir(host_id: &str, project_name: &str) -> PathBuf {
    let safe = |raw: &str| -> String {
        let s: String = raw
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        if s.is_empty() {
            "default".into()
        } else {
            s
        }
    };
    let dir = dirs::home_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(".agent-doctor")
        .join("remote-vps")
        .join(safe(host_id))
        .join(safe(project_name));
    let _ = fs::create_dir_all(&dir);
    dir
}

pub fn run_linked_prompt(
    link_sessions: &LinkSessionState,
    host_id: &str,
    remote_workspace_name: &str,
    options: &PromptSessionOptions,
    cancel: PromptSessionCancel,
    control: PromptSessionControl,
    mut on_event: impl FnMut(PromptSessionEvent),
) -> Result<PromptSessionReport, String> {
    let mut client = link_sessions.open_fresh_client(host_id)?;
    let hello = client.ping().map_err(|e| format!("{e:#}"))?;
    let workspaces = client.list_workspaces().unwrap_or_default();
    let remote_cwd = options
        .cwd
        .as_ref()
        .and_then(|p| p.to_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| {
            workspaces
                .iter()
                .find(|w| w.name == remote_workspace_name)
                .or_else(|| workspaces.iter().find(|w| w.active))
                .or_else(|| workspaces.first())
                .map(|w| w.path.clone())
        })
        .unwrap_or_else(|| "~".to_string());

    let exec_prefix = remote_exec_command_prefix(host_id);
    let preamble =
        build_remote_vps_preamble(&hello.computer_name, host_id, &remote_cwd, &exec_prefix);

    let mut local_options = options.clone();
    local_options.cwd = Some(local_scratch_dir(host_id, remote_workspace_name));
    local_options.workspace_name = None;
    local_options.prompt = format!("{preamble}\n\n{}", options.prompt);

    let needs_control = {
        let rt = local_options.runtime.as_str();
        (rt == "claude-code" && !local_options.dangerously_skip_permissions)
            || (rt == "codex" && !local_options.full_auto)
            || rt == "deepseek-harness"
            || rt == "acp"
            || rt == agent_doctor_repair::OFFICIAL_AGENT_RUNTIME_ID
    };
    let control_for_run = if needs_control { Some(control) } else { None };
    let display_cwd = format!("{} · {}", hello.computer_name, remote_cwd);

    let mut report =
        run_prompt_session_with_cancel(&local_options, cancel, control_for_run, |event| {
            let patched = match event {
                PromptSessionEvent::Started {
                    session_id,
                    runtime,
                    command,
                    client_run_id,
                    ..
                } => PromptSessionEvent::Started {
                    session_id,
                    runtime,
                    cwd: display_cwd.clone(),
                    command,
                    client_run_id,
                },
                other => other,
            };
            on_event(patched);
        })
        .map_err(|err| format!("{err:#}"))?;

    report.cwd = display_cwd;
    Ok(report)
}

fn build_remote_vps_preamble(
    computer_name: &str,
    host_id: &str,
    remote_cwd: &str,
    exec_prefix: &str,
) -> String {
    format!(
        "REMOTE COMPUTER CONTEXT (treat `{computer_name}` like a passwordless remote VPS):\n\
         - Target computer: `{computer_name}` (id `{host_id}`), already connected over the LAN client link with no login/password needed.\n\
         - Remote project folder on `{computer_name}`: `{remote_cwd}`\n\
         - Your agent process is running on THIS local computer so it does not depend on `{computer_name}` having an agent or model configured.\n\
         - To read files/configs/logs, check processes/ports/exo, or run repair commands on `{computer_name}`, run commands through `{exec_prefix} '<shell command>'` (works just like `ssh host '<shell command>'`).\n\
         - Never mistake local files on this controller computer for `{computer_name}`'s files."
    )
}

pub fn cancel_linked_prompt(
    link_sessions: &LinkSessionState,
    index: &LinkedPromptIndex,
    client_run_id: &str,
) -> Result<bool, String> {
    let Some(host_id) = index.host_for_client(client_run_id) else {
        return Ok(false);
    };
    let mut client = link_sessions.open_fresh_client(&host_id)?;
    client
        .cancel_prompt(client_run_id)
        .map_err(|err| format!("{err:#}"))
}

pub fn resolve_linked_permission(
    link_sessions: &LinkSessionState,
    index: &LinkedPromptIndex,
    session_id: &str,
    request_id: &str,
    allow: bool,
    text: Option<&str>,
    updated_permissions_json: Option<&str>,
) -> Result<bool, String> {
    let Some(host_id) = index.host_for_backend(session_id) else {
        return Ok(false);
    };
    let mut client = link_sessions.open_fresh_client(&host_id)?;
    client
        .resolve_permission(
            session_id,
            request_id,
            allow,
            text,
            updated_permissions_json,
        )
        .map_err(|err| format!("{err:#}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_linked_workspace_parses_host_and_project() {
        assert_eq!(
            split_linked_workspace(Some("lan-abc::default")),
            Some(("lan-abc".into(), "default".into()))
        );
        assert_eq!(split_linked_workspace(Some("local-only")), None);
    }

    #[test]
    fn vps_preamble_points_to_remote_exec_command() {
        let text = build_remote_vps_preamble(
            "yydeMac-Mini",
            "lan-123",
            "/Users/yy/.agent-doctor/workspaces/default",
            "'/Applications/Agent Doctor.app/Contents/MacOS/agent-doctor-desktop' --remote-exec 'lan-123'",
        );
        assert!(text.contains("yydeMac-Mini"));
        assert!(text.contains("--remote-exec 'lan-123'"));
        assert!(text.contains("ssh host"));
    }
}
