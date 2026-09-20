//! Turns preferences and the egress policy into a libtorrent session config.
//!
//! Some preferences apply at once and some only at the next start -- the
//! discovery switches and the whole egress policy. The session is always built
//! from the start-up copy of the latter, so saving Preferences can never route
//! traffic differently until the log has said it will.

use ztorrent_core::Settings;
use ztorrent_core::egress::EgressPolicy;
use ztorrent_lt_sys::SessionConfig;

pub fn session_config(restart: &Settings, live: &Settings, policy: Option<&EgressPolicy>, version: &str) -> SessionConfig {
    let guarded = EgressPolicy::guarded(policy);
    let port = if restart.randomize_port { 0 } else { restart.listen_port.min(65535) };
    let (dn, up) = live.throttle_rates();

    let (listen, outgoing) = match policy {
        // Under a proxy nothing should reach us directly, so there are no
        // listeners at all.
        Some(p) if p.proxy.is_some() => (String::new(), p.bind.clone().unwrap_or_default()),
        Some(p) => {
            let name = p.bind.clone().unwrap_or_default();
            (format!("{name}:{port}"), name)
        }
        None => (format!("0.0.0.0:{port},[::]:{port}"), String::new()),
    };

    let (proxy_host, proxy_port, proxy_username, proxy_password) = match policy.and_then(|p| p.proxy.as_ref()) {
        Some(px) => (px.host.clone(), px.port, px.username.clone().unwrap_or_default(), px.password.clone()),
        None => (String::new(), 0, String::new(), String::new()),
    };

    SessionConfig {
        listen_interfaces: listen,
        outgoing_interfaces: outgoing,
        enable_dht: restart.enable_dht && !guarded.dht,
        enable_lsd: restart.enable_lsd && !guarded.lsd,
        enable_upnp: restart.enable_upnp && !guarded.upnp,
        enable_utp: restart.enable_utp && !guarded.utp,
        enable_incoming_tcp: !guarded.inbound,
        encryption: live.encryption.min(2) as u8,
        proxy_host,
        proxy_port,
        proxy_username,
        proxy_password,
        download_rate: dn.min(i32::MAX as u64) as i32,
        upload_rate: up.min(i32::MAX as u64) as i32,
        connections_limit: live.global_max_connections.clamp(10, i32::MAX as u64) as i32,
        user_agent: format!("ztorrent/{version}"),
        peer_fingerprint: fingerprint(version),
    }
}

/// Azureus-style peer id prefix: -ZT0500- for 0.5.0.
pub fn fingerprint(version: &str) -> String {
    let digit = |s: Option<&str>| {
        s.and_then(|p| p.chars().take_while(char::is_ascii_digit).collect::<String>().parse::<u32>().ok())
            .map(|n| std::char::from_digit(n.min(35), 36).unwrap_or('0').to_ascii_uppercase())
            .unwrap_or('0')
    };
    let mut parts = version.split('.');
    let (a, b, c) = (digit(parts.next()), digit(parts.next()), digit(parts.next()));
    format!("-ZT{a}{b}{c}0-")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proxied() -> Settings {
        Settings {
            proxy_enabled: true,
            proxy_host: "127.0.0.1".into(),
            proxy_port: 1080,
            proxy_username: "zaf".into(),
            proxy_password: "hunter2".into(),
            ..Default::default()
        }
    }

    #[test]
    fn a_proxy_closes_listeners_and_everything_unroutable() {
        let s = proxied();
        let policy = EgressPolicy::from_settings(&s);
        let c = session_config(&s, &s, policy.as_ref(), "0.5.0");
        assert_eq!(c.listen_interfaces, "");
        assert!(!c.enable_dht && !c.enable_lsd && !c.enable_upnp && !c.enable_utp && !c.enable_incoming_tcp);
        assert_eq!((c.proxy_host.as_str(), c.proxy_port, c.proxy_username.as_str()), ("127.0.0.1", 1080, "zaf"));
    }

    #[test]
    fn a_bind_keeps_dht_and_pins_everything_to_the_interface() {
        let s = Settings { bind_interface: "utun4".into(), listen_port: 6881, randomize_port: false, ..Default::default() };
        let policy = EgressPolicy::from_settings(&s);
        let c = session_config(&s, &s, policy.as_ref(), "0.5.0");
        assert_eq!(c.listen_interfaces, "utun4:6881");
        assert_eq!(c.outgoing_interfaces, "utun4");
        assert!(c.enable_dht && !c.enable_lsd && !c.enable_utp && !c.enable_upnp && c.enable_incoming_tcp);
        assert_eq!(c.proxy_host, "");
    }

    #[test]
    fn restart_only_fields_come_from_the_start_up_copy() {
        let start = Settings::default();
        let mut live = proxied();
        live.enable_dht = false;
        live.max_upload_rate = 50;
        let c = session_config(&start, &live, None, "0.5.0");
        assert!(c.enable_dht, "discovery waits for a restart");
        assert_eq!(c.proxy_host, "", "so does the proxy");
        assert_eq!(c.upload_rate, 50 * 1024, "rates apply at once");
        assert_eq!(fingerprint("0.5.0-alpha.1"), "-ZT0500-");
    }
}
