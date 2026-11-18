//! How much a signed-in account can still spend.
//!
//! The read follows each product's own usage call: Qoder's credit pools,
//! WorkBuddy's plan plus extra credits, and a ZCode coding plan's 5-hour
//! and weekly windows. A missing sign-in or a reply that does not name a
//! number stays empty. Nothing here is invented.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use aes::cipher::block_padding::Pkcs7;
use aes::cipher::{BlockDecryptMut, KeyIvInit};
use aes::Aes128;
use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine;
use cbc::Decryptor;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::adapters::util::{home_dir, home_join, is_test_home_active};

const QUOTA_OK_TTL: Duration = Duration::from_secs(90);
const QUOTA_MISS_TTL: Duration = Duration::from_secs(15);
const QUOTA_WAIT: Duration = Duration::from_secs(8);

/// Credits still left in the pool that stops the account.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct CreditAllowance {
    pub left: Option<f64>,
}

impl CreditAllowance {
    fn known(self) -> bool {
        self.left.is_some()
    }
}

/// A coding plan's short window and weekly window, as used percents.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PlanWindows {
    pub hours: Option<u32>,
    pub hours_used_percent: Option<u8>,
    pub week_used_percent: Option<u8>,
}

impl PlanWindows {
    fn known(self) -> bool {
        self.hours_used_percent.is_some() || self.week_used_percent.is_some()
    }
}

struct CreditSnapshot {
    at: Instant,
    allowance: CreditAllowance,
}

struct WindowSnapshot {
    at: Instant,
    windows: PlanWindows,
}

static QODER_CREDITS: Mutex<Option<CreditSnapshot>> = Mutex::new(None);
static WORKBUDDY_CREDITS: Mutex<Option<CreditSnapshot>> = Mutex::new(None);
static ZCODE_WINDOWS: Mutex<Option<WindowSnapshot>> = Mutex::new(None);

/// Qoder credits still left. Cached so opening the menu does not ask again
/// on every status check.
pub fn qoder_credits() -> CreditAllowance {
    cached_credits(&QODER_CREDITS)
        .unwrap_or_else(|| store_credits(&QODER_CREDITS, fetch_qoder_credits()))
}

/// WorkBuddy credits still left, plan and extra credits counted together.
pub fn workbuddy_credits() -> CreditAllowance {
    cached_credits(&WORKBUDDY_CREDITS)
        .unwrap_or_else(|| store_credits(&WORKBUDDY_CREDITS, fetch_workbuddy_credits()))
}

/// ZCode coding plan windows. The 5-hour window and the weekly window are
/// one pool for every model on that plan.
pub fn zcode_plan_windows() -> PlanWindows {
    cached_windows(&ZCODE_WINDOWS)
        .unwrap_or_else(|| store_windows(&ZCODE_WINDOWS, fetch_zcode_windows()))
}

fn cached_credits(slot: &Mutex<Option<CreditSnapshot>>) -> Option<CreditAllowance> {
    let guard = slot.lock().ok()?;
    let snap = guard.as_ref()?;
    let ttl = if snap.allowance.known() {
        QUOTA_OK_TTL
    } else {
        QUOTA_MISS_TTL
    };
    (snap.at.elapsed() < ttl).then_some(snap.allowance)
}

fn store_credits(
    slot: &Mutex<Option<CreditSnapshot>>,
    allowance: CreditAllowance,
) -> CreditAllowance {
    if let Ok(mut guard) = slot.lock() {
        *guard = Some(CreditSnapshot {
            at: Instant::now(),
            allowance,
        });
    }
    allowance
}

fn cached_windows(slot: &Mutex<Option<WindowSnapshot>>) -> Option<PlanWindows> {
    let guard = slot.lock().ok()?;
    let snap = guard.as_ref()?;
    let ttl = if snap.windows.known() {
        QUOTA_OK_TTL
    } else {
        QUOTA_MISS_TTL
    };
    (snap.at.elapsed() < ttl).then_some(snap.windows)
}

fn store_windows(slot: &Mutex<Option<WindowSnapshot>>, windows: PlanWindows) -> PlanWindows {
    if let Ok(mut guard) = slot.lock() {
        *guard = Some(WindowSnapshot {
            at: Instant::now(),
            windows,
        });
    }
    windows
}

fn fetch_qoder_credits() -> CreditAllowance {
    if is_test_home_active() {
        return CreditAllowance::default();
    }
    let Some((token, cn)) = qoder_device_token() else {
        return CreditAllowance::default();
    };
    let host = if cn {
        "https://openapi.qoder.com.cn"
    } else {
        "https://openapi.qoder.sh"
    };
    let Some(body) = http_json(
        reqwest::Method::GET,
        &format!("{host}/sash/api/v2/me/usage"),
        &[
            ("Accept", "application/json"),
            ("Authorization", &format!("Bearer {token}")),
            ("Cosy-ClientType", "10"),
            ("User-Agent", "Qoder"),
        ],
        None,
    ) else {
        return CreditAllowance::default();
    };
    parse_qoder_usage(&body)
}

fn fetch_workbuddy_credits() -> CreditAllowance {
    if is_test_home_active() {
        return CreditAllowance::default();
    }
    let Some(sign_in) = workbuddy_sign_in() else {
        return CreditAllowance::default();
    };
    let auth = format!("Bearer {}", sign_in.access);
    let Some(body) = http_json(
        reqwest::Method::POST,
        &format!(
            "{}/billing/meter/get-user-resource-summary",
            sign_in.endpoint
        ),
        &[
            ("Content-Type", "application/json"),
            ("Accept", "application/json"),
            ("User-Agent", "WorkBuddy/5.9.3"),
            ("Authorization", &auth),
            ("X-User-Id", &sign_in.uid),
            ("X-Domain", &sign_in.domain),
            ("X-Product", "SaaS"),
            ("X-IDE-Type", "WorkBuddy"),
        ],
        Some("{}"),
    ) else {
        return CreditAllowance::default();
    };
    if body
        .get("code")
        .and_then(json_number)
        .is_some_and(|code| code != 0.0)
    {
        return CreditAllowance::default();
    }
    parse_workbuddy_summary(&body)
}

fn fetch_zcode_windows() -> PlanWindows {
    if is_test_home_active() {
        return PlanWindows::default();
    }
    let Some((root, key)) = zcode_plan_key() else {
        return PlanWindows::default();
    };
    let Some(body) = http_json(
        reqwest::Method::GET,
        &format!("{root}/api/monitor/usage/quota/limit"),
        &[
            ("Accept", "application/json"),
            ("Content-Type", "application/json"),
            ("Authorization", &key),
            ("User-Agent", "ZCode/3.15.3"),
        ],
        None,
    ) else {
        return PlanWindows::default();
    };
    parse_zcode_quota_limit(&body)
}

fn http_json(
    method: reqwest::Method,
    url: &str,
    headers: &[(&str, &str)],
    body: Option<&str>,
) -> Option<Value> {
    let client = reqwest::blocking::Client::builder()
        .timeout(QUOTA_WAIT)
        .build()
        .ok()?;
    let mut request = client.request(method, url);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    if let Some(body) = body {
        request = request.body(body.to_string());
    }
    let response = request.send().ok()?;
    if !response.status().is_success() {
        return None;
    }
    response.json().ok()
}

/// Credits left across the pools Qoder spends one after another.
pub fn parse_qoder_usage(body: &Value) -> CreditAllowance {
    if body.get("displayMode").and_then(Value::as_str) != Some("qoder") {
        return CreditAllowance::default();
    }
    let Some(usage) = body
        .get("qoderUsage")
        .or_else(|| body.get("qoder_usage"))
        .filter(|value| value.is_object())
    else {
        return CreditAllowance::default();
    };
    let pools = [
        qoder_pool(usage, &["userQuota", "user_quota"]),
        qoder_pool(usage, &["addOnQuota", "add_on_quota"]),
        qoder_pool(usage, &["orgResourcePackage", "org_resource_package"]),
    ];
    let mut left = 0.0;
    let mut total = 0.0;
    let mut any = false;
    let mut zero = false;
    for pool in pools.into_iter().flatten() {
        any = true;
        if pool.total == 0.0 {
            zero = true;
            continue;
        }
        total += pool.total;
        left += (pool.total - pool.used).max(0.0);
    }
    if total > 0.0 {
        return CreditAllowance {
            left: Some(round_credits(left)),
        };
    }
    if any && zero {
        return CreditAllowance { left: Some(0.0) };
    }
    CreditAllowance::default()
}

struct Pool {
    used: f64,
    total: f64,
}

fn qoder_pool(usage: &Value, names: &[&str]) -> Option<Pool> {
    let pool = names.iter().find_map(|name| usage.get(*name))?;
    if !pool.is_object() {
        return None;
    }
    let total = pool
        .get("total")
        .or_else(|| pool.get("cap"))
        .and_then(json_number)?;
    if !total.is_finite() || total < 0.0 {
        return None;
    }
    let used = if let Some(used) = pool.get("used").and_then(json_number) {
        used
    } else if let Some(remaining) = pool.get("remaining").and_then(json_number) {
        total - remaining
    } else if total == 0.0 {
        0.0
    } else {
        return None;
    };
    if !used.is_finite() || used < 0.0 {
        return None;
    }
    Some(Pool {
        used: used.min(total),
        total,
    })
}

/// Plan credits and extra credits, counted as the one pool WorkBuddy spends.
pub fn parse_workbuddy_summary(body: &Value) -> CreditAllowance {
    let summary = body
        .get("data")
        .filter(|value| value.is_object())
        .unwrap_or(body);
    let groups = summary
        .get("CreditGroups")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter(|item| has_capacity(item, "CycleTotalCapacity"))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut plan_used = 0.0;
    let mut plan_total = 0.0;
    let mut extra_used = 0.0;
    let mut extra_total = 0.0;
    if groups.is_empty() {
        for package in summary
            .get("Packages")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if !has_capacity(package, "CycleTotalCapacity") {
                continue;
            }
            extra_total += capacity(package.get("CycleTotalCapacity"));
            extra_used += capacity(package.get("CycleUsedCapacity"));
        }
    } else {
        for group in groups {
            let kind = group.get("Kind").and_then(Value::as_str).unwrap_or("");
            let total = capacity(group.get("CycleTotalCapacity"));
            let used = capacity(group.get("CycleUsedCapacity"));
            if kind == "plan_base" {
                plan_total += total;
                plan_used += used;
            } else {
                extra_total += total;
                extra_used += used;
            }
        }
    }
    let total = plan_total + extra_total;
    if total > 0.0 {
        let left = (total - plan_used - extra_used).max(0.0);
        return CreditAllowance {
            left: Some(round_credits(left)),
        };
    }
    let mut prepaid = 0.0;
    let mut bags = 0;
    for package in summary
        .get("Packages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if has_capacity(package, "CycleTotalCapacity")
            || !has_capacity(package, "CycleRemainCapacity")
        {
            continue;
        }
        prepaid += capacity(package.get("CycleRemainCapacity"));
        bags += 1;
    }
    if bags > 0 {
        return CreditAllowance {
            left: Some(round_credits(prepaid.max(0.0))),
        };
    }
    CreditAllowance::default()
}

fn has_capacity(value: &Value, name: &str) -> bool {
    match value.get(name) {
        None | Some(Value::Null) => false,
        Some(Value::String(text)) => !text.is_empty(),
        Some(_) => true,
    }
}

fn capacity(value: Option<&Value>) -> f64 {
    value
        .and_then(json_number)
        .filter(|n| n.is_finite() && *n >= 0.0)
        .unwrap_or(0.0)
}

/// Coding-plan limits as a 5-hour window and a weekly window.
/// A month of MCP calls, and a limit whose whole is 0, are left out.
pub fn parse_zcode_quota_limit(body: &Value) -> PlanWindows {
    let data = body
        .get("data")
        .filter(|value| value.is_object())
        .unwrap_or(body);
    let mut windows = PlanWindows::default();
    for limit in data
        .get("limits")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if limit
            .get("type")
            .and_then(Value::as_str)
            .is_some_and(|kind| kind.eq_ignore_ascii_case("TIME_LIMIT"))
        {
            continue;
        }
        if limit.get("usage").and_then(json_number) == Some(0.0) {
            continue;
        }
        let Some(percent) = zcode_used_percent(limit) else {
            continue;
        };
        let span = zcode_span_seconds(limit);
        if span > 0 && span < 86_400 {
            if windows.hours.is_none() {
                let hours = (span / 3600).max(1);
                windows.hours = Some(hours as u32);
                windows.hours_used_percent = Some(percent);
            }
        } else if span == 7 * 86_400 && windows.week_used_percent.is_none() {
            windows.week_used_percent = Some(percent);
        }
    }
    windows
}

fn zcode_used_percent(limit: &Value) -> Option<u8> {
    let usage = limit.get("usage").and_then(json_number);
    let remaining = limit.get("remaining").and_then(json_number);
    if let (Some(total), Some(left)) = (usage, remaining) {
        if total > 0.0 {
            return used_percent(total - left, total);
        }
    }
    if limit.get("percentage").is_none() {
        if let (Some(total), Some(current)) =
            (usage, limit.get("currentValue").and_then(json_number))
        {
            if total > 0.0 {
                return used_percent(current, total);
            }
        }
    }
    limit
        .get("percentage")
        .and_then(json_number)
        .and_then(|value| used_percent(value, 100.0))
}

fn zcode_span_seconds(limit: &Value) -> u64 {
    let unit = limit
        .get("unit")
        .and_then(json_number)
        .unwrap_or(0.0)
        .trunc() as i64;
    let number = limit
        .get("number")
        .and_then(json_number)
        .unwrap_or(0.0)
        .trunc() as i64;
    let base: i64 = match unit {
        1 => 60,
        3 => 3600,
        4 => 86_400,
        5 => 30 * 86_400,
        6 => 7 * 86_400,
        _ => 0,
    };
    if base <= 0 || number <= 0 {
        0
    } else {
        (base * number) as u64
    }
}

fn used_percent(used: f64, total: f64) -> Option<u8> {
    if !used.is_finite() || !total.is_finite() || total <= 0.0 {
        return None;
    }
    let percent = (100.0 * used / total).clamp(0.0, 100.0).round();
    Some(percent as u8)
}

fn round_credits(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

fn json_number(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64().filter(|n| n.is_finite()),
        Value::String(text) => text.trim().parse::<f64>().ok().filter(|n| n.is_finite()),
        _ => None,
    }
}

fn qoder_device_token() -> Option<(String, bool)> {
    let preferred = crate::adapters::qoder_config_dir();
    let mut dirs = vec![preferred.clone()];
    for dir in [home_join(".qoder-cn"), home_join(".qoder")] {
        if !dirs.iter().any(|have| have == &dir) {
            dirs.push(dir);
        }
    }
    for dir in dirs {
        if let Some(token) = qoder_token_in(&dir) {
            let cn = dir.file_name().and_then(|name| name.to_str()) == Some(".qoder-cn");
            return Some((token, cn));
        }
    }
    None
}

fn qoder_token_in(dir: &Path) -> Option<String> {
    let raw = fs::read_to_string(dir.join(".auth/user")).ok()?;
    let cipher = STANDARD.decode(raw.trim()).ok()?;
    let mut keys = Vec::new();
    if let Ok(id) = fs::read_to_string(dir.join(".auth/machine_id")) {
        push_aes_key(&mut keys, id.trim());
    }
    let cache_id =
        home_dir().join("Library/Application Support/QoderCN/SharedClientCache/cache/id");
    if let Ok(id) = fs::read_to_string(cache_id) {
        push_aes_key(&mut keys, id.trim());
    }
    for key in keys {
        let Some(plain) = aes128_cbc_decrypt(&key, &cipher) else {
            continue;
        };
        if let Some(token) = token_from_account(&plain) {
            return Some(token);
        }
    }
    None
}

fn push_aes_key(keys: &mut Vec<[u8; 16]>, id: &str) {
    let bytes = id.as_bytes();
    if bytes.len() < 16 {
        return;
    }
    let mut key = [0_u8; 16];
    key.copy_from_slice(&bytes[..16]);
    if !keys.contains(&key) {
        keys.push(key);
    }
}

fn token_from_account(plain: &[u8]) -> Option<String> {
    let value: Value = serde_json::from_slice(plain).ok()?;
    for name in ["security_oauth_token", "access_token", "token"] {
        if let Some(token) = value.get(name).and_then(Value::as_str) {
            let token = token.trim();
            if !token.is_empty() {
                return Some(token.to_string());
            }
        }
    }
    None
}

fn aes128_cbc_decrypt(key: &[u8; 16], cipher: &[u8]) -> Option<Vec<u8>> {
    if cipher.is_empty() || cipher.len() % 16 != 0 {
        return None;
    }
    let decryptor = Decryptor::<Aes128>::new(key.into(), key.into());
    let mut buf = cipher.to_vec();
    decryptor
        .decrypt_padded_mut::<Pkcs7>(&mut buf)
        .ok()
        .map(|plain| plain.to_vec())
}

struct WorkbuddySignIn {
    access: String,
    uid: String,
    domain: String,
    endpoint: String,
}

fn workbuddy_sign_in() -> Option<WorkbuddySignIn> {
    let dir = workbuddy_auth_dir();
    let mut files = fs::read_dir(&dir)
        .ok()?
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let name = path.file_name()?.to_str()?;
            if !name.ends_with(".info") {
                return None;
            }
            let modified = entry.metadata().ok()?.modified().ok()?;
            Some((modified, path))
        })
        .collect::<Vec<_>>();
    files.sort_by_key(|entry| std::cmp::Reverse(entry.0));
    for (_, path) in files {
        if let Some(sign_in) = workbuddy_sign_in_file(&path) {
            return Some(sign_in);
        }
    }
    None
}

fn workbuddy_auth_dir() -> PathBuf {
    let home = home_dir();
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support/CodeBuddyExtension/Data/Public/auth")
    } else if cfg!(windows) {
        home.join("AppData/Local/CodeBuddyExtension/Data/Public/auth")
    } else {
        home.join(".local/share/CodeBuddyExtension/Data/Public/auth")
    }
}

fn workbuddy_sign_in_file(path: &Path) -> Option<WorkbuddySignIn> {
    let value: Value = serde_json::from_str(&fs::read_to_string(path).ok()?).ok()?;
    let access = value
        .pointer("/auth/accessToken")
        .and_then(Value::as_str)?
        .trim();
    let uid = value
        .pointer("/account/uid")
        .and_then(Value::as_str)?
        .trim();
    if access.is_empty() || uid.is_empty() {
        return None;
    }
    let domain = value
        .pointer("/auth/domain")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    let endpoint = workbuddy_endpoint(&domain).to_string();
    let domain = if domain.is_empty() {
        endpoint.trim_start_matches("https://").to_string()
    } else {
        domain
    };
    Some(WorkbuddySignIn {
        access: access.to_string(),
        uid: uid.to_string(),
        domain,
        endpoint,
    })
}

fn workbuddy_endpoint(domain: &str) -> String {
    let domain = domain.trim().to_ascii_lowercase();
    if domain.ends_with("workbuddy.ai") {
        return "https://www.workbuddy.ai".to_string();
    }
    let cloud = domain.ends_with(".sso.copilot.tencent.com")
        || domain.ends_with(".copilot.qq.com")
        || domain.ends_with(".sso.codebuddy.cn");
    let ioa = domain == "tencent.sso.copilot.tencent.com" || domain == "tencent.sso.codebuddy.cn";
    if cloud
        && !ioa
        && !domain.is_empty()
        && domain
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '.' || ch == '-')
    {
        return format!("https://{domain}");
    }
    "https://copilot.tencent.com".to_string()
}

fn zcode_plan_key() -> Option<(String, String)> {
    let path = home_join(".zcode/v2/credentials.json");
    let value: Value = serde_json::from_str(&fs::read_to_string(path).ok()?).ok()?;
    let map = value.as_object()?;
    let secret = zcode_secret();
    let mut plain = Vec::new();
    for (name, stored) in map {
        let Some(text) = stored
            .as_str()
            .and_then(|text| zcode_decrypt(&secret, text))
        else {
            continue;
        };
        plain.push((name.clone(), text));
    }
    for (name, text) in &plain {
        if name.contains(":coding-plan:") && name.ends_with(":api-key") && text.contains('.') {
            let root = if name.contains(":bigmodel-") {
                "https://open.bigmodel.cn"
            } else {
                "https://api.z.ai"
            };
            return Some((root.to_string(), text.clone()));
        }
    }
    for family in zcode_families() {
        let Some(token) = plain
            .iter()
            .find(|(name, _)| name == &format!("oauth:{family}:access_token"))
            .map(|(_, text)| text.clone())
        else {
            continue;
        };
        let Some(location) = plain
            .iter()
            .find_map(|(name, text)| zcode_location(name, text, family))
        else {
            continue;
        };
        let Some(key) = zcode_copy_key(&location, &token, family == "zai") else {
            continue;
        };
        let root = if family == "bigmodel" {
            "https://open.bigmodel.cn"
        } else {
            "https://api.z.ai"
        };
        return Some((root.to_string(), key));
    }
    None
}

struct ZcodeLocation {
    origin: String,
    org: String,
    project: String,
    api_key_id: String,
}

fn zcode_families() -> [&'static str; 2] {
    let setting = home_join(".zcode/v2/setting.json");
    let family = fs::read_to_string(setting)
        .ok()
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .and_then(|value| {
            value
                .get("providerFamilyDomain")
                .and_then(Value::as_str)
                .map(str::to_ascii_lowercase)
        });
    if family.as_deref() == Some("bigmodel") {
        ["bigmodel", "zai"]
    } else {
        ["zai", "bigmodel"]
    }
}

fn zcode_location(name: &str, plain: &str, family: &str) -> Option<ZcodeLocation> {
    const PREFIX: &str = "project-key-location:v1:";
    let encoded = name.strip_prefix(PREFIX)?;
    let decoded = urlencoding_decode(encoded)?;
    let parts: Vec<Value> = serde_json::from_str(&decoded).ok()?;
    if parts.get(1).and_then(Value::as_str) != Some(family)
        || !parts.get(3).is_some_and(Value::is_null)
    {
        return None;
    }
    if parts.get(4).and_then(json_number) != Some(1.0)
        && parts.get(4).and_then(Value::as_str) != Some("1")
    {
        return None;
    }
    let loc: Value = serde_json::from_str(plain).ok()?;
    let org = loc
        .get("organizationId")
        .and_then(Value::as_str)?
        .trim()
        .to_string();
    let project = loc
        .get("projectId")
        .and_then(Value::as_str)?
        .trim()
        .to_string();
    let api_key_id = loc
        .get("apiKeyId")
        .and_then(Value::as_str)?
        .trim()
        .to_string();
    if org.is_empty() || project.is_empty() || api_key_id.is_empty() {
        return None;
    }
    let origin = parts
        .first()
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|origin| zcode_origin_ok(origin, family))
        .unwrap_or(if family == "bigmodel" {
            "https://bigmodel.cn"
        } else {
            "https://z.ai"
        })
        .trim_end_matches('/')
        .to_string();
    Some(ZcodeLocation {
        origin,
        org,
        project,
        api_key_id,
    })
}

fn zcode_origin_ok(origin: &str, family: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(origin) else {
        return false;
    };
    if url.scheme() != "https" {
        return false;
    }
    let host = url.host_str().unwrap_or("").to_ascii_lowercase();
    if family == "bigmodel" {
        host == "bigmodel.cn" || host.ends_with(".bigmodel.cn")
    } else {
        host == "z.ai" || host.ends_with(".z.ai")
    }
}

fn zcode_copy_key(loc: &ZcodeLocation, token: &str, bearer: bool) -> Option<String> {
    let auth = if bearer {
        format!("Bearer {token}")
    } else {
        token.to_string()
    };
    let url = format!(
        "{}/api/biz/v1/organization/{}/projects/{}/api_keys/copy/{}",
        loc.origin,
        path_segment(&loc.org),
        path_segment(&loc.project),
        path_segment(&loc.api_key_id),
    );
    let body = http_json(
        reqwest::Method::GET,
        &url,
        &[
            ("Accept", "application/json"),
            ("Content-Type", "application/json"),
            ("Authorization", &auth),
            ("User-Agent", "ZCode/3.15.3"),
        ],
        None,
    )?;
    let code = body.get("code").and_then(json_number);
    if code.is_some_and(|code| code != 0.0 && code != 200.0) {
        return None;
    }
    let secret = body
        .pointer("/data/secretKey")
        .or_else(|| body.get("secretKey"))
        .and_then(Value::as_str)?
        .trim();
    if secret.is_empty() {
        return None;
    }
    Some(format!("{}.{secret}", loc.api_key_id))
}

fn zcode_secret() -> [u8; 32] {
    let name = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_default();
    let name = name.rsplit(['\\', '/']).next().unwrap_or("").to_string();
    let platform = if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(windows) {
        "win32"
    } else {
        "linux"
    };
    let seed = format!(
        "zcode-credential-fallback:{platform}:{}:{name}",
        home_dir().display()
    );
    Sha256::digest(seed.as_bytes()).into()
}

fn zcode_decrypt(key: &[u8; 32], stored: &str) -> Option<String> {
    let rest = stored.strip_prefix("enc:v1:")?;
    let mut parts = rest.split('.');
    let iv = b64url(parts.next()?)?;
    let tag = b64url(parts.next()?)?;
    let data = b64url(parts.next()?)?;
    if parts.next().is_some() || iv.len() != 12 || tag.len() != 16 {
        return None;
    }
    let cipher = Aes256Gcm::new_from_slice(key).ok()?;
    let mut payload = data;
    payload.extend_from_slice(&tag);
    let plain = cipher
        .decrypt(Nonce::from_slice(&iv), payload.as_ref())
        .ok()?;
    String::from_utf8(plain).ok()
}

fn b64url(text: &str) -> Option<Vec<u8>> {
    let trimmed = text.trim_end_matches('=');
    URL_SAFE_NO_PAD.decode(trimmed).ok()
}

fn path_segment(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn urlencoding_decode(text: &str) -> Option<String> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok()?;
                out.push(u8::from_str_radix(hex, 16).ok()?);
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn qoder_credits_left_are_the_pools_added_together() {
        let one = parse_qoder_usage(&json!({
            "displayMode": "qoder",
            "qoderUsage": { "userQuota": { "total": 300, "used": 3, "remaining": 297, "unit": "credits" } }
        }));
        assert_eq!(one.left, Some(297.0));

        let both = parse_qoder_usage(&json!({
            "displayMode": "qoder",
            "qoderUsage": {
                "userQuota": { "total": 200, "used": 100 },
                "addOnQuota": { "total": 50, "remaining": 50 }
            }
        }));
        assert_eq!(both.left, Some(150.0));

        let empty = parse_qoder_usage(&json!({
            "displayMode": "qoder",
            "qoderUsage": { "userQuota": { "total": 0, "used": 0 } }
        }));
        assert_eq!(empty.left, Some(0.0));
        assert_eq!(
            parse_qoder_usage(&json!({"displayMode": "enterprise"})).left,
            None
        );
    }

    #[test]
    fn workbuddy_credits_left_add_the_plan_and_the_extra() {
        let summary = parse_workbuddy_summary(&json!({
            "code": 0,
            "data": {
                "CreditGroups": [
                    { "Kind": "plan_base", "CycleTotalCapacity": "500", "CycleUsedCapacity": "0" },
                    { "Kind": "plan_bonus", "CycleTotalCapacity": "0", "CycleUsedCapacity": "0" },
                    { "Kind": "addon", "CycleTotalCapacity": "0", "CycleUsedCapacity": "0" },
                    { "Kind": "benefit", "CycleTotalCapacity": "1500", "CycleUsedCapacity": "0" }
                ]
            }
        }));
        assert_eq!(summary.left, Some(2000.0));

        let spent = parse_workbuddy_summary(&json!({
            "CreditGroups": [
                { "Kind": "plan_base", "CycleTotalCapacity": "500", "CycleUsedCapacity": "100" }
            ]
        }));
        assert_eq!(spent.left, Some(400.0));
    }

    #[test]
    fn zcode_windows_use_the_counts_not_the_rounded_percentage() {
        let windows = parse_zcode_quota_limit(&json!({
            "code": 200,
            "data": {
                "level": "pro",
                "limits": [
                    {
                        "type": "CREDIT_LIMIT",
                        "unit": 3,
                        "number": 5,
                        "percentage": 1,
                        "usage": 12000,
                        "remaining": 11871
                    },
                    {
                        "type": "CREDIT_LIMIT",
                        "unit": 6,
                        "number": 1,
                        "percentage": 99,
                        "usage": 60000,
                        "remaining": 58498
                    },
                    { "type": "TIME_LIMIT", "unit": 5, "number": 1, "percentage": 80, "usage": 100, "remaining": 20 }
                ]
            }
        }));
        assert_eq!(windows.hours, Some(5));
        assert_eq!(windows.hours_used_percent, Some(1));
        assert_eq!(windows.week_used_percent, Some(3));
    }
}
