//! A named Gemini credential for one employee sandbox.
//!
//! This is an API key, or a company Cloud project plus a key. It is not an
//! Antigravity sign-in. The key stays in the local secrets file. The pool
//! report only carries the name.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

use super::subscription_pool::{
    load_reported_subscriptions_from, remember_reported_subscription_in, ReportedSubscription,
    SubscriptionPoolRow,
};
use crate::edition::{product_edition, ProductEdition};
use crate::store::{open_settings_store, SettingsStore};

pub const RUNTIME_API: &str = "gemini";
pub const RUNTIME_CLOUD: &str = "gemini-cloud";

const API_BASE: &str = "https://generativelanguage.googleapis.com/v1beta";
/// Large enough that the company list can keep handing this key out.
/// A Gemini key has no subscription quota counter on this computer.
const HANDOUT_REMAINING: i64 = 1_000_000_000;
const HANDOUT_EXPIRES: &str = "2099-12-31";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct GeminiSecret {
    mode: String,
    api_key: String,
    #[serde(default)]
    project: String,
    #[serde(default)]
    location: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeminiJobEnv {
    pub set: Vec<(String, String)>,
    pub remove: Vec<String>,
}

pub fn save_gemini_key(
    name: &str,
    mode: &str,
    api_key: &str,
    project: &str,
    location: &str,
) -> Result<SubscriptionPoolRow> {
    if product_edition() != ProductEdition::Team {
        bail!("Gemini 密钥只在团队版里");
    }
    let store = open_settings_store()?;
    save_gemini_key_in(&store, name, mode, api_key, project, location)
}

pub fn save_gemini_key_in(
    store: &SettingsStore,
    name: &str,
    mode: &str,
    api_key: &str,
    project: &str,
    location: &str,
) -> Result<SubscriptionPoolRow> {
    let name = name.trim();
    if name.is_empty() {
        bail!("先写一个名字，用来认出这个号。");
    }
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        bail!("这个名字不能用来建目录。");
    }
    let mode = match mode.trim() {
        "api" | RUNTIME_API => "api",
        "cloud" | RUNTIME_CLOUD => "cloud",
        _ => bail!("先选择自己的密钥或公司 Cloud。"),
    };
    let api_key = api_key.trim();
    if api_key.is_empty() || api_key.chars().any(|ch| ch.is_control()) {
        bail!("先粘贴 Gemini 密钥。");
    }
    let project = project.trim();
    let location = {
        let location = location.trim();
        if location.is_empty() {
            "global"
        } else {
            location
        }
    };
    if mode == "cloud" && project.is_empty() {
        bail!("先填写公司的项目编号。");
    }
    if let Some(existing) = load_reported_subscriptions_from(store)?
        .into_iter()
        .find(|item| item.name == name)
    {
        let same = existing.runtime == runtime_for(mode) || existing.runtime.is_empty();
        if !existing.runtime.is_empty() && !same && !is_gemini_runtime(&existing.runtime) {
            bail!("这个名字已经是{}。", kind_label(&existing.runtime));
        }
    }
    let secret = GeminiSecret {
        mode: mode.to_string(),
        api_key: api_key.to_string(),
        project: if mode == "cloud" {
            project.to_string()
        } else {
            String::new()
        },
        location: if mode == "cloud" {
            location.to_string()
        } else {
            String::new()
        },
    };
    store.set_subscription_gemini_secret(name, &serde_json::to_string(&secret)?)?;
    let mut account = ReportedSubscription {
        name: name.to_string(),
        remaining: HANDOUT_REMAINING,
        expires_at: HANDOUT_EXPIRES.to_string(),
        disabled: false,
        cap: None,
        holder: None,
        proxy: None,
        runtime: runtime_for(mode).to_string(),
        quota_known: true,
    };
    if let Some(existing) = load_reported_subscriptions_from(store)?
        .into_iter()
        .find(|item| item.name == name)
    {
        account.disabled = existing.disabled;
        account.cap = existing.cap;
        account.holder = existing.holder;
        account.proxy = existing.proxy;
    }
    remember_reported_subscription_in(store, account)?;
    super::subscription_pool::list_subscription_pool_from(store)
        .into_iter()
        .find(|row| row.name == name)
        .ok_or_else(|| anyhow::anyhow!("这次没记下这个号。"))
}

pub fn env_for_account(name: &str) -> Result<GeminiJobEnv> {
    let store = open_settings_store()?;
    env_for_account_in(&store, name)
}

pub fn env_for_account_in(store: &SettingsStore, name: &str) -> Result<GeminiJobEnv> {
    let Some(raw) = store.get_subscription_gemini_secret(name.trim())? else {
        return Ok(GeminiJobEnv {
            set: Vec::new(),
            remove: Vec::new(),
        });
    };
    let secret: GeminiSecret = match serde_json::from_str(&raw) {
        Ok(secret) => secret,
        Err(_) => {
            return Ok(GeminiJobEnv {
                set: Vec::new(),
                remove: Vec::new(),
            })
        }
    };
    Ok(env_from_secret(&secret))
}

fn env_from_secret(secret: &GeminiSecret) -> GeminiJobEnv {
    if secret.mode == "cloud" {
        GeminiJobEnv {
            set: vec![
                ("GEMINI_API_KEY".into(), secret.api_key.clone()),
                ("GOOGLE_GENAI_USE_VERTEXAI".into(), "true".into()),
                ("GOOGLE_CLOUD_PROJECT".into(), secret.project.clone()),
                ("GOOGLE_CLOUD_LOCATION".into(), secret.location.clone()),
            ],
            remove: vec!["GOOGLE_GEMINI_BASE_URL".into()],
        }
    } else {
        GeminiJobEnv {
            set: vec![
                ("GEMINI_API_KEY".into(), secret.api_key.clone()),
                ("GOOGLE_GEMINI_BASE_URL".into(), API_BASE.into()),
            ],
            remove: vec![
                "GOOGLE_GENAI_USE_VERTEXAI".into(),
                "GOOGLE_CLOUD_PROJECT".into(),
                "GOOGLE_CLOUD_LOCATION".into(),
            ],
        }
    }
}

pub fn is_gemini_runtime(runtime: &str) -> bool {
    matches!(runtime, RUNTIME_API | RUNTIME_CLOUD)
}

fn runtime_for(mode: &str) -> &'static str {
    if mode == "cloud" {
        RUNTIME_CLOUD
    } else {
        RUNTIME_API
    }
}

fn kind_label(runtime: &str) -> &'static str {
    match runtime {
        "codex" => "Codex",
        "claude-code" => "Claude Code",
        RUNTIME_CLOUD => "公司 Cloud",
        _ => "Gemini",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::secrets::MemorySecretBackend;
    use std::sync::Arc;

    fn open() -> (tempfile::TempDir, SettingsStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = SettingsStore::open_at(
            &dir.path().join("settings.db"),
            Arc::new(MemorySecretBackend::new()),
        )
        .unwrap();
        (dir, store)
    }

    #[test]
    fn api_key_stays_out_of_the_pool_report() {
        let (_dir, store) = open();
        let row = save_gemini_key_in(&store, "甲", "api", "AIza-secret", "", "").unwrap();
        assert_eq!(row.runtime, RUNTIME_API);
        let raw = store
            .meta_get("team.subscription.accounts")
            .unwrap()
            .unwrap();
        assert!(!raw.contains("AIza-secret"));
        let frame = super::super::subscription_pool::report_frame(
            &load_reported_subscriptions_from(&store).unwrap()[0],
        );
        assert!(frame.get("api_key").is_none());
        assert!(!frame.to_string().contains("AIza-secret"));
        let env = env_for_account_in(&store, "甲").unwrap();
        assert!(env
            .set
            .iter()
            .any(|(key, value)| key == "GEMINI_API_KEY" && value == "AIza-secret"));
        assert!(env
            .set
            .iter()
            .any(|(key, value)| key == "GOOGLE_GEMINI_BASE_URL" && value == API_BASE));
        assert!(env.remove.iter().any(|key| key == "GOOGLE_CLOUD_PROJECT"));
    }

    #[test]
    fn cloud_project_uses_vertex_and_not_the_consumer_endpoint() {
        let (_dir, store) = open();
        save_gemini_key_in(&store, "乙", "cloud", "cloud-key", "company-project", "").unwrap();
        let env = env_for_account_in(&store, "乙").unwrap();
        assert!(env
            .set
            .iter()
            .any(|(key, value)| key == "GOOGLE_CLOUD_PROJECT" && value == "company-project"));
        assert!(env
            .set
            .iter()
            .any(|(key, value)| key == "GOOGLE_CLOUD_LOCATION" && value == "global"));
        assert!(env
            .set
            .iter()
            .any(|(key, value)| key == "GOOGLE_GENAI_USE_VERTEXAI" && value == "true"));
        assert!(env.remove.iter().any(|key| key == "GOOGLE_GEMINI_BASE_URL"));
        assert_eq!(
            load_reported_subscriptions_from(&store).unwrap()[0].runtime,
            RUNTIME_CLOUD
        );
    }

    #[test]
    fn empty_key_and_a_taken_name_are_refused() {
        let (_dir, store) = open();
        let err = save_gemini_key_in(&store, "甲", "api", "  ", "", "").unwrap_err();
        assert!(err.to_string().contains("密钥"));
        remember_reported_subscription_in(
            &store,
            ReportedSubscription {
                name: "甲".into(),
                remaining: 1,
                expires_at: "2099-12-31".into(),
                disabled: false,
                cap: None,
                holder: None,
                proxy: None,
                runtime: "codex".into(),
                quota_known: true,
            },
        )
        .unwrap();
        let err = save_gemini_key_in(&store, "甲", "api", "AIza-secret", "", "").unwrap_err();
        assert!(err.to_string().contains("Codex"));
    }

    #[test]
    fn forgetting_the_row_drops_the_key() {
        let (_dir, store) = open();
        save_gemini_key_in(&store, "甲", "api", "AIza-secret", "", "").unwrap();
        super::super::subscription_pool::forget_reported_subscription_in(&store, "甲").unwrap();
        let env = env_for_account_in(&store, "甲").unwrap();
        assert!(env.set.is_empty());
    }
}
