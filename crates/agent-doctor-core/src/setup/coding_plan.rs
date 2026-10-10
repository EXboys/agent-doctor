//! Vendors that sell a coding subscription on a different address from pay-as-you-go.
//!
//! MiniMax is not here: its subscription key and its pay-as-you-go key share one address.

use super::zhipu::{zhipu_anthropic_url, zhipu_host, zhipu_is_coding_plan, zhipu_openai_url};

pub(crate) struct PlanEndpoints {
    pub openai_url: String,
    pub anthropic_url: String,
}

pub(crate) fn plan_endpoints(url: &str) -> Option<PlanEndpoints> {
    if let Some(host) = zhipu_host(url) {
        return Some(PlanEndpoints {
            openai_url: zhipu_openai_url(host, zhipu_is_coding_plan(url)),
            anthropic_url: zhipu_anthropic_url(host),
        });
    }
    qwen_endpoints(url)
        .or_else(|| kimi_endpoints(url))
        .or_else(|| volcengine_endpoints(url))
}

/// Other OpenAI addresses this key may belong to, in the order to try them.
pub(crate) fn plan_openai_alternates(url: &str) -> Vec<String> {
    if let Some(hosts) = minimax_other_hosts(url) {
        return hosts;
    }
    if zhipu_host(url).is_some() {
        return super::zhipu::zhipu_openai_sibling(url)
            .into_iter()
            .collect();
    }
    qwen_sibling(url)
        .or_else(|| kimi_sibling(url))
        .or_else(|| volcengine_sibling(url))
        .into_iter()
        .collect()
}

fn volcengine_endpoints(url: &str) -> Option<PlanEndpoints> {
    let lower = url.to_ascii_lowercase();
    if !lower.contains("ark.cn-beijing.volces.com") {
        return None;
    }
    if lower.contains("/api/coding") {
        Some(line(
            "https://ark.cn-beijing.volces.com/api/coding/v3",
            "https://ark.cn-beijing.volces.com/api/coding",
        ))
    } else {
        Some(line(
            "https://ark.cn-beijing.volces.com/api/v3",
            "https://ark.cn-beijing.volces.com/api/compatible",
        ))
    }
}

fn volcengine_sibling(url: &str) -> Option<String> {
    let current = volcengine_endpoints(url)?;
    if current.openai_url.contains("/api/coding/") {
        Some("https://ark.cn-beijing.volces.com/api/v3".into())
    } else {
        Some("https://ark.cn-beijing.volces.com/api/coding/v3".into())
    }
}

const MINIMAX_CN: &str = "https://api.minimax.cn/v1";
/// The older China platform host. Its keys are China keys.
const MINIMAX_CN_OLD: &str = "https://api.minimaxi.com/v1";
const MINIMAX_INTL: &str = "https://api.minimax.io/v1";

/// MiniMax subscription and pay-as-you-go keys share a path. A key only works on
/// the host of the site it was made on: China (two hosts) or international.
fn minimax_other_hosts(url: &str) -> Option<Vec<String>> {
    let lower = url.to_ascii_lowercase();
    let order: [&str; 2] = if lower.contains("api.minimax.cn") {
        [MINIMAX_CN_OLD, MINIMAX_INTL]
    } else if lower.contains("api.minimaxi.com") {
        [MINIMAX_CN, MINIMAX_INTL]
    } else if lower.contains("api.minimax.io") {
        [MINIMAX_CN, MINIMAX_CN_OLD]
    } else {
        return None;
    };
    Some(order.iter().map(|host| host.to_string()).collect())
}

/// Qwen's coding-plan model list answers without a key, so HTTP 200 does not prove the key.
pub(crate) fn models_list_is_public(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.contains("://coding.dashscope.") || lower.contains("://coding-intl.dashscope.")
}

fn qwen_endpoints(url: &str) -> Option<PlanEndpoints> {
    let lower = url.to_ascii_lowercase();
    if lower.contains("coding-intl.dashscope.") {
        return Some(line(
            "https://coding-intl.dashscope.aliyuncs.com/v1",
            "https://coding-intl.dashscope.aliyuncs.com/apps/anthropic",
        ));
    }
    if lower.contains("coding.dashscope.") {
        return Some(line(
            "https://coding.dashscope.aliyuncs.com/v1",
            "https://coding.dashscope.aliyuncs.com/apps/anthropic",
        ));
    }
    if lower.contains("cn-hongkong.dashscope.") {
        return Some(line(
            "https://cn-hongkong.dashscope.aliyuncs.com/compatible-mode/v1",
            "https://cn-hongkong.dashscope.aliyuncs.com/apps/anthropic",
        ));
    }
    if lower.contains("dashscope-us.") {
        return Some(line(
            "https://dashscope-us.aliyuncs.com/compatible-mode/v1",
            "https://dashscope-us.aliyuncs.com/apps/anthropic",
        ));
    }
    if lower.contains("dashscope-intl.") {
        return Some(line(
            "https://dashscope-intl.aliyuncs.com/compatible-mode/v1",
            "https://dashscope-intl.aliyuncs.com/apps/anthropic",
        ));
    }
    if lower.contains("dashscope.aliyuncs.com") {
        return Some(line(
            "https://dashscope.aliyuncs.com/compatible-mode/v1",
            "https://dashscope.aliyuncs.com/apps/anthropic",
        ));
    }
    None
}

fn qwen_sibling(url: &str) -> Option<String> {
    let lower = url.to_ascii_lowercase();
    if lower.contains("coding-intl.dashscope.") {
        return Some("https://dashscope-intl.aliyuncs.com/compatible-mode/v1".into());
    }
    if lower.contains("coding.dashscope.") {
        return Some("https://dashscope.aliyuncs.com/compatible-mode/v1".into());
    }
    if lower.contains("dashscope-intl.aliyuncs.com") {
        return Some("https://coding-intl.dashscope.aliyuncs.com/v1".into());
    }
    if lower.contains("dashscope.aliyuncs.com") {
        return Some("https://coding.dashscope.aliyuncs.com/v1".into());
    }
    None
}

fn kimi_endpoints(url: &str) -> Option<PlanEndpoints> {
    let lower = url.to_ascii_lowercase();
    let intl = lower.contains("api.kimi.ai") || lower.contains("api.moonshot.ai");
    let cn = lower.contains("api.kimi.com") || lower.contains("api.moonshot.cn");
    if !intl && !cn {
        return None;
    }
    let coding = lower.contains("/coding");
    if coding || lower.contains("api.kimi.") {
        return Some(if intl {
            line(
                "https://api.kimi.ai/coding/v1",
                "https://api.kimi.ai/coding",
            )
        } else {
            line(
                "https://api.kimi.com/coding/v1",
                "https://api.kimi.com/coding",
            )
        });
    }
    Some(if intl {
        line(
            "https://api.moonshot.ai/v1",
            "https://api.moonshot.ai/anthropic",
        )
    } else {
        line(
            "https://api.moonshot.cn/v1",
            "https://api.moonshot.cn/anthropic",
        )
    })
}

fn kimi_sibling(url: &str) -> Option<String> {
    let current = kimi_endpoints(url)?;
    let lower = url.to_ascii_lowercase();
    let intl = lower.contains("api.kimi.ai") || lower.contains("api.moonshot.ai");
    if current.openai_url.contains("/coding/") {
        Some(if intl {
            "https://api.moonshot.ai/v1".into()
        } else {
            "https://api.moonshot.cn/v1".into()
        })
    } else {
        Some(if intl {
            "https://api.kimi.ai/coding/v1".into()
        } else {
            "https://api.kimi.com/coding/v1".into()
        })
    }
}

fn line(openai_url: &str, anthropic_url: &str) -> PlanEndpoints {
    PlanEndpoints {
        openai_url: openai_url.to_string(),
        anthropic_url: anthropic_url.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qwen_coding_plan_is_not_rewritten_to_payg() {
        let coding = plan_endpoints("https://coding.dashscope.aliyuncs.com/v1").expect("qwen");
        assert_eq!(
            coding.openai_url,
            "https://coding.dashscope.aliyuncs.com/v1"
        );
        assert_eq!(
            coding.anthropic_url,
            "https://coding.dashscope.aliyuncs.com/apps/anthropic"
        );
        assert_eq!(
            plan_openai_alternates("https://dashscope.aliyuncs.com/compatible-mode/v1")
                .first()
                .map(String::as_str),
            Some("https://coding.dashscope.aliyuncs.com/v1")
        );
        assert!(models_list_is_public(
            "https://coding.dashscope.aliyuncs.com/v1"
        ));
        assert!(!models_list_is_public(
            "https://dashscope.aliyuncs.com/compatible-mode/v1"
        ));
    }

    #[test]
    fn kimi_code_stays_off_the_moonshot_payg_host() {
        let coding = plan_endpoints("https://api.kimi.com/coding/v1").expect("kimi");
        assert_eq!(coding.openai_url, "https://api.kimi.com/coding/v1");
        assert_eq!(coding.anthropic_url, "https://api.kimi.com/coding");
        assert_eq!(
            plan_openai_alternates("https://api.moonshot.cn/v1")
                .first()
                .map(String::as_str),
            Some("https://api.kimi.com/coding/v1")
        );
        let payg = plan_endpoints("https://api.moonshot.ai/v1").expect("moonshot");
        assert_eq!(payg.openai_url, "https://api.moonshot.ai/v1");
    }

    #[test]
    fn volcengine_coding_plan_keeps_its_address() {
        let coding =
            plan_endpoints("https://ark.cn-beijing.volces.com/api/coding/v3").expect("ark");
        assert_eq!(
            coding.openai_url,
            "https://ark.cn-beijing.volces.com/api/coding/v3"
        );
        assert_eq!(
            coding.anthropic_url,
            "https://ark.cn-beijing.volces.com/api/coding"
        );
        assert_eq!(
            plan_openai_alternates("https://ark.cn-beijing.volces.com/api/v3")
                .first()
                .map(String::as_str),
            Some("https://ark.cn-beijing.volces.com/api/coding/v3")
        );
    }

    #[test]
    fn minimax_tries_every_region_host() {
        assert_eq!(
            plan_openai_alternates("https://api.minimax.cn/anthropic"),
            vec!["https://api.minimaxi.com/v1", "https://api.minimax.io/v1"]
        );
        assert_eq!(
            plan_openai_alternates("https://api.minimax.io/v1"),
            vec!["https://api.minimax.cn/v1", "https://api.minimaxi.com/v1"]
        );
    }
}
