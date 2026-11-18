//! 排期任务的桌面端管理命令：站点上那批排期行的 CRUD + 立即跑。
//! 手机端管理的是同一批数据；谁改都互通。

use agent_doctor_core::{
    teamups_computers, teamups_create_schedule, teamups_delete_schedule, teamups_run_schedule,
    teamups_schedules, teamups_update_schedule, TeamupsComputer, TeamupsRunOutcome,
    TeamupsSchedule, TeamupsScheduleInput,
};

#[tauri::command]
pub async fn teamups_schedules_command() -> Result<Vec<TeamupsSchedule>, String> {
    tauri::async_runtime::spawn_blocking(teamups_schedules)
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn teamups_create_schedule_command(
    input: TeamupsScheduleInput,
) -> Result<TeamupsSchedule, String> {
    tauri::async_runtime::spawn_blocking(move || teamups_create_schedule(&input))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
}

/// patch 透传站点认识的字段（prompt/freq/hhmm/weekdays/tz/clientId/enabled）。
#[tauri::command]
pub async fn teamups_update_schedule_command(
    id: String,
    patch: serde_json::Value,
) -> Result<TeamupsSchedule, String> {
    tauri::async_runtime::spawn_blocking(move || teamups_update_schedule(&id, &patch))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn teamups_delete_schedule_command(id: String) -> Result<bool, String> {
    tauri::async_runtime::spawn_blocking(move || teamups_delete_schedule(&id))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn teamups_run_schedule_command(id: String) -> Result<TeamupsRunOutcome, String> {
    tauri::async_runtime::spawn_blocking(move || teamups_run_schedule(&id))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn teamups_computers_command() -> Result<Vec<TeamupsComputer>, String> {
    tauri::async_runtime::spawn_blocking(teamups_computers)
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
}

/// 本机的 channel id，新建排期时绑自己用。
#[tauri::command]
pub fn teamups_channel_client_id_command() -> Result<String, String> {
    agent_doctor_core::teamups_channel_client_id().map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn teamups_toggle_schedule_command(
    id: String,
    enabled: bool,
) -> Result<TeamupsSchedule, String> {
    let patch = serde_json::json!({ "enabled": enabled });
    tauri::async_runtime::spawn_blocking(move || teamups_update_schedule(&id, &patch))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
}

fn clean_polished_text(raw: &str) -> String {
    let mut text = raw.trim();
    if let Some(rest) = text.strip_prefix("```") {
        let rest = match rest.find('\n') {
            Some(idx) => &rest[idx + 1..],
            None => rest,
        };
        text = rest.trim_end_matches("```").trim();
    }
    if (text.starts_with('"') && text.ends_with('"') && text.len() > 2)
        || (text.starts_with('“') && text.ends_with('”') && text.len() > 6)
    {
        let mut chars = text.chars();
        chars.next();
        chars.next_back();
        text = chars.as_str().trim();
    }
    text.to_string()
}

fn polish_system_prompt(zh: bool) -> &'static str {
    if zh {
        "你是排期任务指令润色助手。用户会给你一段想让电脑定时自动执行的任务草稿。\
请把它润色成一段清晰、具体、适合电脑在无人值守时直接执行的中文任务说明。\n\
要求：\n\
1. 严格保留用户的真实意图、提到的项目名、路径、网址或关键要求，绝不凭空编造文件名、链接或额外需求；\n\
2. 明确告诉助手完成后如何简洁汇报重点，以及遇到“没有新变化/没有异常”时一句话如实说明即可，不要停下来追问用户；\n\
3. 不要把“每天几点/每周几”硬写进正文（执行时间已由下方时间选择器单独指定，除非用户提到统计范围如“过去24小时/今天”）；\n\
4. 语言自然直白、简明扼要（通常 2～4 句话），普通用户一眼能看懂并修改；\n\
5. 只直接输出润色后的任务正文，不要加引号、代码块、标题或任何解释。"
    } else {
        "You rewrite rough scheduled-task drafts into clear, concise instructions for an unattended desktop AI assistant.\n\
Rules:\n\
1. Preserve every project name, path, URL, and goal from the draft — never invent paths, links, or unrelated work.\n\
2. Make the expected brief summary and the 'if nothing changed, say so briefly' behavior explicit so the assistant finishes without asking follow-up questions.\n\
3. Do not add a clock schedule like 'Every day at 8am' unless the draft names a lookback window such as 'since yesterday'.\n\
4. Keep it concise and plain (2–4 sentences).\n\
5. Output ONLY the rewritten task text — no quotes, code fences, or commentary."
    }
}

fn polish_via_http(
    draft: &str,
    runtime: &str,
    provider_id: Option<&str>,
    model: Option<&str>,
    zh: bool,
) -> anyhow::Result<String> {
    let prefer_official =
        runtime.is_empty() || runtime == agent_doctor_repair::OFFICIAL_AGENT_RUNTIME_ID;
    let config = agent_doctor_repair::default_llm_config_for(provider_id, model, prefer_official)?;
    let user_msg = if zh {
        format!("请润色这条定时任务草稿：\n{draft}")
    } else {
        format!("Rewrite this scheduled task draft:\n{draft}")
    };
    let reply =
        agent_doctor_repair::complete_plain_text(&config, polish_system_prompt(zh), &user_msg)?;
    let cleaned = clean_polished_text(&reply);
    if cleaned.is_empty() {
        anyhow::bail!("empty polish output");
    }
    Ok(cleaned)
}

fn polish_via_cli(
    draft: &str,
    runtime: &str,
    provider_id: Option<&str>,
    model: Option<&str>,
    zh: bool,
) -> anyhow::Result<String> {
    use agent_doctor_prompt::{
        run_prompt_session_with_cancel, PromptSessionCancel, PromptSessionEvent,
        PromptSessionOptions, PromptSessionStatus,
    };

    let combined_prompt = format!("{}\n\n---\n{}", polish_system_prompt(zh), draft);
    let options = PromptSessionOptions {
        runtime: runtime.to_string(),
        prompt: combined_prompt,
        cwd: std::env::temp_dir()
            .canonicalize()
            .ok()
            .or_else(|| Some(std::env::temp_dir())),
        timeout_sec: 60,
        dangerously_skip_permissions: false,
        full_auto: false,
        resume_thread_id: None,
        selected_mcps: Vec::new(),
        image_paths: Vec::new(),
        workspace_name: None,
        provider_id: provider_id
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        model: model
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        browser_tools_enabled: Some(false),
        computer_use_enabled: Some(false),
    };
    let mut streamed = String::new();
    let report =
        run_prompt_session_with_cancel(&options, PromptSessionCancel::new(), None, |event| {
            if let PromptSessionEvent::Delta { text, .. } = event {
                streamed.push_str(&text);
            }
        })?;
    let raw = if !streamed.trim().is_empty() {
        streamed
    } else {
        report.summary
    };
    if report.status != PromptSessionStatus::Succeeded && raw.trim().is_empty() {
        anyhow::bail!("cli polish failed");
    }
    let cleaned = clean_polished_text(&raw);
    if cleaned.is_empty() {
        anyhow::bail!("empty cli polish output");
    }
    Ok(cleaned)
}

/// 使用 Agent Doctor 当前默认选择的 AI 模型一键润色排期任务输入。
#[tauri::command]
pub async fn polish_schedule_prompt_command(
    prompt: String,
    runtime: Option<String>,
    provider_id: Option<String>,
    model: Option<String>,
    locale: Option<String>,
) -> Result<String, String> {
    let draft = prompt.trim().to_string();
    if draft.is_empty() {
        return Err("empty_prompt".to_string());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let zh = locale.as_deref() != Some("en");
        let rt = runtime.as_deref().unwrap_or("").trim();
        let pid = provider_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let mdl = model.as_deref().map(str::trim).filter(|s| !s.is_empty());

        let uses_native_cli = !rt.is_empty()
            && rt != agent_doctor_repair::OFFICIAL_AGENT_RUNTIME_ID
            && agent_doctor_core::turn_uses_native_account(rt, mdl, pid);

        let outcome = if uses_native_cli {
            polish_via_cli(&draft, rt, pid, mdl, zh)
                .or_else(|_| polish_via_http(&draft, rt, None, None, zh))
        } else {
            polish_via_http(&draft, rt, pid, mdl, zh).or_else(|http_err| {
                if !rt.is_empty() && rt != agent_doctor_repair::OFFICIAL_AGENT_RUNTIME_ID {
                    polish_via_cli(&draft, rt, pid, mdl, zh)
                } else {
                    Err(http_err)
                }
            })
        };

        outcome.map_err(|err| {
            let msg = format!("{err:#}");
            if msg.contains("official_provider_missing")
                || msg.contains("pinned provider not found")
            {
                "no_model_ready".to_string()
            } else {
                msg
            }
        })
    })
    .await
    .map_err(|error| error.to_string())?
}
