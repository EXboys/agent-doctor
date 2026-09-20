use std::fs;
use std::path::Path;

use crate::adapters::util::home_join;
use crate::adapters::CodexAdapter;
use crate::prompt_session::env::collect_overlay_env;
use crate::repair::{DiagnosticFact, SensitivityLevel};

use super::super::config::ParsedConfig;
use super::super::schema::schema_warn;
use super::super::{ProbeCheck, ProbeSeverity, ProbeStatus};

pub(crate) fn probe_schema(
    path: &Path,
    parsed: &ParsedConfig,
    checks: &mut Vec<ProbeCheck>,
    facts: &mut Vec<DiagnosticFact>,
) {
    let ParsedConfig::Toml(value) = parsed else {
        return;
    };

    let provider = value
        .get("model_provider")
        .and_then(toml::Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    if let Some(model) = value.get("model").and_then(toml::Value::as_str) {
        facts.push(DiagnosticFact::new(
            "model.name",
            model,
            SensitivityLevel::ConfigShape,
        ));
    }

    let Some(provider) = provider else {
        checks.push(schema_warn(path, "model_provider is missing".to_string()));
        return;
    };

    facts.push(DiagnosticFact::new(
        "model.provider",
        &provider,
        SensitivityLevel::ConfigShape,
    ));

    let entry = value
        .get("model_providers")
        .and_then(|providers| providers.get(&provider));
    let Some(entry) = entry else {
        checks.push(schema_warn(
            path,
            format!("model_provider '{provider}' has no matching model_providers entry"),
        ));
        return;
    };

    let base_url = entry
        .get("base_url")
        .and_then(toml::Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| {
            value
                .get("openai_base_url")
                .and_then(toml::Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        });

    match base_url {
        Some(url) => {
            facts.push(DiagnosticFact::new(
                "gateway.url",
                &url,
                SensitivityLevel::ConfigShape,
            ));
            if !url.starts_with("http://") && !url.starts_with("https://") {
                checks.push(schema_warn(
                    path,
                    "model_providers.*.base_url / openai_base_url should start with http:// or https://"
                        .to_string(),
                ));
            }
        }
        None => checks.push(schema_warn(
            path,
            format!("model_providers.{provider}.base_url (or openai_base_url) is missing"),
        )),
    }

    let env_key = entry
        .get("env_key")
        .and_then(toml::Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    match &env_key {
        Some(key) => facts.push(DiagnosticFact::new(
            "codex.api_key.env",
            key,
            SensitivityLevel::ConfigShape,
        )),
        None => checks.push(schema_warn(
            path,
            format!("model_providers.{provider}.env_key is missing"),
        )),
    }

    let wire_api = entry
        .get("wire_api")
        .and_then(toml::Value::as_str)
        .map(str::trim)
        .unwrap_or("");
    if wire_api.is_empty() {
        checks.push(ProbeCheck::new(
            "codex.schema.wire_api_missing",
            "Codex wire_api",
            ProbeStatus::Warn,
            ProbeSeverity::Warning,
            format!(
                "model_providers.{provider}.wire_api is missing; Codex ≥0.84 expects 'responses'"
            ),
            SensitivityLevel::ConfigShape,
        ));
    } else if wire_api != "responses" {
        checks.push(ProbeCheck::new(
            "codex.schema.wire_api",
            "Codex wire_api",
            ProbeStatus::Warn,
            ProbeSeverity::Warning,
            format!("model_providers.{provider}.wire_api is '{wire_api}'; expected 'responses'"),
            SensitivityLevel::ConfigShape,
        ));
    }

    if value
        .get("openai_base_url")
        .and_then(toml::Value::as_str)
        .is_none()
        && provider != "openai"
    {
        checks.push(ProbeCheck::new(
            "codex.schema.openai_base_url_missing",
            "Codex openai_base_url",
            ProbeStatus::Warn,
            ProbeSeverity::Warning,
            "openai_base_url is missing; Codex may fall back to api.openai.com".to_string(),
            SensitivityLevel::ConfigShape,
        ));
    }
}

pub(crate) fn probe_deep(checks: &mut Vec<ProbeCheck>, facts: &mut Vec<DiagnosticFact>) {
    probe_api_key_configured(checks, facts);
    probe_placeholder_auth(checks, facts);
}

fn probe_api_key_configured(checks: &mut Vec<ProbeCheck>, facts: &mut Vec<DiagnosticFact>) {
    let env_key = facts
        .iter()
        .find(|f| f.key == "codex.api_key.env")
        .map(|f| f.value.clone())
        .or_else(read_env_key_from_config);

    let overlay = collect_overlay_env();
    let from_env = env_key.as_ref().is_some_and(|key| {
        overlay.get(key).is_some_and(|v| !v.trim().is_empty())
            || std::env::var(key)
                .ok()
                .is_some_and(|v| !v.trim().is_empty())
    });
    let from_auth = auth_has_usable_credentials();

    let configured = from_env || from_auth;
    checks.push(ProbeCheck::new(
        "codex.api_key.configured",
        "Codex API key configured",
        if configured {
            ProbeStatus::Pass
        } else {
            ProbeStatus::Warn
        },
        if configured {
            ProbeSeverity::Info
        } else {
            ProbeSeverity::Warning
        },
        match (from_env, from_auth, env_key.as_deref()) {
            (true, _, Some(key)) => format!("{key} is set in Agent Doctor wiring or process env"),
            (true, _, None) => "API key env is set".to_string(),
            (_, true, _) => "Codex auth.json has usable credentials".to_string(),
            (_, _, Some(key)) => {
                format!("{key} is missing and auth.json has no usable credentials")
            }
            _ => "no Codex API key env_key and no usable auth.json".to_string(),
        },
        SensitivityLevel::ConfigShape,
    ));
    facts.push(DiagnosticFact::new(
        "codex.api_key.configured",
        if configured { "true" } else { "false" },
        SensitivityLevel::Public,
    ));
}

fn probe_placeholder_auth(checks: &mut Vec<ProbeCheck>, facts: &mut Vec<DiagnosticFact>) {
    let path = home_join(".codex/auth.json");
    if !path.exists() {
        return;
    }
    facts.push(DiagnosticFact::new(
        "codex.auth.path",
        path.display().to_string(),
        SensitivityLevel::LocalPath,
    ));
    let Ok(raw) = fs::read_to_string(&path) else {
        return;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return;
    };
    let is_placeholder = value
        .get("placeholder")
        .and_then(serde_json::Value::as_bool)
        == Some(true);
    let is_empty_apikey = value.get("auth_mode").and_then(serde_json::Value::as_str)
        == Some("apikey")
        && value.get("OPENAI_API_KEY").is_none()
        && value.get("api_key").is_none()
        && value.get("tokens").is_none();
    if is_placeholder || is_empty_apikey {
        checks.push(ProbeCheck::new(
            "codex.auth.placeholder",
            "Codex placeholder auth",
            ProbeStatus::Warn,
            ProbeSeverity::Warning,
            "auth.json is a placeholder/empty apikey file and can block env_key auth".to_string(),
            SensitivityLevel::ConfigShape,
        ));
    }
}

fn read_env_key_from_config() -> Option<String> {
    let path = CodexAdapter::config_path();
    let raw = fs::read_to_string(path).ok()?;
    let value: toml::Value = toml::from_str(&raw).ok()?;
    let provider = value.get("model_provider")?.as_str()?;
    value
        .get("model_providers")?
        .get(provider)?
        .get("env_key")?
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn auth_has_usable_credentials() -> bool {
    let path = home_join(".codex/auth.json");
    let Ok(raw) = fs::read_to_string(path) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return false;
    };
    if value
        .get("placeholder")
        .and_then(serde_json::Value::as_bool)
        == Some(true)
    {
        return false;
    }
    let has_key = value
        .get("OPENAI_API_KEY")
        .or_else(|| value.get("api_key"))
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.trim().is_empty());
    let has_tokens = value.get("tokens").is_some();
    has_key || has_tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_warns_on_orphan_provider() {
        let mut checks = Vec::new();
        let mut facts = Vec::new();
        let value: toml::Value = toml::from_str(
            r#"
model = "gpt-5"
model_provider = "company"
"#,
        )
        .unwrap();
        probe_schema(
            Path::new("/tmp/config.toml"),
            &ParsedConfig::Toml(value),
            &mut checks,
            &mut facts,
        );
        assert!(checks.iter().any(|c| c.message.contains("no matching")));
        assert_eq!(
            facts
                .iter()
                .find(|f| f.key == "model.provider")
                .map(|f| f.value.as_str()),
            Some("company")
        );
    }

    #[test]
    fn schema_warns_on_missing_wire_api_and_openai_base_url() {
        let mut checks = Vec::new();
        let mut facts = Vec::new();
        let value: toml::Value = toml::from_str(
            r#"
model = "deepseek-v4-flash"
model_provider = "personal"

[model_providers.personal]
name = "Personal"
base_url = "https://api.deepseek.com/v1"
env_key = "OPENAI_API_KEY"
"#,
        )
        .unwrap();
        probe_schema(
            Path::new("/tmp/config.toml"),
            &ParsedConfig::Toml(value),
            &mut checks,
            &mut facts,
        );
        assert!(checks
            .iter()
            .any(|c| c.id == "codex.schema.wire_api_missing"));
        assert!(checks
            .iter()
            .any(|c| c.id == "codex.schema.openai_base_url_missing"));
        assert!(facts
            .iter()
            .any(|f| f.key == "gateway.url" && f.value.contains("deepseek")));
        assert!(facts
            .iter()
            .any(|f| f.key == "codex.api_key.env" && f.value == "OPENAI_API_KEY"));
    }
}
