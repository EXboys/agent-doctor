use agent_doctor_core::{check_runtime_versions, RuntimeVersionStatus};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeInstalledVersion {
    pub runtime_id: String,
    pub version: Option<String>,
}

#[tauri::command]
pub fn check_runtime_versions_command(
    installed: Vec<RuntimeInstalledVersion>,
) -> Vec<RuntimeVersionStatus> {
    let pairs: Vec<(String, Option<String>)> = installed
        .into_iter()
        .map(|row| (row.runtime_id, row.version))
        .collect();
    check_runtime_versions(&pairs)
}
