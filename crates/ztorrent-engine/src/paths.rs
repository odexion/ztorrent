//! Paths built from torrent data. A torrent names its own files, and a name is
//! untrusted input: nothing here joins one onto a folder without checking the
//! result stays inside it.

use std::path::{Component, Path, PathBuf};

/// `base` joined with a relative path from a torrent, or None if the relative
/// path would climb out of `base` or replace it.
pub fn safe_join(base: &Path, relative: &str) -> Option<PathBuf> {
    let rel = Path::new(relative);
    if relative.is_empty() {
        return None;
    }
    for c in rel.components() {
        match c {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(base.join(rel))
}

/// A torrent's top-level name, as used for its folder or single file.
pub fn safe_top_level(base: &Path, name: &str) -> Option<PathBuf> {
    let mut comps = Path::new(name).components();
    match (comps.next(), comps.next()) {
        (Some(Component::Normal(_)), None) => Some(base.join(name)),
        _ => None,
    }
}

/// Deletes exactly the files a torrent lists -- each under its final name and
/// its `.part` name -- then the folders that held them, if that left them
/// empty. Never a whole folder: a torrent chooses its own name, and a name that
/// matches a folder already there must not cost that folder's other contents.
/// Returns the paths it refused and the errors it met.
pub fn delete_listed<'a>(base: &Path, files: impl IntoIterator<Item = &'a str>, part_suffix: &str) -> (Vec<String>, Vec<String>) {
    let (mut refused, mut errors) = (Vec::new(), Vec::new());
    let mut dirs = Vec::new();
    for rel in files {
        let Some(path) = safe_join(base, rel) else {
            refused.push(rel.to_string());
            continue;
        };
        for candidate in [path.clone(), PathBuf::from(format!("{}{part_suffix}", path.display()))] {
            // A folder where a file should be is not the torrent's to delete.
            match std::fs::symlink_metadata(&candidate) {
                Ok(m) if !m.is_dir() => {
                    if let Err(e) = std::fs::remove_file(&candidate) {
                        errors.push(format!("{}: {e}", candidate.display()));
                    }
                }
                _ => {}
            }
        }
        let mut dir = path.parent();
        while let Some(d) = dir.filter(|d| *d != base && d.starts_with(base)) {
            dirs.push(d.to_path_buf());
            dir = d.parent();
        }
    }
    // Deepest first, so a folder is looked at after everything inside it.
    dirs.sort_by_key(|d| std::cmp::Reverse(d.components().count()));
    dirs.dedup();
    for d in dirs {
        // remove_dir only succeeds on an empty folder, which is the point.
        let _ = std::fs::remove_dir(&d);
    }
    (refused, errors)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deletes_only_the_listed_files_and_keeps_a_colliding_folder() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        // The user's own folder, which a torrent happens to share a name with.
        std::fs::create_dir_all(base.join("Projects/sub")).unwrap();
        std::fs::write(base.join("Projects/mine.txt"), b"keep").unwrap();
        std::fs::write(base.join("Projects/a.bin"), b"torrent").unwrap();
        std::fs::write(base.join("Projects/sub/b.bin.part"), b"torrent").unwrap();
        // A torrent of its own, fully removable.
        std::fs::create_dir_all(base.join("Sintel/extras")).unwrap();
        std::fs::write(base.join("Sintel/extras/c.srt"), b"x").unwrap();

        let (refused, errors) = delete_listed(base, ["Projects/a.bin", "Projects/sub/b.bin", "Sintel/extras/c.srt", "../escape"], ".part");
        assert_eq!(refused, vec!["../escape".to_string()]);
        assert!(errors.is_empty(), "{errors:?}");
        assert!(base.join("Projects/mine.txt").exists(), "the user's file survived");
        assert!(!base.join("Projects/a.bin").exists());
        assert!(!base.join("Projects/sub").exists(), "an emptied folder goes");
        assert!(!base.join("Sintel").exists(), "a torrent's own folder goes once empty");
        assert!(base.exists(), "the save folder itself stays");
    }

    #[test]
    fn refuses_to_leave_the_folder() {
        let base = Path::new("/downloads");
        assert_eq!(safe_join(base, "Sintel/sintel.mp4"), Some(PathBuf::from("/downloads/Sintel/sintel.mp4")));
        assert_eq!(safe_join(base, "../etc/passwd"), None);
        assert_eq!(safe_join(base, "/etc/passwd"), None);
        assert_eq!(safe_join(base, "a/../../b"), None);
        assert_eq!(safe_top_level(base, "Sintel"), Some(PathBuf::from("/downloads/Sintel")));
        assert_eq!(safe_top_level(base, ".."), None);
        assert_eq!(safe_top_level(base, "a/b"), None);
        assert_eq!(safe_top_level(base, ""), None);
    }
}
