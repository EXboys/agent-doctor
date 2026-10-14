use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::path::{paths_equal, resolve_project_path};
use super::{
    load_workspaces, use_workspace_with_options, UseWorkspaceOptions, UseWorkspaceReport,
    WorkspaceEntry,
};

const HOOK_FILE: &str = "hooks/workspace.zsh";
const BASH_HOOK_FILE: &str = "hooks/workspace.bash";
const FISH_HOOK_FILE: &str = "hooks/workspace.fish";
const POWERSHELL_HOOK_FILE: &str = "hooks/workspace.ps1";

pub fn match_workspace_for_path(
    path: Option<PathBuf>,
    prefer_git_root: bool,
) -> Result<Option<String>> {
    let project_path = resolve_project_path(path, prefer_git_root)?;
    let doc = load_workspaces()?;
    Ok(doc
        .workspaces
        .iter()
        .find(|(_, entry)| {
            paths_equal(&entry.path, &project_path) || project_path.starts_with(&entry.path)
        })
        .map(|(name, _)| name.clone()))
}

pub fn enter_workspace(
    path: Option<PathBuf>,
    prefer_git_root: bool,
) -> Result<EnterWorkspaceReport> {
    let project_path = resolve_project_path(path.clone(), prefer_git_root)?;
    let Some(name) = match_workspace_for_path(path, prefer_git_root)? else {
        anyhow::bail!(
            "no workspace registered for {} — run `agent-doctor workspace init` first",
            project_path.display()
        );
    };

    let doc = load_workspaces()?;
    let switched = doc.active.as_deref() != Some(name.as_str());
    let use_report = use_workspace_with_options(
        &name,
        &UseWorkspaceOptions {
            backup: true,
            restart_gateways: false,
        },
    )?;

    let zsh_eval = render_shell_env("zsh", &use_report)?;
    let bash_eval = render_shell_env("bash", &use_report)?;
    let powershell_eval = render_shell_env("powershell", &use_report)?;
    let cd_command = format!("cd {}", shell_single_quote(&use_report.path));
    let zsh_enter =
        format!("{cd_command} && eval \"$(agent-doctor workspace env --shell zsh --name {name})\"");
    let bash_enter = format!(
        "{cd_command} && eval \"$(agent-doctor workspace env --shell bash --name {name})\""
    );
    let powershell_enter = format!(
        "Set-Location -LiteralPath {}; {}",
        powershell_single_quote(&use_report.path),
        powershell_eval.replace('\n', "; ")
    );

    Ok(EnterWorkspaceReport {
        name,
        path: use_report.path.clone(),
        switched,
        use_report,
        cd_command,
        zsh_eval,
        bash_eval,
        zsh_enter,
        bash_enter,
        powershell_eval,
        powershell_enter,
    })
}

pub fn render_shell_env_for_name(name: &str, shell: &str) -> Result<String> {
    let doc = load_workspaces()?;
    let entry = doc
        .workspaces
        .get(name)
        .with_context(|| format!("workspace '{name}' not found"))?;
    let report = UseWorkspaceReport {
        name: name.to_string(),
        path: entry.path.clone(),
        env_file: super::active_env_path()?,
        bindings: Vec::new(),
        backup_id: None,
        gateway_restarts: Vec::new(),
    };
    render_shell_env(shell, &report)
}

pub fn render_shell_env(shell: &str, report: &UseWorkspaceReport) -> Result<String> {
    let entry = load_workspaces()?
        .workspaces
        .get(&report.name)
        .cloned()
        .with_context(|| format!("workspace '{}' missing", report.name))?;

    let mut lines = Vec::new();
    match shell {
        "zsh" | "bash" => {
            lines.push(format!(
                "export AGENT_DOCTOR_WORKSPACE={}",
                shell_single_quote(&report.name)
            ));
            lines.push(format!(
                "export AGENT_DOCTOR_PROJECT_ROOT={}",
                shell_single_quote(&entry.path)
            ));
            lines.push(format!(
                "export HERMES_HOME={}",
                shell_single_quote(entry.hermes_profile_home())
            ));
            lines.push("export HERMES_DISABLE_LAZY_INSTALLS=1".to_string());
            lines.push(format!(
                "export CODEX_HOME={}",
                shell_single_quote(&entry.codex_home)
            ));
            lines.push(format!(
                "export OPENCLAW_AGENT_ID={}",
                shell_single_quote(&entry.openclaw_agent_id)
            ));
            lines.push(format!(
                "export OPENCLAW_WORKSPACE={}",
                shell_single_quote(&entry.openclaw_workspace)
            ));
        }
        "powershell" | "pwsh" => {
            lines.push(format!(
                "$env:AGENT_DOCTOR_WORKSPACE={}",
                powershell_single_quote(&report.name)
            ));
            lines.push(format!(
                "$env:AGENT_DOCTOR_PROJECT_ROOT={}",
                powershell_single_quote(&entry.path)
            ));
            lines.push(format!(
                "$env:HERMES_HOME={}",
                powershell_single_quote(entry.hermes_profile_home())
            ));
            lines.push("$env:HERMES_DISABLE_LAZY_INSTALLS='1'".to_string());
            lines.push(format!(
                "$env:CODEX_HOME={}",
                powershell_single_quote(&entry.codex_home)
            ));
            lines.push(format!(
                "$env:OPENCLAW_AGENT_ID={}",
                powershell_single_quote(&entry.openclaw_agent_id)
            ));
            lines.push(format!(
                "$env:OPENCLAW_WORKSPACE={}",
                powershell_single_quote(&entry.openclaw_workspace)
            ));
        }
        _ => {
            lines.push(format!(
                "set -a && source \"{}\" && set +a",
                report.env_file.display()
            ));
        }
    }
    Ok(lines.join("\n"))
}

pub fn install_zsh_hook() -> Result<PathBuf> {
    write_zsh_hook(&hook_file_path()?)
}

pub fn install_bash_hook() -> Result<PathBuf> {
    write_bash_hook(&bash_hook_file_path()?)
}

pub fn install_fish_hook() -> Result<PathBuf> {
    write_fish_hook(&fish_hook_file_path()?)
}

pub fn install_powershell_hook() -> Result<PathBuf> {
    write_powershell_hook(&powershell_hook_file_path()?)
}

fn write_zsh_hook(hook_path: &Path) -> Result<PathBuf> {
    if let Some(parent) = hook_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let binary = agent_doctor_binary();

    let contents = format!(
        r#"# Agent Doctor workspace auto-align (zsh)
# Add to ~/.zshrc: source "{hook}"

agent_doctor_workspace_chpwd() {{
  local ws
  ws=$({binary} workspace match 2>/dev/null) || return 0
  [[ -z "$ws" ]] && return 0
  if [[ "${{AGENT_DOCTOR_WORKSPACE:-}}" != "$ws" ]]; then
    eval "$({binary} workspace env --shell zsh --name "$ws" 2>/dev/null)" || return 0
  fi
}}

if [[ -n "${{chpwd_functions[(r)agent_doctor_workspace_chpwd]:-}}" ]]; then
  :
else
  chpwd_functions+=(agent_doctor_workspace_chpwd)
fi
"#,
        hook = hook_path.display(),
        binary = binary,
    );

    fs::write(hook_path, contents).with_context(|| format!("write {}", hook_path.display()))?;
    Ok(hook_path.to_path_buf())
}

fn write_bash_hook(hook_path: &Path) -> Result<PathBuf> {
    if let Some(parent) = hook_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let binary = agent_doctor_binary();

    let contents = format!(
        r#"# Agent Doctor workspace auto-align (bash)
# Add to ~/.bashrc: source "{hook}"

agent_doctor_workspace_prompt() {{
  local ws
  ws=$({binary} workspace match 2>/dev/null) || return 0
  [[ -z "$ws" ]] && return 0
  if [[ "${{AGENT_DOCTOR_WORKSPACE:-}}" != "$ws" ]]; then
    eval "$({binary} workspace env --shell bash --name "$ws" 2>/dev/null)" || return 0
  fi
}}

if [[ ":${{PROMPT_COMMAND:-}}:" != *":agent_doctor_workspace_prompt:"* ]]; then
  PROMPT_COMMAND="agent_doctor_workspace_prompt${{PROMPT_COMMAND:+;$PROMPT_COMMAND}}"
fi
"#,
        hook = hook_path.display(),
        binary = binary,
    );

    fs::write(hook_path, contents).with_context(|| format!("write {}", hook_path.display()))?;
    Ok(hook_path.to_path_buf())
}

fn agent_doctor_binary() -> String {
    std::env::current_exe()
        .ok()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "agent-doctor".to_string())
}

pub fn hook_file_path() -> Result<PathBuf> {
    dirs::config_dir()
        .map(|dir| dir.join("agent-doctor").join(HOOK_FILE))
        .context("could not resolve config directory")
}

pub fn bash_hook_file_path() -> Result<PathBuf> {
    dirs::config_dir()
        .map(|dir| dir.join("agent-doctor").join(BASH_HOOK_FILE))
        .context("could not resolve config directory")
}

pub fn fish_hook_file_path() -> Result<PathBuf> {
    dirs::config_dir()
        .map(|dir| dir.join("agent-doctor").join(FISH_HOOK_FILE))
        .context("could not resolve config directory")
}

pub fn powershell_hook_file_path() -> Result<PathBuf> {
    dirs::config_dir()
        .map(|dir| dir.join("agent-doctor").join(POWERSHELL_HOOK_FILE))
        .context("could not resolve config directory")
}

pub fn render_direnv_envrc(name: &str) -> Result<String> {
    Ok(format!(
        r#"# Agent Doctor workspace — allow with: direnv allow
if command -v agent-doctor >/dev/null 2>&1; then
  eval "$(agent-doctor workspace env --shell bash --name {name})"
elif [ -f "$HOME/.config/agent-doctor/active-workspace.env" ]; then
  set -a
  # shellcheck disable=SC1091
  source "$HOME/.config/agent-doctor/active-workspace.env"
  set +a
fi
"#,
        name = name,
    ))
}

pub fn write_direnv_envrc(name: &str) -> Result<PathBuf> {
    let doc = load_workspaces()?;
    let entry = doc
        .workspaces
        .get(name)
        .with_context(|| format!("workspace '{name}' not found"))?;
    let envrc = entry.path.join(".envrc");
    let contents = render_direnv_envrc(name)?;
    fs::write(&envrc, contents).with_context(|| format!("write {}", envrc.display()))?;
    Ok(envrc)
}

fn write_fish_hook(hook_path: &Path) -> Result<PathBuf> {
    if let Some(parent) = hook_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let binary = agent_doctor_binary();

    let contents = format!(
        r#"# Agent Doctor workspace auto-align (fish)
# Add to ~/.config/fish/config.fish: source "{hook}"

function __agent_doctor_workspace_align --on-variable PWD
    set -l ws ({binary} workspace match 2>/dev/null)
    if test -z "$ws"
        return
    end
    if test "$AGENT_DOCTOR_WORKSPACE" != "$ws"
        eval ({binary} workspace env --shell bash --name $ws 2>/dev/null)
    end
end
"#,
        hook = hook_path.display(),
        binary = binary,
    );

    fs::write(hook_path, contents).with_context(|| format!("write {}", hook_path.display()))?;
    Ok(hook_path.to_path_buf())
}

fn write_powershell_hook(hook_path: &Path) -> Result<PathBuf> {
    if let Some(parent) = hook_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let binary = agent_doctor_binary();

    let contents = format!(
        r#"# Agent Doctor workspace auto-align (PowerShell)
# Add to $PROFILE: . "{hook}"

function Global:__agent_doctor_workspace_align {{
  $ws = & {binary} workspace match 2>$null
  if (-not $ws) {{ return }}
  if ($env:AGENT_DOCTOR_WORKSPACE -ne $ws) {{
    Invoke-Expression (& {binary} workspace env --shell powershell --name $ws 2>$null)
  }}
}}

if (-not (Get-Command __agent_doctor_workspace_align -ErrorAction SilentlyContinue)) {{
  function Global:__agent_doctor_workspace_align {{ }}
}}

if ($null -eq $function:prompt -or $function:prompt -notlike '*__agent_doctor_workspace_align*') {{
  $script:__agent_doctor_old_prompt = $function:prompt
  function Global:Prompt {{
    __agent_doctor_workspace_align
    if ($null -ne $script:__agent_doctor_old_prompt) {{
      & $script:__agent_doctor_old_prompt
    }}
  }}
}}
"#,
        hook = hook_path.display(),
        binary = binary,
    );

    fs::write(hook_path, contents).with_context(|| format!("write {}", hook_path.display()))?;
    Ok(hook_path.to_path_buf())
}

fn shell_single_quote(value: impl AsRef<std::ffi::OsStr>) -> String {
    let text = value.as_ref().to_string_lossy();
    format!("'{}'", text.replace('\'', "'\\''"))
}

fn powershell_single_quote(value: impl AsRef<std::ffi::OsStr>) -> String {
    let text = value.as_ref().to_string_lossy();
    format!("'{}'", text.replace('\'', "''"))
}

impl WorkspaceEntry {
    pub(crate) fn hermes_profile_home(&self) -> PathBuf {
        crate::adapters::util::home_join(".hermes/profiles").join(&self.hermes_profile)
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct EnterWorkspaceReport {
    pub name: String,
    pub path: PathBuf,
    pub switched: bool,
    pub use_report: UseWorkspaceReport,
    pub cd_command: String,
    pub zsh_eval: String,
    pub bash_eval: String,
    pub zsh_enter: String,
    pub bash_enter: String,
    pub powershell_eval: String,
    pub powershell_enter: String,
}
