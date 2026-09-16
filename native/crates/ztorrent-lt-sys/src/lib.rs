//! The bridge to libtorrent. Everything C++ stays behind the functions declared
//! here: exceptions come back as `Err`, handles are plain numbers, and every
//! value crosses as a copy the Rust side owns.

#[cxx::bridge(namespace = "ztlt")]
pub mod ffi {
    /// Session-wide settings, already reduced to what libtorrent needs. The
    /// egress policy has been applied by the time a config reaches here.
    #[derive(Clone, Debug, Default)]
    pub struct SessionConfig {
        pub listen_interfaces: String,
        pub outgoing_interfaces: String,
        pub enable_dht: bool,
        pub enable_lsd: bool,
        pub enable_upnp: bool,
        pub enable_utp: bool,
        pub enable_incoming_tcp: bool,
        /// 0 off, 1 prefer, 2 require
        pub encryption: u8,
        pub proxy_host: String,
        pub proxy_port: u16,
        pub proxy_username: String,
        pub proxy_password: String,
        /// bytes/s, 0 unlimited
        pub download_rate: i32,
        pub upload_rate: i32,
        pub connections_limit: i32,
        pub user_agent: String,
        pub peer_fingerprint: String,
    }

    #[derive(Clone, Debug, Default)]
    pub struct RenamedFile {
        pub index: i32,
        pub path: String,
    }

    #[derive(Clone, Debug, Default)]
    pub struct AddParams {
        /// Raw .torrent bytes, or empty.
        pub torrent: Vec<u8>,
        /// A magnet link, used when there are no bytes.
        pub magnet: String,
        /// libtorrent resume data from an earlier run, or empty.
        pub resume: Vec<u8>,
        pub save_path: String,
        /// One per file (0 skip, 4 normal, 7 high); empty leaves them alone.
        pub file_priorities: Vec<u8>,
        pub renamed: Vec<RenamedFile>,
        pub sequential: bool,
        pub paused: bool,
        pub disable_pex: bool,
        pub strip_udp_trackers: bool,
        pub max_uploads: i32,
    }

    #[derive(Clone, Debug, Default)]
    pub struct Status {
        pub valid: bool,
        /// libtorrent torrent_status::state_t
        pub state: u8,
        pub paused: bool,
        pub has_metadata: bool,
        pub progress: f64,
        pub total: i64,
        pub total_done: i64,
        pub total_wanted: i64,
        pub total_wanted_done: i64,
        pub all_time_download: i64,
        pub all_time_upload: i64,
        pub download_rate: i32,
        pub upload_rate: i32,
        pub num_peers: i32,
        pub num_seeds: i32,
        pub list_seeds: i32,
        pub list_peers: i32,
        pub distributed_copies: f32,
        pub error: String,
        pub name: String,
    }

    #[derive(Clone, Debug, Default)]
    pub struct FileEntry {
        pub path: String,
        pub name: String,
        pub size: i64,
        pub downloaded: i64,
        pub priority: u8,
        pub pad: bool,
    }

    #[derive(Clone, Debug, Default)]
    pub struct TrackerInfo {
        pub url: String,
        pub tier: i32,
        pub updating: bool,
        pub contacted: bool,
        /// Some endpoint was reached and answered without an error.
        pub working: bool,
        pub failed: bool,
        pub message: String,
        pub complete: i32,
        pub incomplete: i32,
        pub next_announce_secs: i64,
    }

    #[derive(Clone, Debug, Default)]
    pub struct PeerEntry {
        pub ip: String,
        pub port: u16,
        pub client: String,
        pub progress: f32,
        pub down_rate: i32,
        pub up_rate: i32,
        pub total_download: i64,
        pub total_upload: i64,
        pub interesting: bool,
        pub choked: bool,
        pub remote_interested: bool,
        pub remote_choked: bool,
        pub seed: bool,
        pub utp: bool,
        pub outgoing: bool,
        pub web_seed: bool,
        pub encrypted: bool,
    }

    #[derive(Clone, Debug, Default)]
    pub struct MetaInfo {
        pub has_metadata: bool,
        pub name: String,
        pub info_hash: String,
        pub comment: String,
        pub creator: String,
        pub created: i64,
        pub private_flag: bool,
        pub piece_length: i64,
        pub num_pieces: i64,
        pub total_size: i64,
        pub trackers: Vec<String>,
        pub url_seeds: Vec<String>,
        pub files: Vec<FileEntry>,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    #[repr(u8)]
    pub enum AlertKind {
        MetadataReceived = 1,
        TorrentFinished = 2,
        TorrentError = 3,
        FileError = 4,
        ResumeData = 5,
        ResumeDataFailed = 6,
        FileRenamed = 7,
        FileRenameFailed = 8,
        TorrentChecked = 9,
        ListenSucceeded = 12,
        ListenFailed = 13,
        TorrentRemoved = 14,
        TorrentDeleted = 15,
        TorrentDeleteFailed = 16,
        DhtBootstrap = 17,
        PortmapError = 18,
    }

    #[derive(Clone, Debug)]
    pub struct Alert {
        pub kind: AlertKind,
        /// 0 when the alert has no torrent, or the torrent is already gone.
        pub handle: u64,
        pub index: i32,
        pub number: i64,
        pub message: String,
        pub data: Vec<u8>,
    }

    #[derive(Clone, Debug, Default)]
    pub struct CreateParams {
        pub input: String,
        pub trackers: Vec<String>,
        /// Tier per tracker, parallel to `trackers`.
        pub tiers: Vec<i32>,
        pub web_seeds: Vec<String>,
        pub comment: String,
        pub creator: String,
        pub private_flag: bool,
        /// 0 = automatic
        pub piece_size: i32,
    }

    extern "Rust" {
        type ProgressSink;
        fn report(self: &ProgressSink, done: i32, total: i32);
    }

    unsafe extern "C++" {
        include!("ztorrent-lt-sys/include/bridge.h");

        type Session;

        fn new_session(config: &SessionConfig) -> Result<UniquePtr<Session>>;
        fn apply_config(self: Pin<&mut Session>, config: &SessionConfig);

        fn add(self: Pin<&mut Session>, params: &AddParams) -> Result<u64>;
        fn remove(self: Pin<&mut Session>, handle: u64, delete_files: bool) -> bool;
        fn pause(self: &Session, handle: u64);
        fn resume(self: &Session, handle: u64);
        fn recheck(self: &Session, handle: u64);
        fn reannounce(self: &Session, handle: u64);
        fn set_sequential(self: &Session, handle: u64, on: bool);
        fn set_file_priority(self: &Session, handle: u64, index: i32, priority: u8);
        fn set_max_uploads(self: &Session, handle: u64, slots: i32);
        fn add_tracker(self: &Session, handle: u64, url: &str) -> bool;
        fn connect_peer(self: &Session, handle: u64, ip: &str, port: u16) -> Result<()>;
        fn rename_file(self: &Session, handle: u64, index: i32, path: &str);
        fn save_resume_data(self: &Session, handle: u64) -> bool;

        fn status(self: &Session, handle: u64) -> Status;
        fn meta(self: &Session, handle: u64) -> MetaInfo;
        fn files(self: &Session, handle: u64) -> Vec<FileEntry>;
        fn trackers(self: &Session, handle: u64) -> Vec<TrackerInfo>;
        fn peers(self: &Session, handle: u64) -> Vec<PeerEntry>;
        fn piece_map(self: &Session, handle: u64) -> Vec<u8>;
        fn torrent_bytes(self: &Session, handle: u64) -> Vec<u8>;
        fn magnet_uri(self: &Session, handle: u64) -> String;

        fn wait_for_alert(self: Pin<&mut Session>, millis: i32) -> bool;
        fn pop_alerts(self: Pin<&mut Session>) -> Vec<Alert>;
        fn post_session_stats(self: Pin<&mut Session>);
        fn dht_nodes(self: &Session) -> i64;
        fn dht_running(self: &Session) -> bool;
        fn listen_port(self: &Session) -> u16;
        fn reopen_network_sockets(self: Pin<&mut Session>);

        fn inspect_buffer(buffer: &[u8]) -> Result<MetaInfo>;
        fn inspect_magnet(uri: &str) -> Result<MetaInfo>;
        fn create_torrent(params: &CreateParams, sink: &ProgressSink) -> Result<Vec<u8>>;
    }
}

pub use cxx;

unsafe impl Send for ffi::Session {}

/// Receives hashing progress from `create_torrent`.
pub struct ProgressSink(pub Box<dyn Fn(i32, i32) + Send>);

impl ProgressSink {
    fn report(&self, done: i32, total: i32) {
        (self.0)(done, total)
    }
}

pub use ffi::*;

/// libtorrent's per-file priority for ztorrent's 0 / 1 / 2.
pub fn lt_priority(p: u8) -> u8 {
    match p {
        0 => 0,
        2 => 7,
        _ => 4,
    }
}

/// libtorrent torrent_status::state_t
pub mod state {
    pub const CHECKING_FILES: u8 = 1;
    pub const DOWNLOADING_METADATA: u8 = 2;
    pub const DOWNLOADING: u8 = 3;
    pub const FINISHED: u8 = 4;
    pub const SEEDING: u8 = 5;
    pub const CHECKING_RESUME_DATA: u8 = 7;
}
