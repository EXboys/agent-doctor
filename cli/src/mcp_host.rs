//! Windows entry Claude, Codex, and the other agents start for MCP.
//!
//! Those programs spawn the configured command themselves. The real CLI is a
//! console-subsystem executable. When its parent was started with
//! `CREATE_NO_WINDOW` (no console of its own), Windows allocates a new terminal
//! for every console child. That is the window that appears on each message.
//! This binary is a GUI-subsystem image, so Windows does not allocate one.
//! It then starts the real CLI with `CREATE_NO_WINDOW` and the same stdin,
//! stdout, and stderr pipes. The MCP server, its arguments, and the protocol
//! stay in the CLI.

#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() {
    #[cfg(windows)]
    {
        let code = relay();
        std::process::exit(code);
    }
    #[cfg(not(windows))]
    {
        eprintln!("agent-doctor-mcp is the Windows MCP launcher");
        std::process::exit(1);
    }
}

#[cfg(windows)]
fn relay() -> i32 {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};

    /// Same flag the desktop uses for its own hidden children. Must not be
    /// combined with `DETACHED_PROCESS`: Windows ignores `CREATE_NO_WINDOW`
    /// when both are set.
    const CREATE_NO_WINDOW: u32 = 0x08000000;

    let Ok(exe) = std::env::current_exe() else {
        return 1;
    };
    let Some(dir) = exe.parent() else {
        return 1;
    };
    let Some(real) = ["agent-doctor.exe", "agent-doctor-cli.exe"]
        .into_iter()
        .map(|name| dir.join(name))
        .find(|path| path.is_file())
    else {
        eprintln!("Agent Doctor CLI is not next to the MCP launcher");
        return 1;
    };

    let mut command = Command::new(real);
    command
        .args(std::env::args_os().skip(1))
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .creation_flags(CREATE_NO_WINDOW);
    match command.status() {
        Ok(status) => status.code().unwrap_or(1),
        Err(err) => {
            eprintln!("Agent Doctor MCP launcher could not start the CLI: {err}");
            1
        }
    }
}
