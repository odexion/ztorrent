//! Reads the season and episode a release name carries, such as `S02E05`,
//! `2x05`, `S01-S03` or `Season 4`, so search results can be narrowed to one
//! season or one episode.

/// The seasons and episodes a release name covers. A single season or episode
/// is a range of one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EpisodeTag {
    pub seasons: Option<(u32, u32)>,
    pub episodes: Option<(u32, u32)>,
}

impl EpisodeTag {
    /// Whether this release holds the wanted season and episode; None wants any.
    /// Asking for an episode leaves out whole-season packs, which would bring
    /// every other episode with it.
    pub fn matches(&self, season: Option<u32>, episode: Option<u32>) -> bool {
        let within = |range: Option<(u32, u32)>, n: u32| range.is_some_and(|(a, b)| a <= n && n <= b);
        if let Some(s) = season
            && !within(self.seasons, s)
        {
            return false;
        }
        if let Some(e) = episode
            && !within(self.episodes, e)
        {
            return false;
        }
        true
    }
}

impl EpisodeTag {
    /// A short label for the row, such as `S06E11`, `S03E01–E04`, `Season 4`
    /// or `S01–S03`; None when the name carries no season.
    pub fn label(&self) -> Option<String> {
        let (s1, s2) = self.seasons?;
        Some(match self.episodes {
            Some((e1, e2)) if e1 == e2 => format!("S{s1:02}E{e1:02}"),
            Some((e1, e2)) => format!("S{s1:02}E{e1:02}–E{e2:02}"),
            None if s1 == s2 => format!("Season {s1}"),
            None => format!("S{s1:02}–S{s2:02}"),
        })
    }
}

/// The picture qualities a search can be narrowed to, best first.
pub const QUALITIES: [&str; 4] = ["2160p", "1080p", "720p", "480p"];

/// The picture quality a release name gives, as one of [`QUALITIES`]. `4K`
/// and `UHD` count as 2160p.
pub fn quality(title: &str) -> Option<&'static str> {
    let t = title.to_ascii_lowercase();
    let word = |w: &str| {
        t.match_indices(w).any(|(i, _)| {
            let before = t[..i].chars().next_back().is_none_or(|c| !c.is_ascii_alphanumeric());
            let after = t[i + w.len()..].chars().next().is_none_or(|c| !c.is_ascii_alphanumeric());
            before && after
        })
    };
    if word("2160p") || word("4k") || word("uhd") {
        return Some("2160p");
    }
    QUALITIES[1..].iter().copied().find(|q| word(q))
}

/// Finds the first season/episode marker in a release name.
pub fn episode_tag(title: &str) -> EpisodeTag {
    let t: Vec<u8> = title.bytes().map(|b| b.to_ascii_uppercase()).collect();
    let mut tag = EpisodeTag::default();
    let mut i = 0;
    while i < t.len() {
        if i > 0 && t[i - 1].is_ascii_alphanumeric() {
            i += 1;
            continue;
        }
        // S01E02, S01E02E03, S01E02-E04, S01E02-04, S01, S01-S03, S01-03
        if t[i] == b'S'
            && let Some((s, mut j)) = number(&t, i + 1, 2)
        {
            if t.get(j) == Some(&b'E')
                && let Some((e, k)) = number(&t, j + 1, 3)
            {
                let (end, _) = range_end(&t, k, b'E', 3).unwrap_or((e, k));
                tag.seasons = Some((s, s));
                tag.episodes = Some((e, end.max(e)));
                return tag;
            }
            if let Some((end, k)) = range_end(&t, j, b'S', 2) {
                j = k;
                if boundary(&t, j) {
                    tag.seasons = Some((s, end.max(s)));
                    return tag;
                }
            } else if boundary(&t, j) {
                tag.seasons = Some((s, s));
                return tag;
            }
        }
        // 2x05: one or two digits, so 1920x1080 is not read as one.
        if let Some((s, j)) = number(&t, i, 2)
            && t.get(j) == Some(&b'X')
            && let Some((e, k)) = number(&t, j + 1, 3)
            && boundary(&t, k)
        {
            tag.seasons = Some((s, s));
            tag.episodes = Some((e, e));
            return tag;
        }
        // Season 4, Season 1-3, Season 2 Episode 5
        if t[i..].starts_with(b"SEASON")
            && let Some((s, j)) = number(&t, skip_sep(&t, i + 6), 2)
        {
            let (end, j) = range_end(&t, j, b'S', 2).unwrap_or((s, j));
            tag.seasons = Some((s, end.max(s)));
            let k = skip_sep(&t, j);
            if t[k..].starts_with(b"EPISODE")
                && let Some((e, _)) = number(&t, skip_sep(&t, k + 7), 3)
            {
                tag.episodes = Some((e, e));
            }
            return tag;
        }
        i += 1;
    }
    tag
}

/// Up to `max` digits at `at`, and where they end.
fn number(t: &[u8], at: usize, max: usize) -> Option<(u32, usize)> {
    let len = t[at.min(t.len())..].iter().take_while(|b| b.is_ascii_digit()).count();
    if len == 0 || len > max {
        return None;
    }
    let n = std::str::from_utf8(&t[at..at + len]).ok()?.parse().ok()?;
    Some((n, at + len))
}

/// The far end of a range written `-04`, `-E04` or `E04` (for the given
/// letter) straight after a number.
fn range_end(t: &[u8], at: usize, letter: u8, max: usize) -> Option<(u32, usize)> {
    let mut j = at;
    if t.get(j) == Some(&b'-') {
        j += 1;
    }
    if t.get(j) == Some(&letter) {
        j += 1;
    }
    if j == at {
        return None;
    }
    number(t, j, max)
}

fn skip_sep(t: &[u8], mut at: usize) -> usize {
    while matches!(t.get(at), Some(b' ' | b'.' | b'_' | b'-')) {
        at += 1;
    }
    at
}

fn boundary(t: &[u8], at: usize) -> bool {
    t.get(at).is_none_or(|b| !b.is_ascii_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag(title: &str) -> (Option<(u32, u32)>, Option<(u32, u32)>) {
        let t = episode_tag(title);
        (t.seasons, t.episodes)
    }

    #[test]
    fn reads_the_common_forms() {
        assert_eq!(tag("Show.Name.S02E05.1080p.WEB"), (Some((2, 2)), Some((5, 5))));
        assert_eq!(tag("show name s1e12 720p"), (Some((1, 1)), Some((12, 12))));
        assert_eq!(tag("Show S03E01E02"), (Some((3, 3)), Some((1, 2))));
        assert_eq!(tag("Show S03E01-E04"), (Some((3, 3)), Some((1, 4))));
        assert_eq!(tag("Show S03E01-04"), (Some((3, 3)), Some((1, 4))));
        assert_eq!(tag("Show 2x05 HDTV"), (Some((2, 2)), Some((5, 5))));
        assert_eq!(tag("Show S04 Complete 1080p"), (Some((4, 4)), None));
        assert_eq!(tag("Show S01-S03 Pack"), (Some((1, 3)), None));
        assert_eq!(tag("Show S01-03"), (Some((1, 3)), None));
        assert_eq!(tag("Show Season 4 Complete"), (Some((4, 4)), None));
        assert_eq!(tag("Show Season 1-3"), (Some((1, 3)), None));
        assert_eq!(tag("Show Season 2 Episode 7"), (Some((2, 2)), Some((7, 7))));
    }

    #[test]
    fn leaves_other_numbers_alone() {
        assert_eq!(tag("Film 2019 1920x1080 x264"), (None, None));
        assert_eq!(tag("Ubuntu 24.04 desktop amd64"), (None, None));
        assert_eq!(tag("SuperSeries S2000"), (None, None));
        assert_eq!(tag("PS2 Classics"), (None, None));
    }

    #[test]
    fn labels_and_quality() {
        assert_eq!(episode_tag("Show S06E11").label().as_deref(), Some("S06E11"));
        assert_eq!(episode_tag("Show S03E01-E04").label().as_deref(), Some("S03E01–E04"));
        assert_eq!(episode_tag("Show Season 4").label().as_deref(), Some("Season 4"));
        assert_eq!(episode_tag("Show S01-S03").label().as_deref(), Some("S01–S03"));
        assert_eq!(episode_tag("Film 2019").label(), None);
        assert_eq!(quality("Show.S01E01.1080p.WEB"), Some("1080p"));
        assert_eq!(quality("Film 2019 4K HDR"), Some("2160p"));
        assert_eq!(quality("Film [720p]"), Some("720p"));
        assert_eq!(quality("Film 1920x1080 x264"), None);
        assert_eq!(quality("Album 4kids"), None);
    }

    #[test]
    fn matching() {
        let ep = episode_tag("Show S02E05");
        assert!(ep.matches(None, None));
        assert!(ep.matches(Some(2), None));
        assert!(ep.matches(Some(2), Some(5)));
        assert!(!ep.matches(Some(1), None));
        assert!(!ep.matches(Some(2), Some(6)));
        let pack = episode_tag("Show S01-S03");
        assert!(pack.matches(Some(2), None));
        assert!(!pack.matches(Some(2), Some(1)));
        assert!(!episode_tag("Film 2019").matches(Some(1), None));
    }
}
