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

#[cfg(test)]
mod tests {
    use super::*;

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
