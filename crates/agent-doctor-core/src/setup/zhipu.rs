//! Zhipu GLM sells two keys that do not share a balance.
//!
//! Pay-as-you-go keys only work on `/api/paas/v4`.
//! Coding Plan keys (Lite, Pro, Max, personal or team) only spend quota on
//! `/api/coding/paas/v4`. The same key on the other address comes back as
//! "no balance" (1113) or "wrong product" (1315).

pub(crate) fn zhipu_host(url: &str) -> Option<&'static str> {
    let lower = url.to_ascii_lowercase();
    if lower.contains("open.bigmodel.cn") {
        Some("https://open.bigmodel.cn")
    } else if lower.contains("api.z.ai") {
        Some("https://api.z.ai")
    } else {
        None
    }
}

pub(crate) fn zhipu_is_coding_plan(url: &str) -> bool {
    url.to_ascii_lowercase().contains("/api/coding/")
}

pub(crate) fn zhipu_openai_url(host: &str, coding: bool) -> String {
    if coding {
        format!("{host}/api/coding/paas/v4")
    } else {
        format!("{host}/api/paas/v4")
    }
}

pub(crate) fn zhipu_anthropic_url(host: &str) -> String {
    format!("{host}/api/anthropic")
}

/// The other GLM product line on the same host (pay-as-you-go ↔ coding plan).
pub(crate) fn zhipu_openai_sibling(url: &str) -> Option<String> {
    let host = zhipu_host(url)?;
    Some(zhipu_openai_url(host, !zhipu_is_coding_plan(url)))
}

/// Body looks like a key that belongs to the other GLM product line.
pub(crate) fn zhipu_plan_mismatch(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    let code_1113 = lower.contains("\"1113\"")
        || lower.contains("\"code\":1113")
        || lower.contains("\"code\": 1113");
    let code_1315 = lower.contains("\"1315\"")
        || lower.contains("\"code\":1315")
        || lower.contains("\"code\": 1315");
    let says_plan = body.contains("编程套餐") || lower.contains("coding plan");
    (code_1113
        && (body.contains("余额")
            || body.contains("资源包")
            || lower.contains("balance")
            || lower.contains("resource")))
        || code_1315
        || says_plan
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coding_plan_keeps_its_own_openai_address() {
        let coding = "https://open.bigmodel.cn/api/coding/paas/v4";
        assert!(zhipu_is_coding_plan(coding));
        assert_eq!(
            zhipu_openai_url(zhipu_host(coding).unwrap(), true),
            "https://open.bigmodel.cn/api/coding/paas/v4"
        );
        assert_eq!(
            zhipu_openai_sibling(coding).as_deref(),
            Some("https://open.bigmodel.cn/api/paas/v4")
        );
        assert_eq!(
            zhipu_openai_sibling("https://api.z.ai/api/paas/v4").as_deref(),
            Some("https://api.z.ai/api/coding/paas/v4")
        );
    }

    #[test]
    fn balance_error_on_the_wrong_line_is_a_plan_mismatch() {
        let body = r#"{"error":{"code":"1113","message":"余额不足或无可用资源包,请充值。"}}"#;
        assert!(zhipu_plan_mismatch(body));
        let team = r#"{"error":{"code":"1315","message":"该 API Key 仅限企业编程套餐场景使用"}}"#;
        assert!(zhipu_plan_mismatch(team));
        assert!(!zhipu_plan_mismatch(
            r#"{"error":{"code":"1000","message":"身份验证失败"}}"#
        ));
    }
}
