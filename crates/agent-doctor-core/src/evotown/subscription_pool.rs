//! Team-edition subscription pool on the existing doctor WebSocket.
//!
//! The relay only routes accounts this computer has reported. Reports carry
//! the account name, remaining quota, and expiry. Login material stays on
//! this computer and is never written into the global Codex or Claude
//! account files by this path.
//!
//! Jobs keep today's launcher until the relay answers that it owns the pool.
//! After that, a job starts only once the relay assigns an account.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::edition::{product_edition, ProductEdition};
use crate::store::{open_settings_store, SettingsStore};

const META_KEY: &str = "team.subscription.accounts";
const CHECKOUT_WAIT: Duration = Duration::from_secs(30);

const IDLE_SENTENCE: &str = "现在没有空闲的订阅号。";
const QUOTA_SENTENCE: &str = "现在没有还有额度的订阅号。";
const REVOKED_SENTENCE: &str = "这个号已被公司收回。";

/// Account this computer logged in, as reported to the relay.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportedSubscription {
    pub name: String,
    pub remaining: i64,
    pub expires_at: String,
    /// Company turned this account off. Not sent to the relay in a report frame.
    #[serde(default)]
    pub disabled: bool,
    /// Optional per-checkout ceiling. Absent means this run may use the remaining quota.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cap: Option<i64>,
    /// Who currently holds the account. Empty when it is free.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub holder: Option<String>,
    /// `host:port` for this sandbox. Absent means the run uses this computer's network.
    /// Not sent on a quota report.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy: Option<String>,
    /// `claude-code` or `codex`. Empty on rows saved before this field existed.
    #[serde(default)]
    pub runtime: String,
    /// False until this computer has reported a real remaining amount.
    #[serde(default = "quota_is_known")]
    pub quota_known: bool,
}

fn quota_is_known() -> bool {
    true
}

/// One row of the team provider pool list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SubscriptionPoolRow {
    pub name: String,
    pub remaining: i64,
    pub expires_at: String,
    pub disabled: bool,
    pub holder: String,
    pub cap: Option<i64>,
    /// Bound egress `host:port`. Empty when this sandbox uses the computer's own network.
    #[serde(default)]
    pub proxy: String,
    #[serde(default)]
    pub runtime: String,
    #[serde(default = "quota_is_known")]
    pub quota_known: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssignedLease {
    pub job_id: String,
    pub name: String,
    pub lease_id: String,
    pub cap: Option<i64>,
    pub revoked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PendingCheckout {
    Waiting { since: Instant },
}

/// What the doctor socket should do after a pool event.
#[derive(Debug, Clone, PartialEq)]
pub enum PoolStep {
    Send(Value),
    /// Relay assigned an account. Start the job with today's launcher.
    Start {
        job_id: String,
        account: String,
        notice: Option<String>,
    },
    /// Do not start the tool. The sentence is the whole result.
    Stop {
        job_id: String,
        sentence: String,
    },
    /// Tell the running job the relay took the account back.
    Notice {
        job_id: String,
        sentence: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ServerPoolEvent {
    Assign {
        job_id: String,
        name: String,
        lease_id: String,
        cap: Option<i64>,
        switched_from: Option<String>,
    },
    None {
        job_id: String,
        reason: String,
    },
    Revoke {
        lease_id: String,
    },
}

/// In-memory checkout state for one doctor WebSocket session.
#[derive(Debug)]
pub struct PoolSession {
    enabled: bool,
    pending: BTreeMap<String, PendingCheckout>,
    leases: BTreeMap<String, AssignedLease>,
    reported: BTreeMap<String, ReportedSubscription>,
}

impl PoolSession {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            pending: BTreeMap::new(),
            leases: BTreeMap::new(),
            reported: BTreeMap::new(),
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn begin(&mut self, job_id: &str, prefer: Option<&str>, now: Instant) -> Vec<PoolStep> {
        if !self.enabled {
            return vec![PoolStep::Start {
                job_id: job_id.to_string(),
                account: String::new(),
                notice: None,
            }];
        }
        self.pending
            .insert(job_id.to_string(), PendingCheckout::Waiting { since: now });
        vec![PoolStep::Send(checkout_frame(job_id, prefer))]
    }

    pub fn on_server(&mut self, text: &str) -> Result<Vec<PoolStep>> {
        if !self.enabled {
            return Ok(Vec::new());
        }
        let Some(event) = parse_server_event(text)? else {
            return Ok(Vec::new());
        };
        Ok(self.apply(event))
    }

    fn apply(&mut self, event: ServerPoolEvent) -> Vec<PoolStep> {
        match event {
            ServerPoolEvent::Assign {
                job_id,
                name,
                lease_id,
                cap,
                switched_from,
            } => {
                if self.pending.remove(&job_id).is_none() {
                    return Vec::new();
                }
                self.leases.insert(
                    job_id.clone(),
                    AssignedLease {
                        job_id: job_id.clone(),
                        name: name.clone(),
                        lease_id,
                        cap,
                        revoked: false,
                    },
                );
                let notice = switched_from
                    .filter(|from| !from.is_empty() && from != &name)
                    .map(|_| format!("这个号正在用，这次改用{name}。"));
                vec![PoolStep::Start {
                    job_id,
                    account: name,
                    notice,
                }]
            }
            ServerPoolEvent::None { job_id, reason } => {
                if self.pending.remove(&job_id).is_none() {
                    return Vec::new();
                }
                vec![PoolStep::Stop {
                    job_id,
                    sentence: none_sentence(&reason).to_string(),
                }]
            }
            ServerPoolEvent::Revoke { lease_id } => {
                let Some(lease) = self
                    .leases
                    .values_mut()
                    .find(|lease| lease.lease_id == lease_id)
                else {
                    return Vec::new();
                };
                lease.revoked = true;
                let job_id = lease.job_id.clone();
                vec![PoolStep::Notice {
                    job_id,
                    sentence: REVOKED_SENTENCE.to_string(),
                }]
            }
        }
    }

    pub fn expire(&mut self, now: Instant) -> Vec<PoolStep> {
        if !self.enabled {
            return Vec::new();
        }
        let due: Vec<String> = self
            .pending
            .iter()
            .filter_map(|(job_id, pending)| match pending {
                PendingCheckout::Waiting { since }
                    if now.saturating_duration_since(*since) >= CHECKOUT_WAIT =>
                {
                    Some(job_id.clone())
                }
                _ => None,
            })
            .collect();
        due.into_iter()
            .filter_map(|job_id| {
                self.pending.remove(&job_id).map(|_| PoolStep::Stop {
                    job_id,
                    sentence: IDLE_SENTENCE.to_string(),
                })
            })
            .collect()
    }

    pub fn finish(&mut self, job_id: &str) -> Option<Value> {
        let lease = self.leases.remove(job_id)?;
        let reason = if lease.revoked { "revoked" } else { "finished" };
        Some(return_frame(&lease.name, &lease.lease_id, reason))
    }

    pub fn renew_frames(&self) -> Vec<Value> {
        if !self.enabled {
            return Vec::new();
        }
        self.leases
            .values()
            .filter(|lease| !lease.revoked)
            .map(|lease| renew_frame(&lease.name, &lease.lease_id))
            .collect()
    }

    /// Frames for accounts that appeared, changed, or left since the last sync.
    pub fn sync_reports(&mut self, next: &[ReportedSubscription]) -> Vec<Value> {
        if !self.enabled {
            return Vec::new();
        }
        let mut frames = Vec::new();
        let incoming: BTreeMap<_, _> = next
            .iter()
            .filter(|account| !account.name.is_empty() && account.quota_known)
            .map(|account| (account.name.clone(), account.clone()))
            .collect();
        for name in self.reported.keys() {
            if !incoming.contains_key(name) {
                frames.push(offline_frame(name));
            }
        }
        for (name, account) in &incoming {
            if self.reported.get(name) != Some(account) {
                frames.push(report_frame(account));
            }
        }
        self.reported = incoming;
        frames
    }
}

pub fn team_pool_build() -> bool {
    product_edition() == ProductEdition::Team
}

pub fn server_ack_enables_pool(ack: &Value) -> bool {
    if ack.get("subscription_pool").and_then(Value::as_bool) == Some(true) {
        return true;
    }
    ack.get("capabilities")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .any(|item| item.as_str() == Some("subscription.pool"))
        })
        .unwrap_or(false)
}

pub fn prefer_account(payload: &Value) -> Option<String> {
    for key in ["subscription", "account"] {
        if let Some(name) = payload
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty())
        {
            return Some(name.to_string());
        }
    }
    None
}

pub fn report_frame(account: &ReportedSubscription) -> Value {
    json!({
        "type": "subscription.report",
        "name": account.name,
        "remaining": account.remaining,
        "expires_at": account.expires_at,
        "runtime": account.runtime,
    })
}

fn checkout_frame(job_id: &str, prefer: Option<&str>) -> Value {
    let mut frame = json!({
        "type": "subscription.checkout",
        "job_id": job_id,
    });
    if let Some(name) = prefer.map(str::trim).filter(|name| !name.is_empty()) {
        frame["prefer"] = json!(name);
    }
    frame
}

fn renew_frame(name: &str, lease_id: &str) -> Value {
    json!({
        "type": "subscription.renew",
        "name": name,
        "lease_id": lease_id,
    })
}

fn return_frame(name: &str, lease_id: &str, reason: &str) -> Value {
    json!({
        "type": "subscription.return",
        "name": name,
        "lease_id": lease_id,
        "reason": reason,
    })
}

fn offline_frame(name: &str) -> Value {
    json!({
        "type": "subscription.offline",
        "name": name,
    })
}

fn none_sentence(reason: &str) -> &'static str {
    if reason == "quota" {
        QUOTA_SENTENCE
    } else {
        IDLE_SENTENCE
    }
}

fn parse_server_event(text: &str) -> Result<Option<ServerPoolEvent>> {
    let msg: Value = serde_json::from_str(text)?;
    let kind = msg.get("type").and_then(Value::as_str).unwrap_or("");
    if !kind.starts_with("subscription.") {
        return Ok(None);
    }
    match kind {
        "subscription.assign" => {
            let job_id = required_str(&msg, "job_id")?;
            let name = required_str(&msg, "name")?;
            let lease_id = required_str(&msg, "lease_id")?;
            let cap = msg.get("cap").and_then(Value::as_i64);
            let switched_from = msg
                .get("switched_from")
                .and_then(Value::as_str)
                .map(str::to_string);
            Ok(Some(ServerPoolEvent::Assign {
                job_id,
                name,
                lease_id,
                cap,
                switched_from,
            }))
        }
        "subscription.none" => Ok(Some(ServerPoolEvent::None {
            job_id: required_str(&msg, "job_id")?,
            reason: msg
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("idle")
                .to_string(),
        })),
        "subscription.revoke" => Ok(Some(ServerPoolEvent::Revoke {
            lease_id: required_str(&msg, "lease_id")?,
        })),
        _ => Ok(None),
    }
}

fn required_str(msg: &Value, key: &str) -> Result<String> {
    msg.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("subscription frame missing {key}"))
}

#[cfg(test)]
pub fn frame_carries_login(frame: &Value) -> bool {
    const SECRET_KEYS: &[&str] = &[
        "token",
        "password",
        "secret",
        "license",
        "api_key",
        "authorization",
        "credential",
        "refresh_token",
        "access_token",
    ];
    frame.as_object().is_some_and(|object| {
        object.keys().any(|key| {
            SECRET_KEYS
                .iter()
                .any(|secret| key.eq_ignore_ascii_case(secret))
        })
    })
}

fn normalize_account(account: ReportedSubscription) -> Result<ReportedSubscription> {
    let name = account.name.trim();
    let expires_at = account.expires_at.trim();
    if name.is_empty() || name.chars().count() > 64 {
        bail!("订阅号名字无效");
    }
    if expires_at.is_empty() || expires_at.chars().count() > 64 {
        bail!("订阅号有效期无效");
    }
    if account.remaining < 0 {
        bail!("订阅号剩余额度无效");
    }
    let holder = account
        .holder
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let proxy = normalize_proxy(account.proxy)?;
    if let Some(cap) = account.cap {
        if cap < 0 {
            bail!("一次上限无效");
        }
    }
    Ok(ReportedSubscription {
        name: name.to_string(),
        remaining: account.remaining,
        expires_at: expires_at.to_string(),
        disabled: account.disabled,
        cap: account.cap,
        holder,
        proxy,
        runtime: normalize_runtime(&account.runtime),
        quota_known: account.quota_known,
    })
}

pub fn load_reported_subscriptions() -> Result<Vec<ReportedSubscription>> {
    if !team_pool_build() {
        return Ok(Vec::new());
    }
    let store = open_settings_store()?;
    load_reported_subscriptions_from(&store)
}

pub fn load_reported_subscriptions_from(
    store: &SettingsStore,
) -> Result<Vec<ReportedSubscription>> {
    let Some(raw) = store.meta_get(META_KEY)? else {
        return Ok(Vec::new());
    };
    let parsed = serde_json::from_str::<Vec<ReportedSubscription>>(&raw).unwrap_or_default();
    Ok(parsed
        .into_iter()
        .filter_map(|account| normalize_account(account).ok())
        .collect())
}

pub fn remember_reported_subscription(account: ReportedSubscription) -> Result<()> {
    if !team_pool_build() {
        bail!("订阅号上报只在团队版里");
    }
    let store = open_settings_store()?;
    remember_reported_subscription_in(&store, account)
}

pub fn remember_reported_subscription_in(
    store: &SettingsStore,
    account: ReportedSubscription,
) -> Result<()> {
    let account = normalize_account(account)?;
    let mut accounts = load_reported_subscriptions_from(store)?;
    if let Some(existing) = accounts.iter_mut().find(|item| item.name == account.name) {
        existing.remaining = account.remaining;
        existing.expires_at = account.expires_at;
        if !account.runtime.is_empty() {
            existing.runtime = account.runtime;
        }
    } else {
        accounts.push(account);
    }
    save_reported_subscriptions(store, &accounts)
}

pub fn list_subscription_pool() -> Result<Vec<SubscriptionPoolRow>> {
    if !team_pool_build() {
        return Ok(Vec::new());
    }
    let store = open_settings_store()?;
    Ok(list_subscription_pool_from(&store))
}

pub fn list_subscription_pool_from(store: &SettingsStore) -> Vec<SubscriptionPoolRow> {
    load_reported_subscriptions_from(store)
        .unwrap_or_default()
        .into_iter()
        .map(pool_row)
        .collect()
}

pub fn set_subscription_disabled(name: &str, disabled: bool) -> Result<SubscriptionPoolRow> {
    if !team_pool_build() {
        bail!("订阅号号池只在团队版里");
    }
    let store = open_settings_store()?;
    set_subscription_disabled_in(&store, name, disabled)
}

pub fn set_subscription_disabled_in(
    store: &SettingsStore,
    name: &str,
    disabled: bool,
) -> Result<SubscriptionPoolRow> {
    update_subscription(store, name, |account| {
        account.disabled = disabled;
    })
}

pub fn reclaim_subscription(name: &str) -> Result<SubscriptionPoolRow> {
    if !team_pool_build() {
        bail!("订阅号号池只在团队版里");
    }
    let store = open_settings_store()?;
    reclaim_subscription_in(&store, name)
}

pub fn reclaim_subscription_in(store: &SettingsStore, name: &str) -> Result<SubscriptionPoolRow> {
    update_subscription(store, name, |account| {
        account.holder = None;
    })
}

pub fn set_subscription_cap(name: &str, cap: Option<i64>) -> Result<SubscriptionPoolRow> {
    if !team_pool_build() {
        bail!("订阅号号池只在团队版里");
    }
    let store = open_settings_store()?;
    set_subscription_cap_in(&store, name, cap)
}

pub fn set_subscription_cap_in(
    store: &SettingsStore,
    name: &str,
    cap: Option<i64>,
) -> Result<SubscriptionPoolRow> {
    if let Some(cap) = cap {
        if cap < 0 {
            bail!("一次上限无效");
        }
    }
    update_subscription(store, name, |account| {
        account.cap = cap.filter(|value| *value > 0);
    })
}

pub fn set_subscription_proxy(name: &str, proxy: Option<String>) -> Result<SubscriptionPoolRow> {
    if !team_pool_build() {
        bail!("订阅号号池只在团队版里");
    }
    let store = open_settings_store()?;
    set_subscription_proxy_in(&store, name, proxy)
}

pub fn set_subscription_proxy_in(
    store: &SettingsStore,
    name: &str,
    proxy: Option<String>,
) -> Result<SubscriptionPoolRow> {
    let proxy = normalize_proxy(proxy)?;
    update_subscription(store, name, |account| {
        account.proxy = proxy;
    })
}

/// Accept `1.2.3.4:8080` or `http://1.2.3.4:8080`. Empty clears the binding.
fn normalize_runtime(runtime: &str) -> String {
    match runtime.trim() {
        "claude-code" | "codex" => runtime.trim().to_string(),
        _ => String::new(),
    }
}

fn normalize_proxy(proxy: Option<String>) -> Result<Option<String>> {
    let Some(proxy) = proxy else {
        return Ok(None);
    };
    let text = proxy.trim();
    if text.is_empty() {
        return Ok(None);
    }
    if text.chars().count() > 200 {
        bail!("要写成地址加端口，例如 1.2.3.4:8080。");
    }
    let without_scheme = text
        .strip_prefix("https://")
        .or_else(|| text.strip_prefix("http://"))
        .unwrap_or(text);
    if without_scheme.contains('@') || without_scheme.contains('/') || without_scheme.contains(' ')
    {
        bail!("要写成地址加端口，例如 1.2.3.4:8080。");
    }
    let (host, port) = split_proxy_host_port(without_scheme)?;
    if host.is_empty() || port == 0 {
        bail!("要写成地址加端口，例如 1.2.3.4:8080。");
    }
    Ok(Some(format!("{host}:{port}")))
}

fn split_proxy_host_port(text: &str) -> Result<(String, u16)> {
    let invalid = || anyhow::anyhow!("要写成地址加端口，例如 1.2.3.4:8080。");
    if let Some(rest) = text.strip_prefix('[') {
        let Some((host, port_part)) = rest.split_once(']') else {
            bail!("要写成地址加端口，例如 1.2.3.4:8080。");
        };
        let Some(port) = port_part.strip_prefix(':') else {
            return Err(invalid());
        };
        let port: u16 = port.parse().map_err(|_| invalid())?;
        if host.is_empty() {
            return Err(invalid());
        }
        return Ok((format!("[{host}]"), port));
    }
    let Some((host, port)) = text.rsplit_once(':') else {
        return Err(invalid());
    };
    if host.is_empty() || host.contains(':') {
        return Err(invalid());
    }
    let port: u16 = port.parse().map_err(|_| invalid())?;
    Ok((host.to_string(), port))
}

fn update_subscription(
    store: &SettingsStore,
    name: &str,
    change: impl FnOnce(&mut ReportedSubscription),
) -> Result<SubscriptionPoolRow> {
    let name = name.trim();
    let mut accounts = load_reported_subscriptions_from(store)?;
    let Some(account) = accounts.iter_mut().find(|item| item.name == name) else {
        bail!("号池里没有这个订阅号");
    };
    change(account);
    let row = pool_row(account.clone());
    save_reported_subscriptions(store, &accounts)?;
    Ok(row)
}

fn pool_row(account: ReportedSubscription) -> SubscriptionPoolRow {
    SubscriptionPoolRow {
        name: account.name,
        remaining: account.remaining,
        expires_at: account.expires_at,
        disabled: account.disabled,
        holder: account.holder.unwrap_or_default(),
        cap: account.cap,
        proxy: account.proxy.unwrap_or_default(),
        runtime: account.runtime,
        quota_known: account.quota_known,
    }
}

pub fn forget_reported_subscription(name: &str) -> Result<()> {
    if !team_pool_build() {
        bail!("订阅号上报只在团队版里");
    }
    let store = open_settings_store()?;
    forget_reported_subscription_in(&store, name)
}

pub fn forget_reported_subscription_in(store: &SettingsStore, name: &str) -> Result<()> {
    let name = name.trim();
    let accounts = load_reported_subscriptions_from(store)?
        .into_iter()
        .filter(|account| account.name != name)
        .collect::<Vec<_>>();
    save_reported_subscriptions(store, &accounts)
}

fn save_reported_subscriptions(
    store: &SettingsStore,
    accounts: &[ReportedSubscription],
) -> Result<()> {
    store.meta_set(META_KEY, &serde_json::to_string(accounts)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::secrets::MemorySecretBackend;
    use crate::store::SettingsStore;
    use std::sync::Arc;

    fn account(name: &str, remaining: i64) -> ReportedSubscription {
        ReportedSubscription {
            name: name.into(),
            remaining,
            expires_at: "2026-12-01T00:00:00Z".into(),
            disabled: false,
            cap: None,
            holder: None,
            proxy: None,
            runtime: String::new(),
            quota_known: true,
        }
    }

    fn enabled() -> PoolSession {
        PoolSession::new(true)
    }

    #[test]
    fn report_frame_has_no_login_material() {
        let frame = report_frame(&account("甲", 20));
        assert!(!frame_carries_login(&frame));
        assert_eq!(frame["type"], "subscription.report");
        assert!(frame.get("token").is_none());
        assert!(frame.get("api_key").is_none());
    }

    #[test]
    fn checkout_assign_starts_without_login_frame() {
        let mut pool = enabled();
        let now = Instant::now();
        let steps = pool.begin("job-1", Some("甲"), now);
        assert!(
            matches!(&steps[0], PoolStep::Send(frame) if frame["type"] == "subscription.checkout" && !frame_carries_login(frame))
        );
        let steps = pool
            .on_server(
                r#"{"type":"subscription.assign","job_id":"job-1","name":"甲","lease_id":"lease-1"}"#,
            )
            .unwrap();
        assert_eq!(
            steps,
            vec![PoolStep::Start {
                job_id: "job-1".into(),
                account: "甲".into(),
                notice: None,
            }]
        );
        let done = pool.finish("job-1").unwrap();
        assert_eq!(done["reason"], "finished");
        assert!(!frame_carries_login(&done));
    }

    #[test]
    fn none_quota_stops_with_sentence() {
        let mut pool = enabled();
        pool.begin("job-1", None, Instant::now());
        let steps = pool
            .on_server(r#"{"type":"subscription.none","job_id":"job-1","reason":"quota"}"#)
            .unwrap();
        assert_eq!(
            steps,
            vec![PoolStep::Stop {
                job_id: "job-1".into(),
                sentence: QUOTA_SENTENCE.into(),
            }]
        );
        assert!(pool.finish("job-1").is_none());
    }

    #[test]
    fn switched_account_names_the_replacement() {
        let mut pool = enabled();
        pool.begin("job-1", Some("甲"), Instant::now());
        let steps = pool
            .on_server(
                r#"{"type":"subscription.assign","job_id":"job-1","name":"乙","lease_id":"lease-2","switched_from":"甲"}"#,
            )
            .unwrap();
        match &steps[0] {
            PoolStep::Start {
                account, notice, ..
            } => {
                assert_eq!(account, "乙");
                assert_eq!(notice.as_deref(), Some("这个号正在用，这次改用乙。"));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn revoke_is_reported_when_the_job_ends() {
        let mut pool = enabled();
        pool.begin("job-1", None, Instant::now());
        pool.on_server(
            r#"{"type":"subscription.assign","job_id":"job-1","name":"甲","lease_id":"lease-9"}"#,
        )
        .unwrap();
        let steps = pool
            .on_server(r#"{"type":"subscription.revoke","lease_id":"lease-9"}"#)
            .unwrap();
        assert_eq!(
            steps,
            vec![PoolStep::Notice {
                job_id: "job-1".into(),
                sentence: REVOKED_SENTENCE.into(),
            }]
        );
        assert_eq!(pool.finish("job-1").unwrap()["reason"], "revoked");
        assert!(pool.renew_frames().is_empty());
    }

    #[test]
    fn checkout_expires_into_the_idle_sentence() {
        let mut pool = enabled();
        let start = Instant::now();
        pool.begin("job-1", None, start);
        assert!(pool.expire(start + Duration::from_secs(5)).is_empty());
        let steps = pool.expire(start + CHECKOUT_WAIT);
        assert_eq!(
            steps,
            vec![PoolStep::Stop {
                job_id: "job-1".into(),
                sentence: IDLE_SENTENCE.into(),
            }]
        );
    }

    #[test]
    fn sync_reports_changes_and_departures() {
        let mut pool = enabled();
        let first = pool.sync_reports(&[account("甲", 10)]);
        assert_eq!(first.len(), 1);
        assert!(!frame_carries_login(&first[0]));
        assert!(pool.sync_reports(&[account("甲", 10)]).is_empty());
        let changed = pool.sync_reports(&[account("甲", 3), account("乙", 1)]);
        assert_eq!(changed.len(), 2);
        assert!(changed.iter().any(|frame| frame["name"] == "甲"));
        assert!(changed.iter().any(|frame| frame["name"] == "乙"));
        let left = pool.sync_reports(&[account("乙", 1)]);
        assert_eq!(left[0]["type"], "subscription.offline");
        assert_eq!(left[0]["name"], "甲");
    }

    #[test]
    fn server_ack_turns_the_pool_on() {
        assert!(!server_ack_enables_pool(&json!({"type": "ack"})));
        assert!(server_ack_enables_pool(&json!({"subscription_pool": true})));
        assert!(server_ack_enables_pool(
            &json!({"capabilities": ["subscription.pool"]})
        ));
    }

    #[test]
    fn personal_build_refuses_to_record_an_account() {
        if team_pool_build() {
            return;
        }
        let err = remember_reported_subscription(account("甲", 1)).unwrap_err();
        assert!(err.to_string().contains("团队版"));
        assert!(load_reported_subscriptions().unwrap().is_empty());
    }

    #[test]
    fn stored_accounts_roundtrip_without_login_fields() {
        let dir = tempfile::tempdir().unwrap();
        let store = SettingsStore::open_at(
            &dir.path().join("settings.db"),
            Arc::new(MemorySecretBackend::new()),
        )
        .unwrap();
        remember_reported_subscription_in(&store, account("甲", 8)).unwrap();
        remember_reported_subscription_in(&store, account("甲", 4)).unwrap();
        let loaded = load_reported_subscriptions_from(&store).unwrap();
        assert_eq!(loaded, vec![account("甲", 4)]);
        let raw = store.meta_get(META_KEY).unwrap().unwrap();
        assert!(!raw.to_ascii_lowercase().contains("token"));
        assert!(!raw.to_ascii_lowercase().contains("api_key"));
        forget_reported_subscription_in(&store, "甲").unwrap();
        assert!(load_reported_subscriptions_from(&store).unwrap().is_empty());
    }

    #[test]
    fn pool_admin_keeps_a_later_quota_report() {
        let dir = tempfile::tempdir().unwrap();
        let store = SettingsStore::open_at(
            &dir.path().join("settings.db"),
            Arc::new(MemorySecretBackend::new()),
        )
        .unwrap();
        remember_reported_subscription_in(&store, account("甲", 8)).unwrap();
        set_subscription_disabled_in(&store, "甲", true).unwrap();
        set_subscription_cap_in(&store, "甲", Some(3)).unwrap();
        let mut accounts = load_reported_subscriptions_from(&store).unwrap();
        accounts[0].holder = Some("这台电脑".into());
        save_reported_subscriptions(&store, &accounts).unwrap();
        remember_reported_subscription_in(&store, account("甲", 2)).unwrap();
        let row = reclaim_subscription_in(&store, "甲").unwrap();
        assert!(row.disabled);
        assert_eq!(row.remaining, 2);
        assert_eq!(row.cap, Some(3));
        assert_eq!(row.holder, "");
        let frame = report_frame(&load_reported_subscriptions_from(&store).unwrap()[0]);
        assert!(frame.get("disabled").is_none());
        assert!(frame.get("cap").is_none());
        assert!(frame.get("holder").is_none());
        assert!(frame.get("proxy").is_none());
    }

    #[test]
    fn proxy_binding_stays_on_the_account_and_off_the_report() {
        let dir = tempfile::tempdir().unwrap();
        let store = SettingsStore::open_at(
            &dir.path().join("settings.db"),
            Arc::new(MemorySecretBackend::new()),
        )
        .unwrap();
        remember_reported_subscription_in(&store, account("甲", 8)).unwrap();
        let row =
            set_subscription_proxy_in(&store, "甲", Some("http://1.2.3.4:8080".into())).unwrap();
        assert_eq!(row.proxy, "1.2.3.4:8080");
        remember_reported_subscription_in(&store, account("甲", 2)).unwrap();
        let row = list_subscription_pool_from(&store)
            .into_iter()
            .next()
            .unwrap();
        assert_eq!(row.remaining, 2);
        assert_eq!(row.proxy, "1.2.3.4:8080");
        let cleared = set_subscription_proxy_in(&store, "甲", None).unwrap();
        assert_eq!(cleared.proxy, "");
        let err = set_subscription_proxy_in(&store, "甲", Some("1.2.3.4".into())).unwrap_err();
        assert!(err.to_string().contains("1.2.3.4:8080"));
        let frame = report_frame(&load_reported_subscriptions_from(&store).unwrap()[0]);
        assert!(frame.get("proxy").is_none());
    }

    #[test]
    fn disabled_session_does_not_speak_the_pool() {
        let mut pool = PoolSession::new(false);
        let steps = pool.begin("job-1", None, Instant::now());
        assert!(matches!(steps[0], PoolStep::Start { .. }));
        assert!(pool
            .on_server(r#"{"type":"subscription.none","job_id":"job-1","reason":"idle"}"#)
            .unwrap()
            .is_empty());
        assert!(pool.sync_reports(&[account("甲", 1)]).is_empty());
    }
}
