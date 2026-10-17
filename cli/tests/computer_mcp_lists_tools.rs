//! The binary this release builds must actually serve the computer tools.
//!
//! A unit test of the source would still pass if the `mcp computer` command
//! were dropped from the CLI. This starts that command and reads `tools/list`.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

#[test]
fn computer_mcp_lists_the_screen_tools() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_agent-doctor"))
        .args(["mcp", "computer"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn mcp computer");
    let mut stdin = child.stdin.take().expect("stdin");
    let stdout = child.stdout.take().expect("stdout");
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"2024-11-05","capabilities":{{}},"clientInfo":{{"name":"preflight","version":"0"}}}}}}"#
    )
    .unwrap();
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","method":"notifications/initialized"}}"#
    )
    .unwrap();
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{{}}}}"#
    )
    .unwrap();
    drop(stdin);

    let mut listed = String::new();
    for line in BufReader::new(stdout).lines() {
        let line = line.unwrap();
        if line.contains("screen_snapshot") {
            listed = line;
            break;
        }
    }
    let _ = child.kill();
    let _ = child.wait();

    for name in [
        "screen_apps",
        "screen_open",
        "screen_snapshot",
        "screen_click",
        "screen_type",
        "screen_press",
        "screen_scroll",
        "screen_windows",
        "screen_state",
        "screen_set",
        "screen_menu",
    ] {
        assert!(
            listed.contains(name),
            "{name} missing from tools/list: {listed}"
        );
    }
}
