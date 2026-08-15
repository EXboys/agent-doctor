use std::fs;
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::adapter::RuntimeAdapter;
use crate::adapters::util::{find_all_binaries, read_version_result};
use crate::repair::{DiagnosticBundle, DiagnosticFact, SensitivityLevel};
use crate::runtime::{adapter_by_id, all_adapters, descriptor_by_id, ConfigFormat};

pub(crate) mod config;
mod mode_drift;
pub(crate) mod runtimes;
mod schema;

pub(crate) use config::ParsedConfig;
use config::{config_format_for_path, parse_config};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProbeStatus {
    Pass,
    Warn,
    Fail,
    NotApplicable,
    NotChecked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProbeSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeCheck {
    pub id: String,
    pub title: String,
    pub status: ProbeStatus,
    pub severity: ProbeSeverity,
    pub message: String,
    pub details: Vec<String>,
    pub sensitivity: SensitivityLevel,
}

impl ProbeCheck {
    pub(crate) fn new(
        id: impl Into<String>,
        title: impl Into<String>,
        status: ProbeStatus,
        severity: ProbeSeverity,
        message: impl Into<String>,
        sensitivity: SensitivityLevel,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            status,
            severity,
            message: message.into(),
            details: Vec::new(),
            sensitivity,
        }
    }

    pub(crate) fn with_details(mut self, details: Vec<String>) -> Self {
        self.details = details;
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeProbeReport {
    pub runtime_id: String,
    pub display_name: String,
    pub binary_name: String,
    pub checks: Vec<ProbeCheck>,
    pub facts: Vec<DiagnosticFact>,
}

impl RuntimeProbeReport {
    pub fn to_diagnostic_bundle(&self) -> DiagnosticBundle {
        DiagnosticBundle {
            runtime_id: self.runtime_id.clone(),
            facts: self.facts.clone(),
            notes: self
                .checks
                .iter()
                .filter(|check| matches!(check.status, ProbeStatus::Warn | ProbeStatus::Fail))
                .map(|check| format!("{}: {}", check.title, check.message))
                .collect(),
        }
    }
}

#[derive(Clone, Copy)]
struct RuntimeProbeContext<'a> {
    runtime_id: &'a str,
    binary_name: &'a str,
    config_format: ConfigFormat,
    env_keywords: &'a [&'static str],
}

pub fn probe_all_runtimes() -> Vec<RuntimeProbeReport> {
    all_adapters()
        .iter()
        .filter_map(|adapter| probe_adapter(adapter.as_ref()).ok())
        .collect()
}

pub fn probe_runtime(runtime_id: &str) -> Result<RuntimeProbeReport> {
    let adapter =
        adapter_by_id(runtime_id).with_context(|| format!("unknown runtime '{runtime_id}'"))?;
    probe_adapter(adapter.as_ref())
}

fn probe_adapter(adapter: &dyn RuntimeAdapter) -> Result<RuntimeProbeReport> {
    let descriptor = descriptor_by_id(adapter.id())
        .with_context(|| format!("no runtime descriptor for '{}'", adapter.id()))?;
    let spec = &descriptor.probe;
    let ctx = RuntimeProbeContext {
        runtime_id: descriptor.id,
        binary_name: spec.binary_name,
        config_format: spec.config_format,
        env_keywords: spec.env_keywords,
    };
    let mut checks = Vec::new();
    let mut facts = Vec::new();

    probe_binary(&ctx, &mut checks, &mut facts);
    probe_configs(adapter, &ctx, &mut checks, &mut facts);
    probe_env_conflicts(&ctx, &mut checks, &mut facts);
    descriptor.run_deep_probe(&mut checks, &mut facts);
    probe_gateway(adapter, &mut checks, &mut facts);
    mode_drift::probe_mode_drift(adapter.id(), &mut checks, &mut facts);
    crate::workspace::probe_browser_mcp_for_runtime(adapter.id(), &mut checks);

    Ok(RuntimeProbeReport {
        runtime_id: adapter.id().to_string(),
        display_name: adapter.display_name().to_string(),
        binary_name: spec.binary_name.to_string(),
        checks,
        facts,
    })
}

fn probe_binary(
    ctx: &RuntimeProbeContext<'_>,
    checks: &mut Vec<ProbeCheck>,
    facts: &mut Vec<DiagnosticFact>,
) {
    let binaries = find_all_binaries(ctx.binary_name);
    if binaries.is_empty() {
        checks.push(ProbeCheck::new(
            "binary.exists",
            "Binary exists",
            ProbeStatus::Fail,
            ProbeSeverity::Error,
            format!(
                "{} was not found in PATH or common bin directories",
                ctx.binary_name
            ),
            SensitivityLevel::Public,
        ));
        facts.push(DiagnosticFact::new(
            "binary.installed",
            "false",
            SensitivityLevel::Public,
        ));
        return;
    }

    let default_binary = binaries[0].display().to_string();
    checks.push(
        ProbeCheck::new(
            "binary.exists",
            "Binary exists",
            ProbeStatus::Pass,
            ProbeSeverity::Info,
            format!("{} was found", ctx.binary_name),
            SensitivityLevel::LocalPath,
        )
        .with_details(vec![default_binary.clone()]),
    );
    facts.push(DiagnosticFact::new(
        "binary.path",
        default_binary,
        SensitivityLevel::LocalPath,
    ));

    let conflict_status = if binaries.len() > 1 {
        ProbeStatus::Warn
    } else {
        ProbeStatus::Pass
    };
    checks.push(
        ProbeCheck::new(
            "binary.path_conflict",
            "Multiple installs",
            conflict_status,
            if binaries.len() > 1 {
                ProbeSeverity::Warning
            } else {
                ProbeSeverity::Info
            },
            if binaries.len() > 1 {
                format!("found {} candidate binaries", binaries.len())
            } else {
                "no duplicate install candidates found".to_string()
            },
            SensitivityLevel::LocalPath,
        )
        .with_details(
            binaries
                .iter()
                .map(|path| path.display().to_string())
                .collect(),
        ),
    );

    match read_version_result(&binaries[0]) {
        Ok(Some(version)) => {
            checks.push(ProbeCheck::new(
                "binary.version",
                "Version command",
                ProbeStatus::Pass,
                ProbeSeverity::Info,
                version.clone(),
                SensitivityLevel::Public,
            ));
            facts.push(DiagnosticFact::new(
                "binary.version",
                version,
                SensitivityLevel::Public,
            ));
        }
        Ok(None) => checks.push(ProbeCheck::new(
            "binary.version",
            "Version command",
            ProbeStatus::Warn,
            ProbeSeverity::Warning,
            "version command ran but returned no output",
            SensitivityLevel::Public,
        )),
        Err(error) => checks.push(ProbeCheck::new(
            "binary.version",
            "Version command",
            ProbeStatus::Fail,
            ProbeSeverity::Error,
            error,
            SensitivityLevel::SensitiveLog,
        )),
    }
}

fn probe_configs(
    adapter: &dyn RuntimeAdapter,
    ctx: &RuntimeProbeContext<'_>,
    checks: &mut Vec<ProbeCheck>,
    facts: &mut Vec<DiagnosticFact>,
) {
    let config_paths = adapter.config_paths();
    if config_paths.is_empty() {
        checks.push(ProbeCheck::new(
            "config.paths",
            "Config paths",
            ProbeStatus::NotApplicable,
            ProbeSeverity::Info,
            "adapter has no config paths",
            SensitivityLevel::Public,
        ));
        return;
    }

    for path in config_paths {
        let path_text = path.display().to_string();
        facts.push(DiagnosticFact::new(
            "config.path",
            path_text.clone(),
            SensitivityLevel::LocalPath,
        ));

        if !path.exists() {
            checks.push(ProbeCheck::new(
                format!("config.exists:{}", path.display()),
                "Config exists",
                ProbeStatus::Warn,
                ProbeSeverity::Warning,
                format!("config file not found at {}", path.display()),
                SensitivityLevel::LocalPath,
            ));
            continue;
        }

        checks.push(ProbeCheck::new(
            format!("config.exists:{}", path.display()),
            "Config exists",
            ProbeStatus::Pass,
            ProbeSeverity::Info,
            format!("config file exists at {}", path.display()),
            SensitivityLevel::LocalPath,
        ));

        let format = config_format_for_path(&path, ctx.config_format);
        match fs::read_to_string(&path) {
            Ok(raw) => match parse_config(&raw, format) {
                Ok(parsed) => {
                    checks.push(ProbeCheck::new(
                        format!("config.parse:{}", path.display()),
                        "Config parse",
                        ProbeStatus::Pass,
                        ProbeSeverity::Info,
                        "config parsed successfully",
                        SensitivityLevel::ConfigShape,
                    ));
                    probe_schema(ctx.runtime_id, &path, &parsed, checks, facts);
                    probe_path_references(&path, &parsed, checks, facts);
                }
                Err(error) => checks.push(ProbeCheck::new(
                    format!("config.parse:{}", path.display()),
                    "Config parse",
                    ProbeStatus::Fail,
                    ProbeSeverity::Error,
                    error,
                    SensitivityLevel::SensitiveLog,
                )),
            },
            Err(error) => checks.push(ProbeCheck::new(
                format!("config.read:{}", path.display()),
                "Config read",
                ProbeStatus::Fail,
                ProbeSeverity::Error,
                format!("failed to read {}: {}", path.display(), error),
                SensitivityLevel::LocalPath,
            )),
        }
    }
}

fn probe_schema(
    runtime_id: &str,
    path: &Path,
    parsed: &ParsedConfig,
    checks: &mut Vec<ProbeCheck>,
    facts: &mut Vec<DiagnosticFact>,
) {
    if let Some(descriptor) = descriptor_by_id(runtime_id) {
        descriptor.run_schema_probe(path, parsed, checks, facts);
    }
}

fn probe_env_conflicts(
    ctx: &RuntimeProbeContext<'_>,
    checks: &mut Vec<ProbeCheck>,
    facts: &mut Vec<DiagnosticFact>,
) {
    let conflicts = collect_env_conflicts(ctx.env_keywords);
    if conflicts.is_empty() {
        checks.push(ProbeCheck::new(
            "env.conflicts",
            "Environment conflicts",
            ProbeStatus::Pass,
            ProbeSeverity::Info,
            "no matching environment variables found in process or common shell files",
            SensitivityLevel::ConfigShape,
        ));
        return;
    }

    for conflict in &conflicts {
        facts.push(DiagnosticFact::new(
            "env.conflict",
            conflict.clone(),
            SensitivityLevel::SensitiveLog,
        ));
    }
    checks.push(
        ProbeCheck::new(
            "env.conflicts",
            "Environment conflicts",
            ProbeStatus::Warn,
            ProbeSeverity::Warning,
            format!(
                "found {} environment entries that may override runtime config",
                conflicts.len()
            ),
            SensitivityLevel::SensitiveLog,
        )
        .with_details(conflicts),
    );
}

fn collect_env_conflicts(keywords: &[&str]) -> Vec<String> {
    let mut conflicts = Vec::new();
    for (key, value) in std::env::vars() {
        if keywords
            .iter()
            .any(|keyword| key.to_uppercase().contains(keyword))
        {
            let visible = if looks_sensitive_env_key(&key) {
                "[REDACTED]".to_string()
            } else {
                value
            };
            conflicts.push(format!("process:{key}={visible}"));
        }
    }

    for path in shell_config_paths() {
        if let Ok(raw) = fs::read_to_string(&path) {
            for (idx, line) in raw.lines().enumerate() {
                let trimmed = line.trim();
                if trimmed.starts_with('#') || !trimmed.contains('=') {
                    continue;
                }
                let assignment = trimmed.strip_prefix("export ").unwrap_or(trimmed);
                let Some((name, _)) = assignment.split_once('=') else {
                    continue;
                };
                let name = name.trim();
                if keywords
                    .iter()
                    .any(|keyword| name.to_uppercase().contains(keyword))
                {
                    conflicts.push(format!("{}:{}:{}", path.display(), idx + 1, name));
                }
            }
        }
    }
    conflicts
}

fn looks_sensitive_env_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    ["key", "token", "secret", "password", "auth"]
        .iter()
        .any(|needle| key.contains(needle))
}

fn shell_config_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(home) = dirs::home_dir() {
        paths.extend([
            home.join(".bashrc"),
            home.join(".bash_profile"),
            home.join(".zshrc"),
            home.join(".zprofile"),
            home.join(".profile"),
        ]);
    }
    paths
}

fn probe_gateway(
    adapter: &dyn RuntimeAdapter,
    checks: &mut Vec<ProbeCheck>,
    facts: &mut Vec<DiagnosticFact>,
) {
    let profile = match adapter.read_profile() {
        Ok(profile) => profile,
        Err(error) => {
            checks.push(ProbeCheck::new(
                "gateway.profile_read",
                "Gateway profile",
                ProbeStatus::Warn,
                ProbeSeverity::Warning,
                format!("failed to read gateway profile: {error}"),
                SensitivityLevel::SensitiveLog,
            ));
            return;
        }
    };

    let Some(url) = profile.gateway_url.filter(|url| !url.trim().is_empty()) else {
        checks.push(ProbeCheck::new(
            "gateway.configured",
            "Gateway configured",
            ProbeStatus::NotApplicable,
            ProbeSeverity::Info,
            "no gateway/base_url configured",
            SensitivityLevel::ConfigShape,
        ));
        return;
    };

    facts.push(DiagnosticFact::new(
        "gateway.url",
        url.clone(),
        SensitivityLevel::ConfigShape,
    ));

    match gateway_socket_addr(&url) {
        Some(addr) => match addr.to_socket_addrs() {
            Ok(addrs) => {
                let deadline = Instant::now() + Duration::from_millis(800);
                let reachable = addrs.into_iter().any(|addr| {
                    if Instant::now() >= deadline {
                        return false;
                    }
                    let remain = deadline.saturating_duration_since(Instant::now());
                    TcpStream::connect_timeout(&addr, remain.min(Duration::from_millis(400)))
                        .is_ok()
                });
                checks.push(ProbeCheck::new(
                    "gateway.connectivity",
                    "Gateway connectivity",
                    if reachable {
                        ProbeStatus::Pass
                    } else {
                        ProbeStatus::Warn
                    },
                    if reachable {
                        ProbeSeverity::Info
                    } else {
                        ProbeSeverity::Warning
                    },
                    if reachable {
                        "gateway host accepted a TCP connection".to_string()
                    } else {
                        "gateway host did not accept a TCP connection within timeout".to_string()
                    },
                    SensitivityLevel::ConfigShape,
                ));
            }
            Err(error) => checks.push(ProbeCheck::new(
                "gateway.connectivity",
                "Gateway connectivity",
                ProbeStatus::Warn,
                ProbeSeverity::Warning,
                format!("failed to resolve gateway host: {error}"),
                SensitivityLevel::ConfigShape,
            )),
        },
        None => checks.push(ProbeCheck::new(
            "gateway.connectivity",
            "Gateway connectivity",
            ProbeStatus::Warn,
            ProbeSeverity::Warning,
            "gateway URL could not be parsed for connectivity check",
            SensitivityLevel::ConfigShape,
        )),
    }
}

fn gateway_socket_addr(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("https://")
        .map(|value| (value, 443))
        .or_else(|| url.strip_prefix("http://").map(|value| (value, 80)))?;
    let (host_port_path, default_port) = rest;
    let host_port = host_port_path.split('/').next()?.split('?').next()?;
    if host_port.is_empty() {
        return None;
    }
    if host_port.contains(':') {
        Some(host_port.to_string())
    } else {
        Some(format!("{host_port}:{default_port}"))
    }
}

fn probe_path_references(
    config_path: &Path,
    parsed: &ParsedConfig,
    checks: &mut Vec<ProbeCheck>,
    facts: &mut Vec<DiagnosticFact>,
) {
    let mut refs = Vec::new();
    collect_path_references(parsed, &mut refs);
    if refs.is_empty() {
        checks.push(ProbeCheck::new(
            format!("paths.references:{}", config_path.display()),
            "MCP/Skills path references",
            ProbeStatus::NotChecked,
            ProbeSeverity::Info,
            "no obvious MCP/Skills path references found",
            SensitivityLevel::ConfigShape,
        ));
        return;
    }

    let mut missing = Vec::new();
    for reference in refs {
        facts.push(DiagnosticFact::new(
            "path.reference",
            reference.clone(),
            SensitivityLevel::LocalPath,
        ));
        if !Path::new(&reference).exists() {
            missing.push(reference);
        }
    }

    checks.push(
        ProbeCheck::new(
            format!("paths.references:{}", config_path.display()),
            "MCP/Skills path references",
            if missing.is_empty() {
                ProbeStatus::Pass
            } else {
                ProbeStatus::Warn
            },
            if missing.is_empty() {
                ProbeSeverity::Info
            } else {
                ProbeSeverity::Warning
            },
            if missing.is_empty() {
                "all obvious MCP/Skills path references exist".to_string()
            } else {
                format!("{} obvious path references are missing", missing.len())
            },
            SensitivityLevel::LocalPath,
        )
        .with_details(missing),
    );
}

fn collect_path_references(parsed: &ParsedConfig, out: &mut Vec<String>) {
    match parsed {
        ParsedConfig::Json(value) => collect_json_paths("", value, out),
        ParsedConfig::Yaml(value) => collect_yaml_paths("", value, out),
        ParsedConfig::Toml(value) => collect_toml_paths("", value, out),
        ParsedConfig::Env(_) => {}
    }
}

fn collect_json_paths(key_path: &str, value: &serde_json::Value, out: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, value) in map {
                let next = join_key(key_path, key);
                collect_json_paths(&next, value, out);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_json_paths(key_path, item, out);
            }
        }
        serde_json::Value::String(text)
            if is_interesting_path_key(key_path) && is_path_like(text) =>
        {
            out.push(expand_home(text));
        }
        _ => {}
    }
}

fn collect_yaml_paths(key_path: &str, value: &serde_yaml::Value, out: &mut Vec<String>) {
    match value {
        serde_yaml::Value::Mapping(map) => {
            for (key, value) in map {
                let key = key.as_str().unwrap_or_default();
                let next = join_key(key_path, key);
                collect_yaml_paths(&next, value, out);
            }
        }
        serde_yaml::Value::Sequence(items) => {
            for item in items {
                collect_yaml_paths(key_path, item, out);
            }
        }
        serde_yaml::Value::String(text)
            if is_interesting_path_key(key_path) && is_path_like(text) =>
        {
            out.push(expand_home(text));
        }
        _ => {}
    }
}

fn collect_toml_paths(key_path: &str, value: &toml::Value, out: &mut Vec<String>) {
    match value {
        toml::Value::Table(map) => {
            for (key, value) in map {
                let next = join_key(key_path, key);
                collect_toml_paths(&next, value, out);
            }
        }
        toml::Value::Array(items) => {
            for item in items {
                collect_toml_paths(key_path, item, out);
            }
        }
        toml::Value::String(text) if is_interesting_path_key(key_path) && is_path_like(text) => {
            out.push(expand_home(text));
        }
        _ => {}
    }
}

fn join_key(base: &str, key: &str) -> String {
    if base.is_empty() {
        key.to_string()
    } else {
        format!("{base}.{key}")
    }
}

fn is_interesting_path_key(key_path: &str) -> bool {
    let key_path = key_path.to_ascii_lowercase();
    ["mcp", "skill", "skills", "manifest", "path", "command"]
        .iter()
        .any(|needle| key_path.contains(needle))
}

fn is_path_like(value: &str) -> bool {
    value.starts_with('/')
        || value.starts_with("~/")
        || value.starts_with("./")
        || value.starts_with("../")
}

fn expand_home(value: &str) -> String {
    if let Some(rest) = value.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest).display().to_string();
        }
    }
    value.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_gateway_socket_addr() {
        assert_eq!(
            gateway_socket_addr("https://gateway.example/v1").as_deref(),
            Some("gateway.example:443")
        );
        assert_eq!(
            gateway_socket_addr("http://127.0.0.1:11434/v1").as_deref(),
            Some("127.0.0.1:11434")
        );
    }

    #[test]
    fn collects_path_references_from_interesting_keys() {
        let value = serde_json::json!({
            "mcp": { "servers": [{ "path": "~/missing-mcp" }] },
            "ordinary": "/not/collected"
        });
        let mut refs = Vec::new();
        collect_json_paths("", &value, &mut refs);
        assert_eq!(refs.len(), 1);
        assert!(refs[0].contains("missing-mcp"));
    }
}
