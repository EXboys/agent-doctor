use agent_doctor_core::{sync_knowledge, KnowledgeSyncReport};

#[tauri::command]
pub async fn knowledge_sync_command(
    project_path: Option<String>,
) -> Result<KnowledgeSyncReport, String> {
    tauri::async_runtime::spawn_blocking(move || sync_knowledge(project_path.as_deref()))
        .await
        .map_err(|_| "这次没同步到账号。".to_string())
}
