//! Where traffic is allowed to leave from. The policy itself, and the rules
//! about what it switches off, live here so the engine that enforces them and
//! the Preferences sheet that greys them out cannot disagree.
//!
//! The rule throughout is fail-closed: anything that cannot be routed under a
//! policy is switched off rather than sent around it.

use crate::settings::Settings;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProxyConfig {
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: String,
}

/// Two policies, usable together: a SOCKS5 proxy, and every socket pinned to
/// one interface. `PartialEq` is what decides whether a settings change needs a
/// restart.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EgressPolicy {
    pub proxy: Option<ProxyConfig>,
    pub bind: Option<String>,
}

/// Reads the proxy out of settings, or None when it is off or incomplete.
pub fn proxy_config(s: &Settings) -> Option<ProxyConfig> {
    if !s.proxy_enabled {
        return None;
    }
    let host = s.proxy_host.trim();
    if host.is_empty() || s.proxy_port == 0 || s.proxy_port > u16::MAX as u64 {
        return None;
    }
    Some(ProxyConfig {
        host: host.to_string(),
        port: s.proxy_port as u16,
        username: (!s.proxy_username.is_empty()).then(|| s.proxy_username.clone()),
        password: if s.proxy_username.is_empty() { String::new() } else { s.proxy_password.clone() },
    })
}

impl EgressPolicy {
    pub fn from_settings(s: &Settings) -> Option<EgressPolicy> {
        let proxy = proxy_config(s);
        let bind = (!s.bind_interface.is_empty()).then(|| s.bind_interface.clone());
        (proxy.is_some() || bind.is_some()).then_some(EgressPolicy { proxy, bind })
    }

    /// What the policy forces off, whatever the checkboxes say.
    pub fn guarded(policy: Option<&EgressPolicy>) -> Suppressed {
        match policy {
            None => Suppressed::default(),
            Some(p) => Suppressed {
                // DHT survives a bind -- its socket can be pinned too -- but not a proxy.
                dht: p.proxy.is_some(),
                lsd: true,
                utp: true,
                upnp: true,
                udp_trackers: true,
                inbound: p.proxy.is_some(),
            },
        }
    }

    /// The human part of the start-up log line.
    pub fn describe(&self, bound_address: Option<&str>) -> String {
        let mut parts = Vec::new();
        if let Some(p) = &self.proxy {
            parts.push(format!(
                "SOCKS5 {}:{}{}",
                p.host,
                p.port,
                if p.username.is_some() { " (authenticated)" } else { "" }
            ));
        }
        if let Some(b) = &self.bind {
            parts.push(match bound_address {
                Some(a) => format!("bound to {b} ({a})"),
                None => format!("bound to {b} -- currently has no address"),
            });
        }
        parts.join(", ")
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Suppressed {
    pub dht: bool,
    pub lsd: bool,
    pub utp: bool,
    pub upnp: bool,
    pub udp_trackers: bool,
    pub inbound: bool,
}

/// What the Preferences sheet greys out while it is being edited: it follows the
/// boxes as ticked, not whether the proxy fields are complete yet.
pub fn suppressed_in_sheet(proxy_checked: bool, bind_interface: &str) -> Suppressed {
    let pinned = proxy_checked || !bind_interface.is_empty();
    Suppressed {
        dht: proxy_checked,
        lsd: pinned,
        utp: pinned,
        upnp: pinned,
        udp_trackers: pinned,
        inbound: proxy_checked,
    }
}

pub fn is_udp_tracker(url: &str) -> bool {
    url.len() >= 4 && url[..4].eq_ignore_ascii_case("udp:")
}

/// Announce lists filtered at the last moment before they are used.
pub fn filter_trackers<'a>(urls: impl IntoIterator<Item = &'a String>, policy: Option<&EgressPolicy>) -> Vec<String> {
    urls.into_iter()
        .filter(|u| policy.is_none() || !is_udp_tracker(u))
        .cloned()
        .collect()
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
    fn proxy_config_rejects_disabled_or_incomplete() {
        let s = proxied();
        assert!(proxy_config(&Settings { proxy_enabled: false, ..s.clone() }).is_none());
        assert!(proxy_config(&Settings { proxy_host: "  ".into(), ..s.clone() }).is_none());
        assert!(proxy_config(&Settings { proxy_port: 0, ..s.clone() }).is_none());
        assert_eq!(EgressPolicy::from_settings(&s), EgressPolicy::from_settings(&s.clone()));
        assert_ne!(
            EgressPolicy::from_settings(&s),
            EgressPolicy::from_settings(&Settings { proxy_port: 9999, ..s.clone() })
        );
        assert_eq!(EgressPolicy::from_settings(&Settings::default()), None);
    }

    #[test]
    fn guarded_disables_every_unroutable_channel() {
        let proxy = EgressPolicy::from_settings(&proxied());
        let g = EgressPolicy::guarded(proxy.as_ref());
        assert!(g.dht && g.lsd && g.utp && g.upnp && g.udp_trackers && g.inbound);
        let bind = EgressPolicy { proxy: None, bind: Some("utun4".into()) };
        let g = EgressPolicy::guarded(Some(&bind));
        assert!(!g.dht && g.lsd && g.utp && g.upnp && g.udp_trackers && !g.inbound);
        assert_eq!(EgressPolicy::guarded(None), Suppressed::default());
    }

    #[test]
    fn udp_trackers_are_stripped_only_under_a_policy() {
        let urls = vec![
            "udp://tracker.example:1337/announce".to_string(),
            "https://tracker.example/announce".to_string(),
            "UDP://SHOUTY.example:80".to_string(),
            "wss://tracker.example".to_string(),
        ];
        let bind = EgressPolicy { proxy: None, bind: Some("utun4".into()) };
        assert_eq!(
            filter_trackers(&urls, Some(&bind)),
            vec!["https://tracker.example/announce".to_string(), "wss://tracker.example".to_string()]
        );
        assert_eq!(filter_trackers(&urls, None).len(), 4);
    }
}
