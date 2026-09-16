//! Flat JSON persistence for preferences and the resume session, in the same
//! file and the same shape electron/store.js used. Writes are debounced and
//! atomic (temp file + fsync + rename), so a crash mid-save cannot leave a
//! truncated settings file behind.

use crate::columns::Columns;
use crate::model::{LabelStyle, LabelStyles, TorrentRecord};
use crate::settings::{SECRET_KEYS, Settings, SettingsPatch};
use serde_json::{Map, Value};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub const STATE_FILE: &str = "ztorrent-state.json";
const DEBOUNCE: Duration = Duration::from_millis(400);

/// Seals and opens secrets. The app's implementation is the operating system's
/// credential store; tests use a stand-in; with none at all, secrets stay in
/// plain text rather than being lost.
pub trait SecretCodec: Send {
    fn encrypt(&self, plain: &str) -> anyhow::Result<String>;
    fn decrypt(&self, sealed: &str) -> anyhow::Result<String>;
}

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct WindowBounds {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub y: Option<f64>,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Debug, Default)]
pub struct StateData {
    pub settings: Settings,
    pub torrents: Vec<TorrentRecord>,
    pub labels: Vec<String>,
    /// Absent for labels left as they came.
    pub label_styles: LabelStyles,
    pub columns: Option<Columns>,
    pub window: Option<WindowBounds>,
    /// Top-level keys this build does not know.
    pub extra: Map<String, Value>,
}

pub struct Store {
    pub dir: PathBuf,
    pub file: PathBuf,
    tmp: PathBuf,
    pub legacy_file: Option<PathBuf>,
    pub data: StateData,
    secrets: Option<Box<dyn SecretCodec>>,
    dirty_since: Option<Instant>,
}

impl Store {
    pub fn open(dir: impl Into<PathBuf>, secrets: Option<Box<dyn SecretCodec>>) -> Store {
        let dir = dir.into();
        let file = dir.join(STATE_FILE);
        let tmp = dir.join(format!("{STATE_FILE}.tmp"));
        let legacy_file = find_legacy(&dir, &file);
        let mut store = Store { dir, file, tmp, legacy_file, data: StateData::default(), secrets, dirty_since: None };
        store.data = store.read();
        store
    }

    fn read(&self) -> StateData {
        let source = self.legacy_file.as_deref().unwrap_or(&self.file);
        let parsed: Value = fs::read_to_string(source)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .filter(Value::is_object)
            .unwrap_or_else(|| Value::Object(Map::new()));
        let Value::Object(mut obj) = parsed else { unreachable!() };

        let mut settings = Settings::from_json(obj.get("settings").unwrap_or(&Value::Null));
        // Alternate limits are a "right now" decision, not a preference: every
        // launch starts at full speed. The two rates themselves do persist.
        settings.alt_speed_enabled = false;
        self.open_secrets(&mut settings);

        let torrents = match obj.remove("torrents") {
            Some(Value::Array(rows)) => rows.into_iter().filter_map(|r| serde_json::from_value(r).ok()).collect(),
            _ => Vec::new(),
        };
        let labels = match obj.remove("labels") {
            Some(Value::Array(a)) => a.into_iter().filter_map(|v| v.as_str().map(String::from)).collect(),
            _ => Vec::new(),
        };
        let label_styles = match obj.remove("labelStyles") {
            Some(Value::Object(m)) => m
                .into_iter()
                .filter_map(|(k, v)| serde_json::from_value::<LabelStyle>(v).ok().map(|s| (k, s)))
                .collect(),
            _ => LabelStyles::new(),
        };
        let columns = obj.remove("columns").and_then(|v| serde_json::from_value(v).ok());
        let window = obj.remove("window").and_then(|v| serde_json::from_value(v).ok());
        obj.remove("settings");
        StateData { settings, torrents, labels, label_styles, columns, window, extra: obj }
    }

    /// Opens sealed secrets. A refused or missing codec leaves the value empty in
    /// memory but holds the sealed copy, so the next save writes it back rather
    /// than wiping it.
    fn open_secrets(&self, settings: &mut Settings) {
        for key in SECRET_KEYS {
            debug_assert_eq!(key, "proxyPassword");
            let Some(sealed) = settings.proxy_password_enc.clone() else { continue };
            match self.secrets.as_ref().and_then(|c| c.decrypt(&sealed).ok()) {
                Some(opened) => {
                    settings.proxy_password = opened;
                    settings.proxy_password_enc = None;
                }
                None => settings.proxy_password = String::new(),
            }
        }
    }

    pub fn settings(&self) -> &Settings {
        &self.data.settings
    }

    /// An explicit new value for a secret replaces whatever was sealed before,
    /// including when it is cleared to ''.
    pub fn patch_settings(&mut self, patch: &SettingsPatch) -> &Settings {
        let mut next = self.data.settings.patched(patch);
        if SECRET_KEYS.iter().any(|k| patch.contains_key(*k)) {
            next.proxy_password_enc = None;
        }
        self.data.settings = next;
        self.save();
        &self.data.settings
    }

    /// Marks the file for a debounced write.
    pub fn save(&mut self) {
        if self.dirty_since.is_none() {
            self.dirty_since = Some(Instant::now());
        }
    }

    /// Writes if a save has been waiting long enough. Call from a loop.
    pub fn flush_if_due(&mut self) {
        if self.dirty_since.is_some_and(|t| t.elapsed() >= DEBOUNCE) {
            self.flush();
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty_since.is_some()
    }

    /// The on-disk shape: secrets sealed and never written under their own name,
    /// the session-only switch not written at all.
    pub fn serialise(&self) -> Value {
        let mut settings = match serde_json::to_value(&self.data.settings) {
            Ok(Value::Object(m)) => m,
            _ => Map::new(),
        };
        settings.remove("altSpeedEnabled");
        if let Some(codec) = &self.secrets {
            for key in SECRET_KEYS {
                let value = settings.remove(key).and_then(|v| v.as_str().map(String::from)).unwrap_or_default();
                if value.is_empty() {
                    continue; // empty, or a sealed value we could not open (kept under <key>Enc)
                }
                if let Ok(sealed) = codec.encrypt(&value) {
                    settings.insert(format!("{key}Enc"), Value::String(sealed));
                }
            }
        }

        let mut out = Map::new();
        out.insert("settings".into(), Value::Object(settings));
        out.insert("torrents".into(), serde_json::to_value(&self.data.torrents).unwrap_or(Value::Array(vec![])));
        out.insert("labels".into(), serde_json::to_value(&self.data.labels).unwrap_or(Value::Array(vec![])));
        out.insert("labelStyles".into(), serde_json::to_value(&self.data.label_styles).unwrap_or_default());
        out.insert("columns".into(), serde_json::to_value(&self.data.columns).unwrap_or(Value::Null));
        out.insert("window".into(), serde_json::to_value(&self.data.window).unwrap_or(Value::Null));
        for (k, v) in &self.data.extra {
            out.entry(k.clone()).or_insert_with(|| v.clone());
        }
        Value::Object(out)
    }

    pub fn flush(&mut self) {
        self.dirty_since = None;
        if let Err(err) = self.write_atomically() {
            eprintln!("[store] save failed: {err}");
        } else {
            // From here on the file of our own is the one to read.
            self.legacy_file = None;
        }
    }

    fn write_atomically(&self) -> std::io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        let text = serde_json::to_string_pretty(&self.serialise()).map_err(std::io::Error::other)?;
        {
            let mut f = open_private(&self.tmp)?;
            f.write_all(text.as_bytes())?;
            f.sync_all()?;
        }
        fs::rename(&self.tmp, &self.file)
    }
}

/// The app used to be called Murmur. With no state of our own yet, adopt the
/// old library rather than starting empty; the old file is left untouched.
fn find_legacy(dir: &Path, current: &Path) -> Option<PathBuf> {
    if current.exists() {
        return None;
    }
    let mut candidates = vec![dir.join("murmur-state.json")];
    if let Some(parent) = dir.parent() {
        candidates.push(parent.join("Murmur").join("murmur-state.json"));
    }
    candidates.into_iter().find(|p| p.exists())
}

#[cfg(unix)]
fn open_private(path: &Path) -> std::io::Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path)
}

#[cfg(not(unix))]
fn open_private(path: &Path) -> std::io::Result<fs::File> {
    fs::File::create(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::patch;

    struct Fake;
    impl SecretCodec for Fake {
        fn encrypt(&self, v: &str) -> anyhow::Result<String> {
            Ok(format!("ENC({v})"))
        }
        fn decrypt(&self, s: &str) -> anyhow::Result<String> {
            s.strip_prefix("ENC(").and_then(|s| s.strip_suffix(')')).map(String::from).ok_or_else(|| anyhow::anyhow!("bad"))
        }
    }
    struct Denied;
    impl SecretCodec for Denied {
        fn encrypt(&self, v: &str) -> anyhow::Result<String> {
            Fake.encrypt(v)
        }
        fn decrypt(&self, _: &str) -> anyhow::Result<String> {
            anyhow::bail!("denied")
        }
    }

    fn on_disk(dir: &Path) -> Value {
        serde_json::from_str(&fs::read_to_string(dir.join(STATE_FILE)).unwrap()).unwrap()
    }

    /// Ported from scripts/test-secrets.mjs, assertion for assertion.
    #[test]
    fn secrets_are_sealed_and_survive_a_denied_keychain() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path(), Some(Box::new(Fake)));
        store.patch_settings(&{
            let mut p = patch("proxyEnabled", true);
            p.insert("proxyHost".into(), "p.example".into());
            p.insert("proxyPassword".into(), "hunter2".into());
            p
        });
        store.flush();
        let disk = on_disk(dir.path());
        assert!(disk["settings"].get("proxyPassword").is_none(), "plaintext password not written");
        assert_eq!(disk["settings"]["proxyPasswordEnc"], "ENC(hunter2)");
        assert_eq!(fs::read_to_string(dir.path().join(STATE_FILE)).unwrap().matches("hunter2").count(), 1);

        let store = Store::open(dir.path(), Some(Box::new(Fake)));
        assert_eq!(store.settings().proxy_password, "hunter2", "round-trips on reload");
        assert!(store.settings().proxy_password_enc.is_none(), "sealed key not left in memory");
        assert_eq!(store.settings().proxy_host, "p.example");

        let mut denied = Store::open(dir.path(), Some(Box::new(Denied)));
        assert_eq!(denied.settings().proxy_password, "", "unreadable password reads as empty");
        denied.patch_settings(&patch("maxUploadRate", 50));
        denied.flush();
        assert_eq!(on_disk(dir.path())["settings"]["proxyPasswordEnc"], "ENC(hunter2)", "sealed value survived");
        assert_eq!(Store::open(dir.path(), Some(Box::new(Fake))).settings().proxy_password, "hunter2");

        let mut clearing = Store::open(dir.path(), Some(Box::new(Fake)));
        clearing.patch_settings(&patch("proxyPassword", ""));
        clearing.flush();
        let after = on_disk(dir.path());
        assert!(after["settings"].get("proxyPasswordEnc").is_none(), "sealed value removed");
        assert!(after["settings"].get("proxyPassword").is_none(), "and not written in the clear");

        let mut plain = Store::open(dir.path(), None);
        plain.patch_settings(&patch("proxyPassword", "fallback"));
        plain.flush();
        assert_eq!(on_disk(dir.path())["settings"]["proxyPassword"], "fallback");
    }

    #[test]
    fn alternate_speed_is_session_only_and_file_is_private() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path(), None);
        store.patch_settings(&patch("altSpeedEnabled", true));
        assert!(store.settings().alt_speed_enabled);
        store.flush();
        assert!(on_disk(dir.path())["settings"].get("altSpeedEnabled").is_none());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(dir.path().join(STATE_FILE)).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn adopts_murmur_and_tolerates_corruption() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("murmur-state.json"), r#"{"labels":["old"],"settings":{"theme":"graphite"}}"#).unwrap();
        let mut store = Store::open(dir.path(), None);
        assert_eq!(store.data.labels, vec!["old".to_string()]);
        assert!(store.settings().is_dark());
        store.flush();
        assert!(dir.path().join("murmur-state.json").exists(), "old file left alone");
        assert!(dir.path().join(STATE_FILE).exists());

        let broken = tempfile::tempdir().unwrap();
        fs::write(broken.path().join(STATE_FILE), "{ not json").unwrap();
        let store = Store::open(broken.path(), None);
        assert!(store.data.torrents.is_empty());
        assert_eq!(store.settings().global_max_connections, 200);
    }

    #[test]
    fn unknown_top_level_keys_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(STATE_FILE), r#"{"settings":{},"torrents":[],"someday":{"a":1}}"#).unwrap();
        let mut store = Store::open(dir.path(), None);
        store.flush();
        assert_eq!(on_disk(dir.path())["someday"]["a"], 1);
    }
}
