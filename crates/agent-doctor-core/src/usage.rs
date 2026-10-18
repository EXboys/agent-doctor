//! Token usage per Ask turn, kept on this computer.
//!
//! Each runtime reports usage in its own shape. [`find_usage`] reads them all
//! into one [`TokenUsage`]; [`record_turn`] appends one row per finished turn to
//! `usage.db` beside the settings store. Only turns sent through Agent Doctor
//! are counted: a runtime used straight from a terminal is not.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Tokens for one model call or one turn. `input` never includes cache reads,
/// so the four parts add up to the total.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input: u64,
    pub output: u64,
    #[serde(default)]
    pub cache_read: u64,
    #[serde(default)]
    pub cache_write: u64,
}

impl TokenUsage {
    pub fn total(&self) -> u64 {
        self.input + self.output + self.cache_read + self.cache_write
    }

    pub fn is_empty(&self) -> bool {
        self.total() == 0
    }

    pub fn add(&mut self, other: &TokenUsage) {
        self.input += other.input;
        self.output += other.output;
        self.cache_read += other.cache_read;
        self.cache_write += other.cache_write;
    }
}

fn first_u64(value: &Value, keys: &[&str]) -> Option<u64> {
    keys.iter().find_map(|key| {
        let field = value.get(*key)?;
        field
            .as_u64()
            .or_else(|| field.as_f64().filter(|n| *n >= 0.0).map(|n| n as u64))
    })
}

/// One usage object: Anthropic, OpenAI, Codex app-server, ACP or OpenClaw names.
pub fn usage_from_value(value: &Value) -> Option<TokenUsage> {
    if !value.is_object() {
        return None;
    }
    let input = first_u64(
        value,
        &[
            "input_tokens",
            "inputTokens",
            "prompt_tokens",
            "promptTokens",
            "input",
        ],
    );
    let output = first_u64(
        value,
        &[
            "output_tokens",
            "outputTokens",
            "completion_tokens",
            "completionTokens",
            "output",
        ],
    );
    if input.is_none() && output.is_none() {
        return None;
    }
    let mut input = input.unwrap_or(0);
    // OpenAI and Codex count cached tokens inside the input number.
    let cached_inside = first_u64(value, &["cachedInputTokens", "cached_input_tokens"])
        .or_else(|| {
            value
                .get("prompt_tokens_details")
                .and_then(|d| first_u64(d, &["cached_tokens"]))
        })
        .unwrap_or(0);
    input = input.saturating_sub(cached_inside);
    let cache_read = first_u64(
        value,
        &[
            "cache_read_input_tokens",
            "cacheReadInputTokens",
            "cacheRead",
            "cache_read_tokens",
            "cachedReadTokens",
        ],
    )
    .unwrap_or(0)
        + cached_inside;
    let cache_write = first_u64(
        value,
        &[
            "cache_creation_input_tokens",
            "cacheCreationInputTokens",
            "cacheWrite",
            "cache_write_tokens",
            "cachedWriteTokens",
        ],
    )
    .unwrap_or(0);
    let usage = TokenUsage {
        input,
        output: output.unwrap_or(0),
        cache_read,
        cache_write,
    };
    (!usage.is_empty()).then_some(usage)
}

/// The first usage object under a `usage` / `tokenUsage` key, a few levels deep.
pub fn find_usage(value: &Value) -> Option<TokenUsage> {
    find_usage_depth(value, 0)
}

fn find_usage_depth(value: &Value, depth: usize) -> Option<TokenUsage> {
    if depth > 4 {
        return None;
    }
    match value {
        Value::Object(map) => {
            for key in ["usage", "tokenUsage", "token_usage"] {
                if let Some(found) = map.get(key).and_then(usage_from_value) {
                    return Some(found);
                }
            }
            map.values()
                .find_map(|child| find_usage_depth(child, depth + 1))
        }
        Value::Array(items) => items
            .iter()
            .find_map(|child| find_usage_depth(child, depth + 1)),
        _ => None,
    }
}

/// One finished turn as stored.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageRecord {
    pub ts: i64,
    pub runtime: String,
    /// Saved service id; empty for team mode and turns stored before ids were kept.
    #[serde(default)]
    pub provider_id: String,
    pub provider: String,
    pub model: String,
    pub usage: TokenUsage,
}

/// Turns added up per local day, tool, service and model.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UsageDayRow {
    /// Local calendar day as days since 1970-01-01.
    pub day: i64,
    pub runtime: String,
    pub provider_id: String,
    pub provider: String,
    pub model: String,
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub turns: u64,
}

pub fn usage_db_path() -> Option<PathBuf> {
    crate::store::agent_doctor_config_dir().map(|dir| dir.join("usage.db"))
}

fn open(path: &Path) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let conn = Connection::open(path)
        .with_context(|| format!("failed to open usage db at {}", path.display()))?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS turns (
            ts INTEGER NOT NULL,
            runtime TEXT NOT NULL,
            provider TEXT NOT NULL,
            model TEXT NOT NULL,
            input INTEGER NOT NULL,
            output INTEGER NOT NULL,
            cache_read INTEGER NOT NULL,
            cache_write INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS turns_ts ON turns(ts);",
    )?;
    let has_provider_id = conn
        .prepare("SELECT 1 FROM pragma_table_info('turns') WHERE name = 'provider_id'")?
        .exists([])?;
    if !has_provider_id {
        conn.execute_batch("ALTER TABLE turns ADD COLUMN provider_id TEXT NOT NULL DEFAULT ''")?;
    }
    Ok(conn)
}

pub fn record_at(path: &Path, record: &UsageRecord) -> Result<()> {
    let conn = open(path)?;
    conn.execute(
        "INSERT INTO turns(ts, runtime, provider_id, provider, model, input, output, cache_read, cache_write)
         VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            record.ts,
            record.runtime,
            record.provider_id,
            record.provider,
            record.model,
            record.usage.input as i64,
            record.usage.output as i64,
            record.usage.cache_read as i64,
            record.usage.cache_write as i64,
        ],
    )?;
    Ok(())
}

pub fn usage_by_day_at(path: &Path, since_ts: i64, tz_offset_sec: i64) -> Result<Vec<UsageDayRow>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let conn = open(path)?;
    let mut stmt = conn.prepare(
        "SELECT (ts + ?2) / 86400 AS day, runtime, provider_id, provider, model,
                SUM(input), SUM(output), SUM(cache_read), SUM(cache_write), COUNT(*)
         FROM turns WHERE ts >= ?1
         GROUP BY day, runtime, provider_id, provider, model
         ORDER BY day",
    )?;
    let rows = stmt.query_map(params![since_ts, tz_offset_sec], |row| {
        Ok(UsageDayRow {
            day: row.get(0)?,
            runtime: row.get(1)?,
            provider_id: row.get(2)?,
            provider: row.get(3)?,
            model: row.get(4)?,
            input: row.get::<_, i64>(5)?.max(0) as u64,
            output: row.get::<_, i64>(6)?.max(0) as u64,
            cache_read: row.get::<_, i64>(7)?.max(0) as u64,
            cache_write: row.get::<_, i64>(8)?.max(0) as u64,
            turns: row.get::<_, i64>(9)?.max(0) as u64,
        })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

/// Daily totals since `since_ts`, bucketed by the caller's local day.
pub fn usage_by_day(since_ts: i64, tz_offset_sec: i64) -> Result<Vec<UsageDayRow>> {
    let path = usage_db_path().context("could not resolve agent-doctor config dir")?;
    usage_by_day_at(&path, since_ts, tz_offset_sec)
}

/// Which saved service a finished turn should be counted under.
///
/// A chat can use a provider other than the checked one. DeepSeek Harness
/// always talks to a saved DeepSeek provider. A ChatGPT or Claude login is
/// that account, not the checked service.
fn billed_service(
    runtime: &str,
    provider_id: Option<&str>,
    reported_model: Option<&str>,
    team_label: Option<&str>,
    subscription: bool,
    providers: &[crate::setup::PersonalProviderListItem],
) -> (String, String, String) {
    let reported = reported_model
        .map(str::trim)
        .filter(|model| !model.is_empty());
    if let Some(label) = team_label.map(str::trim).filter(|label| !label.is_empty()) {
        return (
            String::new(),
            label.to_string(),
            reported.unwrap_or("").to_string(),
        );
    }
    if subscription {
        let name = if runtime == "codex" {
            "ChatGPT"
        } else {
            "Claude"
        };
        return (
            String::new(),
            name.to_string(),
            reported.unwrap_or("").to_string(),
        );
    }
    if runtime == crate::DEEPSEEK_HARNESS_RUNTIME_ID {
        let deepseek = providers
            .iter()
            .filter(|provider| {
                provider
                    .url
                    .to_ascii_lowercase()
                    .contains("api.deepseek.com")
            })
            .max_by_key(|provider| provider.active);
        return match deepseek {
            Some(provider) => (
                provider.id.clone(),
                provider.name.clone(),
                reported
                    .map(str::to_string)
                    .unwrap_or_else(|| provider.model.clone()),
            ),
            None => (
                String::new(),
                "DeepSeek".to_string(),
                reported.unwrap_or("").to_string(),
            ),
        };
    }
    let pinned = provider_id
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .and_then(|id| providers.iter().find(|provider| provider.id == id));
    let chosen = pinned.or_else(|| providers.iter().find(|provider| provider.active));
    match chosen {
        Some(provider) => (
            provider.id.clone(),
            provider.name.clone(),
            reported
                .map(str::to_string)
                .unwrap_or_else(|| provider.model.clone()),
        ),
        None => (
            String::new(),
            String::new(),
            reported.unwrap_or("").to_string(),
        ),
    }
}

fn resolve_billed_service(
    runtime: &str,
    provider_id: Option<&str>,
    reported_model: Option<&str>,
) -> (String, String, String) {
    let status = crate::setup::load_mode_status().ok();
    let team_label = status.as_ref().and_then(|status| {
        (status.mode == crate::setup::MODE_TEAM).then(|| {
            status
                .active_label
                .clone()
                .unwrap_or_else(|| "team".to_string())
        })
    });
    let providers = crate::setup::list_personal_providers()
        .map(|doc| doc.providers)
        .unwrap_or_default();
    billed_service(
        runtime,
        provider_id,
        reported_model,
        team_label.as_deref(),
        crate::setup::is_native_subscription(runtime),
        &providers,
    )
}

/// Store one finished turn.
///
/// `provider_id` is the saved service this chat used. Empty means the checked
/// one. `model` is what the runtime reported, or the model this chat selected.
pub fn record_turn(
    runtime: &str,
    provider_id: Option<&str>,
    model: Option<&str>,
    usage: &TokenUsage,
) -> Result<()> {
    if usage.is_empty() {
        return Ok(());
    }
    let path = usage_db_path().context("could not resolve agent-doctor config dir")?;
    let (provider_id, provider, model) = resolve_billed_service(runtime, provider_id, model);
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    record_at(
        &path,
        &UsageRecord {
            ts,
            runtime: runtime.to_string(),
            provider_id,
            provider,
            model,
            usage: *usage,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_every_runtime_shape() {
        let claude = json!({"type": "result", "usage": {
            "input_tokens": 12, "output_tokens": 30,
            "cache_read_input_tokens": 500, "cache_creation_input_tokens": 40
        }});
        assert_eq!(
            find_usage(&claude),
            Some(TokenUsage {
                input: 12,
                output: 30,
                cache_read: 500,
                cache_write: 40
            })
        );

        let codex = json!({"inputTokens": 1000, "cachedInputTokens": 800, "outputTokens": 50});
        assert_eq!(
            usage_from_value(&codex),
            Some(TokenUsage {
                input: 200,
                output: 50,
                cache_read: 800,
                cache_write: 0
            })
        );

        let openclaw = json!({"meta": {"agentMeta": {"usage": {
            "input": 7, "output": 9, "cacheRead": 3, "cacheWrite": 0, "total": 19
        }}}});
        assert_eq!(
            find_usage(&openclaw),
            Some(TokenUsage {
                input: 7,
                output: 9,
                cache_read: 3,
                cache_write: 0
            })
        );

        let openai = json!({"usage": {
            "prompt_tokens": 100, "completion_tokens": 20,
            "prompt_tokens_details": {"cached_tokens": 60}
        }});
        assert_eq!(
            find_usage(&openai),
            Some(TokenUsage {
                input: 40,
                output: 20,
                cache_read: 60,
                cache_write: 0
            })
        );

        assert_eq!(find_usage(&json!({"usage": {"input_tokens": 0}})), None);
        assert_eq!(find_usage(&json!({"text": "hi"})), None);
    }

    #[test]
    fn groups_turns_by_local_day() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("usage.db");
        let day = 20_000_i64 * 86_400;
        let tz = 8 * 3600;
        let row = |ts: i64, model: &str, input: u64| UsageRecord {
            ts,
            runtime: "claude-code".into(),
            provider_id: "p-deepseek".into(),
            provider: "DeepSeek".into(),
            model: model.into(),
            usage: TokenUsage {
                input,
                output: 1,
                cache_read: 0,
                cache_write: 0,
            },
        };
        // 23:30 local the day before, then twice on the next local day.
        record_at(&path, &row(day - tz - 1800, "deepseek-v4-flash", 10)).unwrap();
        record_at(&path, &row(day - tz + 60, "deepseek-v4-flash", 20)).unwrap();
        record_at(&path, &row(day - tz + 120, "deepseek-v4-flash", 30)).unwrap();
        record_at(&path, &row(day - tz + 180, "glm-5.3", 5)).unwrap();

        let rows = usage_by_day_at(&path, 0, tz).unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].day, 19_999);
        assert_eq!(rows[0].input, 10);
        let same_day: Vec<_> = rows.iter().filter(|r| r.day == 20_000).collect();
        let flash = same_day
            .iter()
            .find(|r| r.model == "deepseek-v4-flash")
            .unwrap();
        assert_eq!((flash.input, flash.turns), (50, 2));

        let later = usage_by_day_at(&path, day - tz, tz).unwrap();
        assert_eq!(later.iter().map(|r| r.turns).sum::<u64>(), 3);
        assert!(usage_by_day_at(&dir.path().join("missing.db"), 0, 0)
            .unwrap()
            .is_empty());
        assert!(rows.iter().all(|r| r.provider_id == "p-deepseek"));
    }

    #[test]
    fn opens_a_ledger_written_before_service_ids() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("usage.db");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE turns (ts INTEGER NOT NULL, runtime TEXT NOT NULL,
                provider TEXT NOT NULL, model TEXT NOT NULL, input INTEGER NOT NULL,
                output INTEGER NOT NULL, cache_read INTEGER NOT NULL, cache_write INTEGER NOT NULL);
             INSERT INTO turns VALUES (100, 'codex', 'GLM', 'glm-5.3', 4, 2, 0, 0);",
        )
        .unwrap();
        drop(conn);

        let rows = usage_by_day_at(&path, 0, 0).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(
            (rows[0].provider_id.as_str(), rows[0].provider.as_str()),
            ("", "GLM")
        );
    }

    fn provider(
        id: &str,
        name: &str,
        url: &str,
        model: &str,
        active: bool,
    ) -> crate::setup::PersonalProviderListItem {
        crate::setup::PersonalProviderListItem {
            id: id.into(),
            name: name.into(),
            url: url.into(),
            model: model.into(),
            protocol: "openai".into(),
            api_key_hint: String::new(),
            active,
        }
    }

    #[test]
    fn counts_a_chat_under_the_service_it_used() {
        let providers = vec![
            provider(
                "glm",
                "GLM",
                "https://open.bigmodel.cn/api/paas/v4",
                "glm-5.3",
                true,
            ),
            provider(
                "ds",
                "DeepSeek",
                "https://api.deepseek.com/v1",
                "deepseek-v4-flash",
                false,
            ),
        ];

        let (id, name, model) = billed_service(
            "claude-code",
            Some("ds"),
            Some("deepseek-v4-flash"),
            None,
            false,
            &providers,
        );
        assert_eq!(
            (id.as_str(), name.as_str(), model.as_str()),
            ("ds", "DeepSeek", "deepseek-v4-flash")
        );

        // Codex often omits the model. The chat's choice still wins over the checked GLM model.
        let (id, name, model) = billed_service(
            "codex",
            Some("ds"),
            Some("deepseek-v4-pro"),
            None,
            false,
            &providers,
        );
        assert_eq!(
            (id.as_str(), name.as_str(), model.as_str()),
            ("ds", "DeepSeek", "deepseek-v4-pro")
        );

        let (id, name, model) = billed_service("claude-code", None, None, None, false, &providers);
        assert_eq!(
            (id.as_str(), name.as_str(), model.as_str()),
            ("glm", "GLM", "glm-5.3")
        );
    }

    #[test]
    fn deepseek_harness_is_not_billed_to_the_checked_service() {
        let providers = vec![
            provider(
                "glm",
                "GLM",
                "https://open.bigmodel.cn/api/paas/v4",
                "glm-5.3",
                true,
            ),
            provider(
                "ds",
                "DeepSeek",
                "https://api.deepseek.com/v1",
                "deepseek-v4-flash",
                false,
            ),
        ];
        let (id, name, model) = billed_service(
            crate::DEEPSEEK_HARNESS_RUNTIME_ID,
            Some("glm"),
            None,
            None,
            false,
            &providers,
        );
        assert_eq!(
            (id.as_str(), name.as_str(), model.as_str()),
            ("ds", "DeepSeek", "deepseek-v4-flash")
        );

        let (id, name, model) = billed_service(
            crate::DEEPSEEK_HARNESS_RUNTIME_ID,
            None,
            None,
            None,
            false,
            &[providers[0].clone()],
        );
        assert_eq!((id.as_str(), name.as_str()), ("", "DeepSeek"));
        assert!(model.is_empty());
    }

    #[test]
    fn subscription_and_team_are_not_the_checked_service() {
        let providers = vec![provider(
            "glm",
            "GLM",
            "https://open.bigmodel.cn/api/paas/v4",
            "glm-5.3",
            true,
        )];
        let (id, name, model) = billed_service(
            "codex",
            Some("glm"),
            Some("gpt-5.6-sol"),
            None,
            true,
            &providers,
        );
        assert_eq!(
            (id.as_str(), name.as_str(), model.as_str()),
            ("", "ChatGPT", "gpt-5.6-sol")
        );

        let (id, name, model) =
            billed_service("claude-code", None, None, Some("公司"), false, &providers);
        assert_eq!(
            (id.as_str(), name.as_str(), model.as_str()),
            ("", "公司", "")
        );
    }
}
