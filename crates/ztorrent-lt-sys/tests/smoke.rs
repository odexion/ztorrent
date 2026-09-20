use std::sync::{Arc, Mutex};
use ztorrent_lt_sys::*;

fn sample(name: &str) -> Vec<u8> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../sample-torrents/");
    std::fs::read(format!("{path}{name}")).expect("sample torrent")
}

#[test]
fn reads_a_sample_torrent_like_parse_torrent_did() {
    let meta = inspect_buffer(&sample("sintel.torrent")).expect("parses");
    assert_eq!(meta.name, "Sintel");
    assert_eq!(meta.info_hash, "08ada5a7a6183aae1e09d831df6748d566095a10");
    assert_eq!(meta.files.len(), 11);
    assert!(meta.trackers.len() > 0);
    assert!(inspect_buffer(b"not a torrent").is_err(), "garbage is an error, not a crash");
}

#[test]
fn reads_a_magnet() {
    let m = inspect_magnet("magnet:?xt=urn:btih:08ada5a7a6183aae1e09d831df6748d566095a10&dn=Sintel&tr=udp%3A%2F%2Fexplodie.org%3A6969").unwrap();
    assert_eq!(m.info_hash, "08ada5a7a6183aae1e09d831df6748d566095a10");
    assert_eq!(m.name, "Sintel");
    assert_eq!(m.trackers.len(), 1);
}

#[test]
fn creates_a_v1_torrent_and_reports_progress() {
    let dir = std::env::temp_dir().join(format!("ztlt-create-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("share")).unwrap();
    std::fs::write(dir.join("share/a.bin"), vec![7u8; 300_000]).unwrap();
    std::fs::write(dir.join("share/.hidden"), b"skip me").unwrap();
    let seen = Arc::new(Mutex::new(0));
    let seen2 = seen.clone();
    let sink = ProgressSink(Box::new(move |done, _| *seen2.lock().unwrap() = done));
    let params = CreateParams {
        input: dir.join("share").to_string_lossy().into_owned(),
        trackers: vec!["https://tracker.example/announce".into()],
        tiers: vec![0],
        comment: "made in a test".into(),
        creator: "ztorrent test".into(),
        piece_size: 65536,
        ..Default::default()
    };
    let bytes = create_torrent(&params, &sink).expect("created");
    let meta = inspect_buffer(&bytes).unwrap();
    assert_eq!(meta.name, "share");
    assert_eq!(meta.files.iter().filter(|f| !f.pad).count(), 1, "hidden file left out");
    assert_eq!(meta.comment, "made in a test");
    assert_eq!(meta.info_hash.len(), 40, "v1 info hash");
    assert!(*seen.lock().unwrap() > 0);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn a_session_starts_adds_and_removes() {
    let cfg = SessionConfig {
        listen_interfaces: "127.0.0.1:0".into(),
        enable_incoming_tcp: true,
        encryption: 1,
        user_agent: "ztorrent/test".into(),
        ..Default::default()
    };
    let mut session = new_session(&cfg).expect("session");
    let dir = std::env::temp_dir().join(format!("ztlt-add-{}", std::process::id()));
    let params = AddParams {
        torrent: sample("sintel.torrent"),
        save_path: dir.to_string_lossy().into_owned(),
        paused: true,
        strip_udp_trackers: true,
        ..Default::default()
    };
    let h = session.pin_mut().add(&params).expect("added");
    assert!(h > 0);
    let st = session.status(h);
    assert!(st.valid && st.has_metadata && st.paused);
    assert_eq!(session.files(h).len(), 11);
    assert!(session.trackers(h).iter().all(|t| !t.url.to_lowercase().starts_with("udp:")), "udp:// stripped");
    assert!(session.torrent_bytes(h).len() > 1000);
    assert!(session.pin_mut().remove(h, false));
    assert!(!session.status(h).valid);
}
