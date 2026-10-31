//! One saved provider is down. The next one with the same protocol takes this turn.
//!
//! A wrong key or a wrong model stays on that provider. A provider that just failed
//! is skipped for a few minutes so the next message does not hit it first.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::{list_personal_providers, normalize_protocol, PersonalProviderListItem};

const COOLDOWN: Duration = Duration::from_secs(3 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outage {
    /// Cannot connect, timed out, rate limited, or the service returned an error.
    Down,
    /// The account has no quota left.
    Quota,
    /// This provider's key was rejected. The next provider has its own key.
    Auth,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailoverSwitch {
    pub id: String,
    pub model: String,
    pub notice: String,
}

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

fn remember(id: &str, outage: Outage) {
    with_cool(|map| {
        map.insert(
            id.to_string(),
            Cool {
                at: Instant::now(),
                outage,
            },
        );
    });
}

fn cooling_outage(id: &str) -> Option<Outage> {
    with_cool(|map| {
        let (outage, fresh) = {
            let cool = map.get(id)?;
            (cool.outage, cool.at.elapsed() < COOLDOWN)
        };
        if !fresh {
            map.remove(id);
        }
        fresh.then_some(outage)
    })
}

/// The provider this turn would call is still in its cooldown. Use another one.
pub fn failover_while_cooling(provider_id: Option<&str>) -> Option<FailoverSwitch> {
    let doc = list_personal_providers().ok()?;
    let current = current_provider(&doc.providers, provider_id.or(doc.active_id.as_deref()))?;
    let outage = cooling_outage(&current.id)?;
    let next = pick_fallback(&doc.providers, &current.id, true)?;
    Some(switch_to(next, outage))
}

/// `failure` is an outage. Cool this provider and name the one to try once.
pub fn failover_after_outage(provider_id: Option<&str>, failure: &str) -> Option<FailoverSwitch> {
    let outage = classify_provider_outage(failure)?;
    let doc = list_personal_providers().ok()?;
    let current = current_provider(&doc.providers, provider_id.or(doc.active_id.as_deref()))?;
    let next = pick_fallback(&doc.providers, &current.id, true)?;
    remember(&current.id, outage);
    Some(switch_to(next, outage))
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
    remember(&current.id, outage);
}

fn current_provider<'a>(
    providers: &'a [PersonalProviderListItem],
    provider_id: Option<&str>,
) -> Option<&'a PersonalProviderListItem> {
    let id = provider_id.map(str::trim).filter(|id| !id.is_empty())?;
    providers.iter().find(|item| item.id == id)
}

fn pick_fallback<'a>(
    providers: &'a [PersonalProviderListItem],
    current_id: &str,
    skip_cooling: bool,
) -> Option<&'a PersonalProviderListItem> {
    let start = providers.iter().position(|item| item.id == current_id)?;
    let current = &providers[start];
    let protocol = normalize_protocol(&current.protocol);
    let count = providers.len();
    for step in 1..count {
        let item = &providers[(start + step) % count];
        if item.id == current_id {
            continue;
        }
        if normalize_protocol(&item.protocol) != protocol {
            continue;
        }
        if skip_cooling && cooling_outage(&item.id).is_some() {
            continue;
        }
        return Some(item);
    }
    None
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
        PersonalProviderListItem {
            id: id.into(),
            name: name.into(),
            url: "https://example.com/v1".into(),
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
    fn fallback_stays_on_the_same_protocol_and_skips_a_cooled_one() {
        let providers = vec![
            provider("a", "甲", "openai"),
            provider("b", "乙", "anthropic"),
            provider("c", "丙", "openai"),
        ];
        let next = pick_fallback(&providers, "a", false).unwrap();
        assert_eq!(next.id, "c");

        remember("c", Outage::Down);
        assert!(pick_fallback(&providers, "a", true).is_none());
        with_cool(|map| map.clear());
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
