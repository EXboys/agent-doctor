use anyhow::Result;
use agent_doctor_core::{load_doctor_node_config, run_connect_loop, ConnectOptions};

pub fn run(
    inventory_interval_secs: u64,
    heartbeat_interval_secs: u64,
    max_backoff_secs: u64,
) -> Result<()> {
    let config = load_doctor_node_config()?;
    eprintln!(
        "Agent Doctor connect — engine_id={} base={} (source {})",
        config.engine_id, config.base_url, config.config_source
    );
    eprintln!("Ctrl+C to stop. Fleet will show this node online while connected.");
    let options = ConnectOptions {
        doctor_version: env!("CARGO_PKG_VERSION").to_string(),
        inventory_interval_secs,
        heartbeat_interval_secs,
        max_backoff_secs,
    };
    run_connect_loop(&config, &options)
}
