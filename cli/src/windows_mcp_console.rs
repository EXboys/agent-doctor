//! Close a console Windows already allocated for an MCP server.
//!
//! The GUI launcher in `mcp_host.rs` is what agents should start, so no console
//! is allocated. Config written by an older build still starts `agent-doctor.exe`
//! directly. That process is console-subsystem: the window exists before `main`.
//! MCP stdio is a pipe, and the console is an extra window. Detach it. A person
//! running `agent-doctor mcp browser` in a terminal keeps that terminal, because
//! stdout there is the console rather than a pipe.

pub fn release_mcp_server_console() {
    let mcp_server = args_are_mcp_stdio_server(std::env::args());
    #[cfg(windows)]
    if mcp_server && stdout_is_pipe() && has_console() {
        free_console();
    }
    #[cfg(not(windows))]
    let _ = mcp_server;
}

pub fn args_are_mcp_stdio_server<I, S>(args: I) -> bool
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut args = args.into_iter();
    let _argv0 = args.next();
    let Some(command) = args.next() else {
        return false;
    };
    if !command.as_ref().eq_ignore_ascii_case("mcp") {
        return false;
    }
    let Some(action) = args.next() else {
        return false;
    };
    matches!(
        action.as_ref().to_ascii_lowercase().as_str(),
        "computer" | "browser"
    )
}

#[cfg(windows)]
fn stdout_is_pipe() -> bool {
    use std::ffi::c_void;
    const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;
    const FILE_TYPE_PIPE: u32 = 3;
    unsafe extern "system" {
        fn GetStdHandle(n_std_handle: u32) -> *mut c_void;
        fn GetFileType(file: *mut c_void) -> u32;
    }
    unsafe {
        let handle = GetStdHandle(STD_OUTPUT_HANDLE);
        if handle.is_null() || handle as isize == -1 {
            return false;
        }
        GetFileType(handle) == FILE_TYPE_PIPE
    }
}

#[cfg(windows)]
fn free_console() {
    unsafe extern "system" {
        fn FreeConsole() -> i32;
    }
    unsafe {
        FreeConsole();
    }
}

#[cfg(windows)]
fn has_console() -> bool {
    use std::ffi::c_void;
    unsafe extern "system" {
        fn GetConsoleWindow() -> *mut c_void;
    }
    unsafe { !GetConsoleWindow().is_null() }
}

#[cfg(test)]
mod tests {
    use super::args_are_mcp_stdio_server;

    #[test]
    fn mcp_servers_are_the_only_commands_that_release_a_console() {
        assert!(args_are_mcp_stdio_server([
            "agent-doctor.exe",
            "mcp",
            "computer"
        ]));
        assert!(args_are_mcp_stdio_server([
            "agent-doctor.exe",
            "mcp",
            "browser",
            "--port",
            "9222"
        ]));
        assert!(!args_are_mcp_stdio_server([
            "agent-doctor.exe",
            "mcp",
            "status"
        ]));
        assert!(!args_are_mcp_stdio_server([
            "agent-doctor.exe",
            "mcp",
            "configure",
            "claude"
        ]));
        assert!(!args_are_mcp_stdio_server(["agent-doctor.exe", "doctor"]));
        assert!(!args_are_mcp_stdio_server(["agent-doctor.exe"]));
    }
}
