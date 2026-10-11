//! The shapes that cross between engine and window, and the few rules about
//! them that both sides must agree on -- what a state is called, which bar
//! colour it wears, which sidebar category it belongs to.

use crate::lenient;
use indexmap::IndexMap;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

/// Torrent lifecycle states, mirroring the vocabulary uTorrent shows in its
/// Status column. Serialised as the same lowercase words the state file uses.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Downloading,
    Seeding,
    Paused,
    Stopped,
    #[default]
    Queued,
    Checking,
    Metadata,
    Finished,
    Error,
}

impl State {
    pub fn parse(s: &str) -> Option<State> {
        serde_json::from_value(Value::String(s.to_string())).ok()
    }
}

fn lenient_state<'de, D: Deserializer<'de>>(d: D) -> Result<State, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::String(s) => State::parse(&s).unwrap_or_default(),
        _ => State::default(),
    })
}

// ------------------------------------------------------------------ records

/// What a stopped torrent still knows about one of its files.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FileSnap {
    #[serde(deserialize_with = "lenient::string")]
    pub name: String,
    #[serde(deserialize_with = "lenient::string")]
    pub path: String,
    #[serde(deserialize_with = "lenient::u64")]
    pub length: u64,
    #[serde(deserialize_with = "lenient::u64")]
    pub downloaded: u64,
    #[serde(deserialize_with = "lenient::f64")]
    pub progress: f64,
}

/// The parts of a .torrent's header the General tab shows.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TorrentMeta {
    #[serde(deserialize_with = "lenient::string")]
    pub comment: String,
    #[serde(deserialize_with = "lenient::string")]
    pub created_by: String,
    #[serde(deserialize_with = "lenient::i64")]
    pub created_on: i64,
    #[serde(deserialize_with = "lenient::bool")]
    pub private: bool,
}

/// One torrent as the state file remembers it, field for field with what
/// engine.js wrote -- same names, same order, same base64 encodings -- so a
/// file this build writes still opens in the Electron build.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TorrentRecord {
    #[serde(deserialize_with = "lenient::string")]
    pub id: String,
    #[serde(deserialize_with = "lenient::opt_string")]
    pub info_hash: Option<String>,
    #[serde(deserialize_with = "lenient::string")]
    pub name: String,
    #[serde(rename = "magnetURI", deserialize_with = "lenient::opt_string")]
    pub magnet_uri: Option<String>,
    /// base64 of the raw .torrent
    #[serde(deserialize_with = "lenient::opt_string")]
    pub torrent_file: Option<String>,
    #[serde(deserialize_with = "lenient::string")]
    pub save_path: String,
    #[serde(deserialize_with = "lenient::string")]
    pub label: String,
    #[serde(deserialize_with = "opt_i64")]
    pub order: Option<i64>,
    #[serde(deserialize_with = "lenient::i64")]
    pub added_on: i64,
    #[serde(deserialize_with = "lenient::i64")]
    pub completed_on: i64,
    #[serde(deserialize_with = "lenient::u64")]
    pub uploaded_base: u64,
    #[serde(deserialize_with = "lenient::u64")]
    pub downloaded_base: u64,
    /// Seconds spent running, and seconds of that spent finished, over every run.
    #[serde(deserialize_with = "lenient::u64")]
    pub active_base: u64,
    #[serde(deserialize_with = "lenient::u64")]
    pub finished_base: u64,
    #[serde(deserialize_with = "lenient::u64")]
    pub length: u64,
    /// Wanted file indices; null means every file.
    #[serde(deserialize_with = "opt_indices")]
    pub wanted: Option<Vec<usize>>,
    /// fileIndex -> 0 skip, 1 normal, 2 high. Absent means normal.
    #[serde(deserialize_with = "priorities")]
    pub priorities: BTreeMap<usize, u8>,
    #[serde(deserialize_with = "lenient::bool")]
    pub sequential: bool,
    #[serde(deserialize_with = "lenient::u64")]
    pub piece_length: u64,
    #[serde(deserialize_with = "lenient::u64")]
    pub piece_count: u64,
    /// base64 packed bits, one per piece, most significant bit first
    #[serde(deserialize_with = "lenient::opt_string")]
    pub bitfield: Option<String>,
    #[serde(deserialize_with = "opt_array")]
    pub files: Option<Vec<FileSnap>>,
    #[serde(deserialize_with = "opt_array")]
    pub announce: Option<Vec<String>>,
    #[serde(deserialize_with = "opt_array")]
    pub web_seeds: Option<Vec<String>>,
    #[serde(deserialize_with = "opt_object")]
    pub meta: Option<TorrentMeta>,
    #[serde(deserialize_with = "lenient::f64")]
    pub progress: f64,
    #[serde(deserialize_with = "lenient_state")]
    pub state: State,
    /// Anything a newer build wrote that this one does not know about.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

fn opt_i64<'de, D: Deserializer<'de>>(d: D) -> Result<Option<i64>, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        _ => None,
    })
}

fn opt_indices<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Vec<usize>>, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::Array(a) => Some(a.iter().filter_map(|v| v.as_f64()).map(|f| f as usize).collect()),
        _ => None,
    })
}

fn priorities<'de, D: Deserializer<'de>>(d: D) -> Result<BTreeMap<usize, u8>, D::Error> {
    let mut out = BTreeMap::new();
    if let Value::Object(map) = Value::deserialize(d)? {
        for (k, v) in map {
            if let (Ok(i), Some(p)) = (k.parse::<usize>(), v.as_f64()) {
                out.insert(i, (p as i64).clamp(0, 2) as u8);
            }
        }
    }
    Ok(out)
}

fn opt_array<'de, D, T>(d: D) -> Result<Option<Vec<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    Ok(match Value::deserialize(d)? {
        Value::Array(a) => Some(a.into_iter().filter_map(|v| serde_json::from_value(v).ok()).collect()),
        _ => None,
    })
}

fn opt_object<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    Ok(match Value::deserialize(d)? {
        v @ Value::Object(_) => serde_json::from_value(v).ok(),
        _ => None,
    })
}

// ------------------------------------------------------------------- labels

/// The symbols a label may wear, in picker order: the tag first as the default,
/// then marks, then the kinds of thing people keep.
pub const TAG_SYMBOLS: [&str; 32] = [
    "label", "star", "heart", "flag", "bookmark", "pin", "key", "lock",
    "folder", "box", "archive", "document", "book", "graduation", "code", "terminal",
    "film", "tv", "music", "headphones", "mic", "image", "camera", "disc",
    "monitor", "gamepad", "globe", "cloud", "rocket", "flame", "trophy", "leaf",
];

/// The hues a label may wear. Slate leads because it is the absence of a choice;
/// ink is the text colour, black on light and white on dark.
pub const TAG_COLORS: [&str; 9] = ["slate", "ink", "blue", "teal", "green", "amber", "red", "violet", "pink"];

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LabelStyle {
    #[serde(deserialize_with = "lenient::string")]
    pub symbol: String,
    #[serde(deserialize_with = "lenient::string")]
    pub color: String,
}

pub type LabelStyles = IndexMap<String, LabelStyle>;

/// Falls back rather than rendering a blank row if a stored name goes stale.
pub fn tag_style(style: Option<&LabelStyle>) -> (&'static str, &'static str) {
    let symbol = style
        .and_then(|s| TAG_SYMBOLS.iter().find(|t| **t == s.symbol))
        .copied()
        .unwrap_or("label");
    let color = style
        .and_then(|s| TAG_COLORS.iter().find(|t| **t == s.color))
        .copied()
        .unwrap_or("slate");
    (symbol, color)
}

// ----------------------------------------------------------------- snapshots

/// Compact per-torrent state for the main list, sent once a second.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Row {
    pub id: String,
    pub info_hash: String,
    pub name: String,
    pub order: i64,
    pub state: State,
    pub error: Option<String>,
    pub label: String,
    pub save_path: String,
    pub size: u64,
    pub wanted_size: u64,
    pub done: f64,
    pub downloaded: u64,
    pub uploaded: u64,
    pub download_speed: f64,
    pub upload_speed: f64,
    pub num_peers: u32,
    pub seeds: u32,
    pub peers: u32,
    /// Milliseconds; infinite when there is no estimate.
    pub eta: f64,
    pub ratio: f64,
    pub availability: f64,
    pub added_on: i64,
    pub completed_on: i64,
    /// Seconds spent running, and seconds of that spent finished.
    pub active_secs: u64,
    pub finished_secs: u64,
    pub sequential: bool,
    pub magnet_uri: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Globals {
    pub download_speed: f64,
    pub upload_speed: f64,
    pub downloaded: u64,
    pub uploaded: u64,
    pub ratio: f64,
    /// DHT is on and not switched off by the egress policy.
    pub dht_enabled: bool,
    pub dht_ready: bool,
    pub dht_nodes: u64,
    pub listen_port: u16,
    pub torrent_count: usize,
    pub alt_speed: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TrackerRow {
    pub url: String,
    pub status: String,
    /// -1 when unknown
    pub seeds: i64,
    pub peers: i64,
    /// seconds; 0 when unknown
    pub interval: u64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PeerRow {
    pub address: String,
    pub port: u16,
    pub client: String,
    pub flags: String,
    pub progress: f64,
    pub down_speed: f64,
    pub up_speed: f64,
    pub downloaded: u64,
    pub uploaded: u64,
    pub kind: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FileRow {
    pub index: usize,
    pub name: String,
    pub path: String,
    pub length: u64,
    pub downloaded: u64,
    pub progress: f64,
    pub priority: u8,
}

/// Rich detail for the selected torrent only.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Details {
    pub id: String,
    pub name: String,
    pub info_hash: String,
    pub save_path: String,
    pub comment: String,
    pub created_by: String,
    pub created_on: i64,
    pub piece_length: u64,
    pub piece_count: u64,
    pub private: bool,
    pub sequential: bool,
    pub trackers: Vec<TrackerRow>,
    pub peers: Vec<PeerRow>,
    /// Empty until metadata arrives.
    pub files: Vec<FileRow>,
    /// One byte per piece: 2 have, 1 partly here, 0 missing. None without metadata.
    pub pieces: Option<Vec<u8>>,
    pub have: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LogLine {
    pub time: i64,
    pub message: String,
    pub level: LogLevel,
}

pub const LOG_LIMIT: usize = 800;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetInterface {
    pub name: String,
    pub address: String,
}

// -------------------------------------------------------- shared list rules

/// Wording only -- the percentage is the caller's to add, so a state never
/// carries its own copy of a number the bar is already appending.
pub fn status_text(r: &Row) -> String {
    match r.state {
        State::Error => format!("Error: {}", r.error.as_deref().unwrap_or("unknown")),
        State::Stopped if r.done >= 1.0 => "Finished".into(),
        s => state_label(s).into(),
    }
}

pub fn state_label(s: State) -> &'static str {
    match s {
        State::Downloading => "Downloading",
        State::Seeding => "Seeding",
        State::Paused => "Paused",
        State::Stopped => "Stopped",
        State::Queued => "Queued",
        State::Checking => "Checking",
        State::Metadata => "Downloading metadata",
        State::Finished => "Finished",
        State::Error => "Error",
    }
}

pub fn is_active(r: &Row) -> bool {
    matches!(r.state, State::Downloading | State::Seeding | State::Metadata)
        && (r.download_speed > 0.0 || r.upload_speed > 0.0 || r.num_peers > 0)
}

/// Which fill a progress bar takes. Complete torrents split two ways: green
/// while they are still giving back, violet once they have stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BarKind {
    Active,
    Seed,
    Finished,
    Idle,
    Error,
}

pub fn bar_kind(r: &Row) -> BarKind {
    if r.state == State::Error {
        return BarKind::Error;
    }
    if r.done >= 1.0 {
        return if r.state == State::Seeding { BarKind::Seed } else { BarKind::Finished };
    }
    match r.state {
        State::Downloading | State::Checking | State::Metadata => BarKind::Active,
        _ => BarKind::Idle,
    }
}

/// The icon in the Name cell.
pub fn state_icon(r: &Row) -> &'static str {
    match r.state {
        State::Error => "error",
        State::Seeding => "seeding",
        State::Downloading | State::Metadata | State::Checking => "downloading",
        _ if r.done >= 1.0 => "completed",
        _ => "inactive",
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Category {
    All,
    Downloading,
    Seeding,
    Completed,
    Active,
    Inactive,
    NoLabel,
    Label(String),
}

impl Category {
    pub fn matches(&self, r: &Row) -> bool {
        match self {
            Category::All => true,
            Category::Downloading => {
                matches!(r.state, State::Downloading | State::Metadata)
                    || (r.done < 1.0 && matches!(r.state, State::Queued | State::Checking))
            }
            Category::Seeding => r.state == State::Seeding,
            Category::Completed => r.done >= 1.0,
            Category::Active => is_active(r),
            Category::Inactive => !is_active(r),
            Category::NoLabel => r.label.is_empty(),
            Category::Label(l) => &r.label == l,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_reads_electron_shape_and_writes_it_back() {
        let json = serde_json::json!({
            "id": "a", "infoHash": "abc", "name": "Sintel", "magnetURI": "magnet:?xt=urn:btih:abc",
            "torrentFile": null, "savePath": "/tmp", "label": "movie", "order": 3, "addedOn": 1.7e12,
            "completedOn": 0, "uploadedBase": 12, "downloadedBase": 1024.0, "length": 99,
            "wanted": [0, 2], "priorities": {"1": 0, "2": 2}, "sequential": false, "pieceLength": 16384,
            "pieceCount": 4, "bitfield": "8A==", "files": null, "announce": ["http://t/a"],
            "webSeeds": null, "meta": {"comment": "hi", "createdBy": "x", "createdOn": 5, "private": true},
            "progress": 0.5, "state": "paused", "future": {"kept": true}
        });
        let r: TorrentRecord = serde_json::from_value(json).unwrap();
        assert_eq!(r.magnet_uri.as_deref(), Some("magnet:?xt=urn:btih:abc"));
        assert_eq!(r.downloaded_base, 1024);
        assert_eq!(r.priorities.get(&1), Some(&0));
        assert_eq!(r.state, State::Paused);
        assert!(r.meta.as_ref().unwrap().private);
        let back = serde_json::to_value(&r).unwrap();
        assert_eq!(back["magnetURI"], "magnet:?xt=urn:btih:abc");
        assert_eq!(back["priorities"]["2"], 2);
        assert_eq!(back["future"]["kept"], true);
        assert!(back["torrentFile"].is_null());
    }

    #[test]
    fn junk_does_not_sink_a_record() {
        let json = serde_json::json!({ "id": 7, "state": "exploded", "files": "nope", "priorities": [1] });
        let r: TorrentRecord = serde_json::from_value(json).unwrap();
        assert_eq!(r.id, "7");
        assert_eq!(r.state, State::Queued);
        assert!(r.files.is_none());
    }

    #[test]
    fn stale_label_style_falls_back() {
        let s = LabelStyle { symbol: "gone".into(), color: "teal".into() };
        assert_eq!(tag_style(Some(&s)), ("label", "teal"));
        assert_eq!(tag_style(None), ("label", "slate"));
    }
}

/// One hit from an online search.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchResult {
    /// Which provider found it.
    pub source: String,
    pub title: String,
    /// A magnet link or an http(s) link to a .torrent, as `TorrentSource::parse` reads them.
    pub link: String,
    /// 0 when the provider does not say.
    pub size: u64,
    pub seeders: Option<u32>,
    pub leechers: Option<u32>,
}

/// The search providers built into the engine, by id and the name the window
/// shows. Only sources that are neutral by nature: an archive of freely
/// licensed works, and the self-hosted search servers whose sites the user
/// chooses. Every other source is one the user adds; see [`CustomSource`].
pub const SEARCH_SOURCES: [(&str, &str); 2] = [("archive", "Internet Archive"), ("indexer", "Your own search server")];

/// The categories every provider can narrow a search to.
pub const SEARCH_CATEGORIES: [(&str, &str); 6] =
    [("all", "All"), ("video", "Video"), ("audio", "Audio"), ("books", "Books"), ("software", "Software"), ("games", "Games")];

/// A search source the user added: any site with a search URL that answers in
/// JSON. ztorrent ships none of these. Every mapping field left empty is found
/// by looking at the answer; one that is set names the field to read instead,
/// as a dotted path such as `data.movies`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CustomSource {
    pub name: String,
    /// The search URL, with `{query}` where the words go.
    pub url: String,
    pub enabled: bool,
    /// Where the list of results is in the answer.
    pub results: String,
    /// A list inside each result that holds its torrents, when one result
    /// comes in several versions.
    pub items: String,
    pub title: String,
    pub hash: String,
    pub magnet: String,
    /// A link to a .torrent file.
    pub link: String,
    pub size: String,
    pub seeders: String,
    pub leechers: String,
    /// Added to magnet links built from an info hash.
    pub trackers: Vec<String>,
    /// Other addresses of the same search, each with `{query}`, tried in turn
    /// when the one before cannot be reached: sites of this kind move.
    pub mirrors: Vec<String>,
}

impl Default for CustomSource {
    fn default() -> Self {
        CustomSource {
            name: String::new(),
            url: String::new(),
            enabled: true,
            results: String::new(),
            items: String::new(),
            title: String::new(),
            hash: String::new(),
            magnet: String::new(),
            link: String::new(),
            size: String::new(),
            seeders: String::new(),
            leechers: String::new(),
            trackers: Vec::new(),
            mirrors: Vec::new(),
        }
    }
}

impl CustomSource {
    pub const QUERY: &str = "{query}";

    /// What is wrong with it, said so the user can fix it.
    pub fn check(&self) -> Result<(), String> {
        let url = self.url.trim();
        if self.name.trim().is_empty() {
            return Err("Give the source a name.".into());
        }
        if !(url.starts_with("https://") || url.starts_with("http://")) {
            return Err("The address must start with https:// or http://.".into());
        }
        if !url.contains(Self::QUERY) {
            return Err("Put {query} in the address where the search words go, e.g. https://example.org/api?q={query}".into());
        }
        let host = url.split("://").nth(1).and_then(|r| r.split(['/', '?', '#']).next()).unwrap_or_default();
        if host.is_empty() || host.contains('{') {
            return Err("{query} must come after the site's address and the API's path, e.g. https://example.org/api?q={query} -- not https://example.org{query}".into());
        }
        if let Some(bad) = self.mirrors.iter().find(|m| !(m.starts_with("https://") || m.starts_with("http://")) || !m.contains(Self::QUERY)) {
            return Err(format!("Each backup address must start with https:// and contain {{query}}: {bad}"));
        }
        Ok(())
    }

    /// A definition someone shared, as the JSON "Copy definition" gives.
    ///
    /// Read out of whatever surrounds it, so an AI assistant's answer can be
    /// pasted as it is: in a code block, after a sentence, with a note below.
    pub fn from_shared(text: &str) -> Result<CustomSource, String> {
        let not_one = || "That is not a source definition. Paste the whole {…} part.".to_string();
        // An assistant that found no API says so in an "error".
        let refusal = json_objects(text)
            .filter_map(parse_lenient::<Value>)
            .find_map(|v| (v.get("url").is_none()).then(|| v.get("error").and_then(Value::as_str).map(String::from)).flatten());
        if let Some(why) = refusal {
            return Err(format!("The assistant found no API: {why}"));
        }
        let mut source: CustomSource = json_objects(text)
            .filter_map(parse_lenient::<CustomSource>)
            .find(|s| !s.url.is_empty())
            .ok_or_else(not_one)?;
        source.enabled = true;
        // Assistants write the placeholder doubled now and then, as in a template.
        source.url = source.url.trim().replace("{{query}}", Self::QUERY);
        source.mirrors = source.mirrors.iter().map(|m| m.trim().replace("{{query}}", Self::QUERY)).filter(|m| !m.is_empty() && *m != source.url).collect();
        source.check()?;
        Ok(source)
    }

    /// Whether pasted text is a definition rather than an address.
    pub fn looks_shared(text: &str) -> bool {
        let t = text.trim();
        !t.starts_with("http") && t.contains('{') && t.contains("\"url\"")
    }

    /// A question to paste into an AI assistant. It ends "for the following
    /// website " so the site's name is typed straight after it, and asks for
    /// nothing but JSON, so the whole reply can be pasted back.
    pub fn assistant_prompt() -> String {
        r#"I use the BitTorrent client ztorrent. It can search any website whose search API answers in JSON. Find the website's public search API, then reply with ONLY a JSON object that defines it as a ztorrent search source. No other text before or after it, and no code block.

The JSON object:

{
  "name": "<a short name for the site>",
  "url": "<the search API address, with {query} exactly where the URL-encoded search words go>"
}

Rules:
- "url" must start with https:// (or http://) and contain {query} exactly once, e.g. https://example.org/api/search?q={query}
- Opening "url" with {query} replaced by a word must return JSON, not a web page. Use the API's documented address, not the address of the site's search page.
- Sites like this often move to a new domain and leave the old one dead. Give the address that works today. If the same API is also reachable at other domains, list them in an optional "mirrors" field: a list of addresses in the same form, each with {query}. ztorrent tries them in turn.
- Each result in the API's answer must have a magnet link, a 40-character info hash, or a link to a .torrent file. Otherwise ztorrent cannot add it.
- ztorrent finds the results and their fields in the answer by itself. Only if the API's answer is unusual, also add any of these optional fields, each a dotted path into the JSON answer (numbers pick from a list, e.g. data.0.items):
  "results": the list of results
  "items": a list inside each result that holds its torrents, when one result comes in several versions
  "title", "hash", "magnet", "link" (the .torrent link), "size", "seeders", "leechers"
  "trackers": a list of tracker addresses to add to magnet links built from an info hash
  Write paths with dots only, no $ or [*]. A path for a field inside "items" starts from one item of that list (write "hash", not "torrents.hash").
- If the API needs a key, put it into "url" the way the API expects, and add a "note" field saying where to get the key.
- If you are not sure such an API exists or works this way, reply with {"error": "<why>"} instead. Do not invent an address: it will be tested.
- The reply must be strictly valid JSON: straight double quotes, no backslash before ":" or "/", no trailing commas, no comments.

Reply with the JSON object for the following website "#
            .to_string()
    }

    /// What an assistant added beside the definition: a "note", such as where
    /// to get an API key.
    pub fn shared_note(text: &str) -> Option<String> {
        json_objects(text)
            .filter_map(parse_lenient::<Value>)
            .find_map(|v| v.get("note").and_then(Value::as_str).map(|n| n.trim().to_string()))
            .filter(|n| !n.is_empty())
    }

    /// The JSON to share it with, leaving out what is empty.
    pub fn to_shared(&self) -> String {
        let mut value = serde_json::to_value(self).unwrap_or_default();
        if let Value::Object(map) = &mut value {
            map.remove("enabled");
            map.retain(|_, v| !matches!(v, Value::String(s) if s.is_empty()) && !matches!(v, Value::Array(a) if a.is_empty()));
        }
        serde_json::to_string_pretty(&value).unwrap_or_default()
    }
}

/// JSON as written, or else with the slips assistants and chat windows make
/// put right: `\:` and other escapes JSON does not have, curly quotes, and a
/// comma before a closing bracket.
fn parse_lenient<T: serde::de::DeserializeOwned>(text: &str) -> Option<T> {
    serde_json::from_str(text).ok().or_else(|| serde_json::from_str(&repair_json(text)).ok())
}

fn repair_json(text: &str) -> String {
    let text = text.replace(['\u{201c}', '\u{201d}'], "\"");
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut in_string = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if in_string {
            match c {
                '\\' => {
                    match chars.get(i + 1) {
                        Some(n) if "\"\\/bfnrtu".contains(*n) => {
                            out.push(c);
                            out.push(*n);
                        }
                        // An escape JSON does not have: keep the character alone.
                        Some(n) => out.push(*n),
                        None => {}
                    }
                    i += 2;
                    continue;
                }
                '"' => in_string = false,
                _ => {}
            }
            out.push(c);
        } else {
            match c {
                '"' => in_string = true,
                ',' if chars[i + 1..].iter().find(|c| !c.is_whitespace()).is_some_and(|n| *n == '}' || *n == ']') => {
                    i += 1;
                    continue;
                }
                _ => {}
            }
            out.push(c);
        }
        i += 1;
    }
    out
}

/// Every balanced `{…}` in some text, outermost first, braces inside JSON
/// strings ignored: the candidates for a JSON object somewhere in prose.
fn json_objects(text: &str) -> impl Iterator<Item = &str> {
    let bytes = text.as_bytes();
    (0..bytes.len()).filter(move |&i| bytes[i] == b'{').filter_map(move |start| {
        let (mut depth, mut in_string, mut escaped) = (0usize, false, false);
        for (i, &b) in bytes.iter().enumerate().skip(start) {
            if in_string {
                match b {
                    _ if escaped => escaped = false,
                    b'\\' => escaped = true,
                    b'"' => in_string = false,
                    _ => {}
                }
                continue;
            }
            match b {
                b'"' => in_string = true,
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(&text[start..=i]);
                    }
                }
                _ => {}
            }
        }
        None
    })
}

/// How to search, as the search settings say.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchOptions {
    /// Ids from `SEARCH_SOURCES`.
    pub sources: Vec<String>,
    pub category: String,
    /// Per provider.
    pub max_results: usize,
    pub hide_dead: bool,
    pub indexer_kind: String,
    pub indexer_url: String,
    pub indexer_key: String,
    /// The user's own sources that are switched on.
    pub custom: Vec<CustomSource>,
}

/// What a search found, and which providers could not be reached. One provider
/// being down must not hide what the others found, nor go unmentioned.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchOutcome {
    pub results: Vec<SearchResult>,
    pub failures: Vec<String>,
}
