//! `agent-doctor-cluster`: multi-device local / LAN AI compute orchestration.
//!
//! v1 ships the `Exo` backend (Apple Silicon MLX cluster host + Windows/Mac LAN consumer),
//! while keeping the status contract, LAN scanner, and `local-cluster` PersonalProvider slot
//! backend-agnostic so `Ollama` and `LlamaCpp` can plug in behind `ClusterBackendKind`.

pub mod exo;
pub mod lan_scan;
pub mod llama;
pub mod modelscope;
pub mod namespace;
pub mod tiers;

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

use agent_doctor_core::edition::ProductEdition;
use agent_doctor_core::product_edition;
use agent_doctor_core::setup::{
    load_active_personal_provider, load_personal_provider, load_personal_provider_status,
    upsert_local_cluster_provider, LOCAL_CLUSTER_PROVIDER_ID,
};

pub use exo::daemon::{DaemonOwner, NodeHostSupport};
pub use exo::lifecycle::cancel_exo_install;
pub use exo::orchestrator::ClusterNodeSummary;
pub use exo::EXO_DEFAULT_PORT;
pub use lan_scan::{
    normalize_cluster_origin, scan_lan_clusters, scan_lan_tcp_port, LanClusterCandidate,
};
pub use namespace::{
    resolve_cluster_namespace, set_custom_pairing_code, NamespaceSource, ResolvedNamespace,
};
pub use tiers::ClusterModelTier;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClusterBackendKind {
    Exo,
    Ollama,
    LlamaCpp,
}

/// EXO stays first when this computer can run it. Otherwise only llama.cpp is offered.
pub fn offered_cluster_backends(exo_compatible: bool) -> Vec<ClusterBackendKind> {
    if exo_compatible {
        vec![ClusterBackendKind::Exo, ClusterBackendKind::LlamaCpp]
    } else {
        vec![ClusterBackendKind::LlamaCpp]
    }
}

pub fn resolve_cluster_backend(
    saved: Option<ClusterBackendKind>,
    offered: &[ClusterBackendKind],
) -> ClusterBackendKind {
    if let Some(saved) = saved {
        if offered.contains(&saved) {
            return saved;
        }
    }
    offered
        .first()
        .copied()
        .unwrap_or(ClusterBackendKind::LlamaCpp)
}

static STOPPED_INCOMPATIBLE_EXO: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterStatus {
    pub backend: ClusterBackendKind,
    /// This computer can run the current exo build (Apple silicon, macOS 26.2+).
    pub exo_compatible: bool,
    /// Backends the card may show. EXO is first when `exo_compatible`.
    pub offered_backends: Vec<ClusterBackendKind>,
    pub host_support: NodeHostSupport,
    pub runtime_installed: bool,
    pub daemon_owner: DaemonOwner,
    /// Reachable HTTP origin (`http://127.0.0.1:52415` or saved LAN Mac `http://192.168.x.x:52415`).
    pub reachable: bool,
    pub dashboard_url: Option<String>,
    pub namespace: ResolvedNamespace,
    pub nodes: Vec<ClusterNodeSummary>,
    pub usable_memory_bytes: u64,
    pub active_models: Vec<String>,
    pub incompatible_peer_count: usize,
    pub tiers: Vec<ClusterModelTier>,
    pub selected_model: String,
    /// Addresses typed for llama.cpp, for example `192.168.1.20, 192.168.1.21`.
    pub peer_hosts: String,
    /// True when `local-cluster` is currently the active PersonalProvider for local agents.
    pub provider_active: bool,
    pub remote_origin: Option<String>,
    /// `modelscope` or `huggingface`. The download address the next model uses.
    #[serde(default = "default_model_source")]
    pub model_source: String,
    /// Model folders already on disk (`org/name`), whether or not one is running.
    #[serde(default)]
    pub installed_models: Vec<String>,
}

fn default_model_source() -> String {
    "modelscope".to_string()
}

/// Inspect current local or saved remote cluster state without blocking the UI thread for long.
pub fn load_cluster_status() -> ClusterStatus {
    let host_support = exo::daemon::detect_host_support();
    let exo_compatible = host_support == NodeHostSupport::Supported;
    let offered_backends = offered_cluster_backends(exo_compatible);
    let backend = resolve_cluster_backend(namespace::saved_cluster_backend(), &offered_backends);
    if !exo_compatible && !STOPPED_INCOMPATIBLE_EXO.swap(true, Ordering::SeqCst) {
        let _ = exo::daemon::stop_exo_daemon();
    }
    let located = exo::lifecycle::locate_exo_runtime();
    let namespace = namespace::resolve_cluster_namespace();
    let remote_origin = namespace::saved_remote_cluster_origin();

    let target_origin = if host_support == NodeHostSupport::Supported {
        remote_origin
            .clone()
            .unwrap_or_else(|| exo::EXO_LOCAL_ORIGIN.to_string())
    } else {
        remote_origin.clone().unwrap_or_default()
    };

    let snapshot = if backend == ClusterBackendKind::LlamaCpp || target_origin.is_empty() {
        None
    } else {
        exo::orchestrator::probe_exo_origin(&target_origin, Duration::from_millis(750)).ok()
    };

    if backend != ClusterBackendKind::LlamaCpp {
        if host_support == NodeHostSupport::Supported && remote_origin.is_none() {
            let (count, seen) = snapshot
                .as_ref()
                .map(|snap| (snap.nodes.len(), snap.other_peer_last_seen_ms))
                .unwrap_or((0, None));
            exo::heal::observe_cluster(count, seen);
        }
        if let Some(ref snap) = snapshot {
            if let Some(model) = snap.active_models.first() {
                adopt_running_exo_model(&snap.origin, model);
            }
            exo::orchestrator::sync_provider_if_dashboard_switched(snap);
            if host_support == NodeHostSupport::Supported
                && backend == ClusterBackendKind::Exo
                && !snap.thunderbolt_cycle_ids.is_empty()
            {
                exo::heal::close_thunderbolt_loop_if_ours(
                    snap.self_node_id.clone(),
                    snap.thunderbolt_cycle_ids.clone(),
                );
            }
        }
    }

    let local_alive = target_origin == exo::EXO_LOCAL_ORIGIN && snapshot.is_some();
    let (daemon_owner, _) = exo::daemon::current_daemon_owner(local_alive);

    let usable_memory_bytes = snapshot
        .as_ref()
        .map(|s| s.usable_memory_bytes)
        .unwrap_or(0);
    let tiers = tiers::model_tiers_for_usable_memory(usable_memory_bytes);

    let installed_models = if backend == ClusterBackendKind::LlamaCpp {
        Vec::new()
    } else {
        modelscope::list_installed_exo_models()
    };

    let provider_status = load_personal_provider_status().ok();
    let provider_active = provider_status
        .as_ref()
        .and_then(|s| s.active_id.as_deref())
        == Some(LOCAL_CLUSTER_PROVIDER_ID);
    let peer_hosts = llama::saved_peer_text();

    if backend == ClusterBackendKind::LlamaCpp {
        let usable_memory_bytes = llama::cluster_usable_memory_bytes();
        let tiers = llama::llama_tiers(usable_memory_bytes);
        let recommended_default = tiers
            .iter()
            .find(|t| t.recommended)
            .or_else(|| tiers.first())
            .map(|t| t.model_id.clone())
            .unwrap_or_default();
        let saved_model = namespace::saved_preferred_cluster_model();
        let selected_model = saved_model
            .clone()
            .filter(|model| tiers.iter().any(|tier| &tier.model_id == model))
            .unwrap_or(recommended_default);
        let reachable = llama::server_reachable();
        let active_models = if reachable {
            llama::running_model_id()
                .or(saved_model)
                .into_iter()
                .collect()
        } else {
            Vec::new()
        };
        return ClusterStatus {
            backend,
            exo_compatible,
            offered_backends,
            host_support,
            runtime_installed: llama::runtime_installed(),
            daemon_owner: llama::current_owner(),
            reachable,
            dashboard_url: None,
            namespace,
            nodes: Vec::new(),
            usable_memory_bytes,
            active_models,
            incompatible_peer_count: 0,
            tiers,
            selected_model,
            provider_active,
            remote_origin: None,
            peer_hosts,
            model_source: namespace::model_source_label(),
            installed_models,
        };
    }

    let recommended_default = tiers
        .iter()
        .find(|t| t.recommended)
        .or_else(|| tiers.first())
        .map(|t| t.model_id.clone())
        .unwrap_or_else(|| "mlx-community/Qwen2.5-7B-Instruct-4bit".to_string());

    let selected_model = snapshot
        .as_ref()
        .and_then(|s| s.active_models.first().cloned())
        .or_else(namespace::saved_preferred_cluster_model)
        .unwrap_or(recommended_default);

    ClusterStatus {
        backend,
        exo_compatible,
        offered_backends,
        host_support,
        runtime_installed: located.is_some(),
        daemon_owner,
        reachable: snapshot.is_some(),
        dashboard_url: snapshot.as_ref().map(|s| s.origin.clone()),
        namespace,
        nodes: snapshot
            .as_ref()
            .map(|s| s.nodes.clone())
            .unwrap_or_default(),
        usable_memory_bytes,
        active_models: snapshot
            .as_ref()
            .map(|s| s.active_models.clone())
            .unwrap_or_default(),
        incompatible_peer_count: snapshot
            .as_ref()
            .map(|s| s.incompatible_peer_count)
            .unwrap_or(0),
        tiers,
        selected_model,
        provider_active,
        remote_origin,
        peer_hosts,
        model_source: namespace::model_source_label(),
        installed_models,
    }
}

/// A model exo is already running becomes the chat model on the team package.
/// Another saved service stays the global default; the row is still recorded.
fn adopt_running_exo_model(origin: &str, model: &str) {
    if product_edition() != ProductEdition::Team {
        return;
    }
    let model = model.trim();
    if model.is_empty() {
        return;
    }
    let gateway = format!("{}/v1", origin.trim().trim_end_matches('/'));
    let existing = load_personal_provider(LOCAL_CLUSTER_PROVIDER_ID)
        .ok()
        .flatten();
    let active = load_active_personal_provider().ok().flatten();
    let row_matches = existing.as_ref().is_some_and(|entry| {
        entry.model.trim() == model
            && entry.url.trim_end_matches('/') == gateway.trim_end_matches('/')
    });
    let cluster_active = active
        .as_ref()
        .is_some_and(|entry| entry.id == LOCAL_CLUSTER_PROVIDER_ID);
    let other_active = active.is_some() && !cluster_active;
    if row_matches && (cluster_active || other_active) {
        return;
    }
    let _ = upsert_local_cluster_provider(&gateway, model, !other_active);
}

/// Remember which compute engine this computer should show.
pub fn set_cluster_backend(kind: ClusterBackendKind) -> Result<ClusterStatus> {
    let offered =
        offered_cluster_backends(exo::daemon::detect_host_support() == NodeHostSupport::Supported);
    if !offered.contains(&kind) {
        bail!("这台电脑不能使用这一套算力。");
    }
    namespace::set_saved_cluster_backend(kind)?;
    if kind != ClusterBackendKind::Exo {
        let _ = exo::daemon::stop_exo_daemon();
    }
    if kind != ClusterBackendKind::LlamaCpp {
        llama::stop();
    }
    Ok(load_cluster_status())
}

pub fn set_llama_peer_hosts(raw: &str) -> Result<ClusterStatus> {
    llama::set_peer_text(raw)?;
    Ok(load_cluster_status())
}

/// Remember whether the next model comes from ModelScope or Hugging Face.
pub fn set_cluster_model_source(raw: &str) -> Result<ClusterStatus> {
    namespace::set_model_download_source(raw)?;
    Ok(load_cluster_status())
}

/// Install (if missing) and start the local `exo` daemon on Apple Silicon Mac.
/// Re-join other Macs on this account. Safe to call repeatedly; it waits between tries.
pub fn heal_local_exo_cluster() -> ClusterStatus {
    let _ = exo::heal::heal_lonely_exo();
    load_cluster_status()
}

pub fn install_and_start_local_cluster<F>(on_progress: F) -> Result<ClusterStatus>
where
    F: FnMut(u64, Option<u64>, &str),
{
    let backend = resolve_cluster_backend(
        namespace::saved_cluster_backend(),
        &offered_cluster_backends(exo::daemon::detect_host_support() == NodeHostSupport::Supported),
    );
    if backend == ClusterBackendKind::LlamaCpp {
        llama::install_and_start_rpc(on_progress)?;
        return Ok(load_cluster_status());
    }
    if exo::daemon::detect_host_support() != NodeHostSupport::Supported {
        bail!("这台电脑的系统用不了 EXO，请改用 llama.cpp。");
    }
    let _ = namespace::set_saved_remote_cluster_origin(None);
    let runtime = match exo::lifecycle::locate_exo_runtime() {
        Some(rt) => rt,
        None => exo::lifecycle::install_exo_runtime(on_progress)?,
    };
    // Even when exo binary was already on disk (DMG install), make sure the
    // Agent Doctor dashboard overlay is present before we spawn.
    let _ = exo::lifecycle::ensure_managed_dashboard_zh();
    let runtime = exo::lifecycle::locate_exo_runtime().unwrap_or(runtime);
    exo::daemon::start_exo_daemon(&runtime)?;
    Ok(heal_local_exo_cluster())
}

/// Stop the managed local daemon (or disconnect a saved remote LAN cluster on consumer devices).
pub fn stop_or_disconnect_cluster() -> Result<ClusterStatus> {
    let _ = exo::daemon::stop_exo_daemon();
    llama::stop();
    let _ = namespace::set_saved_remote_cluster_origin(None);
    Ok(load_cluster_status())
}

/// Place `model_id` on the active local or remote cluster and wire `local-cluster` into all local agents.
pub fn activate_cluster_model<F>(model_id: &str, mut on_progress: F) -> Result<ClusterStatus>
where
    F: FnMut(u64, Option<u64>, &str),
{
    let backend = resolve_cluster_backend(
        namespace::saved_cluster_backend(),
        &offered_cluster_backends(exo::daemon::detect_host_support() == NodeHostSupport::Supported),
    );
    if backend == ClusterBackendKind::LlamaCpp {
        llama::activate_model(model_id, &mut on_progress)?;
        return Ok(load_cluster_status());
    }
    let origin = if let Some(remote) = namespace::saved_remote_cluster_origin() {
        remote
    } else if exo::daemon::detect_host_support() == NodeHostSupport::Supported {
        let runtime = exo::lifecycle::locate_exo_runtime();
        if namespace::prefers_modelscope() {
            if runtime.is_none() {
                bail!("请先下载并开启本机算力。");
            }
            exo::lifecycle::clear_exo_install_cancel();
            let cancelled = || exo::lifecycle::exo_install_cancelled();
            modelscope::ensure_exo_snapshot(model_id, &mut on_progress, &cancelled)?;
        }
        exo::daemon::restart_to_match_model_source()?;
        if !exo::daemon::local_exo_node_reachable() {
            let Some(rt) = runtime else {
                bail!("请先下载并开启本机算力。");
            };
            exo::daemon::start_exo_daemon(&rt)?;
        }
        exo::EXO_LOCAL_ORIGIN.to_string()
    } else {
        bail!("请先查找或填写同一网络里苹果电脑的地址。");
    };

    exo::orchestrator::prepare_model_and_activate(&origin, model_id)?;
    Ok(load_cluster_status())
}

/// Connect this computer (Windows / Intel Mac / secondary Mac) to a LAN cluster origin,
/// verify reachability, and wire `local-cluster` if a model is already running or specified.
pub fn connect_remote_cluster(
    raw_host_or_url: &str,
    model_id: Option<&str>,
) -> Result<ClusterStatus> {
    let origin = lan_scan::normalize_cluster_origin(raw_host_or_url, exo::EXO_DEFAULT_PORT)?;
    let snap = exo::orchestrator::probe_exo_origin(&origin, Duration::from_secs(4))?;
    namespace::set_saved_remote_cluster_origin(Some(&origin))?;

    let chosen_model = model_id
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| snap.active_models.first().cloned());

    if let Some(m) = chosen_model {
        exo::orchestrator::prepare_model_and_activate(&origin, &m)?;
    }

    Ok(load_cluster_status())
}

/// Update custom pairing code and restart managed local daemon if currently running.
pub fn update_cluster_pairing_code(code: Option<&str>) -> Result<ClusterStatus> {
    namespace::set_custom_pairing_code(code)?;
    exo::daemon::restart_exo_daemon_if_managed()?;
    Ok(load_cluster_status())
}

/// Cleanly stop any managed `exo` daemon when the desktop app quits.
pub fn shutdown_managed_cluster() {
    let _ = exo::daemon::stop_exo_daemon();
    llama::stop();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exo_capable_hosts_offer_exo_first_then_llama() {
        let offered = offered_cluster_backends(true);
        assert_eq!(
            offered,
            vec![ClusterBackendKind::Exo, ClusterBackendKind::LlamaCpp]
        );
        assert_eq!(
            resolve_cluster_backend(None, &offered),
            ClusterBackendKind::Exo
        );
        assert_eq!(
            resolve_cluster_backend(Some(ClusterBackendKind::LlamaCpp), &offered),
            ClusterBackendKind::LlamaCpp
        );
    }

    #[test]
    fn incompatible_hosts_only_offer_llama_cpp() {
        let offered = offered_cluster_backends(false);
        assert_eq!(offered, vec![ClusterBackendKind::LlamaCpp]);
        assert_eq!(
            resolve_cluster_backend(Some(ClusterBackendKind::Exo), &offered),
            ClusterBackendKind::LlamaCpp
        );
    }
}
