//! A torrent as the engine holds it: the persisted record, plus what only a
//! running session knows.

use crate::paths::safe_join;
use std::path::Path;
use ztorrent_core::{State, TorrentRecord};
use ztorrent_lt_sys::{FileEntry, Status};

pub const PART_SUFFIX: &str = ".part";

/// Where one file is being written: its final name, and whether it is still
/// parked under `<name>.part`.
#[derive(Clone, Debug, PartialEq)]
pub struct PartEntry {
    pub index: usize,
    pub rel: String,
    pub part: bool,
}

pub struct Torrent {
    pub rec: TorrentRecord,
    /// The session handle, while the torrent is in the session.
    pub handle: Option<u64>,
    pub want_start: bool,
    pub error: Option<String>,
    pub plan: Vec<PartEntry>,
    pub pending_renames: usize,
    pub renamed: usize,
    pub rename_error: Option<String>,
    /// Stopped by the user; waiting for resume data before leaving the session.
    pub stopping: bool,
    pub rechecking: bool,
    /// all_time counters as the session reported them when this run began, so
    /// only what this run added is folded onto the record's bases.
    pub up_at_spawn: Option<i64>,
    pub down_at_spawn: Option<i64>,
    pub status: Status,
}

impl Torrent {
    pub fn new(rec: TorrentRecord) -> Torrent {
        Torrent {
            rec,
            handle: None,
            want_start: false,
            error: None,
            plan: Vec::new(),
            pending_renames: 0,
            renamed: 0,
            rename_error: None,
            stopping: false,
            rechecking: false,
            up_at_spawn: None,
            down_at_spawn: None,
            status: Status::default(),
        }
    }

    pub fn id(&self) -> &str {
        &self.rec.id
    }

    pub fn order(&self) -> i64 {
        self.rec.order.unwrap_or(0)
    }

    pub fn live(&self) -> bool {
        self.handle.is_some() && !self.stopping && self.status.valid
    }

    pub fn uploaded(&self) -> u64 {
        let run = match (self.live(), self.up_at_spawn) {
            (true, Some(start)) => (self.status.all_time_upload - start).max(0) as u64,
            _ => 0,
        };
        self.rec.uploaded_base + run
    }

    pub fn downloaded(&self) -> u64 {
        let run = match (self.live(), self.down_at_spawn) {
            (true, Some(start)) => (self.status.all_time_download - start).max(0) as u64,
            _ => 0,
        };
        self.rec.downloaded_base + run
    }

    pub fn length(&self) -> u64 {
        if self.live() && self.status.total > 0 { self.status.total as u64 } else { self.rec.length }
    }

    pub fn done(&self) -> f64 {
        if self.live() && self.status.has_metadata { self.status.progress } else { self.rec.progress }
    }

    /// The state the list shows, derived from the session while running.
    pub fn state(&self) -> State {
        use ztorrent_lt_sys::state::*;
        if self.handle.is_none() || self.stopping || !self.status.valid {
            return self.rec.state;
        }
        let s = &self.status;
        if !s.error.is_empty() {
            return State::Error;
        }
        // Paused is read before metadata: a magnet paused while it is still looking
        // for its metadata is paused, and must say so rather than "Downloading
        // metadata" (which the Electron build showed).
        if s.paused && !self.rechecking {
            return State::Paused;
        }
        if self.rechecking || s.state == CHECKING_FILES || s.state == CHECKING_RESUME_DATA {
            return State::Checking;
        }
        if !s.has_metadata {
            return State::Metadata;
        }
        if s.progress >= 1.0 || s.state == FINISHED || s.state == SEEDING {
            State::Seeding
        } else {
            State::Downloading
        }
    }

    pub fn error_text(&self) -> Option<String> {
        if self.handle.is_some() && !self.status.error.is_empty() {
            Some(self.status.error.clone())
        } else {
            self.error.clone()
        }
    }

    pub fn priority(&self, index: usize) -> u8 {
        self.rec.priorities.get(&index).copied().unwrap_or(1)
    }

    /// Bytes the user asked for: every file not set to Don't Download.
    pub fn wanted_size(&self) -> u64 {
        match &self.rec.files {
            Some(files) if !files.is_empty() => files
                .iter()
                .enumerate()
                .filter(|(i, _)| self.priority(*i) != 0)
                .map(|(_, f)| f.length)
                .sum(),
            _ => self.length(),
        }
    }
}

/// Removes a trailing `.part` the store added, so names read as the torrent
/// gave them.
pub fn strip_part(path: &str) -> &str {
    path.strip_suffix(PART_SUFFIX).unwrap_or(path)
}

/// Which name each file uses is decided per file by what is already on disk:
///
///   final exists   -> use it            (completed, or re-added later)
///   .part exists   -> use it            (resume it, even with the setting off)
///   neither        -> the setting decides
///
/// Resuming an existing .part regardless of the setting is what stops a
/// mid-download toggle from orphaning data and fetching it again.
pub fn plan_parts(save_path: &Path, files: &[FileEntry], use_part: bool) -> Vec<PartEntry> {
    files
        .iter()
        .enumerate()
        .filter(|(_, f)| !f.pad)
        .filter_map(|(index, f)| {
            let rel = strip_part(&f.path).to_string();
            let final_abs = safe_join(save_path, &rel)?;
            let part_abs = safe_join(save_path, &format!("{rel}{PART_SUFFIX}"))?;
            let part = if final_abs.exists() {
                false
            } else if part_abs.exists() {
                true
            } else {
                use_part
            };
            Some(PartEntry { index, rel, part })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str) -> FileEntry {
        FileEntry { path: path.into(), name: path.rsplit('/').next().unwrap().into(), size: 10, ..Default::default() }
    }

    #[test]
    fn plan_follows_what_is_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("T")).unwrap();
        std::fs::write(dir.path().join("T/done.bin"), b"x").unwrap();
        std::fs::write(dir.path().join("T/half.bin.part"), b"x").unwrap();
        let files = [file("T/done.bin"), file("T/half.bin"), file("T/new.bin")];

        let on = plan_parts(dir.path(), &files, true);
        assert_eq!(on.iter().map(|p| p.part).collect::<Vec<_>>(), [false, true, true]);
        let off = plan_parts(dir.path(), &files, false);
        assert_eq!(off.iter().map(|p| p.part).collect::<Vec<_>>(), [false, true, false], "an existing .part resumes");
        assert_eq!(strip_part("T/half.bin.part"), "T/half.bin");
    }
}
