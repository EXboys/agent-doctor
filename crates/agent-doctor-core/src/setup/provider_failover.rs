//! One saved provider is down. The next compatible one takes this chat.
//!
//! A wrong model name stays on that provider so the typo is visible. Transient
//! outages (rate limits, 5xx, timeouts) rest briefly; empty quota or a rejected
//! key rests longer until the person saves the provider again. When every
//! compatible backup is resting, the one closest to waking up is probed once.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::{
    list_personal_providers, normalize_protocol, provider_supports_protocol,
    PersonalProviderListItem,
};

/// Transient network / 429 / 5xx faults clear quickly.
const DOWN_COOLDOWN: Duration = Duration::from_secs(60);
/// Empty balance or a rejected key stays out of the way until topped up or re-saved.
const ACCOUNT_COOLDOWN: Duration = Duration::from_secs(30 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outage {
    /// Cannot connect, timed out, rate limited, or the service returned an error.
    Down,
    /// The account has no quota left.
    Quota,
    /// This provider's key was rejected. The next provider has its own key.
    Auth,
}

fn cooldown_for(outage: Outage) -> Duration {
    match outage {
        Outage::Down => DOWN_COOLDOWN,
        Outage::Quota | Outage::Auth => ACCOUNT_COOLDOWN,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailoverSwitch {
    pub id: String,
    pub model: String,
    pub notice: String,
}

#[derive(Debug, Clone, Copy)]
struct Cool {
    at: Instant,
    outage: Outage,
}

static COOL: Mutex<Option<HashMap<String, Cool>>> = Mutex::new(None);

fn with_cool<T>(body: impl FnOnce(&mut HashMap<String, Cool>) -> T) -> T {
    let mut guard = COOL.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let map = guard.get_or_insert_with(HashMap::new);
    body(map)
}

fn remember_in(map: &mut HashMap<String, Cool>, id: &str, outage: Outage) {
    map.insert(
        id.to_string(),
        Cool {
            at: Instant::now(),
            outage,
        },
    );
}

fn remaining_in(map: &mut HashMap<String, Cool>, id: &str) -> Option<(Outage, Duration)> {
    let (outage, total, elapsed) = {
        let cool = map.get(id)?;
        (cool.outage, cooldown_for(cool.outage), cool.at.elapsed())
    };
    if elapsed >= total {
        map.remove(id);
        None
    } else {
        Some((outage, total - elapsed))
    }
}

/// Saving, activating, or deleting a provider resets its cooldown so a fresh key
/// or topped-up balance works on the very next send.
pub fn clear_provider_cooldown(provider_id: &str) {
    let id = provider_id.trim();
    if id.is_empty() {
        return;
    }
    with_cool(|map| {
        map.remove(id);
    });
}

/// The provider this turn would call is still in its cooldown. Use another one.
pub fn failover_while_cooling(
    provider_id: Option<&str>,
    required_protocol: Option<&str>,
) -> Option<FailoverSwitch> {
    let doc = list_personal_providers().ok()?;
    let current = current_provider(&doc.providers, provider_id.or(doc.active_id.as_deref()))?;
    with_cool(|map| choose_while_cooling(map, &doc.providers, &current.id, required_protocol))
}

/// `failure` is an outage. Cool this provider and name the one to try once.
pub fn failover_after_outage(
    provider_id: Option<&str>,
    required_protocol: Option<&str>,
    failure: &str,
) -> Option<FailoverSwitch> {
    let outage = classify_provider_outage(failure)?;
    let doc = list_personal_providers().ok()?;
    let current = current_provider(&doc.providers, provider_id.or(doc.active_id.as_deref()))?;
    with_cool(|map| {
        choose_after_outage(map, &doc.providers, &current.id, required_protocol, outage)
    })
}

/// The fallback itself failed the same way. Skip it on the next message too.
pub fn remember_provider_outage(provider_id: Option<&str>, failure: &str) {
    let Some(outage) = classify_provider_outage(failure) else {
        return;
    };
    let Ok(doc) = list_personal_providers() else {
        return;
    };
    let Some(current) = current_provider(&doc.providers, provider_id.or(doc.active_id.as_deref()))
    else {
        return;
    };
    with_cool(|map| remember_in(map, &current.id, outage));
}

fn current_provider<'a>(
    providers: &'a [PersonalProviderListItem],
    provider_id: Option<&str>,
) -> Option<&'a PersonalProviderListItem> {
    let id = provider_id.map(str::trim).filter(|id| !id.is_empty())?;
    providers.iter().find(|item| item.id == id)
}

fn candidate_fits(
    item: &PersonalProviderListItem,
    current: &PersonalProviderListItem,
    required_protocol: Option<&str>,
) -> bool {
    match required_protocol {
        Some("*") => true,
        Some(proto) => provider_supports_protocol(&item.protocol, &item.url, proto),
        None => provider_supports_protocol(
            &item.protocol,
            &item.url,
            &normalize_protocol(&current.protocol),
        ),
    }
}

fn choose_while_cooling(
    map: &mut HashMap<String, Cool>,
    providers: &[PersonalProviderListItem],
    current_id: &str,
    required_protocol: Option<&str>,
) -> Option<FailoverSwitch> {
    let (outage, current_remaining) = remaining_in(map, current_id)?;
    if let Some(awake) = pick_awake_fallback(map, providers, current_id, required_protocol) {
        return Some(switch_to(awake, outage));
    }
    // Every compatible provider is cooling (`probeOne`): switch only when another
    // candidate wakes up sooner than the one already chosen.
    let (soonest, rem) = pick_soonest_cooling(map, providers, current_id, required_protocol)?;
    (rem < current_remaining).then(|| switch_to(soonest, outage))
}

fn choose_after_outage(
    map: &mut HashMap<String, Cool>,
    providers: &[PersonalProviderListItem],
    current_id: &str,
    required_protocol: Option<&str>,
    outage: Outage,
) -> Option<FailoverSwitch> {
    let next =
        pick_awake_fallback(map, providers, current_id, required_protocol).or_else(|| {
            pick_soonest_cooling(map, providers, current_id, required_protocol)
                .map(|(item, _)| item)
        })?;
    remember_in(map, current_id, outage);
    Some(switch_to(next, outage))
}

fn pick_awake_fallback<'a>(
    map: &mut HashMap<String, Cool>,
    providers: &'a [PersonalProviderListItem],
    current_id: &str,
    required_protocol: Option<&str>,
) -> Option<&'a PersonalProviderListItem> {
    let start = providers.iter().position(|item| item.id == current_id)?;
    let current = &providers[start];
    let count = providers.len();
    for step in 1..count {
        let item = &providers[(start + step) % count];
        if item.id == current_id {
            continue;
        }
        if !candidate_fits(item, current, required_protocol) {
            continue;
        }
        if remaining_in(map, &item.id).is_some() {
            continue;
        }
        return Some(item);
    }
    None
}

/// Among cooling alternates that fit this runtime's protocol, pick the one closest
/// to waking up (`probeOne`). Ties preserve list order after `current_id`.
fn pick_soonest_cooling<'a>(
    map: &mut HashMap<String, Cool>,
    providers: &'a [PersonalProviderListItem],
    current_id: &str,
    required_protocol: Option<&str>,
) -> Option<(&'a PersonalProviderListItem, Duration)> {
    let start = providers.iter().position(|item| item.id == current_id)?;
    let current = &providers[start];
    let count = providers.len();
    let mut best: Option<(&'a PersonalProviderListItem, Duration)> = None;
    for step in 1..count {
        let item = &providers[(start + step) % count];
        if item.id == current_id || !candidate_fits(item, current, required_protocol) {
            continue;
        }
        let Some((_, rem)) = remaining_in(map, &item.id) else {
            return Some((item, Duration::ZERO));
        };
        if best.map_or(true, |(_, best_rem)| rem < best_rem) {
            best = Some((item, rem));
        }
    }
    best
}

fn switch_to(next: &PersonalProviderListItem, outage: Outage) -> FailoverSwitch {
    let name = next.name.trim();
    let notice = match outage {
        Outage::Quota => format!("刚才那家额度用完了，这次改用{name}。"),
        Outage::Auth => format!("刚才那家密钥不能用了，这次改用{name}。"),
        Outage::Down => format!("刚才那家连不上，这次改用{name}。"),
    };
    FailoverSwitch {
        id: next.id.clone(),
        model: next.model.clone(),
        notice,
    }
}

fn classify_provider_outage(text: &str) -> Option<Outage> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_lowercase();
    // A wrong model name stays. A rejected key does not: the next provider
    // answers with its own key.
    if is_model_mistake(&lower) {
        return None;
    }
    if is_quota(trimmed, &lower) {
        return Some(Outage::Quota);
    }
    if is_auth(&lower) {
        return Some(Outage::Auth);
    }
    if is_down(&lower) {
        return Some(Outage::Down);
    }
    None
}

fn is_model_mistake(lower: &str) -> bool {
    lower.contains("model_not_found")
        || lower.contains("model is not supported")
        || lower.contains("invalid model")
        || lower.contains("no such model")
        || lower.contains("unknown model")
}

fn is_auth(lower: &str) -> bool {
    lower.contains("invalid api key")
        || lower.contains("incorrect api key")
        || lower.contains("invalid_api_key")
        || lower.contains("invalid access token")
        || lower.contains("token expired")
        || lower.contains("token_expired")
        || lower.contains("failed to authenticate")
        || status_code_present(lower, "401")
        || status_code_present(lower, "403")
}

fn is_quota(raw: &str, lower: &str) -> bool {
    raw.contains("余额不足")
        || raw.contains("无可用资源包")
        || raw.contains("请充值")
        || raw.contains("欠费")
        || raw.contains("额度用完")
        || lower.contains("insufficient_quota")
        || lower.contains("insufficient quota")
        || lower.contains("insufficient balance")
        || lower.contains("insufficient credit")
        || lower.contains("exceeded your current quota")
        || lower.contains("billing hard limit")
        || lower.contains("account balance")
        || status_code_present(lower, "402")
}

fn is_down(lower: &str) -> bool {
    status_code_present(lower, "429")
        || status_code_present(lower, "500")
        || status_code_present(lower, "502")
        || status_code_present(lower, "503")
        || status_code_present(lower, "504")
        || status_code_present(lower, "529")
        || lower.contains("too many requests")
        || lower.contains("rate limit")
        || lower.contains("rate_limit")
        || lower.contains("overloaded")
        || lower.contains("timeout")
        || lower.contains("timed out")
        || lower.contains("connection refused")
        || lower.contains("connection reset")
        || lower.contains("econnrefused")
        || lower.contains("econnreset")
        || lower.contains("enotfound")
        || lower.contains("unreachable")
        || lower.contains("temporary failure")
        || lower.contains("network error")
        || lower.contains("fetch failed")
        || lower.contains("限流")
        || lower.contains("请求过于频繁")
        || lower.contains("服务不可用")
        || lower.contains("服务繁忙")
}

/// A status code as its own token, so a model id like `claude-3-5-sonnet` is not a 500.
fn status_code_present(lower: &str, code: &str) -> bool {
    lower
        .split(|ch: char| !ch.is_ascii_digit())
        .any(|part| part == code)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(id: &str, name: &str, protocol: &str) -> PersonalProviderListItem {
        provider_at(id, name, "https://example.com/v1", protocol)
    }

    fn provider_at(id: &str, name: &str, url: &str, protocol: &str) -> PersonalProviderListItem {
        PersonalProviderListItem {
            id: id.into(),
            name: name.into(),
            url: url.into(),
            model: format!("{id}-model"),
            protocol: protocol.into(),
            api_key_hint: String::new(),
            active: false,
        }
    }

    #[test]
    fn outage_is_a_down_service_or_an_empty_quota() {
        assert_eq!(
            classify_provider_outage("unexpected status 503"),
            Some(Outage::Down)
        );
        assert_eq!(classify_provider_outage("read timeout"), Some(Outage::Down));
        assert_eq!(
            classify_provider_outage("429 too many requests"),
            Some(Outage::Down)
        );
        assert_eq!(
            classify_provider_outage("余额不足或无可用资源包"),
            Some(Outage::Quota)
        );
        assert_eq!(
            classify_provider_outage("invalid api key"),
            Some(Outage::Auth)
        );
        assert_eq!(classify_provider_outage("HTTP 401"), Some(Outage::Auth));
        assert_eq!(
            classify_provider_outage(
                "Failed to authenticate. API Error: 401 invalid access token or token expired"
            ),
            Some(Outage::Auth)
        );
        assert_eq!(classify_provider_outage("model_not_found"), None);
        assert_eq!(classify_provider_outage(""), None);
        assert_eq!(classify_provider_outage("claude-3-5-sonnet"), None);
    }

    #[test]
    fn fallback_stays_on_compatible_protocol_and_skips_a_cooled_one() {
        let providers = vec![
            provider("a", "甲", "openai"),
            provider("b", "乙", "anthropic"),
            provider("c", "丙", "openai"),
        ];
        let mut map = HashMap::new();
        let next = pick_awake_fallback(&mut map, &providers, "a", Some("openai")).unwrap();
        assert_eq!(next.id, "c");

        remember_in(&mut map, "c", Outage::Down);
        assert!(pick_awake_fallback(&mut map, &providers, "a", Some("openai")).is_none());
    }

    #[test]
    fn dual_endpoint_provider_backs_up_both_claude_and_openai_agents() {
        let providers = vec![
            provider_at(
                "claude-raw",
                "Claude",
                "https://api.anthropic.com",
                "anthropic",
            ),
            provider_at(
                "openai-raw",
                "OpenAI",
                "https://api.openai.com/v1",
                "openai",
            ),
            provider_at(
                "deepseek",
                "DeepSeek",
                "https://api.deepseek.com/v1",
                "openai",
            ),
        ];
        let mut map = HashMap::new();
        // Claude Code (`anthropic`) skips pure OpenAI, but accepts DeepSeek even though
        // DeepSeek was saved under `openai` — DeepSeek exposes `/anthropic` on the same key.
        let for_claude = choose_after_outage(
            &mut map,
            &providers,
            "claude-raw",
            Some("anthropic"),
            Outage::Down,
        )
        .expect("dual-endpoint backup for Claude Code");
        assert_eq!(for_claude.id, "deepseek");

        // Conversely, if DeepSeek (saved under `anthropic`) fails on Codex (`openai`),
        // pure OpenAI is chosen while pure Anthropic is skipped.
        let reverse = vec![
            provider_at(
                "deepseek-anth",
                "DeepSeek",
                "https://api.deepseek.com/anthropic",
                "anthropic",
            ),
            provider_at(
                "claude-raw",
                "Claude",
                "https://api.anthropic.com",
                "anthropic",
            ),
            provider_at(
                "openai-raw",
                "OpenAI",
                "https://api.openai.com/v1",
                "openai",
            ),
        ];
        let mut map2 = HashMap::new();
        let for_codex = choose_after_outage(
            &mut map2,
            &reverse,
            "deepseek-anth",
            Some("openai"),
            Outage::Down,
        )
        .expect("openai backup for Codex");
        assert_eq!(for_codex.id, "openai-raw");
    }

    #[test]
    fn cooldown_is_tiered_by_outage_kind_and_cleared_on_save() {
        let mut map = HashMap::new();
        remember_in(&mut map, "transient", Outage::Down);
        remember_in(&mut map, "exhausted", Outage::Quota);
        remember_in(&mut map, "bad-key", Outage::Auth);

        // After 61s, transient Down has expired while Quota and Auth still rest.
        map.get_mut("transient").unwrap().at -= Duration::from_secs(61);
        map.get_mut("exhausted").unwrap().at -= Duration::from_secs(61);
        map.get_mut("bad-key").unwrap().at -= Duration::from_secs(61);

        assert!(remaining_in(&mut map, "transient").is_none());
        assert!(!map.contains_key("transient"));
        assert!(remaining_in(&mut map, "exhausted").is_some());
        assert!(remaining_in(&mut map, "bad-key").is_some());

        // Past 30m, Quota expires too.
        map.get_mut("exhausted").unwrap().at -= ACCOUNT_COOLDOWN;
        assert!(remaining_in(&mut map, "exhausted").is_none());

        // Explicit clear (re-saving/activating a provider) drops cooldown immediately.
        remember_in(&mut map, "re-saved-test", Outage::Auth);
        with_cool(|global| remember_in(global, "re-saved-global", Outage::Quota));
        clear_provider_cooldown("re-saved-global");
        assert!(with_cool(
            |global| remaining_in(global, "re-saved-global").is_none()
        ));
    }

    #[test]
    fn probe_one_picks_the_soonest_waking_backup_when_all_are_cooling() {
        let providers = vec![
            provider("a", "甲", "openai"),
            provider("b", "乙", "openai"),
            provider("c", "丙", "openai"),
        ];
        let mut map = HashMap::new();
        // `b` is resting for 30m (Quota, ~29m left); `c` is resting for 60s (Down, ~15s left).
        remember_in(&mut map, "b", Outage::Quota);
        map.get_mut("b").unwrap().at -= Duration::from_secs(60);
        remember_in(&mut map, "c", Outage::Down);
        map.get_mut("c").unwrap().at -= Duration::from_secs(45);

        // `a` just failed right now: even though both `b` and `c` are cooling,
        // `probeOne` tries `c` (15s left) instead of giving up.
        let switched = choose_after_outage(&mut map, &providers, "a", Some("openai"), Outage::Down)
            .expect("probe soonest cooling backup");
        assert_eq!(switched.id, "c");

        // Pre-turn check while `b` (29m left) is selected: switches to `c` (15s left).
        let pre = choose_while_cooling(&mut map, &providers, "b", Some("openai"))
            .expect("switch from long quota cooldown to nearly-awake backup");
        assert_eq!(pre.id, "c");

        // Pre-turn check while `c` itself is already the soonest to wake: stay on `c`.
        assert!(choose_while_cooling(&mut map, &providers, "c", Some("openai")).is_none());
    }

    #[test]
    fn a_status_code_only_counts_as_its_own_token() {
        // 429 inside "gpt-4290" or "5024 errors" is a model id, not an outage.
        assert_eq!(classify_provider_outage("gpt-4290 kept failing"), None);
        assert_eq!(classify_provider_outage("proxy reported 5024 errors"), None);
        assert_eq!(
            classify_provider_outage("gateway said 5022 and hung up"),
            None
        );
        assert_eq!(
            classify_provider_outage("HTTP 502 from gateway"),
            Some(Outage::Down)
        );
    }

    #[test]
    fn notice_names_the_provider_the_turn_will_use() {
        let next = provider("c", "丙", "openai");
        let switched = switch_to(&next, Outage::Down);
        assert_eq!(switched.notice, "刚才那家连不上，这次改用丙。");
        assert_eq!(switched.model, "c-model");
        let quota = switch_to(&next, Outage::Quota);
        assert_eq!(quota.notice, "刚才那家额度用完了，这次改用丙。");
    }
}
