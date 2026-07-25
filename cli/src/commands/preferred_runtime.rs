use agent_doctor_core::{preferred_runtime_status, set_preferred_runtime};
use anyhow::Result;

pub fn show(json: bool) -> Result<()> {
    let status = preferred_runtime_status();
    if json {
        println!("{}", serde_json::to_string_pretty(&status)?);
        return Ok(());
    }
    match &status.runtime {
        Some(runtime) => {
            println!("Preferred runtime: {runtime}");
            println!(
                "Installed locally: {}",
                if status.installed { "yes" } else { "no (warn)" }
            );
        }
        None => {
            println!("Preferred runtime: (not set)");
            println!(
                "Set with: agent-doctor preferred-runtime use <claude-code|hermes|openclaw|codex>"
            );
        }
    }
    if let Some(path) = &status.env_path {
        println!("Config: {path}");
    }
    println!("Known: {}", status.known_runtimes.join(", "));
    Ok(())
}

pub fn use_runtime(runtime: &str, json: bool) -> Result<()> {
    let status = set_preferred_runtime(runtime)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&status)?);
        return Ok(());
    }
    let name = status.runtime.as_deref().unwrap_or("?");
    println!("✓ Preferred runtime set to {name}");
    if !status.installed {
        eprintln!("! Warning: '{name}' is not installed on this machine yet.");
    }
    if let Some(path) = &status.env_path {
        println!("Wrote EVOTOWN_RUNTIME to {path}");
    }
    println!("Restart `agent-doctor connect` so Evotown receives the updated inventory.");
    Ok(())
}
