use std::cmp::Ordering;

/// Compares two dotted versions numerically. A version carrying a prerelease
/// suffix (1.2.0-beta.1) sorts below the same version without one, which is the
/// only part of the semver ordering rules the updater needs to get right.
pub fn compare_versions(a: &str, b: &str) -> Ordering {
    fn split(v: &str) -> (Vec<u64>, &str) {
        let v = v.strip_prefix('v').unwrap_or(v);
        let (core, pre) = v.split_once('-').unwrap_or((v, ""));
        // A pre like "beta.1-rc" keeps only what precedes its own dash, as
        // String.split('-') destructuring did.
        let pre = pre.split('-').next().unwrap_or("");
        let parts = core
            .split('.')
            .map(|n| {
                let digits: String = n.chars().take_while(char::is_ascii_digit).collect();
                digits.parse().unwrap_or(0)
            })
            .collect();
        (parts, pre)
    }
    let (x, xp) = split(a);
    let (y, yp) = split(b);
    for i in 0..x.len().max(y.len()) {
        let d = x.get(i).copied().unwrap_or(0).cmp(&y.get(i).copied().unwrap_or(0));
        if d != Ordering::Equal {
            return d;
        }
    }
    match (xp.is_empty(), yp.is_empty()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        _ if xp == yp => Ordering::Equal,
        _ => xp.cmp(yp),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Ordering::*;

    #[test]
    fn orders_like_updater_js() {
        assert_eq!(compare_versions("0.4.1", "0.4.0"), Greater);
        assert_eq!(compare_versions("v0.4.0", "0.4.0"), Equal);
        assert_eq!(compare_versions("0.10.0", "0.9.9"), Greater);
        assert_eq!(compare_versions("1.2.0-beta.1", "1.2.0"), Less);
        assert_eq!(compare_versions("1.2.0", "1.2.0-beta.1"), Greater);
        assert_eq!(compare_versions("1.2.0-beta.1", "1.2.0-beta.2"), Less);
        assert_eq!(compare_versions("1.2", "1.2.0"), Equal);
    }
}
