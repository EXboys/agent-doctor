use agent_doctor_core::{
    delete_project_subagent, list_project_subagents, save_project_subagent, SubagentRecord,
    SubagentSaveReport,
};

#[tauri::command]
pub async fn list_project_subagents_command(
    project_name: String,
) -> Result<Vec<SubagentRecord>, String> {
    tauri::async_runtime::spawn_blocking(move || list_project_subagents(&project_name))
        .await
        .map_err(|_| "没有读到子助手，再试一次。".to_string())?
        .map_err(|err| err.to_string())
}

#[tauri::command]
pub async fn save_project_subagent_command(
    project_name: String,
    id: Option<String>,
    name: String,
    responsibility: String,
) -> Result<SubagentSaveReport, String> {
    tauri::async_runtime::spawn_blocking(move || {
        save_project_subagent(&project_name, id.as_deref(), &name, &responsibility)
    })
    .await
    .map_err(|_| "没有保存成功，再试一次。".to_string())?
    .map_err(|err| err.to_string())
}

#[tauri::command]
pub async fn delete_project_subagent_command(
    project_name: String,
    id: String,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || delete_project_subagent(&project_name, &id))
        .await
        .map_err(|_| "没有删除成功，再试一次。".to_string())?
        .map_err(|err| err.to_string())
}
