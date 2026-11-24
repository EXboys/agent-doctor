//! The OS system proxy: what V2rayU, ClashX or Clash Verge set when "system
//! proxy" is on. Browsers follow it; CLIs read only variables, so it is turned
//! into variables here.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostPort {
    pub host: String,
    pub port: u16,
}

impl HostPort {
    pub fn url(&self) -> String {
        if self.host.contains(':') && !self.host.starts_with('[') {
            format!("http://[{}]:{}", self.host, self.port)
        } else {
            format!("http://{}:{}", self.host, self.port)
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SystemProxy {
    pub http: Option<HostPort>,
    pub https: Option<HostPort>,
    pub socks: Option<HostPort>,
    /// A PAC script decides per URL. It has no single address to hand on.
    pub pac: bool,
    /// Already in `NO_PROXY` form.
    pub exceptions: Vec<String>,
}

impl SystemProxy {
    fn is_empty(&self) -> bool {
        self.http.is_none() && self.https.is_none() && self.socks.is_none() && !self.pac
    }
}

/// `*.local` becomes `.local`; `169.254/16` becomes `169.254.0.0/16`;
/// `<local>` (Windows: plain host names) has no `NO_PROXY` form and is dropped.
pub(crate) fn no_proxy_entry(raw: &str) -> Option<String> {
    let entry = raw.trim();
    if entry.is_empty() || entry.eq_ignore_ascii_case("<local>") {
        return None;
    }
    if let Some(rest) = entry.strip_prefix("*.") {
        return Some(format!(".{rest}"));
    }
    if let Some((net, bits)) = entry.split_once('/') {
        let octets: Vec<&str> = net.split('.').collect();
        if !octets.is_empty()
            && octets.len() < 4
            && octets.iter().all(|octet| octet.parse::<u8>().is_ok())
        {
            let mut full: Vec<&str> = octets.clone();
            full.resize(4, "0");
            return Some(format!("{}/{bits}", full.join(".")));
        }
    }
    Some(entry.to_string())
}

#[cfg(target_os = "macos")]
pub(crate) fn read() -> Option<SystemProxy> {
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    // macOS system tool, stdin closed, output captured. `try_wait` remembers
    // the exit status, so the following `wait_with_output` still reads stdout.
    let mut child = Command::new("/usr/sbin/scutil") // raw-spawn-ok
        .arg("--proxy")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let output = child.wait_with_output().ok()?;
    if !output.status.success() {
        return None;
    }
    parse_scutil(&String::from_utf8_lossy(&output.stdout))
}

/// Parse `scutil --proxy`.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn parse_scutil(text: &str) -> Option<SystemProxy> {
    use std::collections::HashMap;

    let mut fields: HashMap<String, String> = HashMap::new();
    let mut exceptions = Vec::new();
    let mut in_exceptions = false;
    for line in text.lines() {
        let line = line.trim();
        if in_exceptions {
            if line.starts_with('}') {
                in_exceptions = false;
            } else if let Some((_, value)) = line.split_once(" : ") {
                if let Some(entry) = no_proxy_entry(value) {
                    exceptions.push(entry);
                }
            }
            continue;
        }
        let Some((key, value)) = line.split_once(" : ") else {
            continue;
        };
        if key == "ExceptionsList" {
            in_exceptions = value.trim().ends_with('{');
            continue;
        }
        fields.insert(key.trim().to_string(), value.trim().to_string());
    }

    let enabled = |key: &str| fields.get(key).is_some_and(|value| value == "1");
    let host_port = |enable: &str, host: &str, port: &str| -> Option<HostPort> {
        if !enabled(enable) {
            return None;
        }
        let host = fields.get(host)?.trim().to_string();
        let port = fields.get(port)?.trim().parse().ok()?;
        (!host.is_empty() && port != 0).then_some(HostPort { host, port })
    };
    let proxy = SystemProxy {
        http: host_port("HTTPEnable", "HTTPProxy", "HTTPPort"),
        https: host_port("HTTPSEnable", "HTTPSProxy", "HTTPSPort"),
        socks: host_port("SOCKSEnable", "SOCKSProxy", "SOCKSPort"),
        pac: enabled("ProxyAutoConfigEnable"),
        exceptions,
    };
    (!proxy.is_empty()).then_some(proxy)
}

#[cfg(windows)]
pub(crate) fn read() -> Option<SystemProxy> {
    let key = windows_registry::CURRENT_USER
        .open(r"Software\Microsoft\Windows\CurrentVersion\Internet Settings")
        .ok()?;
    let enabled = key.get_u32("ProxyEnable").unwrap_or(0) != 0;
    let server = if enabled {
        key.get_string("ProxyServer").unwrap_or_default()
    } else {
        String::new()
    };
    let bypass = key.get_string("ProxyOverride").unwrap_or_default();
    let pac = key
        .get_string("AutoConfigURL")
        .is_ok_and(|url| !url.trim().is_empty());
    parse_windows(&server, &bypass, pac)
}

/// `ProxyServer` is either `host:port` for every scheme, or
/// `http=h:p;https=h:p;socks=h:p`.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn parse_windows(server: &str, bypass: &str, pac: bool) -> Option<SystemProxy> {
    let mut proxy = SystemProxy {
        pac,
        exceptions: bypass.split(';').filter_map(no_proxy_entry).collect(),
        ..SystemProxy::default()
    };
    let server = server.trim();
    if !server.is_empty() {
        if server.contains('=') {
            for part in server.split(';') {
                let Some((scheme, address)) = part.split_once('=') else {
                    continue;
                };
                let address = parse_host_port(address);
                match scheme.trim().to_ascii_lowercase().as_str() {
                    "http" => proxy.http = address,
                    "https" => proxy.https = address,
                    "socks" => proxy.socks = address,
                    _ => {}
                }
            }
        } else {
            let address = parse_host_port(server);
            proxy.http = address.clone();
            proxy.https = address;
        }
    }
    (!proxy.is_empty()).then_some(proxy)
}

#[cfg_attr(not(windows), allow(dead_code))]
fn parse_host_port(text: &str) -> Option<HostPort> {
    let text = text.trim();
    let text = text.split_once("://").map_or(text, |(_, rest)| rest);
    let (host, port) = text.rsplit_once(':')?;
    let port: u16 = port.trim_end_matches('/').parse().ok()?;
    let host = host.trim_start_matches('[').trim_end_matches(']');
    (!host.is_empty() && port != 0).then(|| HostPort {
        host: host.to_string(),
        port,
    })
}

#[cfg(not(any(target_os = "macos", windows)))]
pub(crate) fn read() -> Option<SystemProxy> {
    None
}
