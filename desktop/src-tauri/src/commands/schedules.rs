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
