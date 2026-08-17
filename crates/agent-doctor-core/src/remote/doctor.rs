use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::adapters::configured_base_url;
use crate::exec::ExecBackend;
use crate::probe::config::parse_config;
use crate::probe::{ParsedConfig, ProbeCheck, ProbeSeverity, ProbeStatus};
use crate::repair::SensitivityLevel;
use crate::runtime::{all_runtime_ids, descriptor_by_id, ConfigFormat};

use super::registry::{load_remote_hosts, remote_root_dir, RemoteProjectEntry};
use super::ssh::SshBackend;

#[derive(Debug, Clone, Default)]
pub struct RemoteDoctorOptions {
    /// Limit to a single runtime id (overrides project runtimes list when set).
    pub runtime_filter: Option<String>,
    /// Persist JSON report under remote/reports/ (default true).
    pub save_report: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteDoctorReport {
    pub host_id: String,
    pub ssh_config_host: String,
    pub project_id: String,
    pub project_path: String,
    pub remote_home: Option<String>,
    pub connectivity_ok: bool,
    pub checks: Vec<ProbeCheck>,
    pub runtimes: Vec<RemoteRuntimeDoctorResult>,
    pub report_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteRuntimeDoctorResult {
    pub runtime_id: String,
    pub display_name: String,
    pub binary_name: String,
    pub checks: Vec<ProbeCheck>,
}

/// Run agentless remote doctor for `host/project` (e.g. `prod-vps/api`).
pub fn run_remote_doctor(target: &str, options: RemoteDoctorOptions) -> Result<RemoteDoctorReport> {
    let (host_id, project_id) = parse_target(target)?;
    let doc = load_remote_hosts()?;
    let host = doc
        .hosts
        .get(&host_id)
        .with_context(|| format!("unknown host '{host_id}'"))?;
    let project = host
        .projects
        .get(&project_id)
        .with_context(|| format!("unknown project '{project_id}' on host '{host_id}'"))?
        .clone();

    let backend = SshBackend::new(&host.ssh_config_host);
    let mut report = run_remote_doctor_with_backend(
        &host_id,
        &host.ssh_config_host,
        &project_id,
        &project,
        &backend,
        &options,
    )?;

    if options.save_report {
        let path = write_remote_doctor_report(&report)?;
        report.report_path = Some(path.display().to_string());
    }
    Ok(report)
}

pub fn run_remote_doctor_with_backend(
    host_id: &str,
    ssh_config_host: &str,
    project_id: &str,
    project: &RemoteProjectEntry,
    backend: &dyn ExecBackend,
    options: &RemoteDoctorOptions,
) -> Result<RemoteDoctorReport> {
    let mut host_checks = Vec::new();
    let mut connectivity_ok = false;
    let mut remote_home: Option<String> = None;

    match backend.run(&["true"], None) {
        Ok(out) if out.success() => {
            connectivity_ok = true;
            host_checks.push(ProbeCheck::new(
                "ssh.connectivity",
                "SSH connectivity",
                ProbeStatus::Pass,
                ProbeSeverity::Info,
                format!("connected to '{ssh_config_host}'"),
                SensitivityLevel::Public,
            ));
        }
        Ok(out) => {
            host_checks.push(ProbeCheck::new(
                "ssh.connectivity",
                "SSH connectivity",
                ProbeStatus::Fail,
                ProbeSeverity::Error,
                format!(
                    "ssh to '{ssh_config_host}' failed (exit {}): {}",
                    out.status,
                    out.stderr.trim()
                ),
                SensitivityLevel::Public,
            ));
        }
        Err(err) => {
            host_checks.push(ProbeCheck::new(
                "ssh.connectivity",
                "SSH connectivity",
                ProbeStatus::Fail,
                ProbeSeverity::Error,
                format!("ssh to '{ssh_config_host}' failed: {err}"),
                SensitivityLevel::Public,
            ));
        }
    }

    if connectivity_ok {
        match backend.home_dir() {
            Ok(home) => {
                remote_home = Some(home.display().to_string());
                host_checks.push(
                    ProbeCheck::new(
                        "ssh.home",
                        "Remote HOME",
                        ProbeStatus::Pass,
                        ProbeSeverity::Info,
                        "resolved remote HOME",
                        SensitivityLevel::LocalPath,
                    )
                    .with_details(vec![home.display().to_string()]),
                );
            }
            Err(err) => {
                host_checks.push(ProbeCheck::new(
                    "ssh.home",
                    "Remote HOME",
                    ProbeStatus::Fail,
                    ProbeSeverity::Error,
                    format!("could not resolve HOME: {err}"),
                    SensitivityLevel::Public,
                ));
            }
        }

        let project_path = PathBuf::from(&project.path);
        match backend.is_dir(&project_path) {
            Ok(true) => {
                host_checks.push(
                    ProbeCheck::new(
                        "project.path",
                        "Project path",
                        ProbeStatus::Pass,
                        ProbeSeverity::Info,
                        "project directory exists",
                        SensitivityLevel::LocalPath,
                    )
                    .with_details(vec![project.path.clone()]),
                );
                match backend.run(&["pwd"], Some(&project_path)) {
                    Ok(out) if out.success() => {
                        host_checks.push(
                            ProbeCheck::new(
                                "project.pwd",
                                "Project cwd",
                                ProbeStatus::Pass,
                                ProbeSeverity::Info,
                                "pwd in project path",
                                SensitivityLevel::LocalPath,
                            )
                            .with_details(vec![out.stdout_trim().to_string()]),
                        );
                    }
                    Ok(out) => {
                        host_checks.push(ProbeCheck::new(
                            "project.pwd",
                            "Project cwd",
                            ProbeStatus::Warn,
                            ProbeSeverity::Warning,
                            format!("pwd failed: {}", out.stderr.trim()),
                            SensitivityLevel::Public,
                        ));
                    }
                    Err(err) => {
                        host_checks.push(ProbeCheck::new(
                            "project.pwd",
                            "Project cwd",
                            ProbeStatus::Warn,
                            ProbeSeverity::Warning,
                            format!("pwd error: {err}"),
                            SensitivityLevel::Public,
                        ));
                    }
                }
            }
            Ok(false) => {
                host_checks.push(ProbeCheck::new(
                    "project.path",
                    "Project path",
                    ProbeStatus::Fail,
                    ProbeSeverity::Error,
                    format!("project path is not a directory: {}", project.path),
                    SensitivityLevel::LocalPath,
                ));
            }
            Err(err) => {
                host_checks.push(ProbeCheck::new(
                    "project.path",
                    "Project path",
                    ProbeStatus::Fail,
                    ProbeSeverity::Error,
                    format!("could not check project path: {err}"),
                    SensitivityLevel::Public,
                ));
            }
        }
    }

    let runtime_ids = resolve_runtime_ids(project, options.runtime_filter.as_deref())?;
    let mut runtimes = Vec::new();
    if connectivity_ok {
        let home = remote_home
            .as_ref()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/"));
        for runtime_id in runtime_ids {
            runtimes.push(probe_remote_runtime(
                backend,
                &home,
                Path::new(&project.path),
                runtime_id,
            )?);
        }
    }

    Ok(RemoteDoctorReport {
        host_id: host_id.to_string(),
        ssh_config_host: ssh_config_host.to_string(),
        project_id: project_id.to_string(),
        project_path: project.path.clone(),
        remote_home,
        connectivity_ok,
        checks: host_checks,
        runtimes,
        report_path: None,
    })
}

pub fn write_remote_doctor_report(report: &RemoteDoctorReport) -> Result<PathBuf> {
    let root = remote_root_dir()
        .context("config directory not found")?
        .join("reports")
        .join(&report.host_id)
        .join(&report.project_id);
    fs::create_dir_all(&root).with_context(|| format!("create {}", root.display()))?;
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let path = root.join(format!("{ts}.json"));
    let raw = serde_json::to_string_pretty(report).context("serialize remote doctor report")?;
    fs::write(&path, raw).with_context(|| format!("write {}", path.display()))?;
    Ok(path)
}

fn parse_target(target: &str) -> Result<(String, String)> {
    let Some((host, project)) = target.split_once('/') else {
        bail!("target must be host/project (got '{target}')");
    };
    if host.is_empty() || project.is_empty() {
        bail!("target must be host/project (got '{target}')");
    }
    if target.matches('/').count() != 1 {
        bail!("target must be host/project with a single slash (got '{target}')");
    }
    Ok((host.to_string(), project.to_string()))
}

fn resolve_runtime_ids(
    project: &RemoteProjectEntry,
    filter: Option<&str>,
) -> Result<Vec<&'static str>> {
    let all: Vec<&'static str> = all_runtime_ids().collect();
    if let Some(filter) = filter {
        if descriptor_by_id(filter).is_none() {
            bail!("unknown runtime '{filter}'");
        }
        return Ok(all.into_iter().filter(|id| *id == filter).collect());
    }
    if project.runtimes.is_empty() {
        return Ok(all);
    }
    let mut selected = Vec::new();
    for id in &project.runtimes {
        if descriptor_by_id(id).is_none() {
            bail!("unknown runtime '{id}' in project runtimes");
        }
        if let Some(static_id) = all
            .iter()
            .copied()
            .find(|candidate| *candidate == id.as_str())
        {
            selected.push(static_id);
        }
    }
    Ok(selected)
}

fn probe_remote_runtime(
    backend: &dyn ExecBackend,
    remote_home: &Path,
    project_path: &Path,
    runtime_id: &str,
) -> Result<RemoteRuntimeDoctorResult> {
    let descriptor =
        descriptor_by_id(runtime_id).with_context(|| format!("unknown runtime '{runtime_id}'"))?;
    let binary_name = descriptor.probe.binary_name;
    let adapter = descriptor.create_adapter();
    let mut checks = Vec::new();

    // binary
    match backend.run(&["sh", "-c", &format!("command -v {binary_name}")], None) {
        Ok(out) if out.success() => {
            let path = out.stdout_trim().to_string();
            checks.push(
                ProbeCheck::new(
                    "binary.exists",
                    "Binary exists",
                    ProbeStatus::Pass,
                    ProbeSeverity::Info,
                    format!("{binary_name} found on remote PATH"),
                    SensitivityLevel::LocalPath,
                )
                .with_details(vec![path]),
            );
            match backend.run(&[binary_name, "--version"], None) {
                Ok(ver) if ver.success() => {
                    let version = first_line(ver.stdout_trim());
                    checks.push(
                        ProbeCheck::new(
                            "binary.version",
                            "Binary version",
                            ProbeStatus::Pass,
                            ProbeSeverity::Info,
                            format!("{binary_name} --version ok"),
                            SensitivityLevel::Public,
                        )
                        .with_details(vec![version]),
                    );
                }
                Ok(ver) => {
                    checks.push(ProbeCheck::new(
                        "binary.version",
                        "Binary version",
                        ProbeStatus::Warn,
                        ProbeSeverity::Warning,
                        format!(
                            "{binary_name} --version failed (exit {}): {}",
                            ver.status,
                            ver.stderr.trim()
                        ),
                        SensitivityLevel::Public,
                    ));
                }
                Err(err) => {
                    checks.push(ProbeCheck::new(
                        "binary.version",
                        "Binary version",
                        ProbeStatus::Warn,
                        ProbeSeverity::Warning,
                        format!("{binary_name} --version error: {err}"),
                        SensitivityLevel::Public,
                    ));
                }
            }
        }
        Ok(_) => {
            checks.push(ProbeCheck::new(
                "binary.exists",
                "Binary exists",
                ProbeStatus::Fail,
                ProbeSeverity::Error,
                format!("{binary_name} not found on remote PATH"),
                SensitivityLevel::Public,
            ));
        }
        Err(err) => {
            checks.push(ProbeCheck::new(
                "binary.exists",
                "Binary exists",
                ProbeStatus::Fail,
                ProbeSeverity::Error,
                format!("could not check binary: {err}"),
                SensitivityLevel::Public,
            ));
        }
    }

    // config files under remote HOME (relative paths from local adapter config_paths)
    let local_home = dirs::home_dir();
    for (local_config, required) in adapter
        .config_paths()
        .into_iter()
        .map(|path| (path, true))
        .chain(
            adapter
                .optional_config_paths()
                .into_iter()
                .map(|path| (path, false)),
        )
    {
        let relative = match local_home
            .as_ref()
            .and_then(|h| local_config.strip_prefix(h).ok())
        {
            Some(rel) => rel.to_path_buf(),
            None => {
                // Fallback: use known suffixes
                continue;
            }
        };
        let remote_config = remote_home.join(&relative);
        match backend.exists(&remote_config) {
            Ok(false) => {
                if required {
                    checks.push(ProbeCheck::new(
                        format!("config.exists.{}", relative.display()),
                        "Config exists",
                        ProbeStatus::Warn,
                        ProbeSeverity::Warning,
                        format!("missing {}", remote_config.display()),
                        SensitivityLevel::LocalPath,
                    ));
                }
                continue;
            }
            Err(err) => {
                checks.push(ProbeCheck::new(
                    format!("config.exists.{}", relative.display()),
                    "Config exists",
                    ProbeStatus::Fail,
                    ProbeSeverity::Error,
                    format!("could not check {}: {err}", remote_config.display()),
                    SensitivityLevel::LocalPath,
                ));
                continue;
            }
            Ok(true) => {}
        }

        match backend.read_to_string(&remote_config) {
            Ok(raw) => {
                let format = config_format_for_runtime(
                    runtime_id,
                    &remote_config,
                    descriptor.probe.config_format,
                );
                match parse_config(&raw, format) {
                    Ok(parsed) => {
                        checks.push(
                            ProbeCheck::new(
                                format!("config.parse.{}", relative.display()),
                                "Config parse",
                                ProbeStatus::Pass,
                                ProbeSeverity::Info,
                                format!("parsed {}", remote_config.display()),
                                SensitivityLevel::LocalPath,
                            )
                            .with_details(vec![format!("{:?}", format_name(format))]),
                        );
                        if let Some(gateway) = extract_gateway(runtime_id, &raw, &parsed) {
                            // Keep host/path visible for drift diagnosis; do not dump secrets
                            // (gateway URLs are ConfigShape, not API keys).
                            checks.push(
                                ProbeCheck::new(
                                    format!("config.gateway.{}", relative.display()),
                                    "Gateway / base URL",
                                    ProbeStatus::Pass,
                                    ProbeSeverity::Info,
                                    "gateway field present",
                                    SensitivityLevel::ConfigShape,
                                )
                                .with_details(vec![gateway]),
                            );
                        }
                    }
                    Err(err) => {
                        checks.push(ProbeCheck::new(
                            format!("config.parse.{}", relative.display()),
                            "Config parse",
                            ProbeStatus::Fail,
                            ProbeSeverity::Error,
                            format!("parse {}: {err}", remote_config.display()),
                            SensitivityLevel::ConfigShape,
                        ));
                    }
                }
            }
            Err(err) => {
                checks.push(ProbeCheck::new(
                    format!("config.read.{}", relative.display()),
                    "Config read",
                    ProbeStatus::Fail,
                    ProbeSeverity::Error,
                    format!("read {}: {err}", remote_config.display()),
                    SensitivityLevel::LocalPath,
                ));
            }
        }
    }

    // project-side traces
    for (label, rel) in project_traces(runtime_id) {
        let path = project_path.join(rel);
        match backend.exists(&path) {
            Ok(true) => {
                checks.push(
                    ProbeCheck::new(
                        format!("project.trace.{label}"),
                        "Project trace",
                        ProbeStatus::Pass,
                        ProbeSeverity::Info,
                        format!("found {rel}"),
                        SensitivityLevel::LocalPath,
                    )
                    .with_details(vec![path.display().to_string()]),
                );
            }
            Ok(false) => {
                checks.push(ProbeCheck::new(
                    format!("project.trace.{label}"),
                    "Project trace",
                    ProbeStatus::NotApplicable,
                    ProbeSeverity::Info,
                    format!("no {rel} under project"),
                    SensitivityLevel::LocalPath,
                ));
            }
            Err(err) => {
                checks.push(ProbeCheck::new(
                    format!("project.trace.{label}"),
                    "Project trace",
                    ProbeStatus::Warn,
                    ProbeSeverity::Warning,
                    format!("could not check {rel}: {err}"),
                    SensitivityLevel::Public,
                ));
            }
        }
    }

    Ok(RemoteRuntimeDoctorResult {
        runtime_id: runtime_id.to_string(),
        display_name: adapter.display_name().to_string(),
        binary_name: binary_name.to_string(),
        checks,
    })
}

fn project_traces(runtime_id: &str) -> Vec<(&'static str, &'static str)> {
    match runtime_id {
        "claude-code" => vec![("claude_dir", ".claude"), ("mcp_json", ".mcp.json")],
        "codex" => vec![("codex_dir", ".codex")],
        "hermes" => vec![("hermes_dir", ".hermes")],
        "openclaw" => vec![("openclaw_workspace", ".openclaw")],
        _ => Vec::new(),
    }
}

fn config_format_for_runtime(runtime_id: &str, path: &Path, default: ConfigFormat) -> ConfigFormat {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    if name == ".env" {
        return ConfigFormat::Env;
    }
    match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
    {
        "json" => ConfigFormat::Json,
        "yaml" | "yml" => ConfigFormat::Yaml,
        "toml" => ConfigFormat::Toml,
        _ => match runtime_id {
            "openclaw" | "claude-code" => ConfigFormat::Json,
            "hermes" => ConfigFormat::Yaml,
            "codex" => ConfigFormat::Toml,
            _ => default,
        },
    }
}

fn format_name(format: ConfigFormat) -> &'static str {
    match format {
        ConfigFormat::Json => "json",
        ConfigFormat::Yaml => "yaml",
        ConfigFormat::Toml => "toml",
        ConfigFormat::Env => "env",
    }
}

fn extract_gateway(runtime_id: &str, raw: &str, parsed: &ParsedConfig) -> Option<String> {
    match (runtime_id, parsed) {
        ("openclaw", ParsedConfig::Json(value)) => configured_base_url(value),
        ("claude-code", ParsedConfig::Json(value)) => value
            .get("env")
            .and_then(|env| env.get("ANTHROPIC_BASE_URL"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .or_else(|| {
                value
                    .pointer("/env/ANTHROPIC_BASE_URL")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            }),
        ("hermes", ParsedConfig::Yaml(value)) => value
            .get("model")
            .and_then(|m| m.get("base_url"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        ("codex", ParsedConfig::Toml(value)) => {
            // Prefer model_providers.<active>.base_url if present; else scan providers.
            if let Some(provider) = value.get("model_provider").and_then(|v| v.as_str()) {
                if let Some(url) = value
                    .get("model_providers")
                    .and_then(|t| t.get(provider))
                    .and_then(|p| p.get("base_url"))
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                {
                    return Some(url.to_string());
                }
            }
            value
                .get("model_providers")
                .and_then(|t| t.as_table())
                .and_then(|table| {
                    table.values().find_map(|provider| {
                        provider
                            .get("base_url")
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.is_empty())
                            .map(str::to_string)
                    })
                })
        }
        (_, ParsedConfig::Env(_)) => None,
        _ => {
            // Fallback: look for common URL-looking lines (no secrets dump)
            for line in raw.lines() {
                let lower = line.to_ascii_lowercase();
                if lower.contains("base_url")
                    || lower.contains("baseurl")
                    || lower.contains("gateway")
                {
                    if let Some((_, rest)) = line.split_once(':').or_else(|| line.split_once('=')) {
                        let candidate = rest.trim().trim_matches('"').trim_matches('\'');
                        if candidate.starts_with("http://") || candidate.starts_with("https://") {
                            return Some(candidate.to_string());
                        }
                    }
                }
            }
            None
        }
    }
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or(s).trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::ExecOutput;
    use crate::remote::registry::RemoteProjectEntry;
    use std::collections::HashMap;
    use std::sync::Mutex;

    struct FakeBackend {
        home: PathBuf,
        files: Mutex<HashMap<String, String>>,
        dirs: Mutex<Vec<String>>,
        commands: Mutex<HashMap<String, ExecOutput>>,
    }

    impl FakeBackend {
        fn new() -> Self {
            Self {
                home: PathBuf::from("/home/deploy"),
                files: Mutex::new(HashMap::new()),
                dirs: Mutex::new(vec!["/srv/api".into()]),
                commands: Mutex::new(HashMap::new()),
            }
        }

        fn key(argv: &[&str], cwd: Option<&Path>) -> String {
            let cwd = cwd.map(|p| p.display().to_string()).unwrap_or_default();
            format!("{cwd}|{}", argv.join(" "))
        }
    }

    impl ExecBackend for FakeBackend {
        fn run(&self, argv: &[&str], cwd: Option<&Path>) -> Result<ExecOutput> {
            let key = Self::key(argv, cwd);
            if let Some(out) = self.commands.lock().unwrap().get(&key) {
                return Ok(out.clone());
            }
            // Defaults for common probes
            if argv == ["true"] {
                return Ok(ExecOutput {
                    status: 0,
                    stdout: String::new(),
                    stderr: String::new(),
                });
            }
            if argv == ["pwd"] {
                return Ok(ExecOutput {
                    status: 0,
                    stdout: cwd
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| "/".into()),
                    stderr: String::new(),
                });
            }
            if argv.len() == 3
                && argv[0] == "sh"
                && argv[1] == "-c"
                && argv[2].starts_with("command -v ")
            {
                let name = argv[2].trim_start_matches("command -v ");
                return Ok(ExecOutput {
                    status: 0,
                    stdout: format!("/usr/bin/{name}"),
                    stderr: String::new(),
                });
            }
            if argv.len() == 2 && argv[1] == "--version" {
                return Ok(ExecOutput {
                    status: 0,
                    stdout: format!("{} 1.0.0\n", argv[0]),
                    stderr: String::new(),
                });
            }
            Ok(ExecOutput {
                status: 1,
                stdout: String::new(),
                stderr: format!("unexpected command: {key}"),
            })
        }

        fn read_to_string(&self, path: &Path) -> Result<String> {
            self.files
                .lock()
                .unwrap()
                .get(&path.display().to_string())
                .cloned()
                .with_context(|| format!("missing file {}", path.display()))
        }

        fn exists(&self, path: &Path) -> Result<bool> {
            let key = path.display().to_string();
            if self.files.lock().unwrap().contains_key(&key) {
                return Ok(true);
            }
            Ok(self.dirs.lock().unwrap().iter().any(|d| d == &key))
        }

        fn is_dir(&self, path: &Path) -> Result<bool> {
            let key = path.display().to_string();
            Ok(self.dirs.lock().unwrap().iter().any(|d| d == &key))
        }

        fn home_dir(&self) -> Result<PathBuf> {
            Ok(self.home.clone())
        }
    }

    #[test]
    fn parse_target_ok() {
        assert_eq!(
            parse_target("prod/api").unwrap(),
            ("prod".into(), "api".into())
        );
    }

    #[test]
    fn fake_backend_remote_doctor() {
        let backend = FakeBackend::new();
        backend.files.lock().unwrap().insert(
            "/home/deploy/.hermes/config.yaml".into(),
            "model:\n  provider: openai\n  default: gpt\n  base_url: https://gw.example/v1\n"
                .into(),
        );
        backend
            .dirs
            .lock()
            .unwrap()
            .push("/home/deploy/.hermes".into());

        let project = RemoteProjectEntry {
            path: "/srv/api".into(),
            runtimes: vec!["hermes".into()],
        };
        let report = run_remote_doctor_with_backend(
            "prod",
            "prod-vps",
            "api",
            &project,
            &backend,
            &RemoteDoctorOptions {
                runtime_filter: None,
                save_report: false,
            },
        )
        .unwrap();

        assert!(report.connectivity_ok);
        assert_eq!(report.runtimes.len(), 1);
        assert_eq!(report.runtimes[0].runtime_id, "hermes");
        let gateway = report.runtimes[0]
            .checks
            .iter()
            .find(|c| c.id.starts_with("config.gateway."))
            .expect("gateway check");
        assert!(gateway.details.iter().any(|d| d.contains("gw.example")));
    }
}
