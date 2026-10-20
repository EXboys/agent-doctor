//! Shared secret for the local computer-hand channel (`screen_click` and
//! friends). The desktop app listens on a per-user socket / named pipe, and
//! the computer-use MCP tool connects to it for every request. Path
//! permissions and the default pipe ACL only separate OS users, so both
//! sides also require the token in this file before a request is answered —
//! which also closes the Windows pipe against remote (SMB) clients that
//! would otherwise only face the default DACL.

use std::path::Path;

use anyhow::{Context, Result};

const TOKEN_BYTES: usize = 16; // 128-bit → 32 hex chars

pub fn computer_hand_token_path() -> Result<std::path::PathBuf> {
    let dir = crate::store::agent_doctor_config_dir()
        .context("could not resolve agent-doctor config dir")?;
    Ok(dir.join("computer-hand-token"))
}

/// Hand server side: read the token, creating (or re-creating, when the file
/// is missing or malformed) one before the channel opens.
pub fn load_or_create() -> Result<String> {
    let path = computer_hand_token_path()?;
    load_or_create_at(&path)
}

/// Hand client side: read the token without creating it. The desktop app
/// owns this file; a missing or malformed file means the hand is not up yet.
pub fn load() -> Result<String> {
    let path = computer_hand_token_path()?;
    load_at(&path)
}

fn load_or_create_at(path: &Path) -> Result<String> {
    if let Some(token) = read_valid(path) {
        return Ok(token);
    }
    let token = new_token()?;
    super::secrets::atomic_write_private(path, token.as_bytes()).with_context(|| {
        format!(
            "failed to store the computer hand token at {}",
            path.display()
        )
    })?;
    Ok(token)
}

fn load_at(path: &Path) -> Result<String> {
    read_valid(path).ok_or_else(|| {
        anyhow::anyhow!(
            "no readable computer hand token at {} — the app has not started the hand yet",
            path.display()
        )
    })
}

fn read_valid(path: &Path) -> Option<String> {
    let raw = std::fs::read_to_string(path).ok()?;
    let token = raw.trim();
    if token.len() == TOKEN_BYTES * 2 && token.bytes().all(|b| b.is_ascii_hexdigit()) {
        Some(token.to_ascii_lowercase())
    } else {
        None
    }
}

fn new_token() -> Result<String> {
    let mut bytes = [0u8; TOKEN_BYTES];
    getrandom::fill(&mut bytes).context("could not read OS randomness")?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// Length-checked, constant-time-ish compare so reply timing does not leak
/// how much of the token matched.
pub fn tokens_match(expected: &str, got: &str) -> bool {
    let expected = expected.as_bytes();
    let got = got.as_bytes();
    if expected.len() != got.len() {
        return false;
    }
    expected
        .iter()
        .zip(got)
        .fold(0u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn creates_once_and_reuses() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("computer-hand-token");
        let first = load_or_create_at(&path).unwrap();
        assert_eq!(first.len(), TOKEN_BYTES * 2);
        assert!(first.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(load_or_create_at(&path).unwrap(), first);
        assert_eq!(load_at(&path).unwrap(), first);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn malformed_token_is_regenerated() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("computer-hand-token");
        std::fs::write(&path, "not-a-token\n").unwrap();
        let token = load_or_create_at(&path).unwrap();
        assert_eq!(token.len(), TOKEN_BYTES * 2);
        assert_eq!(load_at(&path).unwrap(), token);
    }

    #[test]
    fn client_load_fails_without_a_token_file() {
        let dir = tempdir().unwrap();
        assert!(load_at(&dir.path().join("absent")).is_err());
    }

    #[test]
    fn token_compare_rejects_wrong_and_unequal_lengths() {
        assert!(tokens_match("abcd", "abcd"));
        assert!(!tokens_match("abcd", "abce"));
        assert!(!tokens_match("abcd", "abc"));
        assert!(!tokens_match("abcd", ""));
    }
}
