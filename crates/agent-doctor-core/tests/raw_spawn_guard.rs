//! Every `std::process::Command::new` outside `exec::process` must carry a
//! `// raw-spawn-ok` marker (or live in a `#[cfg(test)]` module).
//!
//! A GUI app that spawns a console-subsystem child without
//! `DETACHED_PROCESS` / `CREATE_NO_WINDOW` paints a terminal window per call.
//! That bug class shipped four times between 0.1.87 and 0.1.90; this test is
//! the fence. Route new spawns through `crate::exec::run_output` (captures)
//! or `crate::exec::command_for_path` (pipes / long-lived). The marker is for
//! intentional windows (session_launch terminals) and platform tools that
//! cannot start on the other platform.

use std::path::{Path, PathBuf};

const MARKER: &str = "raw-spawn-ok";

#[test]
fn raw_command_spawn_is_confined_or_marked() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // core plus the prompt crate that was split out of it: prompt_session
    // spawns more agent children than anywhere else in the workspace.
    let sources = [
        manifest.join("src"),
        manifest
            .join("../agent-doctor-prompt/src")
            .canonicalize()
            .expect("agent-doctor-prompt/src next to agent-doctor-core"),
    ];
    let mut offenders = Vec::new();
    for src in &sources {
        visit(src, &mut offenders);
    }
    assert!(
        offenders.is_empty(),
        "raw Command::new without '{MARKER}'. Route it through \
         crate::exec::run_output / command_for_path, or mark the line with \
         '// {MARKER}' when the window is intentional:\n{}",
        offenders.join("\n")
    );
}

fn visit(dir: &Path, offenders: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            visit(&path, offenders);
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        // Test-only code does not ship.
        let body = match text.find("#[cfg(test)]") {
            Some(index) => &text[..index],
            None => text.as_str(),
        };
        let is_process_module = path.ends_with("exec/process.rs");
        let mut previous = "";
        for (number, line) in body.lines().enumerate() {
            // A marker comment survives rustfmt reflow when it sits on its own
            // line above the spawn; it only covers that one following line.
            let covers_next = previous.trim_start().starts_with("//") && previous.contains(MARKER);
            let marked = line.contains(MARKER) || covers_next;
            if line.contains("Command::new(") && !marked && !is_process_module {
                offenders.push(format!("{}:{}", path.display(), number + 1));
            }
            previous = line;
        }
    }
}
