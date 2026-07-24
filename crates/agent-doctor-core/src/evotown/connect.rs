//! Evotown Doctor node WebSocket client (protocol v1).
//!
//! Presence + inventory + job.assign execution via local CLI / hooks.

use std::collections::HashMap;
use std::path::Path;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use tungstenite::{connect, Message};
use url::Url;

use crate::doctor::run_doctor;
use crate::evotown::jobs::{execute_job, AssignedJob, JobResult};
use crate::setup::evotown_agent_env_path;

pub const PROTOCOL_VERSION: u32 = 1;
const DEFAULT_INVENTORY_INTERVAL_SECS: u64 = 60;
const DEFAULT_HEARTBEAT_INTERVAL_SECS: u64 = 25;

#[derive(Debug, Clone)]
pub struct ConnectOptions {
    pub doctor_version: String,
    pub inventory_interval_secs: u64,
    pub heartbeat_interval_secs: u64,
    pub max_backoff_secs: u64,
}

impl Default for ConnectOptions {
    fn default() -> Self {
        Self {
            doctor_version: env!("CARGO_PKG_VERSION").to_string(),
            inventory_interval_secs: DEFAULT_INVENTORY_INTERVAL_SECS,
            heartbeat_interval_secs: DEFAULT_HEARTBEAT_INTERVAL_SECS,
            max_backoff_secs: 60,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DoctorNodeConfig {
    pub base_url: String,
    pub engine_id: String,
    pub ingest_token: String,
    pub config_source: String,
}

pub fn load_doctor_node_config() -> Result<DoctorNodeConfig> {
    let path = evotown_agent_env_path().context(
        "could not resolve ~/.config/evotown/evotown.agent.env — run `agent-doctor setup` first",
    )?;
    if !path.exists() {
        bail!(
            "missing {} — run `agent-doctor setup --url <evotown> --key evk_...` then register an engine",
            path.display()
        );
    }
    load_doctor_node_config_from_path(&path)
}

pub fn load_doctor_node_config_from_path(path: &Path) -> Result<DoctorNodeConfig> {
    let values = load_env_map(path)?;
    let base_url = values
        .get("EVOTOWN_URL")
        .cloned()
        .or_else(|| std::env::var("EVOTOWN_URL").ok())
        .map(|v| v.trim().trim_end_matches('/').to_string())
        .filter(|v| !v.is_empty())
        .with_context(|| format!("EVOTOWN_URL is required in {}", path.display()))?;

    let engine_id = values
        .get("EVOTOWN_ENGINE_ID")
        .cloned()
        .or_else(|| std::env::var("EVOTOWN_ENGINE_ID").ok())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .with_context(|| {
            format!(
                "EVOTOWN_ENGINE_ID is required in {} — register with \
                 `evotown-agent-setup.py register --save-token`",
                path.display()
            )
        })?;

    let ingest_token = values
        .get("EVOTOWN_ENGINE_INGEST_TOKEN")
        .cloned()
        .or_else(|| std::env::var("EVOTOWN_ENGINE_INGEST_TOKEN").ok())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .with_context(|| {
            format!(
                "EVOTOWN_ENGINE_INGEST_TOKEN (evi_…) is required in {} — run \
                 `evotown-agent-setup.py register --save-token` with IT bootstrap token",
                path.display()
            )
        })?;

    if !ingest_token.starts_with("evi_") {
        bail!(
            "EVOTOWN_ENGINE_INGEST_TOKEN must start with evi_ (got prefix {:?})",
            ingest_token.chars().take(4).collect::<String>()
        );
    }

    Ok(DoctorNodeConfig {
        base_url,
        engine_id,
        ingest_token,
        config_source: path.display().to_string(),
    })
}

fn load_env_map(path: &Path) -> Result<HashMap<String, String>> {
    let raw = std::fs::read_to_string(path)?;
    let mut values = HashMap::new();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        values.insert(
            key.trim().to_string(),
            value.trim().trim_matches('"').to_string(),
        );
    }
    Ok(values)
}

pub fn build_inventory_payload() -> Value {
    let report = run_doctor();
    let runtimes: Vec<Value> = report
        .runtimes
        .iter()
        .map(|rt| {
            json!({
                "id": rt.id,
                "installed": rt.installed,
                "version": rt.version,
                "binary_path": rt.binary_path,
                "gateway_url": rt.profile.gateway_url,
            })
        })
        .collect();
    json!({
        "runtimes": runtimes,
        "company_gateway_url": report.company_gateway_url,
        "active_preset": report.active_preset,
        "profile_env_exists": report.profile_env_exists,
        "capabilities": ["presence", "inventory", "job.assign"],
    })
}

fn ws_url(base_url: &str, token: &str) -> Result<Url> {
    let trimmed = base_url.trim().trim_end_matches('/');
    let ws_base = if let Some(rest) = trimmed.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = trimmed.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        bail!("EVOTOWN_URL must start with http:// or https://");
    };
    let mut url = Url::parse(&format!("{ws_base}/api/v1/doctor/ws"))?;
    url.query_pairs_mut().append_pair("token", token);
    Ok(url)
}

fn node_id() -> String {
    hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "unknown-host".to_string())
}

enum Outbound {
    Json(Value),
}

/// Blocking connect loop with exponential backoff.
pub fn run_connect_loop(config: &DoctorNodeConfig, options: &ConnectOptions) -> Result<()> {
    let mut backoff = 1u64;
    loop {
        match run_one_session(config, options) {
            Ok(()) => {
                eprintln!("! doctor ws session ended; reconnecting in {backoff}s");
            }
            Err(err) => {
                eprintln!("! doctor ws error: {err:#}; reconnecting in {backoff}s");
            }
        }
        thread::sleep(Duration::from_secs(backoff));
        backoff = (backoff.saturating_mul(2)).min(options.max_backoff_secs);
    }
}

fn run_one_session(config: &DoctorNodeConfig, options: &ConnectOptions) -> Result<()> {
    let url = ws_url(&config.base_url, &config.ingest_token)?;
    eprintln!(
        "→ connecting doctor node engine_id={} url={}",
        config.engine_id,
        url.as_str().split('?').next().unwrap_or(url.as_str())
    );

    let (mut socket, _response) = connect(url.as_str())
        .with_context(|| format!("websocket connect to {}", config.base_url))?;

    let welcome = read_json(&mut socket)?;
    if welcome.get("type").and_then(|v| v.as_str()) != Some("welcome") {
        bail!("expected welcome, got {welcome}");
    }
    eprintln!(
        "✓ welcome protocol_version={} engine_id={}",
        welcome
            .get("protocol_version")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        welcome
            .get("engine_id")
            .and_then(|v| v.as_str())
            .unwrap_or("?")
    );

    let inventory = build_inventory_payload();
    send_json(
        &mut socket,
        json!({
            "type": "hello",
            "engine_id": config.engine_id,
            "node_id": node_id(),
            "doctor_version": options.doctor_version,
            "inventory": inventory,
        }),
    )?;
    let ack = read_json(&mut socket)?;
    if ack.get("type").and_then(|v| v.as_str()) != Some("ack") {
        bail!("expected hello ack, got {ack}");
    }
    eprintln!(
        "✓ hello ack — online (drained_jobs={})",
        ack.get("drained_jobs")
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
    );

    let (tx, rx): (Sender<Outbound>, Receiver<Outbound>) = mpsc::channel();
    let inventory_every = Duration::from_secs(options.inventory_interval_secs.max(10));
    let heartbeat_every = Duration::from_secs(options.heartbeat_interval_secs.max(5));
    let mut last_inventory = std::time::Instant::now();
    let mut last_heartbeat = std::time::Instant::now();

    set_socket_read_timeout(&mut socket, Duration::from_secs(1));

    loop {
        // Flush worker → server messages first
        loop {
            match rx.try_recv() {
                Ok(Outbound::Json(value)) => send_json(&mut socket, value)?,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => bail!("worker channel disconnected"),
            }
        }

        match socket.read() {
            Ok(Message::Text(text)) => {
                handle_server_text(&mut socket, &text, &tx)?;
            }
            Ok(Message::Ping(payload)) => {
                socket.send(Message::Pong(payload))?;
            }
            Ok(Message::Close(_)) => {
                bail!("server closed websocket");
            }
            Ok(_) => {}
            Err(tungstenite::Error::Io(err))
                if err.kind() == std::io::ErrorKind::WouldBlock
                    || err.kind() == std::io::ErrorKind::TimedOut => {}
            Err(err) => return Err(err.into()),
        }

        if last_heartbeat.elapsed() >= heartbeat_every {
            send_json(&mut socket, json!({ "type": "heartbeat" }))?;
            last_heartbeat = std::time::Instant::now();
        }

        if last_inventory.elapsed() >= inventory_every {
            let inventory = build_inventory_payload();
            send_json(
                &mut socket,
                json!({
                    "type": "inventory",
                    "inventory": inventory,
                }),
            )?;
            last_inventory = std::time::Instant::now();
            eprintln!("→ inventory refreshed");
        }
    }
}

fn handle_server_text(
    socket: &mut tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<std::net::TcpStream>>,
    text: &str,
    tx: &Sender<Outbound>,
) -> Result<()> {
    let msg: Value = serde_json::from_str(text).context("invalid server JSON")?;
    match msg.get("type").and_then(|v| v.as_str()).unwrap_or("") {
        "ack" | "welcome" => {}
        "error" => {
            eprintln!(
                "! server error: {}",
                msg.get("detail").and_then(|v| v.as_str()).unwrap_or("?")
            );
        }
        "ping" => {
            send_json(
                socket,
                json!({
                    "type": "pong",
                    "ts": msg.get("ts").cloned().unwrap_or(Value::Null),
                }),
            )?;
        }
        "job.assign" => {
            let job = AssignedJob::from_assign_message(&msg)?;
            eprintln!(
                "→ job.assign job_id={} title={}",
                job.job_id,
                if job.title.is_empty() {
                    "(no title)"
                } else {
                    &job.title
                }
            );
            send_json(
                socket,
                json!({
                    "type": "job.ack",
                    "job_id": job.job_id,
                    "run_id": job.run_id,
                }),
            )?;
            let tx_worker = tx.clone();
            thread::spawn(move || {
                let _ = tx_worker.send(Outbound::Json(json!({
                    "type": "job.event",
                    "job_id": job.job_id,
                    "event": "started",
                    "runtime": crate::evotown::jobs::resolve_runtime(&job),
                })));
                let result = execute_job(&job);
                let _ = tx_worker.send(Outbound::Json(complete_message(&job, &result)));
            });
        }
        other => {
            eprintln!("! unknown server message type={other}");
        }
    }
    Ok(())
}

fn complete_message(job: &AssignedJob, result: &JobResult) -> Value {
    json!({
        "type": "job.complete",
        "job_id": job.job_id,
        "run_id": job.run_id,
        "status": result.status,
        "exit_code": result.exit_code,
        "result_summary": result.result_summary,
        "log_excerpt": result.log_excerpt,
        "signals": {
            "runtime": result.runtime,
            "via": "agent-doctor",
            "extra": result.signals,
        },
    })
}

fn set_socket_read_timeout(
    socket: &mut tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<std::net::TcpStream>>,
    timeout: Duration,
) {
    use tungstenite::stream::MaybeTlsStream;
    match socket.get_mut() {
        MaybeTlsStream::Plain(stream) => {
            let _ = stream.set_read_timeout(Some(timeout));
        }
        MaybeTlsStream::Rustls(stream) => {
            let _ = stream.get_mut().set_read_timeout(Some(timeout));
        }
        _ => {}
    }
}

fn send_json(
    socket: &mut tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<std::net::TcpStream>>,
    value: Value,
) -> Result<()> {
    socket.send(Message::Text(value.to_string().into()))?;
    Ok(())
}

fn read_json(
    socket: &mut tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<std::net::TcpStream>>,
) -> Result<Value> {
    loop {
        match socket.read()? {
            Message::Text(text) => {
                return serde_json::from_str(&text).context("invalid JSON from server");
            }
            Message::Ping(payload) => {
                socket.send(Message::Pong(payload))?;
            }
            Message::Close(_) => bail!("server closed during handshake"),
            _ => continue,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn loads_node_config_from_env_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("evotown.agent.env");
        let mut file = std::fs::File::create(&path).unwrap();
        writeln!(
            file,
            "EVOTOWN_URL=https://evotown.example\nEVOTOWN_ENGINE_ID=doctor-1\nEVOTOWN_ENGINE_INGEST_TOKEN=evi_testtoken\nEVOTOWN_API_KEY=evk_x"
        )
        .unwrap();
        let cfg = load_doctor_node_config_from_path(&path).unwrap();
        assert_eq!(cfg.engine_id, "doctor-1");
        assert!(cfg.ingest_token.starts_with("evi_"));
    }

    #[test]
    fn rejects_missing_evi() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("evotown.agent.env");
        std::fs::write(
            &path,
            "EVOTOWN_URL=https://evotown.example\nEVOTOWN_ENGINE_ID=doctor-1\nEVOTOWN_API_KEY=evk_x\n",
        )
        .unwrap();
        let err = load_doctor_node_config_from_path(&path).unwrap_err();
        assert!(err.to_string().contains("EVOTOWN_ENGINE_INGEST_TOKEN"));
    }
}
