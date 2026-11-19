use std::time::{Duration, Instant};

use agent_doctor_cluster::{
    activate_cluster_model, cancel_exo_install, connect_remote_cluster,
    install_and_start_local_cluster, llama::scan_llama_peer_hosts, load_cluster_status,
    scan_lan_clusters, set_cluster_backend, set_llama_peer_hosts, stop_or_disconnect_cluster,
    update_cluster_pairing_code, ClusterBackendKind, ClusterStatus, LanClusterCandidate,
};
use serde::Serialize;
use tauri::Emitter;

use crate::commands::wiring::notify_provider_changed;
use crate::update_tray_tooltip;

#[derive(Debug, Clone, Serialize)]
pub struct ClusterInstallProgressPayload {
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub stage: String,
}

fn emit_cluster_changed(app: &tauri::AppHandle, status: &ClusterStatus) {
    let _ = app.emit("cluster-status-changed", status);
}

#[tauri::command]
pub async fn get_cluster_status_command() -> Result<ClusterStatus, String> {
    tauri::async_runtime::spawn_blocking(load_cluster_status)
        .await
        .map_err(|err| err.to_string())
}

#[tauri::command]
pub async fn start_local_cluster_command(app: tauri::AppHandle) -> Result<ClusterStatus, String> {
    let progress_app = app.clone();
    let status = tauri::async_runtime::spawn_blocking(move || {
        let mut last_emit = Instant::now() - Duration::from_secs(1);
        let mut last_stage = String::new();
        install_and_start_local_cluster(|downloaded, total, stage| {
            let stage_changed = stage != last_stage;
            if stage_changed || last_emit.elapsed() >= Duration::from_millis(180) {
                last_emit = Instant::now();
                last_stage = stage.to_string();
                let _ = progress_app.emit(
                    "cluster-install-progress",
                    ClusterInstallProgressPayload {
                        downloaded_bytes: downloaded,
                        total_bytes: total,
                        stage: stage.to_string(),
                    },
                );
            }
        })
        .map_err(|err| format!("{err:#}"))
    })
    .await
    .map_err(|err| err.to_string())??;

    emit_cluster_changed(&app, &status);
    Ok(status)
}

#[tauri::command]
pub fn cancel_cluster_install_command() {
    cancel_exo_install();
    agent_doctor_cluster::llama::cancel_llama_install();
}

#[tauri::command]
pub async fn stop_cluster_command(app: tauri::AppHandle) -> Result<ClusterStatus, String> {
    let status = tauri::async_runtime::spawn_blocking(|| {
        stop_or_disconnect_cluster().map_err(|err| format!("{err:#}"))
    })
    .await
    .map_err(|err| err.to_string())??;

    emit_cluster_changed(&app, &status);
    Ok(status)
}

#[tauri::command]
pub async fn activate_cluster_model_command(
    app: tauri::AppHandle,
    model_id: String,
) -> Result<ClusterStatus, String> {
    let progress_app = app.clone();
    let status = tauri::async_runtime::spawn_blocking(move || {
        let mut last_emit = Instant::now() - Duration::from_secs(1);
        let mut last_stage = String::new();
        activate_cluster_model(&model_id, |downloaded, total, stage| {
            let stage_changed = stage != last_stage;
            if stage_changed || last_emit.elapsed() >= Duration::from_millis(180) {
                last_emit = Instant::now();
                last_stage = stage.to_string();
                let _ = progress_app.emit(
                    "cluster-install-progress",
                    ClusterInstallProgressPayload {
                        downloaded_bytes: downloaded,
                        total_bytes: total,
                        stage: stage.to_string(),
                    },
                );
            }
        })
        .map_err(|err| format!("{err:#}"))
    })
    .await
    .map_err(|err| err.to_string())??;

    update_tray_tooltip(&app);
    notify_provider_changed(&app);
    emit_cluster_changed(&app, &status);
    Ok(status)
}

#[tauri::command]
pub async fn scan_lan_clusters_command() -> Result<Vec<LanClusterCandidate>, String> {
    tauri::async_runtime::spawn_blocking(scan_lan_clusters)
        .await
        .map_err(|err| err.to_string())
}

#[tauri::command]
pub async fn scan_llama_peers_command() -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(scan_llama_peer_hosts)
        .await
        .map_err(|err| err.to_string())
}

#[tauri::command]
pub async fn connect_remote_cluster_command(
    app: tauri::AppHandle,
    host_or_url: String,
    model_id: Option<String>,
) -> Result<ClusterStatus, String> {
    let status = tauri::async_runtime::spawn_blocking(move || {
        connect_remote_cluster(&host_or_url, model_id.as_deref()).map_err(|err| format!("{err:#}"))
    })
    .await
    .map_err(|err| err.to_string())??;

    update_tray_tooltip(&app);
    notify_provider_changed(&app);
    emit_cluster_changed(&app, &status);
    Ok(status)
}

#[tauri::command]
pub async fn set_cluster_pairing_code_command(
    app: tauri::AppHandle,
    code: Option<String>,
) -> Result<ClusterStatus, String> {
    let status = tauri::async_runtime::spawn_blocking(move || {
        update_cluster_pairing_code(code.as_deref()).map_err(|err| format!("{err:#}"))
    })
    .await
    .map_err(|err| err.to_string())??;

    emit_cluster_changed(&app, &status);
    Ok(status)
}

#[tauri::command]
pub async fn set_cluster_backend_command(
    app: tauri::AppHandle,
    backend: String,
) -> Result<ClusterStatus, String> {
    let status = tauri::async_runtime::spawn_blocking(move || {
        let kind = match backend.as_str() {
            "exo" => ClusterBackendKind::Exo,
            "llama_cpp" => ClusterBackendKind::LlamaCpp,
            _ => {
                return Err("这台电脑不能使用这一套算力。".to_string());
            }
        };
        set_cluster_backend(kind).map_err(|err| format!("{err:#}"))
    })
    .await
    .map_err(|err| err.to_string())??;

    emit_cluster_changed(&app, &status);
    Ok(status)
}

#[tauri::command]
pub async fn set_llama_peer_hosts_command(
    app: tauri::AppHandle,
    peers: String,
) -> Result<ClusterStatus, String> {
    let status = tauri::async_runtime::spawn_blocking(move || {
        set_llama_peer_hosts(&peers).map_err(|err| format!("{err:#}"))
    })
    .await
    .map_err(|err| err.to_string())??;
    emit_cluster_changed(&app, &status);
    Ok(status)
}
