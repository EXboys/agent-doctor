use agent_doctor_core::{open_interactive_session, OpenSessionOptions};
use anyhow::Result;
use std::path::PathBuf;

pub fn run(
    runtime: &str,
    cwd: Option<&str>,
    prompt: Option<&str>,
    terminal: bool,
    json: bool,
) -> Result<()> {
    let report = open_interactive_session(&OpenSessionOptions {
        runtime: runtime.to_string(),
        cwd: cwd.map(PathBuf::from),
        prompt: prompt.map(str::to_string),
        prefer_deep_link: !terminal,
    })?;

    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    println!("Agent Doctor — open interactive session");
    println!();
    println!("Runtime: {}", report.runtime);
    println!("Method:  {}", method_label(&report.method));
    println!("Cwd:     {}", report.cwd);
    println!("Target:  {}", report.target);
    println!("{}", report.detail);
    Ok(())
}

fn method_label(method: &agent_doctor_core::OpenSessionMethod) -> &'static str {
    match method {
        agent_doctor_core::OpenSessionMethod::DeepLink => "deep-link",
        agent_doctor_core::OpenSessionMethod::Terminal => "terminal",
    }
}
