//! Register prompt RPCs on the loopback client service (B computer side).

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;

use agent_doctor_core::{
    register_link_method, LinkMethodHandler, LinkMethodOutcome, LinkResponse, LinkRpcContext,
};
use agent_doctor_prompt::{
    run_prompt_session_with_cancel, PromptSessionCancel, PromptSessionControl, PromptSessionEvent,
    PromptSessionOptions,
};
use base64::Engine as _;
use serde_json::json;

struct ActivePrompt {
    cancel: PromptSessionCancel,
    control: PromptSessionControl,
}

static ACTIVE: Mutex<Option<HashMap<String, ActivePrompt>>> = Mutex::new(None);

fn with_active<R>(f: impl FnOnce(&mut HashMap<String, ActivePrompt>) -> R) -> R {
    let mut guard = ACTIVE.lock().expect("link prompt lock");
    let map = guard.get_or_insert_with(HashMap::new);
    f(map)
}

fn param_str(params: &serde_json::Value, key: &str) -> Option<String> {
    params
        .get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn write_images(params: &serde_json::Value) -> Vec<PathBuf> {
    let Some(arr) = params.get("images").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    let dir = std::env::temp_dir().join(format!(
        "ad-link-img-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    ));
    let _ = fs::create_dir_all(&dir);
    let mut paths = Vec::new();
    for (idx, item) in arr.iter().enumerate() {
        let name = item
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("image.png");
        let b64 = item.get("base64").and_then(|v| v.as_str()).unwrap_or("");
        let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(b64) else {
            continue;
        };
        let safe = name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect::<String>();
        let path = dir.join(format!("{idx}-{safe}"));
        if fs::write(&path, bytes).is_ok() {
            paths.push(path);
        }
    }
    paths
}

fn handle_start(ctx: LinkRpcContext) -> LinkMethodOutcome {
    let req_id = ctx.request.id.clone();
    let params = ctx.request.params.clone();
    let socket = ctx.socket.clone();

    let client_run_id =
        param_str(&params, "client_run_id").unwrap_or_else(|| format!("link-{}", uuid_like()));
    let runtime = param_str(&params, "runtime").unwrap_or_else(|| "claude-code".into());
    let prompt = param_str(&params, "prompt").unwrap_or_default();
    if prompt.trim().is_empty() {
        return LinkMethodOutcome::Reply(Err(anyhow::anyhow!("请先输入要问的内容")));
    }

    let cancel = PromptSessionCancel::new();
    let control = PromptSessionControl::new();
    with_active(|map| {
        map.insert(
            client_run_id.clone(),
            ActivePrompt {
                cancel: cancel.clone(),
                control: control.clone(),
            },
        );
    });

    let image_paths = write_images(&params);
    let options = PromptSessionOptions {
        runtime: runtime.clone(),
        prompt,
        cwd: param_str(&params, "cwd").map(PathBuf::from),
        timeout_sec: params
            .get("timeout_sec")
            .and_then(|v| v.as_u64())
            .unwrap_or(86_400),
        dangerously_skip_permissions: params
            .get("dangerously_skip_permissions")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        full_auto: params
            .get("full_auto")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        resume_thread_id: param_str(&params, "resume_thread_id"),
        selected_mcps: params
            .get("selected_mcps")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
        image_paths,
        workspace_name: param_str(&params, "workspace_name"),
        provider_id: param_str(&params, "provider_id"),
        model: param_str(&params, "model"),
        browser_tools_enabled: params
            .get("browser_tools_enabled")
            .and_then(|v| v.as_bool()),
        computer_use_enabled: params.get("computer_use_enabled").and_then(|v| v.as_bool()),
    };

    let needs_control = {
        let runtime = options.runtime.as_str();
        let claude_ask = runtime == "claude-code" && !options.dangerously_skip_permissions;
        let codex_ask = runtime == "codex" && !options.full_auto;
        let deepseek_ask = runtime == "deepseek-harness";
        let acp_ask = runtime == "acp";
        let doctor_ask = runtime == agent_doctor_repair::OFFICIAL_AGENT_RUNTIME_ID;
        claude_ask || codex_ask || deepseek_ask || acp_ask || doctor_ask
    };
    let control_for_run = if needs_control {
        Some(control.clone())
    } else {
        None
    };

    let client_run_for_events = client_run_id.clone();
    thread::Builder::new()
        .name("ad-link-prompt".into())
        .spawn(move || {
            let result = run_prompt_session_with_cancel(
                &options,
                cancel,
                control_for_run,
                |event: PromptSessionEvent| {
                    let mut value = match serde_json::to_value(&event) {
                        Ok(v) => v,
                        Err(_) => return,
                    };
                    if let Some(obj) = value.as_object_mut() {
                        // Ensure the A computer can attribute events to the chat.
                        if !obj.contains_key("client_run_id")
                            || obj
                                .get("client_run_id")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .is_empty()
                        {
                            obj.insert(
                                "client_run_id".into(),
                                json!(client_run_for_events.clone()),
                            );
                        }
                    }
                    let _ = socket.emit_prompt_event(value);
                },
            );

            with_active(|map| {
                map.remove(&client_run_id);
            });

            let response = match result {
                Ok(report) => match serde_json::to_value(&report) {
                    Ok(value) => LinkResponse {
                        id: req_id,
                        ok: true,
                        result: Some(value),
                        error: None,
                    },
                    Err(err) => LinkResponse {
                        id: req_id,
                        ok: false,
                        result: None,
                        error: Some(format!("encode report: {err}")),
                    },
                },
                Err(err) => LinkResponse {
                    id: req_id,
                    ok: false,
                    result: None,
                    error: Some(format!("{err:#}")),
                },
            };
            let _ = socket.send_response(&response);
        })
        .ok();

    LinkMethodOutcome::Deferred
}

fn handle_cancel(ctx: LinkRpcContext) -> LinkMethodOutcome {
    let client_run_id = param_str(&ctx.request.params, "client_run_id").unwrap_or_default();
    let cancelled = with_active(|map| {
        if let Some(active) = map.get(&client_run_id) {
            active.cancel.request();
            true
        } else {
            false
        }
    });
    LinkMethodOutcome::Reply(Ok(json!({ "cancelled": cancelled })))
}

fn handle_resolve(ctx: LinkRpcContext) -> LinkMethodOutcome {
    let params = &ctx.request.params;
    let session_id = param_str(params, "session_id").unwrap_or_default();
    let request_id = param_str(params, "request_id").unwrap_or_default();
    let allow = params
        .get("allow")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let text = param_str(params, "text");
    let updated = param_str(params, "updated_permissions_json");

    // Find by scanning active prompts — control is shared; session_id is used
    // by PromptSessionControl's pending map keys indirectly via request_id.
    let result = with_active(|map| {
        for active in map.values() {
            let sent_text = text.as_deref().map(str::trim).filter(|s| !s.is_empty());
            let outcome = if let Some(line) = sent_text {
                active.control.respond_line(&request_id, line)
            } else if let Some(raw) = updated.as_deref() {
                match serde_json::from_str::<serde_json::Value>(raw) {
                    Ok(updated_val) => active.control.respond_permission_with(
                        &request_id,
                        allow,
                        Some(updated_val),
                    ),
                    Err(err) => Err(anyhow::anyhow!("bad updatedPermissions: {err}")),
                }
            } else {
                active.control.respond_permission(&request_id, allow)
            };
            if outcome.is_ok() {
                let _ = session_id;
                return Ok(json!({ "ok": true }));
            }
        }
        Err(anyhow::anyhow!("找不到这次确认请求，可能已经结束了。"))
    });
    LinkMethodOutcome::Reply(result)
}

fn uuid_like() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{t:x}")
}

/// Wire prompt methods into the loopback daemon. Call once at desktop startup.
pub fn register_prompt_link_methods() {
    let start: LinkMethodHandler = Arc::new(handle_start);
    let cancel: LinkMethodHandler = Arc::new(handle_cancel);
    let resolve: LinkMethodHandler = Arc::new(handle_resolve);
    register_link_method("start_prompt", start);
    register_link_method("cancel_prompt", cancel);
    register_link_method("resolve_permission", resolve);
}
