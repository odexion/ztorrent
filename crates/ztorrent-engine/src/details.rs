//! What the window is sent: the list rows, the status-bar totals, and the
//! detail tabs for the one selected torrent.

use crate::Engine;
use crate::torrent::{Torrent, strip_part};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use ztorrent_core::egress::EgressPolicy;
use ztorrent_core::*;

/// Bytes sent per byte received. A torrent seeded from data it already had has
/// downloaded nothing, so its own size stands in as the denominator -- otherwise
/// the ratio would be pinned at zero and seeding goals could never be met.
pub fn share_ratio(downloaded: u64, uploaded: u64, length: u64) -> f64 {
    let base = if downloaded > 0 { downloaded } else { length };
    if base == 0 { 0.0 } else { uploaded as f64 / base as f64 }
}

/// Packs a piece map (2 = have) into the state file's bitfield: one bit per
/// piece, most significant first.
pub fn pack_bitfield(map: &[u8]) -> String {
    let mut bytes = vec![0u8; map.len().div_ceil(8)];
    for (i, v) in map.iter().enumerate() {
        if *v == 2 {
            bytes[i >> 3] |= 0x80 >> (i % 8);
        }
    }
    B64.encode(bytes)
}

pub fn unpack_bitfield(b64: &str, count: usize) -> Option<Vec<u8>> {
    let bytes = B64.decode(b64).ok()?;
    Some((0..count).map(|i| if bytes.get(i >> 3).is_some_and(|b| b & (0x80 >> (i % 8)) != 0) { 2 } else { 0 }).collect())
}

/// A tracker announces on every endpoint it resolves to -- IPv4 and IPv6,
/// typically -- and one of them failing does not make the tracker broken while
/// another answers.
pub fn tracker_status(t: &ztorrent_lt_sys::TrackerInfo) -> &'static str {
    if t.updating {
        "Announcing…"
    } else if t.working {
        "Working"
    } else if t.failed {
        "Not working"
    } else if t.contacted {
        "Working"
    } else {
        "Not contacted yet"
    }
}

fn clean_client(s: &str) -> String {
    let c: String = s.chars().filter(|c| !c.is_control()).collect();
    let c = c.trim();
    if c.is_empty() { "Unknown".into() } else { c.to_string() }
}

fn tracker(url: &str, status: &str) -> TrackerRow {
    TrackerRow { url: url.into(), status: status.into(), seeds: -1, peers: -1, interval: 0 }
}

impl Engine {
    pub(crate) fn row(&self, t: &Torrent) -> Row {
        let s = &t.status;
        let live = t.live();
        let downloaded = t.downloaded();
        let uploaded = t.uploaded();
        let length = t.length();
        let down = if live { s.download_rate as f64 } else { 0.0 };
        let eta = if live && down > 0.0 {
            (s.total_wanted - s.total_wanted_done).max(0) as f64 / down * 1000.0
        } else {
            f64::INFINITY
        };
        Row {
            id: t.rec.id.clone(),
            info_hash: t.rec.info_hash.clone().unwrap_or_default(),
            name: t.rec.name.clone(),
            order: t.order(),
            state: t.state(),
            error: t.error_text(),
            label: t.rec.label.clone(),
            save_path: t.rec.save_path.clone(),
            size: length,
            wanted_size: t.wanted_size(),
            done: t.done(),
            downloaded,
            uploaded,
            download_speed: down,
            upload_speed: if live { s.upload_rate as f64 } else { 0.0 },
            num_peers: if live { s.num_peers.max(0) as u32 } else { 0 },
            seeds: if live { s.num_seeds.max(0) as u32 } else { 0 },
            peers: if live { (s.num_peers - s.num_seeds).max(0) as u32 } else { 0 },
            eta,
            ratio: share_ratio(downloaded, uploaded, length),
            availability: if live {
                if s.distributed_copies < 0.0 { t.done() } else { s.distributed_copies as f64 }
            } else {
                0.0
            },
            added_on: t.rec.added_on,
            completed_on: t.rec.completed_on,
            sequential: t.rec.sequential,
            magnet_uri: t.rec.magnet_uri.clone(),
        }
    }

    pub(crate) fn rows(&self) -> Vec<Row> {
        let mut rows: Vec<Row> = self.torrents.iter().map(|t| self.row(t)).collect();
        rows.sort_by_key(|r| r.order);
        rows
    }

    pub(crate) fn globals(&self) -> Globals {
        let mut g = Globals::default();
        for t in &self.torrents {
            if t.live() {
                g.download_speed += t.status.download_rate.max(0) as f64;
                g.upload_speed += t.status.upload_rate.max(0) as f64;
            }
            // WebTorrent had no cumulative counters, so the totals were summed from
            // the records; the same arithmetic keeps the numbers the same.
            g.downloaded += t.downloaded();
            g.uploaded += t.uploaded();
        }
        g.ratio = if g.downloaded > 0 { g.uploaded as f64 / g.downloaded as f64 } else { 0.0 };
        let guarded = EgressPolicy::guarded(self.policy.as_ref());
        g.dht_enabled = self.started.enable_dht && !guarded.dht;
        g.dht_nodes = self.session.dht_nodes().max(0) as u64;
        g.dht_ready = g.dht_enabled && self.session.dht_running() && g.dht_nodes > 0;
        // Only a socket libtorrent reported opening counts; under a proxy there are none.
        g.listen_port = self.listen_port;
        g.torrent_count = self.torrents.len();
        g.alt_speed = self.store.settings().alt_speed_enabled;
        g
    }

    pub(crate) fn details(&self, id: &str) -> Option<Details> {
        let t = self.torrents.iter().find(|t| t.id() == id)?;
        let meta = t.rec.meta.clone().unwrap_or_default();
        let mut d = Details {
            id: t.rec.id.clone(),
            name: t.rec.name.clone(),
            info_hash: t.rec.info_hash.clone().unwrap_or_default(),
            save_path: t.rec.save_path.clone(),
            comment: meta.comment,
            created_by: meta.created_by,
            created_on: meta.created_on,
            piece_length: t.rec.piece_length,
            piece_count: t.rec.piece_count,
            private: meta.private,
            sequential: t.rec.sequential,
            ..Default::default()
        };

        let Some(h) = t.handle.filter(|_| t.live()) else {
            // The URLs are known even with the swarm down; only their state is not.
            for url in t.rec.announce.iter().flatten() {
                d.trackers.push(tracker(url, "Not contacted yet"));
            }
            for url in t.rec.web_seeds.iter().flatten() {
                d.trackers.push(tracker(url, "Web Seed"));
            }
            if let Some(files) = &t.rec.files {
                d.files = files
                    .iter()
                    .enumerate()
                    .map(|(index, f)| FileRow {
                        index,
                        name: f.name.clone(),
                        path: f.path.clone(),
                        length: f.length,
                        downloaded: f.downloaded,
                        progress: f.progress,
                        priority: t.priority(index),
                    })
                    .collect();
            }
            if let Some(bits) = &t.rec.bitfield {
                if let Some(map) = unpack_bitfield(bits, t.rec.piece_count as usize) {
                    d.have = map.iter().filter(|v| **v == 2).count() as u64;
                    d.pieces = Some(map);
                }
            }
            return Some(d);
        };

        let m = self.session.meta(h);
        if m.has_metadata {
            d.piece_length = m.piece_length.max(0) as u64;
            d.piece_count = m.num_pieces.max(0) as u64;
            d.private = m.private_flag;
            if d.comment.is_empty() {
                d.comment = m.comment.clone();
            }
            if d.created_by.is_empty() {
                d.created_by = m.creator.clone();
            }
            if d.created_on == 0 && m.created > 0 {
                d.created_on = m.created * 1000;
            }
        }

        let guarded = EgressPolicy::guarded(self.policy.as_ref());
        let lsd = self.started.enable_lsd && !guarded.lsd && !d.private;
        let pex = self.started.enable_pex && !d.private;
        d.trackers.push(tracker("[Local Peer Discovery]", if lsd { "Working" } else { "Disabled" }));
        d.trackers.push(tracker("[Peer Exchange]", if pex { "Working" } else { "Disabled" }));
        if self.session.dht_running() && !d.private {
            let nodes = self.session.dht_nodes().max(0);
            d.trackers.push(TrackerRow {
                url: "[DHT]".into(),
                status: if nodes > 0 { "Working" } else { "Bootstrapping" }.into(),
                seeds: -1,
                peers: nodes,
                interval: 0,
            });
        }
        for tr in self.session.trackers(h) {
            let status = tracker_status(&tr);
            d.trackers.push(TrackerRow {
                url: tr.url.clone(),
                status: status.into(),
                seeds: tr.complete as i64,
                peers: tr.incomplete as i64,
                interval: tr.next_announce_secs.max(0) as u64,
            });
        }
        for url in &m.url_seeds {
            d.trackers.push(tracker(url, "Web Seed"));
        }

        for p in self.session.peers(h) {
            let mut flags = String::new();
            if !p.remote_choked {
                flags.push('D');
            }
            if p.remote_interested {
                flags.push('U');
            }
            if p.choked {
                flags.push('C');
            }
            if p.interesting {
                flags.push('I');
            }
            if p.web_seed {
                flags.push('W');
            }
            if p.utp {
                flags.push('P');
            }
            let dir = if p.outgoing { "out" } else { "in" };
            d.peers.push(PeerRow {
                address: p.ip.clone(),
                port: p.port,
                client: if p.web_seed { "Web Seed".into() } else { clean_client(&p.client) },
                flags,
                progress: p.progress as f64,
                down_speed: p.down_rate.max(0) as f64,
                up_speed: p.up_rate.max(0) as f64,
                downloaded: p.total_download.max(0) as u64,
                uploaded: p.total_upload.max(0) as u64,
                kind: if p.web_seed {
                    "Web Seed".into()
                } else if p.utp {
                    format!("µTP {dir}")
                } else {
                    format!("TCP {dir}")
                },
            });
        }

        for (index, f) in self.session.files(h).iter().enumerate() {
            if f.pad {
                continue;
            }
            let size = f.size.max(0) as u64;
            d.files.push(FileRow {
                index,
                name: strip_part(&f.name).to_string(),
                path: strip_part(&f.path).to_string(),
                length: size,
                downloaded: f.downloaded.max(0) as u64,
                progress: if size > 0 { f.downloaded.max(0) as f64 / size as f64 } else { 1.0 },
                priority: t.priority(index),
            });
        }

        let map = self.session.piece_map(h);
        if !map.is_empty() {
            d.have = map.iter().filter(|v| **v == 2).count() as u64;
            d.pieces = Some(map);
        }
        Some(d)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bitfield_round_trips_msb_first() {
        let map = [2, 0, 0, 0, 0, 0, 0, 0, 2, 2];
        let packed = pack_bitfield(&map);
        assert_eq!(B64.decode(&packed).unwrap(), vec![0b1000_0000, 0b1100_0000]);
        assert_eq!(unpack_bitfield(&packed, 10).unwrap(), map);
    }

    #[test]
    fn a_tracker_with_one_working_endpoint_is_working() {
        use ztorrent_lt_sys::TrackerInfo;
        let t = |working, failed, contacted, updating| TrackerInfo { working, failed, contacted, updating, ..Default::default() };
        assert_eq!(tracker_status(&t(true, true, true, false)), "Working", "IPv4 answers, IPv6 fails");
        assert_eq!(tracker_status(&t(false, true, true, false)), "Not working");
        assert_eq!(tracker_status(&t(false, false, false, true)), "Announcing…");
        assert_eq!(tracker_status(&t(false, false, false, false)), "Not contacted yet");
    }

    #[test]
    fn ratio_uses_size_when_nothing_was_downloaded() {
        assert_eq!(share_ratio(0, 50, 100), 0.5);
        assert_eq!(share_ratio(200, 50, 100), 0.25);
        assert_eq!(share_ratio(0, 50, 0), 0.0);
    }
}
