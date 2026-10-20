use agent_doctor_core::{build_explain_input, probe_all_runtimes, probe_issue_score, run_doctor};
use agent_doctor_repair::explain_runtime;
use anyhow::Result;

use crate::commands::print_explain_report;

pub fn run(json: bool, explain: bool) -> Result<()> {
    let report = run_doctor();

    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        if explain {
            for runtime in &report.runtimes {
                let probe = probe_all_runtimes()
                    .into_iter()
                    .find(|item| item.runtime_id == runtime.id);
                if let Some(probe) = probe {
                    if probe_issue_score(&probe) > 0 {
                        let input = build_explain_input(&runtime.id, &probe, None);
                        let explain_report = explain_runtime(&input)?;
                        println!(
                            "\n{}",
                            serde_json::to_string_pretty(&serde_json::json!({
                                "runtime": runtime.id,
                                "explain": explain_report,
                            }))?
                        );
                    }
                }
            }
        }
        return Ok(());
    }

    println!("Agent Doctor — doctor\n");

    if let Some(path) = &report.profile_env_path {
        let status = if report.profile_env_exists {
            "found"
        } else {
            "missing"
        };
        println!("Company profile: {status} ({path})");
    } else {
        println!("Company profile: unknown config directory");
    }

    if let Some(active) = &report.active_preset {
        println!("Active preset: {active}");
    } else {
        println!("Active preset: (none) — run `agent-doctor profile init`");
    }

    if let Some(url) = &report.company_gateway_url {
        println!("Company gateway (profile.env): {url}");
    } else if report.profile_env_exists {
        println!("Company profile: found but gateway URL missing");
    }

    println!();
    for runtime in &report.runtimes {
        let status = if runtime.installed {
            "installed"
        } else {
            "not installed"
        };
        println!("{} ({}) — {status}", runtime.display_name, runtime.id);
        if let Some(version) = &runtime.version {
            println!("  version: {version}");
        }
        if let Some(path) = &runtime.binary_path {
            println!("  binary: {path}");
        }
        for config_path in &runtime.config_paths {
            println!("  config: {config_path}");
        }
        if let Some(url) = &runtime.profile.gateway_url {
            println!("  gateway: {url}");
        }
        println!();
    }

    if explain {
        let probes = probe_all_runtimes();
        for runtime in &report.runtimes {
            if let Some(probe) = probes.iter().find(|item| item.runtime_id == runtime.id) {
                if probe_issue_score(probe) > 0 {
                    let input = build_explain_input(&runtime.id, probe, None);
                    let explain_report = explain_runtime(&input)?;
                    println!("---");
                    print_explain_report(&runtime.id, &explain_report);
                    println!();
                }
            }
        }
    }

    Ok(())
}
