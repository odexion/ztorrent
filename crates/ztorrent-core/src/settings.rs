//! Preferences. Field for field with DEFAULT_SETTINGS in electron/store.js, and
//! serialised under the same camelCase names.

use crate::lenient;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    #[serde(deserialize_with = "lenient::string")]
    pub download_path: String,
    /// Where the last torrent was actually saved. The Add sheet offers this
    /// instead of download_path; cleared when download_path changes.
    #[serde(deserialize_with = "lenient::string")]
    pub last_save_path: String,
    #[serde(deserialize_with = "lenient::bool")]
    pub ask_where_to_save: bool,
    #[serde(deserialize_with = "lenient::bool")]
    pub start_torrents_automatically: bool,
    #[serde(deserialize_with = "lenient::bool")]
    pub sequential_download: bool,
    /// Write to <name>.part until the torrent completes.
    #[serde(deserialize_with = "lenient::bool")]
    pub part_files: bool,

    // Bandwidth, kB/s; 0 = unlimited
    #[serde(deserialize_with = "lenient::u64")]
    pub max_download_rate: u64,
    #[serde(deserialize_with = "lenient::u64")]
    pub max_upload_rate: u64,
    #[serde(deserialize_with = "lenient::u64")]
    pub global_max_connections: u64,
    #[serde(deserialize_with = "lenient::u64")]
    pub max_upload_slots: u64,

    // Connection
    /// 0 = random
    #[serde(deserialize_with = "lenient::u64")]
    pub listen_port: u64,
    #[serde(deserialize_with = "lenient::bool")]
    pub randomize_port: bool,
    #[serde(rename = "enableDHT", deserialize_with = "lenient::bool")]
    pub enable_dht: bool,
    #[serde(rename = "enablePEX", deserialize_with = "lenient::bool")]
    pub enable_pex: bool,
    /// Off by default: LSD multicasts each infohash in the clear to the LAN.
    #[serde(rename = "enableLSD", deserialize_with = "lenient::bool")]
    pub enable_lsd: bool,
    #[serde(rename = "enableUPnP", deserialize_with = "lenient::bool")]
    pub enable_upnp: bool,
    #[serde(rename = "enableUTP", deserialize_with = "lenient::bool")]
    pub enable_utp: bool,
    /// 0 = off, 1 = prefer MSE (plaintext fallback), 2 = require MSE
    #[serde(deserialize_with = "lenient::u64")]
    pub encryption: u64,

    // Proxy. SOCKS5 only, and it takes DHT, LSD, uTP, UPnP and udp:// trackers
    // down with it -- none of them can be routed, so they are not sent direct.
    #[serde(deserialize_with = "lenient::bool")]
    pub proxy_enabled: bool,
    #[serde(deserialize_with = "lenient::string")]
    pub proxy_host: String,
    #[serde(deserialize_with = "lenient::u64")]
    pub proxy_port: u64,
    #[serde(deserialize_with = "lenient::string")]
    pub proxy_username: String,
    /// In memory only. On disk it is sealed under proxyPasswordEnc; see Store.
    #[serde(deserialize_with = "lenient::string")]
    pub proxy_password: String,
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "lenient::opt_string")]
    pub proxy_password_enc: Option<String>,
    /// '' = any; otherwise an interface name, e.g. 'utun4'
    #[serde(deserialize_with = "lenient::string")]
    pub bind_interface: String,

    // Queueing
    #[serde(deserialize_with = "lenient::u64")]
    pub max_active_torrents: u64,
    #[serde(deserialize_with = "lenient::u64")]
    pub max_active_downloads: u64,
    /// percent; 0 = seed forever
    #[serde(deserialize_with = "lenient::u64")]
    pub seed_ratio_limit: u64,
    /// minutes; 0 = forever
    #[serde(deserialize_with = "lenient::u64")]
    pub seed_time_limit: u64,

    /// Checks GitHub for a newer release and stages it. Off means no calls at all.
    #[serde(deserialize_with = "lenient::bool")]
    pub auto_update: bool,

    // UI
    /// "classic" (light) or "graphite" (dark)
    #[serde(deserialize_with = "lenient::string")]
    pub theme: String,
    #[serde(deserialize_with = "lenient::bool")]
    pub confirm_on_delete: bool,
    #[serde(deserialize_with = "lenient::bool")]
    pub notify_on_complete: bool,
    #[serde(deserialize_with = "lenient::bool")]
    pub show_speed_in_dock: bool,
    /// Session only -- never restored from disk and never written to it.
    #[serde(deserialize_with = "lenient::bool")]
    pub alt_speed_enabled: bool,
    #[serde(deserialize_with = "lenient::u64")]
    pub alt_download_rate: u64,
    #[serde(deserialize_with = "lenient::u64")]
    pub alt_upload_rate: u64,

    /// Keys a newer build wrote. Carried through so nothing is lost.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Default for Settings {
    fn default() -> Self {
        let downloads = dirs::home_dir().unwrap_or_default().join("Downloads");
        Settings {
            download_path: downloads.to_string_lossy().into_owned(),
            last_save_path: String::new(),
            ask_where_to_save: true,
            start_torrents_automatically: true,
            sequential_download: false,
            part_files: true,
            max_download_rate: 0,
            max_upload_rate: 0,
            global_max_connections: 200,
            max_upload_slots: 4,
            listen_port: 0,
            randomize_port: true,
            enable_dht: true,
            enable_pex: true,
            enable_lsd: false,
            enable_upnp: true,
            enable_utp: true,
            encryption: 1,
            proxy_enabled: false,
            proxy_host: String::new(),
            proxy_port: 1080,
            proxy_username: String::new(),
            proxy_password: String::new(),
            proxy_password_enc: None,
            bind_interface: String::new(),
            max_active_torrents: 8,
            max_active_downloads: 5,
            seed_ratio_limit: 0,
            seed_time_limit: 0,
            auto_update: true,
            theme: "classic".into(),
            confirm_on_delete: true,
            notify_on_complete: true,
            show_speed_in_dock: true,
            alt_speed_enabled: false,
            alt_download_rate: 100,
            alt_upload_rate: 20,
            extra: Map::new(),
        }
    }
}

/// A partial update, as Preferences sends it: camelCase key -> JSON value.
pub type SettingsPatch = Map<String, Value>;

/// Settings that must not be written to disk in the clear.
pub const SECRET_KEYS: [&str; 1] = ["proxyPassword"];

impl Settings {
    /// `{ ...DEFAULT_SETTINGS, ...parsed }`: every key the file has overrides the
    /// default, and a key whose value this build cannot read keeps the default
    /// rather than costing every other setting.
    pub fn from_json(value: &Value) -> Settings {
        let mut merged = serde_json::to_value(Settings::default()).expect("defaults serialise");
        if let Value::Object(obj) = value {
            if let Value::Object(base) = &mut merged {
                for (k, v) in obj {
                    base.insert(k.clone(), v.clone());
                }
            }
            if let Ok(s) = serde_json::from_value::<Settings>(merged.clone()) {
                return s;
            }
            // Something did not parse: take the keys one at a time.
            let mut good = serde_json::to_value(Settings::default()).expect("defaults serialise");
            for (k, v) in obj {
                let mut trial = good.clone();
                trial[k.as_str()] = v.clone();
                if serde_json::from_value::<Settings>(trial.clone()).is_ok() {
                    good = trial;
                }
            }
            return serde_json::from_value(good).unwrap_or_default();
        }
        Settings::default()
    }

    /// Returns a copy with the patch applied, under the same lenient rules.
    pub fn patched(&self, patch: &SettingsPatch) -> Settings {
        let mut value = serde_json::to_value(self).expect("settings serialise");
        if let Value::Object(map) = &mut value {
            for (k, v) in patch {
                map.insert(k.clone(), v.clone());
            }
        }
        Settings::from_json(&value)
    }

    /// The two rates in force, in bytes/s, with 0 meaning unlimited. Alternate
    /// mode swaps in its own pair. Every place that sets a throttle goes through
    /// here, so alternate mode cannot be forgotten by one of them.
    pub fn throttle_rates(&self) -> (u64, u64) {
        let (dn, up) = if self.alt_speed_enabled {
            (self.alt_download_rate, self.alt_upload_rate)
        } else {
            (self.max_download_rate, self.max_upload_rate)
        };
        (dn.saturating_mul(1024), up.saturating_mul(1024))
    }

    pub fn is_dark(&self) -> bool {
        self.theme == "graphite"
    }

    /// The folder the Add sheet should offer.
    pub fn offered_save_path(&self) -> &str {
        if self.last_save_path.is_empty() { &self.download_path } else { &self.last_save_path }
    }
}

/// Preferences sends every field on every save, so only an actual change of the
/// default download folder counts -- and when it changes, that deliberate choice
/// wins over wherever the last torrent happened to go.
pub fn normalise_patch(current: &Settings, patch: &mut SettingsPatch) {
    if let Some(Value::String(dl)) = patch.get("downloadPath") {
        if !patch.contains_key("lastSavePath") && *dl != current.download_path {
            patch.insert("lastSavePath".into(), Value::String(String::new()));
        }
    }
}

/// Builds a one-key patch.
pub fn patch(key: &str, value: impl Into<Value>) -> SettingsPatch {
    let mut m = Map::new();
    m.insert(key.into(), value.into());
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn defaults_fill_what_the_file_lacks() {
        let s = Settings::from_json(&json!({ "maxUploadRate": 50, "theme": "graphite" }));
        assert_eq!(s.max_upload_rate, 50);
        assert!(s.is_dark());
        assert_eq!(s.global_max_connections, 200);
        assert!(!s.enable_lsd);
    }

    #[test]
    fn unknown_keys_survive_and_bad_values_fall_back() {
        let s = Settings::from_json(&json!({ "futureThing": [1, 2], "encryption": "2", "enableDHT": 0 }));
        assert_eq!(s.extra["futureThing"], json!([1, 2]));
        assert_eq!(s.encryption, 2);
        assert!(!s.enable_dht);
        let out = serde_json::to_value(&s).unwrap();
        assert_eq!(out["futureThing"], json!([1, 2]));
        assert!(out.get("enableDHT").is_some(), "uses the Electron key spelling");
    }

    #[test]
    fn throttle_uses_alternate_pair_and_zero_means_unlimited() {
        let mut s = Settings { max_download_rate: 0, max_upload_rate: 10, ..Default::default() };
        assert_eq!(s.throttle_rates(), (0, 10 * 1024));
        s.alt_speed_enabled = true;
        assert_eq!(s.throttle_rates(), (100 * 1024, 20 * 1024));
    }

    #[test]
    fn changing_download_path_clears_last_save_path() {
        let s = Settings { download_path: "/a".into(), last_save_path: "/b".into(), ..Default::default() };
        let mut same = patch("downloadPath", "/a");
        normalise_patch(&s, &mut same);
        assert!(!same.contains_key("lastSavePath"));
        let mut moved = patch("downloadPath", "/c");
        normalise_patch(&s, &mut moved);
        assert_eq!(s.patched(&moved).last_save_path, "");
    }
}
