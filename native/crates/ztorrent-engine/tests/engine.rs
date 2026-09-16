//! End-to-end tests of the engine thread: real libtorrent sessions on loopback.

mod support;

use std::path::Path;
use std::time::{Duration, Instant};
use ztorrent_core::command::*;
use ztorrent_core::settings::{Settings, patch};
use ztorrent_core::*;
use ztorrent_engine::{EngineHandle, EngineOptions, spawn};

/// A quiet engine: no DHT, LSD or port mapping, so a test never reaches past loopback.
fn engine(dir: &Path, tweak: impl FnOnce(&mut Settings)) -> EngineHandle {
    let mut store = Store::open(dir, None);
    let mut s = store.settings().clone();
    s.enable_dht = false;
    s.enable_lsd = false;
    s.enable_upnp = false;
    s.enable_utp = false;
    s.download_path = dir.join("downloads").to_string_lossy().into_owned();
    tweak(&mut s);
    store.data.settings = s;
    spawn(store, EngineOptions { data_dir: dir.to_path_buf(), version: "0.5.0".into() }).expect("engine starts")
}

fn rows_until(h: &EngineHandle, timeout: Duration, mut done: impl FnMut(&[Row], &Globals) -> bool) -> Option<(Vec<Row>, Globals)> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Ok(Event::Tick { rows, globals, .. }) = h.events.recv_timeout(Duration::from_millis(200)) {
            if done(&rows, &globals) {
                return Some((rows, globals));
            }
        }
    }
    None
}

fn ask<T>(h: &EngineHandle, make: impl FnOnce(Reply<T>) -> Command) -> T {
    let (tx, rx) = futures_channel::oneshot::channel();
    h.commands.send(make(tx)).unwrap();
    futures_executor_block(rx)
}

fn futures_executor_block<T>(mut rx: futures_channel::oneshot::Receiver<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        match rx.try_recv() {
            Ok(Some(v)) => return v,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => panic!("no reply"),
        }
    }
}

fn shutdown(h: EngineHandle) {
    let _ = ask(&h, Command::Shutdown);
    let _ = h.thread.join();
}

fn sample() -> Vec<u8> {
    std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../sample-torrents/sintel.torrent")).unwrap()
}

#[test]
fn add_stop_start_and_remove_keep_the_state_file_in_step() {
    let dir = tempfile::tempdir().unwrap();
    let h = engine(dir.path(), |_| {});
    let info = ask(&h, |r| Command::Inspect { source: TorrentSource::Bytes(sample()), reply: r }).unwrap();
    assert_eq!(info.name, "Sintel");
    assert_eq!(info.files.len(), 11);

    let options = AddOptions { label: "movie".into(), paused: true, ..Default::default() };
    let added = ask(&h, |r| Command::Add { source: TorrentSource::Bytes(sample()), options, reply: Some(r) }).unwrap();
    let AddOutcome::Added(id) = added else { panic!("expected a new torrent") };
    let again = ask(&h, |r| Command::Add { source: TorrentSource::Bytes(sample()), options: AddOptions::default(), reply: Some(r) });
    assert_eq!(again, Ok(AddOutcome::Duplicate(id.clone())), "same info hash is a duplicate");

    let (rows, _) = rows_until(&h, Duration::from_secs(5), |rows, _| rows.len() == 1).expect("row appears");
    assert_eq!(rows[0].state, State::Paused);
    assert_eq!(rows[0].label, "movie");

    h.commands.send(Command::Start(vec![id.clone()])).unwrap();
    rows_until(&h, Duration::from_secs(10), |rows, _| matches!(rows[0].state, State::Downloading | State::Checking | State::Metadata))
        .expect("starts");
    h.commands.send(Command::Stop(vec![id.clone()])).unwrap();
    let (rows, _) = rows_until(&h, Duration::from_secs(10), |rows, _| rows[0].state == State::Stopped).expect("stops");
    assert_eq!(rows[0].num_peers, 0);

    shutdown(h);
    let saved: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.path().join("ztorrent-state.json")).unwrap()).unwrap();
    assert_eq!(saved["torrents"][0]["state"], "stopped");
    assert_eq!(saved["torrents"][0]["infoHash"], "08ada5a7a6183aae1e09d831df6748d566095a10");
    assert_eq!(saved["labels"][0], "movie");

    // A restart keeps it stopped, then removing it empties the list.
    let h = engine(dir.path(), |_| {});
    let (rows, _) = rows_until(&h, Duration::from_secs(5), |rows, _| rows.len() == 1).unwrap();
    assert_eq!(rows[0].state, State::Stopped);
    h.commands.send(Command::Remove { ids: vec![id], delete_data: true }).unwrap();
    rows_until(&h, Duration::from_secs(5), |rows, _| rows.is_empty()).expect("removed");
    shutdown(h);
}

/// Seeds a small torrent from one engine and downloads it with another over
/// loopback, optionally through a SOCKS5 proxy. Returns the downloading engine's
/// directory and whether the download completed.
fn transfer(through_proxy: Option<(u16, &'static str, &'static str)>, wait: Duration) -> (tempfile::TempDir, bool) {
    let seed_dir = tempfile::tempdir().unwrap();
    let leech_dir = tempfile::tempdir().unwrap();
    let share = seed_dir.path().join("share");
    std::fs::create_dir_all(&share).unwrap();
    std::fs::write(share.join("data.bin"), (0..3_000_000u32).map(|i| (i % 251) as u8).collect::<Vec<_>>()).unwrap();

    let seeder = engine(seed_dir.path(), |s| {
        s.listen_port = 0;
        s.randomize_port = true;
    });
    let torrent_path = seed_dir.path().join("share.torrent");
    let options = CreateTorrentOptions {
        input_path: share.clone(),
        output_path: torrent_path.clone(),
        trackers: vec![],
        piece_length: 65536,
        start_seeding: true,
        ..Default::default()
    };
    ask(&seeder, |r| Command::CreateTorrent { options, progress: None, reply: r }).expect("created");
    let (_, globals) = rows_until(&seeder, Duration::from_secs(20), |rows, g| {
        rows.len() == 1 && rows[0].state == State::Seeding && g.listen_port > 0
    })
    .expect("seeding");
    let port = globals.listen_port;

    let leecher = engine(leech_dir.path(), |s| {
        if let Some((proxy_port, user, pass)) = through_proxy {
            s.proxy_enabled = true;
            s.proxy_host = "127.0.0.1".into();
            s.proxy_port = proxy_port as u64;
            s.proxy_username = user.into();
            s.proxy_password = pass.into();
        }
    });
    let bytes = std::fs::read(&torrent_path).unwrap();
    let AddOutcome::Added(id) =
        ask(&leecher, |r| Command::Add { source: TorrentSource::Bytes(bytes), options: AddOptions::default(), reply: Some(r) }).unwrap()
    else {
        panic!()
    };
    let mut completed = false;
    let deadline = Instant::now() + wait;
    let mut asked = Instant::now() - Duration::from_secs(10);
    while Instant::now() < deadline {
        if asked.elapsed() > Duration::from_secs(2) {
            leecher.commands.send(Command::AddPeer { id: id.clone(), address: format!("127.0.0.1:{port}") }).unwrap();
            asked = Instant::now();
        }
        match leecher.events.recv_timeout(Duration::from_millis(200)) {
            Ok(Event::Complete { .. }) => {
                completed = true;
                break;
            }
            _ => {}
        }
    }
    if completed {
        // Give the rename a moment to be persisted before shutting down.
        std::thread::sleep(Duration::from_millis(300));
    }
    shutdown(leecher);
    shutdown(seeder);
    (leech_dir, completed)
}

#[test]
fn downloads_over_loopback_and_drops_the_part_suffix_when_done() {
    let (dir, completed) = transfer(None, Duration::from_secs(60));
    assert!(completed, "the download completed");
    let file = dir.path().join("downloads/share/data.bin");
    assert!(file.exists(), "final name present");
    assert!(!dir.path().join("downloads/share/data.bin.part").exists(), ".part renamed away");
    assert_eq!(std::fs::metadata(file).unwrap().len(), 3_000_000);
}

#[test]
fn peer_connections_go_through_the_proxy_with_auth() {
    let socks = support::start_socks("zaf", "hunter2");
    let (_, completed) = transfer(Some((socks.port, "zaf", "hunter2")), Duration::from_secs(60));
    assert!(completed, "downloaded through the proxy");
    let stats = socks.stats.lock().unwrap();
    assert!(stats.connects.iter().any(|c| c.starts_with("127.0.0.1:")), "proxy saw the peer CONNECT: {:?}", stats.connects);
    assert!(stats.auths >= stats.connects.len(), "auth on every connection");
}

#[test]
fn a_dead_proxy_fails_instead_of_connecting_directly() {
    let (dir, completed) = transfer(Some((1, "zaf", "hunter2")), Duration::from_secs(12));
    assert!(!completed, "LEAKED: downloaded without the proxy");
    assert!(!dir.path().join("downloads/share/data.bin").exists());
}

#[test]
fn a_proxy_closes_the_inbound_listeners() {
    let dir = tempfile::tempdir().unwrap();
    let h = engine(dir.path(), |s| {
        s.proxy_enabled = true;
        s.proxy_host = "127.0.0.1".into();
        s.proxy_port = 1080;
    });
    // Give libtorrent time to open anything it was going to open.
    std::thread::sleep(Duration::from_secs(3));
    let (_, globals) = rows_until(&h, Duration::from_secs(5), |_, _| true).unwrap();
    // As scripts/test-inbound.mjs asserted: nothing accepts TCP. libtorrent does
    // keep one UDP socket for its own outgoing use, with incoming uTP refused.
    assert_eq!(globals.listen_port, 0, "no TCP listener was opened under a proxy");
    assert!(!globals.dht_enabled);
    let log = ask(&h, Command::Bootstrap).log;
    assert!(log.iter().any(|l| l.message == "Inbound listeners closed: with a proxy, connections are outgoing only."));
    assert!(log.iter().any(|l| l.message.starts_with("DHT, local discovery, uTP, port mapping, udp:// trackers are off")));
    shutdown(h);
}

#[test]
fn fetch_goes_through_the_proxy_and_resolves_names_there() {
    use std::io::{Read, Write};
    let web = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let web_port = web.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for mut c in web.incoming().flatten() {
            let mut buf = [0u8; 1024];
            let _ = c.read(&mut buf);
            let _ = c.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\nConnection: close\r\n\r\nannounce-ok");
        }
    });
    let socks = support::start_socks("zaf", "hunter2");
    let policy = ztorrent_core::egress::EgressPolicy {
        proxy: Some(ztorrent_core::egress::ProxyConfig {
            host: "127.0.0.1".into(),
            port: socks.port,
            username: Some("zaf".into()),
            password: "hunter2".into(),
        }),
        bind: None,
    };
    let body = ztorrent_engine::fetch_torrent(&format!("http://127.0.0.1:{web_port}/a.torrent"), Some(&policy), "test").unwrap();
    assert_eq!(body, b"announce-ok");
    let _ = ztorrent_engine::fetch_torrent("http://tracker.invalid/announce", Some(&policy), "test");
    let stats = socks.stats.lock().unwrap();
    assert!(stats.connects.iter().any(|c| c == &format!("127.0.0.1:{web_port}")));
    assert!(stats.connects.iter().any(|c| c.starts_with("tracker.invalid (name):80")), "hostname sent to the proxy: {:?}", stats.connects);

    let dead = ztorrent_core::egress::EgressPolicy { proxy: Some(ztorrent_core::egress::ProxyConfig { port: 1, ..policy.proxy.clone().unwrap() }), bind: None };
    assert!(ztorrent_engine::fetch_torrent(&format!("http://127.0.0.1:{web_port}/leak"), Some(&dead), "test").is_err());
    let gone = ztorrent_core::egress::EgressPolicy { proxy: None, bind: Some("utun-does-not-exist".into()) };
    let err = ztorrent_engine::fetch_torrent(&format!("http://127.0.0.1:{web_port}/leak"), Some(&gone), "test").unwrap_err();
    assert!(err.contains("has no address -- refusing to connect"), "{err}");
    let _ = patch("x", 1);
}
