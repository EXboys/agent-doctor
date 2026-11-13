//! Every `std::process::Command::new` outside `exec::process` must carry a
//! `// raw-spawn-ok` marker (or live in a `#[cfg(test)]` module).
//!
//! A GUI app that spawns a console-subsystem child without
//! `DETACHED_PROCESS` / `CREATE_NO_WINDOW` paints a terminal window per call.
//! That bug class shipped four times between 0.1.87 and 0.1.90; this test is
//! the fence. Route new spawns through `crate::exec::run_output` (captures)
//! or `crate::exec::command_for_path` (pipes / long-lived). The marker is for
//! intentional windows (session_launch terminals), platform tools that
//! cannot start on the other platform, console-app parents whose children
//! inherit the console, and GUI-subsystem targets (browsers, `open`).

use std::path::{Path, PathBuf};

const MARKER: &str = "raw-spawn-ok";

#[test]
fn raw_command_spawn_is_confined_or_marked() {
    let mut offenders = Vec::new();
    for src in &guarded_sources() {
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

/// A creation flag decides whether a child AND its whole subtree get a
/// console: `CREATE_NEW_CONSOLE` only for intentional visible terminals in
/// `session_launch/terminal.rs`, `CREATE_NO_WINDOW` everywhere else, and
/// `DETACHED_PROCESS` nowhere (a detached parent has no console, so any
/// console child it spawns — `codex.js -> codex.exe`, `cmd -> powershell` —
/// opens a visible black terminal window).
#[test]
fn creation_flags_stay_in_their_sanctioned_files() {
    const FLAG_HOMES: &[(&str, &str)] = &[
        ("DETACHED_PROCESS", "__never__"),
        ("CREATE_NEW_CONSOLE", "session_launch/terminal.rs"),
    ];
    let mut offenders = Vec::new();
    for src in &guarded_sources() {
        flag_visit(src, FLAG_HOMES, &mut offenders);
    }
    assert!(
        offenders.is_empty(),
        "creation flag outside its sanctioned file — see \
         docs/zh-CN/windows-spawn-and-runtime-env.md for the flag table:\n{}",
        offenders.join("\n")
    );
}

fn guarded_sources() -> Vec<PathBuf> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // core plus the prompt crate that was split out of it: prompt_session
    // spawns more agent children than anywhere else in the workspace. The
    // mcp crate and the CLI run console-less under the desktop app, and
    // src-tauri is the GUI app itself — all three painted stray terminals.
    vec![
        manifest.join("src"),
        manifest
            .join("../agent-doctor-prompt/src")
            .canonicalize()
            .expect("agent-doctor-prompt/src next to agent-doctor-core"),
        manifest
            .join("../agent-doctor-repair/src")
            .canonicalize()
            .expect("agent-doctor-repair/src next to agent-doctor-core"),
        manifest
            .join("../agent-doctor-mcp/src")
            .canonicalize()
            .expect("agent-doctor-mcp/src next to agent-doctor-core"),
        manifest
            .join("../../cli/src")
            .canonicalize()
            .expect("cli/src at the workspace root"),
        manifest
            .join("../../desktop/src-tauri/src")
            .canonicalize()
            .expect("desktop/src-tauri/src at the workspace root"),
    ]
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

fn flag_visit(dir: &Path, flag_homes: &[(&str, &str)], offenders: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            flag_visit(&path, flag_homes, offenders);
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
        for (number, line) in body.lines().enumerate() {
            // Doc comments may name a flag when explaining the rule; only
            // code lines count.
            if line.trim_start().starts_with("//") {
                continue;
            }
            for (flag, home) in flag_homes {
                if line.contains(flag) && !path.ends_with(home) {
                    offenders.push(format!("{}:{} uses {flag}", path.display(), number + 1));
                }
            }
        }
    }
}
