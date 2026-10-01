use agent_doctor_core::{check_runtime_versions, RuntimeVersionStatus};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeInstalledVersion {
    pub runtime_id: String,
    pub version: Option<String>,
}

#[tauri::command]
pub async fn check_runtime_versions_command(
    installed: Vec<RuntimeInstalledVersion>,
) -> Vec<RuntimeVersionStatus> {
    let pairs: Vec<(String, Option<String>)> = installed
        .into_iter()
        .map(|row| (row.runtime_id, row.version))
        .collect();
    // Network refresh can take several seconds. Keep it off the UI thread.
    tauri::async_runtime::spawn_blocking(move || check_runtime_versions(&pairs))
        .await
        .unwrap_or_default()
}
