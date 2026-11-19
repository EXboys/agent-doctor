//! REST orchestration against an `exo` cluster (`:52415`):
//! topology snapshot, memory-headroom-aware instance placement, state polling
//! until the model is ready, and sync with the `local-cluster` PersonalProvider.

use std::collections::BTreeSet;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use agent_doctor_core::setup::{
    load_active_personal_provider, upsert_local_cluster_provider, LOCAL_CLUSTER_PROVIDER_ID,
};

use crate::namespace::set_saved_preferred_cluster_model;
use crate::tiers::{usable_cluster_memory_bytes, SYSTEM_MEMORY_HEADROOM_BYTES};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterNodeSummary {
    pub node_id: String,
    pub label: String,
    pub total_memory_bytes: u64,
    pub available_memory_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExoClusterSnapshot {
    pub origin: String,
    pub self_node_id: String,
    pub nodes: Vec<ClusterNodeSummary>,
    pub total_memory_bytes: u64,
    pub available_memory_bytes: u64,
    pub usable_memory_bytes: u64,
    pub active_models: Vec<String>,
    pub incompatible_peer_count: usize,
}

/// Probe an `exo` origin (`http://127.0.0.1:52415` or `http://192.168.1.23:52415`).
pub fn probe_exo_origin(origin: &str, timeout: Duration) -> Result<ExoClusterSnapshot> {
    let origin = origin.trim().trim_end_matches('/');
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(timeout)
        .timeout(timeout)
        .build()
        .context("创建集群探测客户端失败")?;

    let node_resp = client
        .get(format!("{origin}/node_id"))
        .send()
        .with_context(|| format!("连不上算力节点 {origin}"))?;
    if !node_resp.status().is_success() {
        bail!(
            "算力节点返回状态异常（HTTP {}）",
            node_resp.status().as_u16()
        );
    }
    let self_node_id = node_resp
        .text()
        .unwrap_or_default()
        .trim()
        .trim_matches('"')
        .to_string();
    if self_node_id.is_empty() {
        bail!("节点未返回有效的标识");
    }

    let state_val: Value = client
        .get(format!("{origin}/state"))
        .send()
        .and_then(|r| r.error_for_status())
        .and_then(|r| r.json())
        .unwrap_or(Value::Null);

    let mismatch_count = client
        .get(format!("{origin}/v1/devices-on-other-versions"))
        .send()
        .ok()
        .filter(|r| r.status().is_success())
        .and_then(|r| r.json::<Value>().ok())
        .map(|v| count_mismatched_devices(&v))
        .unwrap_or(0);

    let mut snapshot = parse_exo_state(origin, &self_node_id, &state_val);
    snapshot.incompatible_peer_count = mismatch_count;
    Ok(snapshot)
}

/// Keep `local-cluster` provider's model synced if the user switched models in the `:52415` web dashboard.
pub fn sync_provider_if_dashboard_switched(snapshot: &ExoClusterSnapshot) {
    let Some(first_live_model) = snapshot.active_models.first() else {
        return;
    };
    let Ok(Some(active)) = load_active_personal_provider() else {
        return;
    };
    if active.id != LOCAL_CLUSTER_PROVIDER_ID {
        return;
    }
    if active.model.trim() != first_live_model.trim() {
        let gateway = format!("{}/v1", snapshot.origin.trim_end_matches('/'));
        let _ = upsert_local_cluster_provider(&gateway, first_live_model, true);
        let _ = set_saved_preferred_cluster_model(first_live_model);
    }
}

/// Place `model_id` across the cluster (if not already running), wait for `ready`,
/// and wire `local-cluster` as the active PersonalProvider for all local agents.
pub fn prepare_model_and_activate(origin: &str, model_id: &str) -> Result<ExoClusterSnapshot> {
    let origin = origin.trim().trim_end_matches('/');
    let model_id = model_id.trim();
    if model_id.is_empty() {
        bail!("请先选择一个模型档位。");
    }

    let snapshot = probe_exo_origin(origin, Duration::from_secs(5))
        .context("连不上算力服务，请确认算力电脑开着且在同一网络里。")?;

    if !snapshot.active_models.iter().any(|m| m == model_id) {
        place_and_await_instance(origin, model_id, &snapshot)?;
    }

    let gateway_v1 = format!("{origin}/v1");
    upsert_local_cluster_provider(&gateway_v1, model_id, true)
        .context("写入本地算力组网模型服务失败")?;
    let _ = set_saved_preferred_cluster_model(model_id);

    probe_exo_origin(origin, Duration::from_secs(5)).or(Ok(snapshot))
}

fn place_and_await_instance(
    origin: &str,
    model_id: &str,
    snapshot: &ExoClusterSnapshot,
) -> Result<()> {
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(60))
        .build()?;

    let mut previews_url = url::Url::parse(&format!("{origin}/instance/previews"))?;
    previews_url
        .query_pairs_mut()
        .append_pair("model_id", model_id);

    let previews_resp = client
        .get(previews_url)
        .send()
        .context("无法预览模型切分方案")?;
    if !previews_resp.status().is_success() {
        bail!(
            "算力集群无法准备该模型（HTTP {}），可在算力看板中确认模型名称。",
            previews_resp.status().as_u16()
        );
    }
    let previews_json: Value = previews_resp.json().context("解析切分方案失败")?;
    let chosen_instance = select_safe_placement(&previews_json, snapshot)?;

    let post_resp = client
        .post(format!("{origin}/instance"))
        .json(&serde_json::json!({ "instance": chosen_instance }))
        .send()
        .context("提交模型切分指令失败")?;
    if !post_resp.status().is_success() {
        let code = post_resp.status().as_u16();
        let body = post_resp.text().unwrap_or_default();
        bail!(
            "这几台电脑暂时放不下该模型（HTTP {code}）：{}",
            body.chars().take(140).collect::<String>()
        );
    }

    // exo 1.0.71 has no `/instance/await`. Watch `/state` until runners are ready.
    wait_until_model_ready(origin, model_id, Duration::from_secs(300))
}

#[derive(Debug, PartialEq, Eq)]
enum ModelReadiness {
    Pending,
    Ready,
    Failed(String),
}

fn wait_until_model_ready(origin: &str, model_id: &str, limit: Duration) -> Result<()> {
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(8))
        .build()?;
    let started = Instant::now();
    let mut saw_instance = false;
    loop {
        let state = client
            .get(format!("{origin}/state"))
            .send()
            .and_then(|resp| resp.error_for_status())
            .and_then(|resp| resp.json())
            .unwrap_or(Value::Null);
        if let Some(instances) = state.get("instances") {
            match readiness_of_matching_instances(instances, state.get("runners"), model_id) {
                ModelReadiness::Ready => return Ok(()),
                ModelReadiness::Failed(message) => bail!("{message}"),
                ModelReadiness::Pending => {
                    if state_has_model_instance(&state, model_id) {
                        saw_instance = true;
                    }
                }
            }
        }
        if started.elapsed() >= limit {
            break;
        }
        // A launch that vanishes and leaves a failed runner will not become ready.
        if !saw_instance && started.elapsed() >= Duration::from_secs(12) {
            if let Some(message) = first_runner_failure(&state) {
                bail!("{message}");
            }
        }
        thread::sleep(Duration::from_secs(2));
    }
    if saw_instance {
        bail!("模型仍在下载中（超过 5 分钟），可以点「打开算力看板」看下载完后再点启用。");
    }
    bail!("模型尚未准备完成，请稍后再试或打开算力看板查看。")
}

fn readiness_of_matching_instances(
    instances: &Value,
    runners: Option<&Value>,
    model_id: &str,
) -> ModelReadiness {
    let visit = |inst: &Value| -> Option<ModelReadiness> {
        if find_model_id_recursive(inst, 0).as_deref() != Some(model_id) {
            return None;
        }
        let ids = runner_ids_in(inst);
        if ids.is_empty() {
            return None;
        }
        let mut ready = true;
        for id in ids {
            let Some(runner) = runners.and_then(|all| all.get(&id)) else {
                ready = false;
                continue;
            };
            match tagged_name(runner) {
                Some("RunnerReady" | "RunnerLoaded" | "RunnerRunning") => {}
                Some("RunnerFailed") => {
                    return Some(ModelReadiness::Failed(runner_failure_message(runner)));
                }
                _ => ready = false,
            }
        }
        if ready {
            Some(ModelReadiness::Ready)
        } else {
            None
        }
    };

    let mut found_ready = false;
    match instances {
        Value::Object(map) => {
            for inst in map.values() {
                match visit(inst) {
                    Some(ModelReadiness::Failed(message)) => {
                        return ModelReadiness::Failed(message);
                    }
                    Some(ModelReadiness::Ready) => found_ready = true,
                    _ => {}
                }
            }
        }
        Value::Array(items) => {
            for inst in items {
                match visit(inst) {
                    Some(ModelReadiness::Failed(message)) => {
                        return ModelReadiness::Failed(message);
                    }
                    Some(ModelReadiness::Ready) => found_ready = true,
                    _ => {}
                }
            }
        }
        _ => {}
    }
    if found_ready {
        ModelReadiness::Ready
    } else {
        ModelReadiness::Pending
    }
}

fn state_has_model_instance(state: &Value, model_id: &str) -> bool {
    let Some(instances) = state.get("instances") else {
        return false;
    };
    let matches = |inst: &Value| find_model_id_recursive(inst, 0).as_deref() == Some(model_id);
    match instances {
        Value::Object(map) => map.values().any(matches),
        Value::Array(items) => items.iter().any(matches),
        _ => false,
    }
}

fn first_runner_failure(state: &Value) -> Option<String> {
    let runners = state.get("runners")?.as_object()?;
    for runner in runners.values() {
        if tagged_name(runner) == Some("RunnerFailed") {
            return Some(runner_failure_message(runner));
        }
    }
    None
}

fn runner_failure_message(runner: &Value) -> String {
    let raw = tagged_body(runner)
        .and_then(|body| {
            body.get("errorMessage")
                .or_else(|| body.get("error_message"))
                .and_then(Value::as_str)
        })
        .unwrap_or("")
        .trim();
    let lower = raw.to_ascii_lowercase();
    if lower.contains("metallib") || lower.contains("language version") {
        return "这台 macOS 还认不了算力组件，把系统更新后再试。这和内存、模型下载无关。"
            .to_string();
    }
    if raw.is_empty() {
        return "模型没有加载起来。可以点「打开算力看板」看这台电脑的状态。".to_string();
    }
    "模型没有加载起来。可以点「打开算力看板」看进度。".to_string()
}

fn tagged_name(value: &Value) -> Option<&str> {
    let obj = value.as_object()?;
    if obj.len() != 1 {
        return None;
    }
    obj.keys().next().map(String::as_str)
}

fn tagged_body(value: &Value) -> Option<&Value> {
    let obj = value.as_object()?;
    if obj.len() != 1 {
        return None;
    }
    obj.values().next()
}

fn runner_ids_in(value: &Value) -> Vec<String> {
    let mut ids = Vec::new();
    collect_runner_ids(value, &mut ids);
    ids
}

fn collect_runner_ids(value: &Value, ids: &mut Vec<String>) {
    let Some(obj) = value.as_object() else {
        return;
    };
    for key in ["runnerToShard", "runner_to_shard"] {
        if let Some(map) = obj.get(key).and_then(Value::as_object) {
            for id in map.keys() {
                if !ids.iter().any(|existing| existing == id) {
                    ids.push(id.clone());
                }
            }
        }
    }
    for child in obj.values() {
        collect_runner_ids(child, ids);
    }
}

pub(crate) fn select_safe_placement(
    previews_json: &Value,
    snapshot: &ExoClusterSnapshot,
) -> Result<Value> {
    let list = previews_json
        .get("previews")
        .and_then(Value::as_array)
        .or_else(|| previews_json.as_array())
        .context("算力节点未返回可用的摆放预览")?;

    let valid: Vec<&Value> = list
        .iter()
        .filter(|item| item.get("error").map_or(true, Value::is_null))
        .filter(|item| item.get("instance").is_some_and(|i| !i.is_null()))
        .collect();

    if valid.is_empty() {
        bail!("这几台电脑加起来可用内存不够放这个模型，请换小一档模型，或再打开一台苹果电脑加入。");
    }

    // Prefer a placement that leaves >= 4 GiB headroom on each node when memory deltas are reported,
    // and prefers multi-node tensor/pipeline placements when multiple Macs are online.
    let mut best: Option<(&Value, usize, bool)> = None;
    for candidate in &valid {
        let leaves_headroom = placement_leaves_headroom(candidate, snapshot);
        let participant_count = candidate
            .get("memory_delta_by_node")
            .and_then(Value::as_object)
            .map(|m| m.len())
            .unwrap_or(1);
        let is_tensor = candidate
            .get("sharding")
            .and_then(Value::as_str)
            .is_some_and(|s| s.eq_ignore_ascii_case("tensor"));

        let replace = match best {
            None => true,
            Some((_, best_nodes, best_headroom)) => {
                if leaves_headroom != best_headroom {
                    leaves_headroom
                } else if participant_count != best_nodes {
                    participant_count > best_nodes
                } else {
                    is_tensor
                }
            }
        };
        if replace {
            best = Some((candidate, participant_count, leaves_headroom));
        }
    }

    let (chosen, _, leaves_headroom) = best.unwrap();
    if !leaves_headroom && snapshot.available_memory_bytes > 0 {
        bail!("放入该模型后电脑剩余内存不足 4 GB，容易卡顿；请换小一档模型或关闭占内存的软件。");
    }

    Ok(chosen.get("instance").cloned().unwrap())
}

fn placement_leaves_headroom(preview: &Value, snapshot: &ExoClusterSnapshot) -> bool {
    let Some(deltas) = preview
        .get("memory_delta_by_node")
        .and_then(Value::as_object)
    else {
        return true;
    };
    if snapshot.nodes.is_empty() {
        return true;
    }
    for (node_key, delta_val) in deltas {
        let delta = delta_val.as_u64().unwrap_or(0);
        if let Some(node) = snapshot.nodes.iter().find(|n| {
            &n.node_id == node_key || (node_key == "local" && n.node_id == snapshot.self_node_id)
        }) {
            if node.available_memory_bytes > 0
                && node.available_memory_bytes.saturating_sub(delta) < SYSTEM_MEMORY_HEADROOM_BYTES
            {
                return false;
            }
        }
    }
    true
}

pub(crate) fn parse_exo_state(
    origin: &str,
    self_node_id: &str,
    state: &Value,
) -> ExoClusterSnapshot {
    let mut nodes = extract_nodes_from_state(state);
    if nodes.is_empty() {
        nodes.push(ClusterNodeSummary {
            node_id: self_node_id.to_string(),
            label: "Mac".to_string(),
            total_memory_bytes: 0,
            available_memory_bytes: 0,
        });
    }

    let total_memory_bytes: u64 = nodes.iter().map(|n| n.total_memory_bytes).sum();
    let available_memory_bytes: u64 = nodes.iter().map(|n| n.available_memory_bytes).sum();
    let avail_vec: Vec<u64> = nodes
        .iter()
        .filter_map(|n| (n.available_memory_bytes > 0).then_some(n.available_memory_bytes))
        .collect();
    let usable_memory_bytes = if avail_vec.is_empty() {
        0
    } else {
        usable_cluster_memory_bytes(&avail_vec)
    };

    let active_models = extract_active_models(state);

    ExoClusterSnapshot {
        origin: origin.to_string(),
        self_node_id: self_node_id.to_string(),
        nodes,
        total_memory_bytes,
        available_memory_bytes,
        usable_memory_bytes,
        active_models,
        incompatible_peer_count: 0,
    }
}

fn extract_nodes_from_state(state: &Value) -> Vec<ClusterNodeSummary> {
    let mut nodes = Vec::new();
    let Some(root) = state.as_object() else {
        return nodes;
    };

    // exo's `/state` places per-node profiles under `node_identities` / `node_memory` / `topology.nodes`.
    let identities = root.get("node_identities").and_then(Value::as_object);
    let memories = root
        .get("node_memory")
        .or_else(|| root.get("memories"))
        .and_then(Value::as_object);

    let mut seen_ids = BTreeSet::new();
    if let Some(id_map) = identities {
        for key in id_map.keys() {
            seen_ids.insert(key.clone());
        }
    }
    if let Some(mem_map) = memories {
        for key in mem_map.keys() {
            seen_ids.insert(key.clone());
        }
    }
    if let Some(topo_nodes) = root
        .get("topology")
        .and_then(|t| t.get("nodes"))
        .or_else(|| root.get("nodes"))
    {
        match topo_nodes {
            Value::Object(map) => {
                for key in map.keys() {
                    seen_ids.insert(key.clone());
                }
            }
            Value::Array(arr) => {
                for item in arr {
                    if let Some(id) = item
                        .as_str()
                        .or_else(|| item.get("node_id").and_then(Value::as_str))
                        .or_else(|| item.get("id").and_then(Value::as_str))
                    {
                        seen_ids.insert(id.to_string());
                    }
                }
            }
            _ => {}
        }
    }

    for node_id in seen_ids {
        let id_obj = identities.and_then(|m| m.get(&node_id));
        let topo_obj = root
            .get("topology")
            .and_then(|t| t.get("nodes"))
            .and_then(Value::as_object)
            .and_then(|m| m.get(&node_id))
            .or_else(|| {
                root.get("nodes")
                    .and_then(Value::as_object)
                    .and_then(|m| m.get(&node_id))
            });
        let mem_obj = memories
            .and_then(|m| m.get(&node_id))
            .or_else(|| topo_obj.and_then(|o| o.get("memory")));

        let label = id_obj
            .and_then(pick_device_label)
            .or_else(|| topo_obj.and_then(pick_device_label))
            .unwrap_or_else(|| short_node_label(&node_id));

        let (total_memory_bytes, available_memory_bytes) = mem_obj
            .map(extract_memory_pair)
            .or_else(|| topo_obj.map(extract_memory_pair))
            .unwrap_or((0, 0));

        nodes.push(ClusterNodeSummary {
            node_id,
            label,
            total_memory_bytes,
            available_memory_bytes,
        });
    }

    nodes
}

fn pick_device_label(obj: &Value) -> Option<String> {
    for key in [
        "friendly_name",
        "device_name",
        "model",
        "chip",
        "hostname",
        "name",
    ] {
        if let Some(s) = obj
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return Some(s.to_string());
        }
    }
    None
}

fn extract_memory_pair(obj: &Value) -> (u64, u64) {
    let total = byte_value(obj, &["ram_total", "total", "total_bytes", "memory_total"]);
    let avail = byte_value(
        obj,
        &[
            "ram_available",
            "available",
            "free",
            "available_bytes",
            "memory_available",
        ],
    );
    (total, if avail > 0 { avail } else { total })
}

fn byte_value(obj: &Value, keys: &[&str]) -> u64 {
    for k in keys {
        if let Some(v) = obj.get(*k) {
            if let Some(n) = v.as_u64() {
                return n;
            }
            if let Some(nested) = v.get("in_bytes").and_then(Value::as_u64) {
                return nested;
            }
        }
    }
    0
}

fn extract_active_models(state: &Value) -> Vec<String> {
    let mut models = Vec::new();
    let Some(instances) = state.get("instances") else {
        return models;
    };
    let mut push_from_val = |inst: &Value| {
        if let Some(m) = find_model_id_recursive(inst, 0) {
            if !models.contains(&m) {
                models.push(m);
            }
        }
    };
    match instances {
        Value::Object(map) => {
            for val in map.values() {
                push_from_val(val);
            }
        }
        Value::Array(arr) => {
            for val in arr {
                push_from_val(val);
            }
        }
        _ => {}
    }
    models
}

fn find_model_id_recursive(v: &Value, depth: usize) -> Option<String> {
    if depth > 4 {
        return None;
    }
    if let Some(obj) = v.as_object() {
        for key in ["model_id", "modelId"] {
            if let Some(s) = obj
                .get(key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                return Some(s.to_string());
            }
        }
        for child in obj.values() {
            if let Some(found) = find_model_id_recursive(child, depth + 1) {
                return Some(found);
            }
        }
    }
    None
}

fn count_mismatched_devices(v: &Value) -> usize {
    if let Some(arr) = v.as_array() {
        return arr.len();
    }
    if let Some(arr) = v.get("devices").and_then(Value::as_array) {
        return arr.len();
    }
    if let Some(obj) = v.as_object() {
        return obj.len();
    }
    0
}

fn short_node_label(node_id: &str) -> String {
    let prefix: String = node_id.chars().take(6).collect();
    format!("Mac ({prefix})")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_multi_node_state_and_enforces_4gb_headroom() {
        let gib = 1024_u64 * 1024 * 1024;
        let state = json!({
            "node_identities": {
                "node-a": { "friendly_name": "Mac mini (M4)" },
                "node-b": { "friendly_name": "MacBook Pro (M3 Max)" }
            },
            "node_memory": {
                "node-a": { "ram_total": { "in_bytes": 16 * gib }, "ram_available": { "in_bytes": 12 * gib } },
                "node-b": { "ram_total": { "in_bytes": 36 * gib }, "ram_available": { "in_bytes": 28 * gib } }
            },
            "instances": {
                "inst-1": {
                    "MlxRingInstance": {
                        "shard_assignments": { "model_id": "mlx-community/Qwen2.5-7B-Instruct-4bit" }
                    }
                }
            }
        });

        let snap = parse_exo_state("http://127.0.0.1:52415", "node-a", &state);
        assert_eq!(snap.nodes.len(), 2);
        assert_eq!(snap.usable_memory_bytes, (12 - 4 + 28 - 4) * gib);
        assert_eq!(
            snap.active_models,
            vec!["mlx-community/Qwen2.5-7B-Instruct-4bit"]
        );

        let tight_preview = json!({
            "previews": [{
                "sharding": "Tensor",
                "instance": { "id": "ok" },
                "memory_delta_by_node": { "node-a": 10 * gib },
                "error": null
            }]
        });
        // 12 GiB available - 10 GiB delta = 2 GiB (< 4 GiB headroom) -> rejected!
        assert!(select_safe_placement(&tight_preview, &snap).is_err());

        let safe_preview = json!({
            "previews": [{
                "sharding": "Tensor",
                "instance": { "id": "safe" },
                "memory_delta_by_node": { "node-a": 6 * gib, "node-b": 10 * gib },
                "error": null
            }]
        });
        let picked = select_safe_placement(&safe_preview, &snap).unwrap();
        assert_eq!(picked["id"], "safe");
    }

    #[test]
    fn readiness_uses_runner_status_not_a_missing_await_route() {
        let ready = json!({
            "instances": {
                "inst-1": {
                    "MlxRingInstance": {
                        "shardAssignments": {
                            "modelId": "mlx-community/Qwen2.5-7B-Instruct-4bit",
                            "runnerToShard": { "runner-a": {} }
                        }
                    }
                }
            },
            "runners": { "runner-a": { "RunnerReady": {} } }
        });
        assert_eq!(
            readiness_of_matching_instances(
                ready.get("instances").unwrap(),
                ready.get("runners"),
                "mlx-community/Qwen2.5-7B-Instruct-4bit"
            ),
            ModelReadiness::Ready
        );

        let failed = json!({
            "instances": {
                "inst-1": {
                    "MlxRingInstance": {
                        "shardAssignments": {
                            "modelId": "mlx-community/Qwen2.5-7B-Instruct-4bit",
                            "runnerToShard": { "runner-a": {} }
                        }
                    }
                }
            },
            "runners": {
                "runner-a": {
                    "RunnerFailed": {
                        "errorMessage": "Failed to load the default metallib. language version 4.0"
                    }
                }
            }
        });
        match readiness_of_matching_instances(
            failed.get("instances").unwrap(),
            failed.get("runners"),
            "mlx-community/Qwen2.5-7B-Instruct-4bit",
        ) {
            ModelReadiness::Failed(message) => {
                assert!(message.contains("macOS"), "{message}");
                assert!(message.contains("内存"), "{message}");
            }
            other => panic!("expected failure, got {other:?}"),
        }
    }
}
