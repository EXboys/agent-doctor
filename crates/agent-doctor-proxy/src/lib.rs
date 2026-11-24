//! Which network proxy an agent CLI should use.
//!
//! This crate does not run a proxy and never forwards traffic. It decides
//! which proxy variables a child process (Claude Code, Codex, Antigravity,
//! Hermes, ...) gets, so the child reaches the same sites the user's own
//! terminal reaches.
//!
//! A desktop app opened from the Dock gets a bare environment: no
//! `https_proxy` from `~/.zshrc`. Every CLI we start then talks to
//! `chatgpt.com` / `api.anthropic.com` directly and times out, while the same
//! CLI works in Terminal. The plan is picked in this order, first hit wins:
//!
//! 1. `AGENT_DOCTOR_PROXY` (explicit URL, or `direct` to strip every proxy).
//! 2. Proxy variables this process already has (launched from a terminal).
//! 3. Proxy variables from the user's login shell, if that proxy port answers.
//! 4. The OS system proxy (macOS network settings, Windows Internet Settings),
//!    if that proxy port answers.
//! 5. Nothing. TUN / VPN users land here and need nothing.
//!
//! Steps 3 and 4 run on a background thread: once at startup, then every 30
//! seconds (is the local proxy port still open, did the system proxy change).
//! Starting a child only reads that stored plan. It never waits on the shell
//! or on a socket.
//!
//! Only proxy and certificate variables are ever taken from the shell. Keys
//! such as `ANTHROPIC_API_KEY` in `~/.zshrc` must not change which service an
//! agent talks to. See `docs/proxy.md`.

use std::collections::BTreeMap;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

mod probe;
mod shell;
mod system;

pub use system::{HostPort, SystemProxy};

/// Proxy variables, lowercase first (curl, undici and Go read that order).
pub const PROXY_KEYS: &[&str] = &[
    "https_proxy",
    "HTTPS_PROXY",
    "http_proxy",
    "HTTP_PROXY",
    "all_proxy",
    "ALL_PROXY",
];

pub const NO_PROXY_KEYS: &[&str] = &["no_proxy", "NO_PROXY"];

/// Extra trust roots for company networks that inspect TLS.
pub const CA_KEYS: &[&str] = &[
    "NODE_EXTRA_CA_CERTS",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "REQUESTS_CA_BUNDLE",
    "CURL_CA_BUNDLE",
];

/// Explicit override. A URL wins over everything; `direct` strips all proxies.
pub const OVERRIDE_ENV: &str = "AGENT_DOCTOR_PROXY";

/// Local gateways this app runs must never go through a proxy.
const LOOPBACK_NO_PROXY: &[&str] = &["localhost", "127.0.0.1", "::1"];

pub type Vars = BTreeMap<String, String>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Explicit {
    Direct,
    Url(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProxySource {
    Explicit,
    Direct,
    Inherited,
    LoginShell,
    System,
    None,
}

impl ProxySource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Explicit => "explicit",
            Self::Direct => "direct",
            Self::Inherited => "inherited",
            Self::LoginShell => "login_shell",
            Self::System => "system",
            Self::None => "none",
        }
    }
}

/// Why a candidate was skipped, for logs and failure copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Note {
    ShellProxyUnreachable,
    SystemProxyUnreachable,
    SystemPacUnsupported,
    SystemSocksOnly,
}

impl Note {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ShellProxyUnreachable => "shell_proxy_unreachable",
            Self::SystemProxyUnreachable => "system_proxy_unreachable",
            Self::SystemPacUnsupported => "system_pac_unsupported",
            Self::SystemSocksOnly => "system_socks_only",
        }
    }
}

/// Everything [`resolve`] looks at. Each field is already read; resolve does no I/O.
#[derive(Debug, Clone, Default)]
pub struct Inputs {
    pub explicit: Option<Explicit>,
    /// Proxy / CA variables this process has.
    pub process: Vars,
    /// Proxy / CA variables from the login shell, when captured.
    pub shell: Option<Vars>,
    pub system: Option<SystemProxy>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxyPlan {
    pub source: ProxySource,
    /// Variables to set on the child.
    pub set: Vars,
    /// Variables to remove from the child.
    pub remove: Vec<String>,
    /// The proxy HTTPS traffic goes through, when one was picked.
    pub https_proxy: Option<String>,
    pub notes: Vec<Note>,
}

impl ProxyPlan {
    fn empty(source: ProxySource) -> Self {
        Self {
            source,
            set: Vars::new(),
            remove: Vec::new(),
            https_proxy: None,
            notes: Vec::new(),
        }
    }

    pub fn apply(&self, cmd: &mut Command) {
        for key in &self.remove {
            cmd.env_remove(key);
        }
        for (key, value) in &self.set {
            cmd.env(key, value);
        }
    }

    /// One log line with credentials removed.
    pub fn summary(&self) -> String {
        let mut line = format!("source={}", self.source.as_str());
        if let Some(url) = &self.https_proxy {
            line.push_str(&format!(" proxy={}", redact(url)));
        }
        if !self.notes.is_empty() {
            let notes: Vec<&str> = self.notes.iter().map(|note| note.as_str()).collect();
            line.push_str(&format!(" notes={}", notes.join(",")));
        }
        line
    }
}

/// Pick the plan. `alive` answers whether a proxy URL accepts connections.
pub fn resolve(inputs: &Inputs, alive: &dyn Fn(&str) -> bool) -> ProxyPlan {
    let mut plan = pick_proxy(inputs, alive);
    if plan.source != ProxySource::Direct {
        add_ca_vars(&mut plan, inputs);
    }
    plan
}

fn pick_proxy(inputs: &Inputs, alive: &dyn Fn(&str) -> bool) -> ProxyPlan {
    match &inputs.explicit {
        Some(Explicit::Direct) => {
            let mut plan = ProxyPlan::empty(ProxySource::Direct);
            plan.remove = PROXY_KEYS
                .iter()
                .chain(NO_PROXY_KEYS)
                .map(|key| key.to_string())
                .collect();
            return plan;
        }
        Some(Explicit::Url(url)) => {
            let mut plan = ProxyPlan::empty(ProxySource::Explicit);
            let no_proxy = merge_no_proxy(first_of(&inputs.process, NO_PROXY_KEYS), &[]);
            set_proxy_vars(&mut plan, url, url, None, &no_proxy);
            return plan;
        }
        None => {}
    }

    if PROXY_KEYS.iter().any(|key| has_value(&inputs.process, key)) {
        let mut plan = ProxyPlan::empty(ProxySource::Inherited);
        plan.https_proxy = https_from(&inputs.process);
        return plan;
    }

    let mut notes = Vec::new();

    if let Some(shell) = &inputs.shell {
        if let Some(https) = https_from(shell) {
            if alive(&https) {
                let http = pair_value(shell, "http_proxy", "HTTP_PROXY").unwrap_or(https.clone());
                let all = pair_value(shell, "all_proxy", "ALL_PROXY");
                let no_proxy = merge_no_proxy(first_of(shell, NO_PROXY_KEYS), &[]);
                let mut plan = ProxyPlan::empty(ProxySource::LoginShell);
                set_proxy_vars(&mut plan, &https, &http, all.as_deref(), &no_proxy);
                plan.notes = notes;
                return plan;
            }
            notes.push(Note::ShellProxyUnreachable);
        }
    }

    if let Some(system) = &inputs.system {
        let https = system.https.as_ref().or(system.http.as_ref());
        let http = system.http.as_ref().or(system.https.as_ref());
        match (https, http) {
            (Some(https), Some(http)) => {
                let https = https.url();
                if alive(&https) {
                    let no_proxy = merge_no_proxy(None, &system.exceptions);
                    let mut plan = ProxyPlan::empty(ProxySource::System);
                    set_proxy_vars(&mut plan, &https, &http.url(), None, &no_proxy);
                    plan.notes = notes;
                    return plan;
                }
                notes.push(Note::SystemProxyUnreachable);
            }
            _ => {
                // Claude Code and Node do not speak SOCKS, and Codex may reject a
                // socks URL it was not built for. Leave the child direct.
                if system.socks.is_some() {
                    notes.push(Note::SystemSocksOnly);
                }
            }
        }
        if system.pac {
            notes.push(Note::SystemPacUnsupported);
        }
    }

    let mut plan = ProxyPlan::empty(ProxySource::None);
    plan.notes = notes;
    plan
}

fn set_proxy_vars(
    plan: &mut ProxyPlan,
    https: &str,
    http: &str,
    all: Option<&str>,
    no_proxy: &str,
) {
    for key in ["https_proxy", "HTTPS_PROXY"] {
        plan.set.insert(key.into(), https.into());
    }
    for key in ["http_proxy", "HTTP_PROXY"] {
        plan.set.insert(key.into(), http.into());
    }
    if let Some(all) = all {
        for key in ["all_proxy", "ALL_PROXY"] {
            plan.set.insert(key.into(), all.into());
        }
    }
    for key in NO_PROXY_KEYS {
        plan.set.insert((*key).into(), no_proxy.into());
    }
    plan.https_proxy = Some(https.to_string());
}

fn add_ca_vars(plan: &mut ProxyPlan, inputs: &Inputs) {
    let Some(shell) = &inputs.shell else {
        return;
    };
    for key in CA_KEYS {
        if has_value(&inputs.process, key) {
            continue;
        }
        if let Some(value) = shell.get(*key).filter(|value| !value.trim().is_empty()) {
            plan.set.insert((*key).into(), value.clone());
        }
    }
}

fn has_value(vars: &Vars, key: &str) -> bool {
    vars.get(key).is_some_and(|value| !value.trim().is_empty())
}

fn first_of(vars: &Vars, keys: &[&str]) -> Option<String> {
    keys.iter()
        .filter_map(|key| vars.get(*key))
        .map(|value| value.trim().to_string())
        .find(|value| !value.is_empty())
}

fn pair_value(vars: &Vars, lower: &str, upper: &str) -> Option<String> {
    first_of(vars, &[lower, upper]).map(|value| normalize_url(&value))
}

/// The proxy HTTPS requests use: `https_proxy`, then `http_proxy`, then `all_proxy`.
fn https_from(vars: &Vars) -> Option<String> {
    first_of(vars, PROXY_KEYS).map(|value| normalize_url(&value))
}

/// `127.0.0.1:7890` becomes `http://127.0.0.1:7890`; a URL with a scheme stays.
pub fn normalize_url(value: &str) -> String {
    let value = value.trim();
    if value.contains("://") {
        value.to_string()
    } else {
        format!("http://{value}")
    }
}

/// Existing entries first, then OS exceptions, then loopback. No duplicates.
fn merge_no_proxy(existing: Option<String>, exceptions: &[String]) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut push = |entry: &str| {
        let entry = entry.trim();
        if !entry.is_empty() && !out.iter().any(|seen| seen.eq_ignore_ascii_case(entry)) {
            out.push(entry.to_string());
        }
    };
    if let Some(existing) = existing {
        for entry in existing.split(',') {
            push(entry);
        }
    }
    for entry in exceptions {
        push(entry);
    }
    for entry in LOOPBACK_NO_PROXY {
        push(entry);
    }
    out.join(",")
}

/// `http://user:pass@host:port` becomes `http://***@host:port`.
pub fn redact(url: &str) -> String {
    let (scheme, rest) = match url.split_once("://") {
        Some((scheme, rest)) => (format!("{scheme}://"), rest),
        None => (String::new(), url),
    };
    let authority_end = rest.find('/').unwrap_or(rest.len());
    match rest[..authority_end].rfind('@') {
        Some(at) => format!("{scheme}***@{}", &rest[at + 1..]),
        None => format!("{scheme}{rest}"),
    }
}

fn wanted_key(key: &str) -> bool {
    PROXY_KEYS.contains(&key) || NO_PROXY_KEYS.contains(&key) || CA_KEYS.contains(&key)
}

fn process_vars() -> Vars {
    std::env::vars()
        .filter(|(key, _)| wanted_key(key))
        .collect()
}

fn explicit_from_env() -> Option<Explicit> {
    let value = std::env::var(OVERRIDE_ENV).ok()?;
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    match value.to_ascii_lowercase().as_str() {
        "direct" | "off" | "none" => Some(Explicit::Direct),
        _ => Some(Explicit::Url(normalize_url(value))),
    }
}

static SHELL_WANTED: AtomicBool = AtomicBool::new(false);
static SHELL_VARS: OnceLock<Option<Vars>> = OnceLock::new();
static WARM_STARTED: AtomicBool = AtomicBool::new(false);

/// How often the background thread re-reads the system proxy and re-checks
/// the local proxy port. The login shell is read once, on the first pass.
const REFRESH_EVERY: Duration = Duration::from_secs(30);

static PLAN: Mutex<ProxyPlan> = Mutex::new(ProxyPlan {
    source: ProxySource::None,
    set: Vars::new(),
    remove: Vec::new(),
    https_proxy: None,
    notes: Vec::new(),
});

/// Start the background refresh. Desktop apps call this at launch so the first
/// ask already has a plan. Safe to call more than once.
pub fn warm() {
    if WARM_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    SHELL_WANTED.store(true, Ordering::SeqCst);
    let _ = std::thread::Builder::new()
        .name("agent-doctor-proxy".into())
        .spawn(|| loop {
            refresh();
            std::thread::sleep(REFRESH_EVERY);
        });
}

fn refresh() {
    let inputs = Inputs {
        shell: shell_vars(),
        system: system::read(),
        ..Inputs::default()
    };
    let plan = resolve(&inputs, &probe::alive);
    log_once(&plan);
    *PLAN.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = plan;
}

/// Blocks while the first read is still running (bounded by the shell timeout).
fn shell_vars() -> Option<Vars> {
    if !SHELL_WANTED.load(Ordering::SeqCst) {
        return None;
    }
    SHELL_VARS
        .get_or_init(|| {
            shell::capture(shell::CAPTURE_BUDGET).map(|vars| {
                vars.into_iter()
                    .filter(|(key, _)| wanted_key(key))
                    .collect()
            })
        })
        .clone()
}

static LAST_LOGGED: Mutex<Option<String>> = Mutex::new(None);

fn log_once(plan: &ProxyPlan) {
    let line = plan.summary();
    let mut last = LAST_LOGGED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if last.as_deref() != Some(line.as_str()) {
        eprintln!("[agent-doctor-proxy] {line}");
        *last = Some(line);
    }
}

/// The plan for a child started now. Reads the stored plan only.
/// `AGENT_DOCTOR_PROXY` and proxy variables already on this process win, and
/// those are read from the environment with no socket and no shell.
pub fn current() -> ProxyPlan {
    let cached = PLAN
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    let plan = plan_for_child(explicit_from_env(), &process_vars(), &cached);
    log_once(&plan);
    plan
}

/// Explicit override and an already-set process proxy win. Otherwise the
/// child gets whatever the background thread stored.
fn plan_for_child(explicit: Option<Explicit>, process: &Vars, cached: &ProxyPlan) -> ProxyPlan {
    if explicit.is_some() || PROXY_KEYS.iter().any(|key| has_value(process, key)) {
        return resolve(
            &Inputs {
                explicit,
                process: process.clone(),
                ..Inputs::default()
            },
            &|_url| true,
        );
    }
    cached.clone()
}

/// Put the current plan on `cmd`. Starts the background refresh if it is not
/// running yet; this call does not wait for it.
pub fn apply(cmd: &mut Command) {
    warm();
    current().apply(cmd);
}

#[cfg(test)]
mod tests;
