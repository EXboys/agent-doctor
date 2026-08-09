use agent_doctor_core::{
    add_host, add_project, list_hosts, list_projects, remove_host, remove_project, run_remote_doctor,
    ProbeStatus, RemoteDoctorOptions,
};
use anyhow::Result;

pub fn host_add(id: &str, ssh_config_host: &str) -> Result<()> {
    add_host(id, ssh_config_host)?;
    println!("Added host '{id}' (ssh {ssh_config_host})");
    Ok(())
}

pub fn host_list(json: bool) -> Result<()> {
    let hosts = list_hosts()?;
    if json {
        let map: serde_json::Map<String, serde_json::Value> = hosts
            .into_iter()
            .map(|(id, entry)| (id, serde_json::to_value(entry).unwrap_or_default()))
            .collect();
        println!("{}", serde_json::to_string_pretty(&map)?);
        return Ok(());
    }
    if hosts.is_empty() {
        println!("No remote hosts. Add one with:");
        println!("  agent-doctor remote host add <id> --ssh-config-host <Host>");
        return Ok(());
    }
    for (id, entry) in hosts {
        println!(
            "{id}  ssh={}  projects={}",
            entry.ssh_config_host,
            entry.projects.len()
        );
    }
    Ok(())
}

pub fn host_remove(id: &str) -> Result<()> {
    remove_host(id)?;
    println!("Removed host '{id}'");
    Ok(())
}

pub fn project_add(host: &str, name: &str, path: &str, runtimes: Vec<String>) -> Result<()> {
    add_project(host, name, path, runtimes)?;
    println!("Added project '{host}/{name}' → {path}");
    Ok(())
}

pub fn project_list(host: Option<&str>, json: bool) -> Result<()> {
    let projects = list_projects(host)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&projects)?);
        return Ok(());
    }
    if projects.is_empty() {
        println!("No remote projects.");
        return Ok(());
    }
    for (host_id, name, entry) in projects {
        let runtimes = if entry.runtimes.is_empty() {
            "all".to_string()
        } else {
            entry.runtimes.join(",")
        };
        println!("{host_id}/{name}  {}  runtimes={runtimes}", entry.path);
    }
    Ok(())
}

pub fn project_remove(host: &str, name: &str) -> Result<()> {
    remove_project(host, name)?;
    println!("Removed project '{host}/{name}'");
    Ok(())
}

pub fn doctor(target: &str, json: bool, runtime: Option<&str>) -> Result<()> {
    let report = run_remote_doctor(
        target,
        RemoteDoctorOptions {
            runtime_filter: runtime.map(str::to_string),
            save_report: true,
        },
    )?;

    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    println!("Agent Doctor — remote doctor\n");
    println!("Target: {}/{}", report.host_id, report.project_id);
    println!("SSH:    {}", report.ssh_config_host);
    println!("Path:   {}", report.project_path);
    if let Some(home) = &report.remote_home {
        println!("HOME:   {home}");
    }
    println!();

    for check in &report.checks {
        println!("{} {}", status_glyph(check.status), format_check(check));
        for detail in &check.details {
            println!("    {detail}");
        }
    }

    for runtime in &report.runtimes {
        println!();
        println!("{} ({})", runtime.display_name, runtime.runtime_id);
        for check in &runtime.checks {
            if matches!(check.status, ProbeStatus::NotApplicable) {
                continue;
            }
            println!("  {} {}", status_glyph(check.status), format_check(check));
            for detail in &check.details {
                println!("      {detail}");
            }
        }
    }

    if let Some(path) = &report.report_path {
        println!();
        println!("Report: {path}");
    }
    Ok(())
}

fn status_glyph(status: ProbeStatus) -> &'static str {
    match status {
        ProbeStatus::Pass => "✓",
        ProbeStatus::Warn => "!",
        ProbeStatus::Fail => "✗",
        ProbeStatus::NotApplicable => "-",
        ProbeStatus::NotChecked => "?",
    }
}

fn format_check(check: &agent_doctor_core::ProbeCheck) -> String {
    format!("{} — {}", check.title, check.message)
}
