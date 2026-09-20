//! The engine's own HTTP (fetching a .torrent by URL) under the egress policy,
//! and the interface list. Everything that carries our address outward goes the
//! way the policy says or not at all.

use std::net::IpAddr;
use std::time::Duration;
use ztorrent_core::NetInterface;
use ztorrent_core::egress::EgressPolicy;

/// A .torrent is read into memory; anything larger is not one anybody meant.
pub const MAX_TORRENT_BYTES: u64 = 16 * 1024 * 1024;

/// Every non-internal IPv4 interface, for the Preferences dropdown.
pub fn list_interfaces() -> Vec<NetInterface> {
    let mut out: Vec<NetInterface> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter(|i| !i.is_loopback() && i.ip().is_ipv4())
        .map(|i| NetInterface { name: i.name.clone(), address: i.ip().to_string() })
        .collect();
    out.dedup();
    out
}

/// Current address of a bound interface, re-read rather than remembered: a VPN
/// that reconnects usually comes back on a different one. None means it is
/// gone, which is the signal to stop connecting entirely.
pub fn bind_address(name: &str) -> Option<IpAddr> {
    if_addrs::get_if_addrs()
        .ok()?
        .into_iter()
        .find(|i| i.name == name && i.ip().is_ipv4() && !i.is_loopback())
        .map(|i| i.ip())
}

pub fn interface_gone(name: &str) -> String {
    format!("bound interface {name} has no address -- refusing to connect")
}

fn encode_userinfo(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// A client that dials under the policy. Built per request, so the bound
/// address is read at the moment of the request, never cached.
pub fn client(policy: Option<&EgressPolicy>, version: &str) -> Result<reqwest::blocking::Client, String> {
    // No system or environment proxy: only the one the user set in ztorrent.
    let mut b = reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(30))
        .user_agent(format!("ztorrent/{version}"));
    if let Some(p) = policy {
        if let Some(name) = &p.bind {
            match bind_address(name) {
                Some(ip) => b = b.local_address(ip),
                None => return Err(interface_gone(name)),
            }
        }
        if let Some(px) = &p.proxy {
            let auth = match &px.username {
                Some(u) => format!("{}:{}@", encode_userinfo(u), encode_userinfo(&px.password)),
                None => String::new(),
            };
            // socks5h: the proxy resolves the name, so no DNS lookup leaks locally.
            let url = format!("socks5h://{auth}{}:{}", px.host, px.port);
            b = b.proxy(reqwest::Proxy::all(url).map_err(|e| e.to_string())?);
        }
    }
    b.build().map_err(|e| e.to_string())
}

pub fn fetch_torrent(url: &str, policy: Option<&EgressPolicy>, version: &str) -> Result<Vec<u8>, String> {
    let client = client(policy, version)?;
    let res = client.get(url).send().map_err(describe)?;
    if !res.status().is_success() {
        return Err(format!("HTTP {}", res.status().as_u16()));
    }
    if res.content_length().is_some_and(|n| n > MAX_TORRENT_BYTES) {
        return Err("the file is too large to be a torrent".into());
    }
    use std::io::Read;
    let mut buf = Vec::new();
    res.take(MAX_TORRENT_BYTES + 1).read_to_end(&mut buf).map_err(|e| e.to_string())?;
    if buf.len() as u64 > MAX_TORRENT_BYTES {
        return Err("the file is too large to be a torrent".into());
    }
    Ok(buf)
}

fn describe(err: reqwest::Error) -> String {
    use std::error::Error;
    let mut msg = err.to_string();
    let mut src = err.source();
    while let Some(s) = src {
        msg = format!("{msg}: {s}");
        src = s.source();
    }
    msg
}
