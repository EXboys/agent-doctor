use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

use super::LlmConfig;
use agent_doctor_core::repair_types::{fallback_explain, ExplainInput, ExplainReport};

pub fn explain_runtime(input: &ExplainInput) -> Result<ExplainReport> {
    let Some(config) = LlmConfig::from_env() else {
        return Ok(fallback_explain(input));
    };

    match explain_with_llm(&config, input) {
        Ok(report) => Ok(report),
        Err(_) => Ok(fallback_explain(input)),
    }
}

fn explain_with_llm(config: &LlmConfig, input: &ExplainInput) -> Result<ExplainReport> {
    let client = reqwest::blocking::Client::new();
    let body = json!({
        "model": config.model,
        "messages": [
            {
                "role": "system",
                "content": "You are Agent Doctor diagnostic assistant. Explain runtime health in plain language. \
                    Reply with JSON only: {\"summary\":\"...\",\"likely_causes\":[\"...\"],\"recommended_action_ids\":[\"...\"],\"user_next_steps\":[\"...\"]}. \
                    recommended_action_ids must be chosen only from suggested_repairs ids. \
                    Do not invent secrets or shell commands outside user_next_steps. \
                    If install failed, explain why and give safe manual next steps."
            },
            {
                "role": "user",
                "content": serde_json::to_string(input)?
            }
        ],
        "temperature": 0.2,
    });

    let response = client
        .post(&config.api_url)
        .bearer_auth(&config.api_key)
        .json(&body)
        .send()
        .context("explain LLM request failed")?;

    let status = response.status();
    let payload: Value = response
        .json()
        .context("failed to parse explain response")?;
    if !status.is_success() {
        bail!("explain LLM HTTP {status}: {payload}");
    }

    let content = payload
        .pointer("/choices/0/message/content")
        .and_then(|value| value.as_str())
        .context("explain response missing content")?;

    parse_explain_json(content).context("failed to parse explain JSON from model")
}

fn parse_explain_json(content: &str) -> Result<ExplainReport> {
    let trimmed = content.trim();
    let json_text = if let Some(start) = trimmed.find('{') {
        if let Some(end) = trimmed.rfind('}') {
            &trimmed[start..=end]
        } else {
            trimmed
        }
    } else {
        trimmed
    };

    serde_json::from_str(json_text).context("invalid explain JSON")
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_doctor_core::repair_types::{ExplainCheck, ExplainSuggestion};

    #[test]
    fn fallback_explain_lists_failed_checks() {
        let report = fallback_explain(&ExplainInput {
            runtime_id: "openclaw".to_string(),
            probe_summary: "fail=1".to_string(),
            issue_score: 100,
            checks: vec![ExplainCheck {
                title: "Binary on PATH".to_string(),
                status: "fail".to_string(),
                message: "openclaw not found".to_string(),
            }],
            suggested_repairs: vec![ExplainSuggestion {
                id: "fix-openclaw-install".to_string(),
                title: "Install OpenClaw".to_string(),
                auto_fixable: true,
            }],
            install_failure: None,
        });
        assert!(report.summary.contains("openclaw"));
        assert!(report
            .recommended_action_ids
            .contains(&"fix-openclaw-install".to_string()));
    }
}
