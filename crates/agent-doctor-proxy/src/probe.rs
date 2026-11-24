//! Does a proxy port answer?
//!
//! A `~/.zshrc` line keeps pointing at `127.0.0.1:7890` after the proxy app is
//! closed. Handing that to a child would break every request, including the
//! ones that work direct, so a closed port is skipped.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const CONNECT_TIMEOUT: Duration = Duration::from_millis(300);
const CACHE_TTL: Duration = Duration::from_secs(5);

static CACHE: Mutex<Option<HashMap<SocketAddr, (Instant, bool)>>> = Mutex::new(None);

/// Only literal IPs and `localhost` are dialed. A host name is trusted as
/// written: resolving it could block on DNS for seconds.
pub(crate) fn alive(url: &str) -> bool {
    let Some(addr) = socket_addr(url) else {
        return true;
    };
    let mut guard = CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let cache = guard.get_or_insert_with(HashMap::new);
    if let Some((at, open)) = cache.get(&addr) {
        if at.elapsed() < CACHE_TTL {
            return *open;
        }
    }
    let open = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT).is_ok();
    cache.insert(addr, (Instant::now(), open));
    open
}

pub(crate) fn socket_addr(url: &str) -> Option<SocketAddr> {
    let (scheme, rest) = match url.split_once("://") {
        Some((scheme, rest)) => (scheme.to_ascii_lowercase(), rest),
        None => ("http".to_string(), url),
    };
    let authority = rest.split('/').next().unwrap_or(rest);
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, hp)| hp);
    let (host, port) = split_host_port(host_port)?;
    let port = match port {
        Some(port) => port,
        None => match scheme.as_str() {
            "https" => 443,
            "socks5" | "socks5h" | "socks4" | "socks" => 1080,
            _ => 80,
        },
    };
    let ip = if host.eq_ignore_ascii_case("localhost") {
        IpAddr::V4(Ipv4Addr::LOCALHOST)
    } else {
        host.parse::<IpAddr>().ok()?
    };
    Some(SocketAddr::new(ip, port))
}

fn split_host_port(text: &str) -> Option<(&str, Option<u16>)> {
    if let Some(rest) = text.strip_prefix('[') {
        let (host, tail) = rest.split_once(']')?;
        let port = match tail.strip_prefix(':') {
            Some(port) => Some(port.parse().ok()?),
            None => None,
        };
        return Some((host, port));
    }
    match text.rsplit_once(':') {
        Some((host, port)) if !host.contains(':') => Some((host, Some(port.parse().ok()?))),
        _ => Some((text, None)),
    }
}
