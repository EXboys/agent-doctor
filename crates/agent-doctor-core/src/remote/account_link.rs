//! Same-account path choice: LAN beacon first, otherwise SSH to port 22.
//!
//! The account site only learns the address. Project files and chat stay on
//! the SSH tunnel into the other computer's loopback client. Sharing to the
//! internet is off until this computer turns it on. This computer does not
//! open the router.

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};

use super::bootstrap::generate_ed25519_key;
use super::registry::{remote_keys_dir, remote_root_dir, RemoteHostEntry};

const MARK_BEGIN: &str = "# agent-doctor-account";
const MARK_END: &str = "# agent-doctor-account end";
const CLIENT_ID_KEY: &str = "teamups_channel_client_id";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountRoute {
    /// The LAN beacon is already announcing this account computer.
    SameLan { lan_id: String },
    /// SSH to the public address the other computer opted in to publish.
    PublicSsh { host: String, user: String },
    /// Same account, but it is not on this LAN and has not shared a public address.
    Unavailable,
}

static CACHED_CLIENT_ID: Mutex<Option<(Instant, String)>> = Mutex::new(None);

pub fn is_account_id(value: &str) -> bool {
    let n = value.len();
    (1..=64).contains(&n)
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
}

pub fn is_ssh_user(value: &str) -> bool {
    let n = value.len();
    (1..=32).contains(&n)
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '.' || ch == '_' || ch == '-')
}

/// OpenSSH ed25519 public key, type and key only, no comment.
pub fn is_ssh_ed25519_pub(value: &str) -> bool {
    let mut parts = value.split_whitespace();
    let kind = parts.next();
    let key = parts.next();
    if parts.next().is_some() {
        return false;
    }
    kind == Some("ssh-ed25519")
        && key.is_some_and(|key| {
            (40..=120).contains(&key.len())
                && key
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '+' || ch == '/' || ch == '=')
        })
}

/// An address another network can open. Private, loopback, link-local, and
/// carrier-grade NAT addresses are not that.
pub fn is_public_ipv4(value: &str) -> bool {
    let mut octets = [0u8; 4];
    let mut count = 0;
    for part in value.split('.') {
        if count == 4 {
            return false;
        }
        let Ok(octet) = part.parse::<u8>() else {
            return false;
        };
        if part.len() > 1 && part.starts_with('0') {
            return false;
        }
        octets[count] = octet;
        count += 1;
    }
    if count != 4 {
        return false;
    }
    let [a, b, c, _] = octets;
    if a == 0 || a == 10 || a == 127 || a >= 224 {
        return false;
    }
    if a == 169 && b == 254 {
        return false;
    }
    if a == 172 && (16..=31).contains(&b) {
        return false;
    }
    if a == 192 && (b == 168 || (b == 0 && c == 2)) {
        return false;
    }
    if a == 100 && (64..=127).contains(&b) {
        return false;
    }
    if a == 198 && (b == 18 || b == 19) {
        return false;
    }
    true
}

/// `nearby` is `(lan beacon id, account computer id)` from computers we can
/// already see on this network.
pub fn choose_account_route(
    peer_id: &str,
    wan: bool,
    public_ip: &str,
    ssh_user: &str,
    nearby: &[(&str, &str)],
) -> AccountRoute {
    if !is_account_id(peer_id) {
        return AccountRoute::Unavailable;
    }
    if let Some((lan_id, _)) = nearby
        .iter()
        .find(|(lan_id, account_id)| is_account_id(lan_id) && *account_id == peer_id)
    {
        return AccountRoute::SameLan {
            lan_id: (*lan_id).to_string(),
        };
    }
    if wan && is_public_ipv4(public_ip) && is_ssh_user(ssh_user) {
        return AccountRoute::PublicSsh {
            host: public_ip.to_string(),
            user: ssh_user.to_string(),
        };
    }
    AccountRoute::Unavailable
}

pub fn account_host_id(client_id: &str) -> Option<String> {
    is_account_id(client_id).then(|| format!("acct-{client_id}"))
}

fn wan_share_path() -> Option<PathBuf> {
    remote_root_dir().map(|dir| dir.join("wan-share"))
}

/// Missing preference means this computer is not shared outside the LAN.
pub fn wan_share_enabled() -> bool {
    wan_share_path()
        .and_then(|path| fs::read_to_string(path).ok())
        .is_some_and(|text| text.trim() == "1")
}

pub fn set_wan_share(enabled: bool) {
    let Some(path) = wan_share_path() else {
        return;
    };
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let _ = fs::write(path, if enabled { "1" } else { "0" });
}

/// Account computer id already saved for the channel. Empty when this
/// computer has not signed in. Does not create a new id.
pub fn saved_account_client_id() -> String {
    if let Ok(guard) = CACHED_CLIENT_ID.lock() {
        if let Some((seen, id)) = guard.as_ref() {
            if seen.elapsed() < Duration::from_secs(10) {
                return id.clone();
            }
        }
    }
    let id = read_account_client_id();
    if let Ok(mut guard) = CACHED_CLIENT_ID.lock() {
        *guard = Some((Instant::now(), id.clone()));
    }
    id
}

fn read_account_client_id() -> String {
    let Ok(store) = crate::store::open_settings_store() else {
        return String::new();
    };
    store
        .meta_get(CLIENT_ID_KEY)
        .ok()
        .flatten()
        .map(|value| value.trim().to_string())
        .filter(|value| is_account_id(value))
        .unwrap_or_default()
}

pub fn account_ssh_user() -> String {
    let raw = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_default();
    let name = raw.trim();
    if is_ssh_user(name) {
        name.to_string()
    } else {
        String::new()
    }
}

fn account_key_path() -> Result<PathBuf> {
    let Some(dir) = remote_keys_dir() else {
        bail!("找不到这台电脑的配置目录。");
    };
    Ok(dir.join("account-link"))
}

/// Private key this computer uses to reach another computer on the account.
pub fn account_identity_file() -> Result<PathBuf> {
    let path = account_key_path()?;
    let pub_path = path.with_extension("pub");
    if path.is_file() && pub_path.is_file() {
        return Ok(path);
    }
    if path.exists() {
        let _ = fs::remove_file(&path);
    }
    if pub_path.exists() {
        let _ = fs::remove_file(&pub_path);
    }
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    }
    generate_ed25519_key(&path, "account")?;
    Ok(path)
}

pub fn account_ssh_public_key() -> Result<String> {
    let path = account_identity_file()?;
    let text =
        fs::read_to_string(path.with_extension("pub")).context("读不到这台电脑的登录钥匙。")?;
    let line = text
        .split_whitespace()
        .take(2)
        .collect::<Vec<_>>()
        .join(" ");
    if !is_ssh_ed25519_pub(&line) {
        bail!("这台电脑的登录钥匙不可用。");
    }
    Ok(line)
}

pub fn public_ssh_entry(host: &str, user: &str) -> Result<RemoteHostEntry> {
    if !is_public_ipv4(host) || !is_ssh_user(user) {
        bail!("这台电脑还不能从外网连接。");
    }
    let key = account_identity_file()?;
    Ok(RemoteHostEntry {
        ssh_config_host: String::new(),
        hostname: Some(host.to_string()),
        user: Some(user.to_string()),
        port: Some(22),
        identity_file: Some(key.display().to_string()),
        display_name: String::new(),
        linked_computer_name: String::new(),
        linked_client: false,
        projects: std::collections::BTreeMap::new(),
    })
}

fn authorized_keys_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    Some(PathBuf::from(home).join(".ssh").join("authorized_keys"))
}

pub(crate) fn render_authorized_block(existing: &str, keys: &[(String, String)]) -> String {
    let mut kept = Vec::new();
    let mut skipping = false;
    for line in existing.lines() {
        let trimmed = line.trim();
        if trimmed == MARK_BEGIN {
            skipping = true;
            continue;
        }
        if skipping {
            if trimmed == MARK_END {
                skipping = false;
            }
            continue;
        }
        kept.push(line);
    }
    while kept.last().is_some_and(|line| line.trim().is_empty()) {
        kept.pop();
    }
    let mut out = String::new();
    if !kept.is_empty() {
        out.push_str(&kept.join("\n"));
        out.push('\n');
    }
    let usable: Vec<(&str, &str)> = keys
        .iter()
        .filter(|(id, key)| is_account_id(id) && is_ssh_ed25519_pub(key))
        .map(|(id, key)| (id.as_str(), key.as_str()))
        .collect();
    if usable.is_empty() {
        return out;
    }
    if !out.is_empty() {
        out.push('\n');
    }
    out.push_str(MARK_BEGIN);
    out.push('\n');
    for (id, key) in usable {
        out.push_str(key);
        out.push(' ');
        out.push_str("agent-doctor-account-");
        out.push_str(id);
        out.push('\n');
    }
    out.push_str(MARK_END);
    out.push('\n');
    out
}

/// Replace this app's block in `~/.ssh/authorized_keys` with the account
/// computers' public keys. Other lines in the file stay.
pub fn sync_account_authorized_keys(keys: &[(String, String)]) -> Result<()> {
    let Some(path) = authorized_keys_path() else {
        bail!("找不到这台电脑的用户目录。");
    };
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).context("没能准备这台电脑的登录钥匙。")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o700));
        }
    }
    let existing = fs::read_to_string(&path).unwrap_or_default();
    let next = render_authorized_block(&existing, keys);
    if existing == next {
        return Ok(());
    }
    fs::write(&path, &next).context("没能把同一账号的钥匙放进这台电脑。")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PUB: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAa0examplekeyvaluepadding0000000000000";

    #[test]
    fn public_addresses_skip_lan_and_carrier_nat() {
        assert!(is_public_ipv4("1.2.3.4"));
        assert!(is_public_ipv4("8.8.8.8"));
        assert!(!is_public_ipv4("192.168.1.8"));
        assert!(!is_public_ipv4("10.0.0.2"));
        assert!(!is_public_ipv4("172.16.0.1"));
        assert!(!is_public_ipv4("127.0.0.1"));
        assert!(!is_public_ipv4("100.64.0.1"));
        assert!(!is_public_ipv4("169.254.1.1"));
        assert!(!is_public_ipv4("0.1.2.3"));
        assert!(!is_public_ipv4("1.2.3"));
        assert!(!is_public_ipv4("01.2.3.4"));
    }

    #[test]
    fn same_account_on_the_lan_does_not_use_the_public_address() {
        let nearby = [("lan-1", "ad-home")];
        assert_eq!(
            choose_account_route("ad-home", true, "1.2.3.4", "air", &nearby),
            AccountRoute::SameLan {
                lan_id: "lan-1".into()
            }
        );
    }

    #[test]
    fn a_shared_public_address_is_used_when_the_lan_beacon_is_absent() {
        assert_eq!(
            choose_account_route("ad-home", true, "1.2.3.4", "air", &[]),
            AccountRoute::PublicSsh {
                host: "1.2.3.4".into(),
                user: "air".into()
            }
        );
        assert_eq!(
            choose_account_route("ad-home", false, "1.2.3.4", "air", &[]),
            AccountRoute::Unavailable
        );
        assert_eq!(
            choose_account_route("ad-home", true, "192.168.1.8", "air", &[]),
            AccountRoute::Unavailable
        );
        assert_eq!(
            choose_account_route("ad-home", true, "1.2.3.4", "", &[]),
            AccountRoute::Unavailable
        );
    }

    #[test]
    fn the_account_block_replaces_itself_and_keeps_other_keys() {
        let existing = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIKeepThisKeyValuepadding000000000000000 mine\n# agent-doctor-account\nssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOldKeyValuepadding000000000000000000000 old\n# agent-doctor-account end\n";
        let next = render_authorized_block(existing, &[("ad-1".into(), PUB.into())]);
        assert!(next.contains("KeepThisKey"));
        assert!(!next.contains("OldKey"));
        assert!(next.contains(&format!("{PUB} agent-doctor-account-ad-1")));
        assert!(next.trim_end().ends_with(MARK_END));
        let cleared = render_authorized_block(&next, &[]);
        assert!(cleared.contains("KeepThisKey"));
        assert!(!cleared.contains("agent-doctor-account"));
    }

    #[test]
    fn a_key_with_a_comment_or_a_newline_is_not_installed() {
        assert!(is_ssh_ed25519_pub(PUB));
        assert!(!is_ssh_ed25519_pub(&format!("{PUB} comment")));
        assert!(!is_ssh_ed25519_pub(&format!("{PUB}\nssh-ed25519 AAAA")));
        let next = render_authorized_block("", &[("bad id".into(), PUB.into())]);
        assert!(next.is_empty());
    }
}
