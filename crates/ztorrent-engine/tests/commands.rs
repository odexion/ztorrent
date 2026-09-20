//! Every command the window can send, exercised against real sessions on
//! loopback. The behavioural scan of the engine.

mod support;

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use ztorrent_core::command::*;
use ztorrent_core::settings::{Settings, patch};
use ztorrent_core::*;
use ztorrent_engine::{EngineHandle, EngineOptions, spawn};

// ------------------------------------------------------------------ helpers

fn quiet(s: &mut Settings, dir: &Path) {
    s.enable_dht = false;
    s.enable_lsd = false;
    s.enable_upnp = false;
    s.enable_utp = false;
    s.download_path = dir.join("downloads").to_string_lossy().into_owned();
}

fn engine(dir: &Path, tweak: impl FnOnce(&mut Settings)) -> EngineHandle {
    let mut store = Store::open(dir, None);
    let mut s = store.settings().clone();
    quiet(&mut s, dir);
    tweak(&mut s);
    store.data.settings = s;
    store.flush();
    spawn(Store::open(dir, None), EngineOptions { data_dir: dir.to_path_buf(), version: "0.5.0".into() }).expect("engine starts")
}

fn ask<T>(h: &EngineHandle, make: impl FnOnce(Reply<T>) -> Command) -> T {
    let (tx, mut rx) = futures_channel::oneshot::channel();
    h.commands.send(make(tx)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        match rx.try_recv() {
            Ok(Some(v)) => return v,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => panic!("no reply"),
        }
    }
}

/// Reads events until `f` says it has what it wants, keeping every log line seen.
struct Watch<'a> {
    h: &'a EngineHandle,
    log: Vec<String>,
    settings: Option<Settings>,
    library: Option<(Vec<String>, LabelStyles)>,
    completed: Vec<String>,
}

impl<'a> Watch<'a> {
    fn new(h: &'a EngineHandle) -> Self {
        Watch { h, log: vec![], settings: None, library: None, completed: vec![] }
    }

    fn until(&mut self, what: &str, secs: u64, mut f: impl FnMut(&[Row], &Option<Box<Details>>, &Globals, &Self) -> bool) -> (Vec<Row>, Option<Box<Details>>) {
        let deadline = Instant::now() + Duration::from_secs(secs);
        while Instant::now() < deadline {
            match self.h.events.recv_timeout(Duration::from_millis(200)) {
                Ok(Event::Tick { rows, globals, details }) => {
                    if f(&rows, &details, &globals, self) {
                        return (rows, details);
                    }
                }
                Ok(Event::Log(l)) => self.log.push(l.message),
                Ok(Event::SettingsChanged(s)) => self.settings = Some(s),
                Ok(Event::LibraryChanged { labels, label_styles }) => self.library = Some((labels, label_styles)),
                Ok(Event::Complete { name, .. }) => self.completed.push(name),
                Err(_) => {}
            }
        }
        panic!("timed out waiting for: {what}\nlog so far: {:#?}", self.log);
    }

    fn logged(&self, needle: &str) -> bool {
        self.log.iter().any(|l| l.contains(needle))
    }

    fn wait_log(&mut self, needle: &str, secs: u64) {
        let n = needle.to_string();
        self.until(&format!("log line containing {needle:?}"), secs, move |_, _, _, w| w.logged(&n));
    }
}

fn shutdown(h: EngineHandle) {
    let _ = ask(&h, Command::Shutdown);
    let _ = h.thread.join();
}

fn sintel() -> Vec<u8> {
    std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../sample-torrents/sintel.torrent")).unwrap()
}

fn state_file(dir: &Path) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(dir.join("ztorrent-state.json")).unwrap()).unwrap()
}

/// A local HTTP server answering every request with `body`.
fn serve(body: Vec<u8>) -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for mut c in listener.incoming().flatten() {
            let mut buf = [0u8; 2048];
            let _ = c.read(&mut buf);
            let _ = write!(c, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
            let _ = c.write_all(&body);
        }
    });
    port
}

/// A small folder to share: two files, so priorities have something to act on.
fn share(dir: &Path) -> PathBuf {
    let share = dir.join("share");
    std::fs::create_dir_all(&share).unwrap();
    std::fs::write(share.join("big.bin"), (0..2_000_000u32).map(|i| (i % 253) as u8).collect::<Vec<_>>()).unwrap();
    std::fs::write(share.join("small.bin"), (0..300_000u32).map(|i| (i % 7) as u8).collect::<Vec<_>>()).unwrap();
    share
}

fn added(outcome: Result<AddOutcome, String>) -> String {
    match outcome {
        Ok(AddOutcome::Added(id)) => id,
        other => panic!("expected Added, got {other:?}"),
    }
}

// --------------------------------------------------------------------- tests

#[test]
fn inspect_and_add_every_kind_of_source() {
    let dir = tempfile::tempdir().unwrap();
    let h = engine(dir.path(), |_| {});
    let mut w = Watch::new(&h);
    let sample = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../sample-torrents/sintel.torrent"));
    let port = serve(sintel());

    for source in [
        TorrentSource::Path(sample.clone()),
        TorrentSource::Bytes(sintel()),
        TorrentSource::Url(format!("http://127.0.0.1:{port}/sintel.torrent")),
    ] {
        let info = ask(&h, |r| Command::Inspect { source: source.clone(), reply: r }).expect("inspects");
        assert_eq!((info.name.as_str(), info.files.len(), info.info_hash.as_str()), ("Sintel", 11, "08ada5a7a6183aae1e09d831df6748d566095a10"), "{source:?}");
        assert_eq!(info.comment, "WebTorrent <https://webtorrent.io>");
        assert!(info.created > 1_400_000_000_000, "created on is in ms");
    }
    let magnet = ask(&h, |r| Command::Inspect { source: TorrentSource::parse("magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567"), reply: r }).unwrap();
    assert_eq!(magnet.name, "0123456789abcdef0123456789abcdef01234567", "a magnet without dn is named by its hash");
    assert!(ask(&h, |r| Command::Inspect { source: TorrentSource::Path(dir.path().join("missing.torrent")), reply: r }).is_err());
    assert!(ask(&h, |r| Command::Inspect { source: TorrentSource::Bytes(b"junk".to_vec()), reply: r }).is_err());

    // Added from a URL, then the same torrent from a file is a duplicate.
    let from_url = added(ask(&h, |r| Command::Add { source: TorrentSource::Url(format!("http://127.0.0.1:{port}/x.torrent")), options: AddOptions { paused: true, ..Default::default() }, reply: Some(r) }));
    let dup = ask(&h, |r| Command::Add { source: TorrentSource::Path(sample.clone()), options: AddOptions::default(), reply: Some(r) });
    assert_eq!(dup, Ok(AddOutcome::Duplicate(from_url.clone())));
    w.wait_log("\"Sintel\" is already in the list.", 5);

    // A magnet with no metadata yet, paused, with a label: named as the sheet names it.
    let hash = "0123456789abcdef0123456789abcdef01234567";
    let m = added(ask(&h, |r| Command::Add {
        source: TorrentSource::parse(hash),
        options: AddOptions { paused: true, label: "tv".into(), ..Default::default() },
        reply: Some(r),
    }));
    let (rows, _) = w.until("both rows", 5, |rows, _, _, _| rows.len() == 2);
    let row = rows.iter().find(|r| r.id == m).unwrap();
    assert_eq!(row.name, "Downloading metadata", "a magnet without a name is called what the Electron build called it");
    assert_eq!(row.state, State::Paused);
    assert_eq!(row.magnet_uri.as_deref(), Some(format!("magnet:?xt=urn:btih:{hash}").as_str()));
    w.until("label registered", 5, |_, _, _, w| w.library.as_ref().is_some_and(|(l, _)| l.contains(&"tv".to_string())));

    // A path that is not there is refused and logged; AddPaths uses the remembered folder.
    let bad = ask(&h, |r| Command::Add { source: TorrentSource::Path(dir.path().join("nope.torrent")), options: AddOptions::default(), reply: Some(r) });
    assert!(bad.is_err());
    w.wait_log("Could not read torrent:", 5);

    let other = tempfile::tempdir().unwrap();
    let created = other.path().join("share.torrent");
    ask(&h, |r| Command::CreateTorrent {
        options: CreateTorrentOptions { input_path: share(other.path()), output_path: created.clone(), piece_length: 65536, ..Default::default() },
        progress: None,
        reply: r,
    })
    .unwrap();
    let remembered = other.path().join("remembered");
    h.commands.send(Command::SetSettings(patch("lastSavePath", remembered.to_string_lossy().to_string()))).unwrap();
    h.commands.send(Command::AddPaths(vec![created])).unwrap();
    let (rows, _) = w.until("the AddPaths row", 10, |rows, _, _, _| rows.len() == 3);
    assert!(rows.iter().any(|r| r.name == "share" && r.save_path == remembered.to_string_lossy()), "added to the remembered folder");
    shutdown(h);
}

#[test]
fn settings_labels_columns_and_window_persist_the_way_electron_did() {
    let dir = tempfile::tempdir().unwrap();
    let h = engine(dir.path(), |s| s.last_save_path = "/somewhere".into());
    let mut w = Watch::new(&h);

    // Sending the same downloadPath keeps lastSavePath; a new one clears it.
    let current = ask(&h, Command::Bootstrap).settings.download_path;
    h.commands.send(Command::SetSettings(patch("downloadPath", current))).unwrap();
    w.until("unchanged path", 5, |_, _, _, w| w.settings.as_ref().is_some_and(|s| s.last_save_path == "/somewhere"));
    h.commands.send(Command::SetSettings(patch("downloadPath", "/elsewhere"))).unwrap();
    w.until("cleared", 5, |_, _, _, w| w.settings.as_ref().is_some_and(|s| s.last_save_path.is_empty() && s.download_path == "/elsewhere"));

    // A policy change says it needs a restart; alternate speed toggles and logs.
    let mut p = patch("proxyEnabled", true);
    p.insert("proxyHost".into(), "127.0.0.1".into());
    p.insert("proxyPort".into(), 9.into());
    h.commands.send(Command::SetSettings(p)).unwrap();
    w.wait_log("Proxy or interface binding changed. It takes effect when ztorrent restarts", 5);
    h.commands.send(Command::ToggleAltSpeed).unwrap();
    w.wait_log("Alternate speed limits enabled.", 5);
    let (_, _) = w.until("alt in globals", 5, |_, _, g, _| g.alt_speed);
    h.commands.send(Command::ToggleAltSpeed).unwrap();
    w.wait_log("Alternate speed limits disabled.", 5);

    // Labels and their styles; resetting a style removes it rather than storing a default.
    let id = added(ask(&h, |r| Command::Add { source: TorrentSource::Bytes(sintel()), options: AddOptions { paused: true, ..Default::default() }, reply: Some(r) }));
    h.commands.send(Command::SetLabel { ids: vec![id.clone()], label: "movie".into() }).unwrap();
    h.commands.send(Command::SetLabelStyle { name: "movie".into(), style: Some(LabelStyle { symbol: "film".into(), color: "violet".into() }) }).unwrap();
    h.commands.send(Command::SetLabelStyle { name: "tv".into(), style: Some(LabelStyle { symbol: "star".into(), color: "red".into() }) }).unwrap();
    h.commands.send(Command::SetLabelStyle { name: "tv".into(), style: None }).unwrap();
    w.until("styles", 5, |_, _, _, w| w.library.as_ref().is_some_and(|(l, s)| l == &vec!["movie".to_string()] && s.len() == 1 && s["movie"].symbol == "film"));
    let (rows, _) = w.until("label on row", 5, |rows, _, _, _| rows.first().is_some_and(|r| r.label == "movie"));
    assert_eq!(rows.len(), 1);

    h.commands.send(Command::SetColumns(ztorrent_core::columns::Columns { order: vec!["#".into(), "name".into()], widths: [("name".to_string(), 420.0)].into_iter().collect() })).unwrap();
    h.commands.send(Command::SetWindow(ztorrent_core::store::WindowBounds { x: Some(10.0), y: Some(20.0), width: 1000.0, height: 700.0 })).unwrap();
    let interfaces = ask(&h, Command::Interfaces);
    assert!(interfaces.iter().all(|i| !i.name.is_empty() && i.address.contains('.')));
    shutdown(h);

    let saved = state_file(dir.path());
    assert_eq!(saved["labels"], serde_json::json!(["movie"]));
    assert_eq!(saved["labelStyles"], serde_json::json!({"movie": {"symbol": "film", "color": "violet"}}));
    assert_eq!(saved["columns"]["widths"]["name"], 420.0);
    assert_eq!(saved["window"]["width"], 1000.0);
    assert_eq!(saved["settings"]["downloadPath"], "/elsewhere");
    assert_eq!(saved["settings"]["proxyEnabled"], true);
    assert!(saved["settings"].get("altSpeedEnabled").is_none(), "session only");
    assert_eq!(saved["torrents"][0]["label"], "movie");
    assert_eq!(saved["torrents"][0]["state"], "paused");
}

#[test]
fn torrent_commands_on_a_live_local_swarm() {
    let seed_dir = tempfile::tempdir().unwrap();
    let leech_dir = tempfile::tempdir().unwrap();
    // Throttled, so the transfer lasts long enough for a tick to list the peer.
    let seeder = engine(seed_dir.path(), |s| s.max_upload_rate = 250);
    let mut sw = Watch::new(&seeder);
    let torrent = seed_dir.path().join("share.torrent");
    let (ptx, prx) = futures_channel::mpsc::unbounded::<f32>();
    let size = ask(&seeder, |r| Command::CreateTorrent {
        options: CreateTorrentOptions {
            input_path: share(seed_dir.path()),
            output_path: torrent.clone(),
            trackers: vec![vec!["http://127.0.0.1:1/announce".into()], vec!["http://127.0.0.1:2/announce".into()]],
            web_seeds: vec![],
            comment: "scan".into(),
            piece_length: 65536,
            start_seeding: true,
            ..Default::default()
        },
        progress: Some(ptx),
        reply: r,
    })
    .unwrap();
    assert!(size > 0 && torrent.exists());
    drop(prx); // progress was sent while hashing; the channel only has to have been usable
    let (_, _) = sw.until("seeding with a TCP port", 20, |rows, _, g, _| rows.first().is_some_and(|r| r.state == State::Seeding) && g.listen_port > 0);
    sw.wait_log("Created torrent \"share\"", 5);
    let seed_port = sw.until("port", 5, |_, _, g, _| g.listen_port > 0).0;
    let _ = seed_port;
    let port = ask(&seeder, Command::Bootstrap).globals.listen_port;

    let leecher = engine(leech_dir.path(), |_| {});
    let mut lw = Watch::new(&leecher);
    // File order follows the order the folder was read in, which is not sorted
    // everywhere (Linux): look the indices up rather than assume them.
    let info = ask(&leecher, |r| Command::Inspect { source: TorrentSource::Path(torrent.clone()), reply: r }).expect("inspects");
    let index = |name: &str| info.files.iter().find(|f| f.name == name).map(|f| f.index).unwrap_or_else(|| panic!("{name} in the torrent"));
    let (big, small) = (index("big.bin"), index("small.bin"));
    let id = added(ask(&leecher, |r| Command::Add {
        source: TorrentSource::Path(torrent.clone()),
        options: AddOptions { priorities: [(big, 1u8), (small, 0u8)].into_iter().collect(), wanted: Some(vec![big]), ..Default::default() },
        reply: Some(r),
    }));
    leecher.commands.send(Command::Details(Some(id.clone()))).unwrap();

    // Details: discovery rows first, then both tiers, with LSD shown as disabled.
    let (_, details) = lw.until("details with trackers and files", 20, |_, d, _, _| d.as_ref().is_some_and(|d| d.trackers.len() >= 4 && !d.files.is_empty()));
    let d = details.unwrap();
    assert_eq!(d.trackers[0].url, "[Local Peer Discovery]");
    assert_eq!(d.trackers[0].status, "Disabled");
    assert_eq!(d.trackers[1].url, "[Peer Exchange]");
    assert!(d.trackers.iter().any(|t| t.url == "http://127.0.0.1:2/announce"));
    assert_eq!(d.comment, "scan");
    assert!(d.files.iter().all(|f| !f.name.ends_with(".part")), "names shown without .part");
    assert_eq!(d.files.iter().find(|f| f.name == "small.bin").unwrap().priority, 0);

    // Only the wanted file downloads; the skipped one stays parked.
    // A peer is listed while the transfer runs; once the wanted file is done the
    // two sides have nothing left to trade and libtorrent lets the connection go,
    // so the peer is recorded when it is seen rather than required at the end.
    let mut asked = Instant::now() - Duration::from_secs(10);
    let mut seen_peer: Option<PeerRow> = None;
    let (rows, _) = lw.until("the wanted file complete, having seen a peer", 60, |rows, d, _, w| {
        if asked.elapsed() > Duration::from_secs(2) {
            let _ = w.h.commands.send(Command::AddPeer { id: id.clone(), address: format!("127.0.0.1:{port}") });
            asked = Instant::now();
        }
        // The latest snapshot: the first comes straight after the handshake,
        // before either side has said what it wants.
        if let Some(p) = d.as_ref().and_then(|d| d.peers.first()) {
            seen_peer = Some(p.clone());
        }
        rows.first().is_some_and(|r| r.done >= 1.0) && seen_peer.is_some()
    });
    let r = &rows[0];
    assert!(r.wanted_size < r.size, "wanted size excludes the skipped file");
    let peer = seen_peer.unwrap();
    assert!(peer.kind.starts_with("TCP"), "{peer:?}");
    assert!(!peer.client.is_empty());
    assert!(peer.flags.contains('D') || peer.flags.contains('I'), "flags describe the choke/interest state: {peer:?}");
    lw.wait_log("Added peer 127.0.0.1:", 5);
    lw.wait_log("finished downloading.", 10);
    lw.until("completion event", 5, |_, _, _, w| !w.completed.is_empty());
    let big_file = leech_dir.path().join("downloads/share/big.bin");
    assert!(big_file.exists(), "the wanted file was renamed from .part");
    assert!(!leech_dir.path().join("downloads/share/small.bin").exists(), "the skipped file was never finalised");

    // Resolving, saving the .torrent, sequential, trackers, reannounce.
    let resolved = ask(&leecher, |rep| Command::ResolvePath { id: id.clone(), file: Some(big), reply: rep }).expect("resolves");
    assert!(resolved.exists && resolved.target.starts_with(&resolved.save_path));
    let top = ask(&leecher, |rep| Command::ResolvePath { id: id.clone(), file: None, reply: rep }).unwrap();
    assert!(top.target.ends_with("share"));
    let copy = leech_dir.path().join("copy.torrent");
    ask(&leecher, |rep| Command::SaveTorrentFile { id: id.clone(), dest: copy.clone(), reply: rep }).unwrap();
    let info = ask(&leecher, |rep| Command::Inspect { source: TorrentSource::Path(copy.clone()), reply: rep }).unwrap();
    assert_eq!(info.name, "share");
    leecher.commands.send(Command::SetSequential { id: id.clone(), on: true }).unwrap();
    lw.until("sequential", 5, |rows, _, _, _| rows[0].sequential);
    leecher.commands.send(Command::AddTracker { id: id.clone(), url: "http://127.0.0.1:3/announce".into() }).unwrap();
    lw.wait_log("Added tracker http://127.0.0.1:3/announce to \"share\".", 5);
    leecher.commands.send(Command::Reannounce(id.clone())).unwrap();
    lw.wait_log("Re-announced \"share\" to its trackers.", 5);

    // Pause, start, recheck, stop, start again.
    leecher.commands.send(Command::Pause(vec![id.clone()])).unwrap();
    lw.until("paused", 5, |rows, _, _, _| rows[0].state == State::Paused);
    lw.wait_log("Paused \"share\".", 5);
    leecher.commands.send(Command::Start(vec![id.clone()])).unwrap();
    lw.until("running again", 10, |rows, _, _, _| rows[0].state == State::Seeding);
    leecher.commands.send(Command::Recheck(vec![id.clone()])).unwrap();
    lw.wait_log("Re-check of \"share\" complete", 20);
    leecher.commands.send(Command::Stop(vec![id.clone()])).unwrap();
    lw.until("stopped", 10, |rows, _, _, _| rows[0].state == State::Stopped && rows[0].num_peers == 0);
    leecher.commands.send(Command::Details(Some(id.clone()))).unwrap();
    let (_, d) = lw.until("stopped details keep what was known", 5, |_, d, _, _| d.is_some());
    let d = d.unwrap();
    assert!(d.pieces.is_some() && d.have > 0, "piece map kept from the snapshot");
    assert!(d.trackers.iter().all(|t| t.status == "Not contacted yet" || t.status == "Web Seed"));
    leecher.commands.send(Command::Start(vec![id.clone()])).unwrap();
    lw.until("seeding after stop/start", 20, |rows, _, _, _| rows[0].state == State::Seeding);

    // The seeder removes its copy with the data.
    let seed_id = ask(&seeder, Command::Bootstrap).rows[0].id.clone();
    seeder.commands.send(Command::Remove { ids: vec![seed_id], delete_data: true }).unwrap();
    sw.until("seeder empty", 10, |rows, _, _, _| rows.is_empty());
    sw.wait_log("Removed \"share\" and deleted its data.", 5);
    std::thread::sleep(Duration::from_millis(500));
    assert!(!seed_dir.path().join("share").exists(), "data deleted");

    shutdown(leecher);
    shutdown(seeder);
}

#[test]
fn queue_limits_move_and_restart_restore() {
    let dir = tempfile::tempdir().unwrap();
    let h = engine(dir.path(), |s| s.max_active_downloads = 1);
    let mut w = Watch::new(&h);
    // Magnets that will never find metadata: each stays a download for as long as it runs.
    let a = added(ask(&h, |r| Command::Add { source: TorrentSource::parse("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"), options: AddOptions::default(), reply: Some(r) }));
    let b = added(ask(&h, |r| Command::Add { source: TorrentSource::parse("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"), options: AddOptions::default(), reply: Some(r) }));
    let (rows, _) = w.until("one running, one queued", 10, |rows, _, _, _| {
        rows.len() == 2 && rows.iter().any(|r| r.state == State::Queued) && rows.iter().any(|r| r.state != State::Queued)
    });
    assert_eq!(rows.iter().find(|r| r.state == State::Queued).unwrap().id, b, "the later one waits");

    h.commands.send(Command::MoveQueue { id: b.clone(), delta: -1 }).unwrap();
    w.until("b moved first", 5, |rows, _, _, _| rows[0].id == b);
    h.commands.send(Command::Stop(vec![a.clone()])).unwrap();
    w.until("stopping a releases the slot", 10, |rows, _, _, _| {
        let (ra, rb) = (rows.iter().find(|r| r.id == a).unwrap(), rows.iter().find(|r| r.id == b).unwrap());
        ra.state == State::Stopped && rb.state != State::Queued
    });
    h.commands.send(Command::Pause(vec![b.clone()])).unwrap();
    w.until("b paused", 5, |rows, _, _, _| rows.iter().any(|r| r.id == b && r.state == State::Paused));
    shutdown(h);

    // A restart brings back exactly what was left: a stopped and a paused torrent, nothing started.
    let h = engine(dir.path(), |s| s.max_active_downloads = 1);
    let mut w = Watch::new(&h);
    let (rows, _) = w.until("restored", 5, |rows, _, _, _| rows.len() == 2);
    assert_eq!(rows.iter().find(|r| r.id == a).unwrap().state, State::Stopped);
    assert_eq!(rows.iter().find(|r| r.id == b).unwrap().state, State::Paused);
    assert!(w.logged("Restored 2 torrent(s) from the previous session."));
    shutdown(h);
}

#[test]
fn seed_time_goal_pauses_a_finished_torrent() {
    let dir = tempfile::tempdir().unwrap();
    let data = share(dir.path());
    let torrent = dir.path().join("share.torrent");
    {
        let h = engine(dir.path(), |_| {});
        ask(&h, |r| Command::CreateTorrent {
            options: CreateTorrentOptions { input_path: data, output_path: torrent.clone(), piece_length: 65536, start_seeding: true, ..Default::default() },
            progress: None,
            reply: r,
        })
        .unwrap();
        let mut w = Watch::new(&h);
        w.until("seeding", 20, |rows, _, _, _| rows.first().is_some_and(|r| r.state == State::Seeding));
        shutdown(h);
    }
    // Pretend it finished long ago, and set a one-minute seeding goal.
    let mut saved = state_file(dir.path());
    saved["torrents"][0]["completedOn"] = 1.into();
    saved["torrents"][0]["state"] = "seeding".into();
    std::fs::write(dir.path().join("ztorrent-state.json"), saved.to_string()).unwrap();
    let h = engine(dir.path(), |s| s.seed_time_limit = 1);
    let mut w = Watch::new(&h);
    w.wait_log("reached its seeding time goal; seeding stopped.", 30);
    w.until("paused by the goal", 5, |rows, _, _, _| rows[0].state == State::Paused);
    shutdown(h);
}

#[test]
fn remove_refuses_to_delete_outside_the_save_folder() {
    let dir = tempfile::tempdir().unwrap();
    let downloads = dir.path().join("downloads");
    std::fs::create_dir_all(&downloads).unwrap();
    std::fs::write(dir.path().join("precious.txt"), b"keep me").unwrap();
    let state = serde_json::json!({
        "settings": { "downloadPath": downloads, "enableDHT": false, "enableUPnP": false },
        "torrents": [{ "id": "evil", "name": "../precious.txt", "savePath": downloads, "state": "stopped" }]
    });
    std::fs::write(dir.path().join("ztorrent-state.json"), state.to_string()).unwrap();
    let h = spawn(Store::open(dir.path(), None), EngineOptions { data_dir: dir.path().to_path_buf(), version: "0.5.0".into() }).unwrap();
    let mut w = Watch::new(&h);
    h.commands.send(Command::Remove { ids: vec!["evil".into()], delete_data: true }).unwrap();
    w.wait_log("Refusing to delete data for \"../precious.txt\"", 5);
    assert!(dir.path().join("precious.txt").exists(), "the file outside the save folder survived");
    assert!(ask(&h, |r| Command::ResolvePath { id: "evil".into(), file: None, reply: r }).is_none());
    shutdown(h);
}

#[test]
fn udp_trackers_are_refused_while_traffic_is_routed() {
    let dir = tempfile::tempdir().unwrap();
    let h = engine(dir.path(), |s| s.bind_interface = "utun-not-here".into());
    let mut w = Watch::new(&h);
    w.wait_log("has no address -- refusing to connect -- every connection is held", 5);
    let id = added(ask(&h, |r| Command::Add { source: TorrentSource::Bytes(sintel()), options: AddOptions::default(), reply: Some(r) }));
    h.commands.send(Command::Details(Some(id.clone()))).unwrap();
    let (_, d) = w.until("details", 10, |_, d, _, _| d.as_ref().is_some_and(|d| !d.trackers.is_empty()));
    assert!(d.unwrap().trackers.iter().all(|t| !t.url.to_lowercase().starts_with("udp:")), "udp:// trackers stripped on add");
    h.commands.send(Command::AddTracker { id, url: "udp://tracker.example:1337/announce".into() }).unwrap();
    w.wait_log("Not adding udp://tracker.example:1337/announce", 5);
    let (rows, _) = w.until("held while the interface is gone", 10, |rows, _, _, _| rows.first().is_some_and(|r| r.download_speed == 0.0 && r.num_peers == 0));
    assert_eq!(rows.len(), 1);
    shutdown(h);
}
