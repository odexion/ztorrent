//! The complete set of things the window can ask of the engine, and everything
//! the engine tells the window. This is the allow-list: a capability the window
//! does not have a variant for is a capability it does not have.

use crate::columns::Columns;
use crate::model::*;
use crate::settings::{Settings, SettingsPatch};
use crate::store::WindowBounds;
use futures_channel::oneshot;
use std::collections::BTreeMap;
use std::path::PathBuf;

pub type Reply<T> = oneshot::Sender<T>;

/// A magnet link, an http(s) link to a .torrent, a path on disk, or raw bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TorrentSource {
    Magnet(String),
    Url(String),
    Path(PathBuf),
    Bytes(Vec<u8>),
}

impl TorrentSource {
    /// Classifies what the user typed, dropped or double-clicked. A bare
    /// 40-character hex info hash becomes a magnet link, as the URL sheet does.
    pub fn parse(input: &str) -> TorrentSource {
        let s = input.trim();
        if s.len() == 40 && s.chars().all(|c| c.is_ascii_hexdigit()) {
            return TorrentSource::Magnet(format!("magnet:?xt=urn:btih:{}", s.to_ascii_lowercase()));
        }
        if s.starts_with("magnet:") {
            TorrentSource::Magnet(s.to_string())
        } else if s.starts_with("http://") || s.starts_with("https://") {
            TorrentSource::Url(s.to_string())
        } else {
            TorrentSource::Path(PathBuf::from(s.strip_prefix("file://").unwrap_or(s)))
        }
    }

    pub fn is_magnet(&self) -> bool {
        matches!(self, TorrentSource::Magnet(_))
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AddOptions {
    /// None: the default download folder.
    pub save_path: Option<String>,
    pub label: String,
    /// None: the preference.
    pub sequential: Option<bool>,
    pub paused: bool,
    pub wanted: Option<Vec<usize>>,
    pub priorities: BTreeMap<usize, u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AddOutcome {
    Added(String),
    Duplicate(String),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct InspectFile {
    pub index: usize,
    pub name: String,
    pub path: String,
    pub length: u64,
}

/// A torrent read without adding it, for the Add sheet.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InspectInfo {
    pub name: String,
    pub info_hash: String,
    pub length: u64,
    pub piece_length: u64,
    pub comment: String,
    pub created_by: String,
    pub created: i64,
    pub private: bool,
    pub announce: Vec<String>,
    pub files: Vec<InspectFile>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CreateTorrentOptions {
    pub input_path: PathBuf,
    pub output_path: PathBuf,
    pub name: String,
    pub comment: String,
    /// Tiers of tracker URLs.
    pub trackers: Vec<Vec<String>>,
    pub web_seeds: Vec<String>,
    /// 0 = automatic
    pub piece_length: u32,
    pub private: bool,
    pub start_seeding: bool,
}

/// Everything the window needs to draw its first frame.
#[derive(Clone, Debug, Default)]
pub struct Bootstrap {
    pub settings: Settings,
    pub labels: Vec<String>,
    pub label_styles: LabelStyles,
    pub log: Vec<LogLine>,
    pub columns: Option<Columns>,
    pub window: Option<WindowBounds>,
    pub rows: Vec<Row>,
    pub globals: Globals,
}

pub enum Command {
    Bootstrap(Reply<Bootstrap>),
    /// The torrent whose details each tick should carry, or none.
    Details(Option<String>),
    SetSettings(SettingsPatch),
    ToggleAltSpeed,
    SetLabelStyle { name: String, style: Option<LabelStyle> },
    SetColumns(Columns),
    SetWindow(WindowBounds),
    Interfaces(Reply<Vec<NetInterface>>),

    Inspect { source: TorrentSource, reply: Reply<Result<InspectInfo, String>> },
    Add { source: TorrentSource, options: AddOptions, reply: Option<Reply<Result<AddOutcome, String>>> },
    /// Several files at once skip the sheet and go to the remembered folder.
    AddPaths(Vec<PathBuf>),
    CreateTorrent { options: CreateTorrentOptions, progress: Option<futures_channel::mpsc::UnboundedSender<f32>>, reply: Reply<Result<u64, String>> },
    SaveTorrentFile { id: String, dest: PathBuf, reply: Reply<Result<(), String>> },

    Start(Vec<String>),
    Pause(Vec<String>),
    Stop(Vec<String>),
    Recheck(Vec<String>),
    Remove { ids: Vec<String>, delete_data: bool },
    MoveQueue { id: String, delta: i32 },
    SetLabel { ids: Vec<String>, label: String },
    SetSequential { id: String, on: bool },
    SetFilePriority { id: String, index: usize, priority: u8 },
    AddTracker { id: String, url: String },
    AddPeer { id: String, address: String },
    Reannounce(String),
    /// Absolute path of a torrent's content, or of one file in it, confined to its save folder.
    ResolvePath { id: String, file: Option<usize>, reply: Reply<Option<ResolvedPath>> },

    Shutdown(Reply<()>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedPath {
    pub target: PathBuf,
    pub exists: bool,
    pub save_path: PathBuf,
}

#[derive(Clone, Debug)]
pub enum Event {
    Tick { rows: Vec<Row>, globals: Globals, details: Option<Box<Details>> },
    Log(LogLine),
    /// Labels or label styles changed.
    LibraryChanged { labels: Vec<String>, label_styles: LabelStyles },
    SettingsChanged(Settings),
    Complete { id: String, name: String, path: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_sources() {
        assert!(matches!(TorrentSource::parse("magnet:?xt=urn:btih:abc"), TorrentSource::Magnet(_)));
        assert_eq!(
            TorrentSource::parse("ABCDEF0123456789ABCDEF0123456789ABCDEF01"),
            TorrentSource::Magnet("magnet:?xt=urn:btih:abcdef0123456789abcdef0123456789abcdef01".into())
        );
        assert!(matches!(TorrentSource::parse(" https://x/y.torrent "), TorrentSource::Url(_)));
        assert_eq!(TorrentSource::parse("/tmp/a.torrent"), TorrentSource::Path("/tmp/a.torrent".into()));
    }
}
