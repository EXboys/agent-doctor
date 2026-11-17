//! 排期任务的桌面端管理：和手机共用站点上的同一批排期行。
//! 电脑只做 CRUD；到点派发仍是 channel-hub 的职责（服务器单点调度）。
//! `client_id` 把排期绑到某台电脑；空 = 跟账号默认电脑走。

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::teamups_auth::{error_body, http_client, mall_base_url};
use crate::store::open_settings_store;

#[derive(Debug, Clone, Serialize)]
pub struct TeamupsSchedule {
    pub id: String,
    pub chat: String,
    pub title: String,
    pub prompt: String,
    pub freq: String,
    pub hhmm: String,
    pub weekdays: String,
    pub tz: String,
    /// 哪台电脑跑。空 = 账号默认电脑。
    pub client_id: String,
    /// 绑定电脑的名字（离线也能展示具体哪台）。
    pub client_name: String,
    pub next_run_at: u64,
    pub enabled: bool,
    pub last_status: String,
    pub last_run_at: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct TeamupsComputer {
    pub id: String,
    pub name: String,
    pub os: String,
    pub default: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamupsScheduleInput {
    pub prompt: String,
    pub freq: String,
    pub hhmm: String,
    pub weekdays: String,
    pub tz: String,
    #[serde(default)]
    pub client_id: String,
    #[serde(default)]
    pub client_name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TeamupsRunOutcome {
    pub sent: bool,
    pub job_id: String,
}

fn auth() -> Result<(String, String)> {
    let store = open_settings_store()?;
    let base = mall_base_url()?;
    let license = store
        .get_teamups_license()?
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    match license {
        Some(license) => Ok((base, license)),
        None => bail!("还没登录 TeamUps，登录后才能管理排期。"),
    }
}

fn clip(value: &str, max: usize) -> String {
    value.trim().chars().take(max).collect()
}

fn parse_schedule(value: &Value) -> Option<TeamupsSchedule> {
    let id = value.get("id").and_then(Value::as_str)?.trim();
    if id.is_empty() {
        return None;
    }
    let chat = value
        .get("chat")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(|text| text.trim_start_matches("phone:").to_string())
        .unwrap_or_else(|| format!("sched-{id}"));
    Some(TeamupsSchedule {
        id: id.to_string(),
        chat,
        title: value
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        prompt: value
            .get("prompt")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        freq: value
            .get("freq")
            .and_then(Value::as_str)
            .unwrap_or("daily")
            .to_string(),
        hhmm: value
            .get("hhmm")
            .and_then(Value::as_str)
            .unwrap_or("08:00")
            .to_string(),
        weekdays: value
            .get("weekdays")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        tz: value
            .get("tz")
            .and_then(Value::as_str)
            .unwrap_or("Asia/Shanghai")
            .to_string(),
        client_id: clip(
            value.get("clientId").and_then(Value::as_str).unwrap_or(""),
            64,
        ),
        client_name: clip(
            value
                .get("clientName")
                .and_then(Value::as_str)
                .unwrap_or(""),
            80,
        ),
        next_run_at: value.get("nextRunAt").and_then(Value::as_u64).unwrap_or(0),
        enabled: value
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        last_status: value
            .get("lastStatus")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        last_run_at: value.get("lastRunAt").and_then(Value::as_u64).unwrap_or(0),
    })
}

/// 带登录态的站点请求；401/403 统一提示重登。
fn request_json(method: reqwest::Method, path: &str, body: Option<Value>) -> Result<Value> {
    let (base, license) = auth()?;
    let url = format!("{base}{path}");
    let mut request = http_client()?
        .request(method.clone(), &url)
        .header("Authorization", format!("Bearer {license}"))
        .header("Accept", "application/json");
    if let Some(body) = body {
        request = request
            .header("Content-Type", "application/json")
            .json(&body);
    }
    let resp = request.send().with_context(|| format!("{method} {url}"))?;
    let status = resp.status();
    let text = resp.text().unwrap_or_default();
    if status.as_u16() == 401 || status.as_u16() == 403 {
        bail!("登录已过期，请重新登录 TeamUps。");
    }
    if !status.is_success() {
        bail!("TeamUps 请求失败（{status}）：{}", error_body(&text));
    }
    if text.trim().is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_str(&text).with_context(|| format!("invalid JSON from {path}"))
}

/// 当前账号的全部排期（手机和本机建的都在）。
pub fn teamups_schedules() -> Result<Vec<TeamupsSchedule>> {
    let body = request_json(reqwest::Method::GET, "/api/v1/doctor/schedules", None)?;
    let rows = body
        .get("schedules")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Ok(rows.iter().filter_map(parse_schedule).collect())
}

/// 新建排期。`client_id` 绑到某台电脑；给空串就跟默认电脑。
pub fn teamups_create_schedule(input: &TeamupsScheduleInput) -> Result<TeamupsSchedule> {
    let prompt = clip(&input.prompt, 4000);
    if prompt.is_empty() {
        bail!("请填写要让电脑做的事。");
    }
    let client_id = clip(&input.client_id, 64);
    let mut client_name = clip(&input.client_name, 80);
    if !client_id.is_empty()
        && client_name.is_empty()
        && client_id == super::teamups_channel::channel_client_id()
    {
        client_name = clip(&super::teamups_channel::computer_name(), 80);
    }
    let body = request_json(
        reqwest::Method::POST,
        "/api/v1/doctor/schedules",
        Some(json!({
            "prompt": prompt,
            "freq": clip(&input.freq, 8),
            "hhmm": clip(&input.hhmm, 5),
            "weekdays": clip(&input.weekdays, 16),
            "tz": clip(&input.tz, 64),
            "clientId": client_id,
            "clientName": client_name,
        })),
    )?;
    parse_schedule(body.get("schedule").unwrap_or(&Value::Null))
        .ok_or_else(|| anyhow::anyhow!("排期保存了，但返回格式不对。"))
}

/// 改排期。`patch` 直接透传站点认识的字段（prompt/freq/hhmm/weekdays/tz/clientId/enabled）。
pub fn teamups_update_schedule(id: &str, patch: &Value) -> Result<TeamupsSchedule> {
    let id = clip(id, 80);
    if id.is_empty() {
        bail!("缺排期 id。");
    }
    let Some(map) = patch.as_object() else {
        bail!("排期修改内容不对。");
    };
    let mut payload = map.clone();
    if let Some(cid) = payload.get("clientId").and_then(Value::as_str) {
        let cid = clip(cid, 64);
        let has_name = payload
            .get("clientName")
            .and_then(Value::as_str)
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false);
        if !cid.is_empty() && !has_name && cid == super::teamups_channel::channel_client_id() {
            payload.insert(
                "clientName".to_string(),
                Value::String(clip(&super::teamups_channel::computer_name(), 80)),
            );
        }
    }
    let body = request_json(
        reqwest::Method::PATCH,
        &format!("/api/v1/doctor/schedules/{id}"),
        Some(Value::Object(payload)),
    )?;
    parse_schedule(body.get("schedule").unwrap_or(&Value::Null))
        .ok_or_else(|| anyhow::anyhow!("排期改了，但返回格式不对。"))
}

/// 删排期；会话历史保留在站点上。
pub fn teamups_delete_schedule(id: &str) -> Result<bool> {
    let id = clip(id, 80);
    if id.is_empty() {
        bail!("缺排期 id。");
    }
    let body = request_json(
        reqwest::Method::DELETE,
        &format!("/api/v1/doctor/schedules/{id}"),
        None,
    )?;
    Ok(body.get("deleted").and_then(Value::as_bool).unwrap_or(true))
}

/// 立即跑一次（不打乱排期节奏）。返回是否当场派到了电脑。
pub fn teamups_run_schedule(id: &str) -> Result<TeamupsRunOutcome> {
    let id = clip(id, 80);
    if id.is_empty() {
        bail!("缺排期 id。");
    }
    let body = request_json(
        reqwest::Method::POST,
        &format!("/api/v1/doctor/schedules/{id}/run"),
        Some(json!({})),
    )?;
    Ok(TeamupsRunOutcome {
        sent: body.get("sent").and_then(Value::as_bool).unwrap_or(false),
        job_id: body
            .get("jobId")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
    })
}

/// 账号的电脑列表，用来把 client_id 翻译成名字。
pub fn teamups_computers() -> Result<Vec<TeamupsComputer>> {
    let body = request_json(reqwest::Method::GET, "/api/v1/account/computers", None)?;
    let rows = body
        .get("computers")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Ok(rows
        .iter()
        .filter_map(|value| {
            let id = value.get("id").and_then(Value::as_str)?.trim();
            if id.is_empty() {
                return None;
            }
            Some(TeamupsComputer {
                id: id.to_string(),
                name: value
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                os: value
                    .get("os")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                default: value
                    .get("default")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            })
        })
        .collect())
}
