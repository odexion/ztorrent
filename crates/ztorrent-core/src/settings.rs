//! Preferences. Field for field with DEFAULT_SETTINGS in electron/store.js, and
//! serialised under the same camelCase names.

use crate::lenient;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use crate::model::{SEARCH_SOURCES, SearchOptions};

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
    /// The torrent list as the old one-line table instead of two-line rows.
    #[serde(deserialize_with = "lenient::bool")]
    pub compact_list: bool,
    /// Session only -- never restored from disk and never written to it.
    #[serde(deserialize_with = "lenient::bool")]
    pub alt_speed_enabled: bool,
    #[serde(deserialize_with = "lenient::u64")]
    pub alt_download_rate: u64,
    #[serde(deserialize_with = "lenient::u64")]
    pub alt_upload_rate: u64,

    // Online search
    /// Comma-separated ids of the providers switched off, so a provider added
    /// later starts on.
    #[serde(deserialize_with = "lenient::string")]
    pub search_disabled_sources: String,
    /// One of the ids in `SEARCH_CATEGORIES`.
    #[serde(deserialize_with = "lenient::string")]
    pub search_category: String,
    /// Per provider.
    #[serde(deserialize_with = "lenient::u64")]
    pub search_max_results: u64,
    /// Leaves out hits nobody seeds, where the provider says.
    #[serde(deserialize_with = "lenient::bool")]
    pub search_hide_dead: bool,
    /// The user's own search sources.
    #[serde(deserialize_with = "lenient::list")]
    pub search_custom_sources: Vec<crate::model::CustomSource>,
    /// "jackett" or "prowlarr"
    #[serde(deserialize_with = "lenient::string")]
    pub search_indexer_kind: String,
    /// e.g. http://127.0.0.1:9117; empty = not set up
    #[serde(deserialize_with = "lenient::string")]
    pub search_indexer_url: String,
    /// In memory only. On disk it is sealed under searchIndexerKeyEnc; see Store.
    #[serde(deserialize_with = "lenient::string")]
    pub search_indexer_key: String,
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "lenient::opt_string")]
    pub search_indexer_key_enc: Option<String>,
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
            compact_list: false,
            alt_speed_enabled: false,
            alt_download_rate: 100,
            alt_upload_rate: 20,
            // Off until it is pointed at an indexer.
            search_disabled_sources: "indexer".into(),
            search_category: "all".into(),
            search_max_results: 40,
            search_hide_dead: true,
            search_custom_sources: Vec::new(),
            search_indexer_kind: "jackett".into(),
            search_indexer_url: String::new(),
            search_indexer_key: String::new(),
            search_indexer_key_enc: None,
            extra: Map::new(),
        }
    }
}

/// A partial update, as Preferences sends it: camelCase key -> JSON value.
pub type SettingsPatch = Map<String, Value>;

/// Settings that must not be written to disk in the clear.
pub const SECRET_KEYS: [&str; 2] = ["proxyPassword", "searchIndexerKey"];

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

    /// Switched on and set up: the indexer is left out of a search until it
    /// has an address, rather than failing every search.
    pub fn search_source_ready(&self, id: &str) -> bool {
        self.search_source_enabled(id) && (id != "indexer" || !self.search_indexer_url.trim().is_empty())
    }

    pub fn search_source_enabled(&self, id: &str) -> bool {
        !self.search_disabled_sources.split(',').any(|s| s.trim() == id)
    }

    /// The disabled list with one source switched on or off.
    pub fn search_sources_with(&self, id: &str, on: bool) -> String {
        let mut off: Vec<&str> = self.search_disabled_sources.split(',').map(str::trim).filter(|s| !s.is_empty() && *s != id).collect();
        if !on {
            off.push(id);
        }
        off.join(",")
    }

    pub fn search_options(&self) -> SearchOptions {
        SearchOptions {
            sources: SEARCH_SOURCES.iter().map(|(id, _)| *id).filter(|id| self.search_source_ready(id)).map(String::from).collect(),
            category: self.search_category.clone(),
            max_results: self.search_max_results.clamp(5, 200) as usize,
            hide_dead: self.search_hide_dead,
            indexer_kind: self.search_indexer_kind.clone(),
            indexer_url: self.search_indexer_url.trim().trim_end_matches('/').to_string(),
            indexer_key: self.search_indexer_key.trim().to_string(),
            custom: self.search_custom_sources.iter().filter(|c| c.enabled).cloned().collect(),
        }
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
    fn search_sources_toggle_by_id() {
        let s = Settings::default();
        assert!(s.search_source_enabled("archive"));
        let s = Settings { search_disabled_sources: s.search_sources_with("archive", false), ..s };
        assert!(!s.search_source_enabled("archive") && !s.search_source_enabled("indexer"));
        assert!(s.search_options().sources.is_empty(), "the indexer starts off");
        let on = Settings { search_disabled_sources: String::new(), ..Settings::default() };
        assert_eq!(on.search_options().sources, ["archive"], "the indexer waits for an address");
        let set_up = Settings { search_indexer_url: "http://127.0.0.1:9117".into(), ..on };
        assert_eq!(set_up.search_options().sources, ["archive", "indexer"]);
        assert_eq!(s.search_sources_with("archive", true), "indexer");
    }

    #[test]
    fn custom_sources_are_checked_and_shared_without_empty_fields() {
        use crate::model::CustomSource;
        let ok = CustomSource { name: "Mine".into(), url: "https://example.org/api?q={query}".into(), ..Default::default() };
        assert!(ok.check().is_ok());
        assert!(CustomSource { url: "https://example.org/api?q=x".into(), ..ok.clone() }.check().unwrap_err().contains("{query}"));
        assert!(CustomSource { url: "ftp://example.org/{query}".into(), ..ok.clone() }.check().is_err());
        assert!(CustomSource { url: "https://example.org{query}".into(), ..ok.clone() }.check().unwrap_err().contains("after the site"));
        assert!(CustomSource { name: " ".into(), ..ok.clone() }.check().is_err());
        let shared = ok.to_shared();
        assert!(!shared.contains("enabled") && !shared.contains("magnet"), "{shared}");
        assert_eq!(CustomSource::from_shared(&shared).unwrap(), ok);
        assert!(CustomSource::from_shared("not json").is_err());
        let answer = "Here you go:\n```json\n{\n  \"name\": \"Mine\",\n  \"url\": \"https://example.org/api?q={query}\"\n}\n```\nGet a key at example.org.";
        assert!(CustomSource::looks_shared(answer) && !CustomSource::looks_shared("https://example.org/api?q={query}"));
        assert_eq!(CustomSource::from_shared(answer).unwrap(), ok, "read out of an assistant's answer");
        let noisy = "Use {this}: {\"name\": \"Mine\", \"url\": \"https://example.org/api?q={query}\"} -- keys live under {settings}.";
        assert_eq!(CustomSource::from_shared(noisy).unwrap(), ok, "braces around it are not part of it");
    }

    #[test]
    fn the_assistant_prompt_asks_for_what_ztorrent_reads() {
        use crate::model::CustomSource;
        let prompt = CustomSource::assistant_prompt();
        assert!(prompt.ends_with("for the following website "), "the site's name is typed straight after it");
        assert!(prompt.contains("ONLY a JSON object") && prompt.contains("?q={query}"));
        for field in ["\"name\"", "\"url\"", "\"results\"", "\"items\"", "\"hash\"", "\"magnet\"", "\"trackers\"", "\"mirrors\"", "\"note\"", "\"error\""] {
            assert!(prompt.contains(field), "{field}");
        }
    }

    #[test]
    fn slips_in_an_assistants_json_are_put_right() {
        use crate::model::CustomSource;
        // As pasted from a real answer: \: is no JSON escape.
        let pasted = r#"{   "name": "Mine",   "url": "https\://example.org/q.php?q={query}&cat=0", "results": "0",   "title": "name",   "hash": " info_hash",   "size": "size", }"#;
        let s = CustomSource::from_shared(pasted).unwrap();
        assert_eq!((s.name.as_str(), s.url.as_str()), ("Mine", "https://example.org/q.php?q={query}&cat=0"));
        let curly = "{\u{201c}name\u{201d}: \u{201c}Mine\u{201d}, \u{201c}url\u{201d}: \u{201c}https://example.org/api?q={query}\u{201d}}";
        assert_eq!(CustomSource::from_shared(curly).unwrap().name, "Mine");
    }

    #[test]
    fn an_assistants_note_and_refusal_are_read() {
        use crate::model::CustomSource;
        let with_note = r#"{"name": "Mine", "url": "https://example.org/api?q={query}&key=KEY", "note": "Get a key at example.org/account."}"#;
        assert!(CustomSource::from_shared(with_note).is_ok());
        assert_eq!(CustomSource::shared_note(with_note).as_deref(), Some("Get a key at example.org/account."));
        let refused = CustomSource::from_shared(r#"{"error": "That site has no public API."}"#).unwrap_err();
        assert!(refused.contains("no public API"), "{refused}");
    }
    #[test]
    fn a_custom_source_this_build_cannot_read_is_dropped_alone() {
        let s = Settings::from_json(&json!({ "searchCustomSources": [
            { "name": "Mine", "url": "https://example.org/api?q={query}", "enabled": false },
            "garbage",
            { "name": "Other", "url": "https://example.org/s?q={query}" },
        ]}));
        assert_eq!(s.search_custom_sources.len(), 2);
        assert!(s.search_custom_sources[1].enabled, "on unless switched off");
        assert_eq!(s.search_options().custom.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["Other"]);
    }

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
