//! Formatting helpers, ported from renderer/util.js. Units and rounding follow
//! what uTorrent prints, and every function here produces the same string the
//! JavaScript did for the same input.

use chrono::{Local, TimeZone};
use std::cmp::Ordering;

const SIZE_UNITS: [&str; 6] = ["B", "kB", "MB", "GB", "TB", "PB"];

pub fn bytes(n: f64, blank_zero: bool) -> String {
    if !n.is_finite() || n < 0.0 {
        return String::new();
    }
    if n == 0.0 {
        return if blank_zero { String::new() } else { "0 B".into() };
    }
    let mut i = 0;
    let mut v = n;
    while v >= 1024.0 && i < SIZE_UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    let dp = if i == 0 { 0 } else if v >= 100.0 { 1 } else { 2 };
    format!("{} {}", to_fixed(v, dp), SIZE_UNITS[i])
}

pub fn speed(n: f64, blank_zero: bool) -> String {
    if n.is_nan() || n < 1.0 {
        return if blank_zero { String::new() } else { "0 B/s".into() };
    }
    format!("{}/s", bytes(n, false))
}

/// Time remaining. The argument is in milliseconds -- util.js called it
/// seconds, but WebTorrent's timeRemaining was always ms and the arithmetic
/// below has always divided by a thousand.
pub fn eta(ms: f64) -> String {
    if !ms.is_finite() || ms <= 0.0 {
        return "∞".into();
    }
    let s = js_round(ms / 1000.0);
    if s < 1.0 {
        return "<1s".into();
    }
    if s > 60.0 * 60.0 * 24.0 * 365.0 {
        return "∞".into();
    }
    let s = s as u64;
    let (d, h, m, sec) = (s / 86400, (s % 86400) / 3600, (s % 3600) / 60, s % 60);
    if d > 0 {
        format!("{d}d {h}h")
    } else if h > 0 {
        format!("{h}h {m}m")
    } else if m > 0 {
        format!("{m}m {sec}s")
    } else {
        format!("{sec}s")
    }
}

pub fn pct(v: f64, dp: usize) -> String {
    let v = if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) };
    format!("{}%", to_fixed(v * 100.0, dp))
}

pub fn ratio(v: f64) -> String {
    if !v.is_finite() {
        return "∞".into();
    }
    to_fixed(v, 3)
}

/// `dd/mm/yyyy HH:MM`, local time. Zero is "never" and prints nothing.
pub fn datetime(ms: i64) -> String {
    if ms == 0 {
        return String::new();
    }
    match Local.timestamp_millis_opt(ms).single() {
        Some(t) => t.format("%d/%m/%Y %H:%M").to_string(),
        None => String::new(),
    }
}

pub fn clock(ms: i64) -> String {
    match Local.timestamp_millis_opt(ms).single() {
        Some(t) => t.format("%H:%M:%S").to_string(),
        None => String::new(),
    }
}

pub fn duration(ms: i64) -> String {
    if ms == 0 {
        return String::new();
    }
    let s = (ms.max(0) / 1000) as u64;
    let (d, h, m) = (s / 86400, (s % 86400) / 3600, (s % 3600) / 60);
    if d > 0 {
        format!("{d}d {h}h")
    } else if h > 0 {
        format!("{h}h {m}m")
    } else {
        format!("{m}m")
    }
}

/// Natural, case-insensitive ordering that keeps "File 2" before "File 10",
/// the way the renderer's Intl.Collator({ numeric: true, sensitivity: 'base' })
/// sorted names, labels and save paths.
pub fn compare_text(a: &str, b: &str) -> Ordering {
    let mut x = a.chars().peekable();
    let mut y = b.chars().peekable();
    loop {
        match (x.peek().copied(), y.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(c), Some(d)) if c.is_ascii_digit() && d.is_ascii_digit() => {
                let n = take_digits(&mut x);
                let m = take_digits(&mut y);
                let (n_trim, m_trim) = (n.trim_start_matches('0'), m.trim_start_matches('0'));
                let ord = n_trim
                    .len()
                    .cmp(&m_trim.len())
                    .then_with(|| n_trim.cmp(m_trim));
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            (Some(c), Some(d)) => {
                let ord = fold(c).cmp(&fold(d));
                if ord != Ordering::Equal {
                    return ord;
                }
                x.next();
                y.next();
            }
        }
    }
}

fn take_digits(it: &mut std::iter::Peekable<std::str::Chars>) -> String {
    let mut s = String::new();
    while let Some(c) = it.peek().copied().filter(char::is_ascii_digit) {
        s.push(c);
        it.next();
    }
    s
}

fn fold(c: char) -> String {
    c.to_lowercase().collect()
}

/// Math.round: halves go up, towards positive infinity.
fn js_round(v: f64) -> f64 {
    (v + 0.5).floor()
}

/// Number.prototype.toFixed for the magnitudes this app prints.
fn to_fixed(v: f64, dp: usize) -> String {
    let scale = 10f64.powi(dp as i32);
    let rounded = js_round(v * scale) / scale;
    format!("{rounded:.dp$}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_match_util_js() {
        assert_eq!(bytes(0.0, false), "0 B");
        assert_eq!(bytes(0.0, true), "");
        assert_eq!(bytes(-1.0, false), "");
        assert_eq!(bytes(1023.0, false), "1023 B");
        assert_eq!(bytes(1024.0, false), "1.00 kB");
        assert_eq!(bytes(123_456.0, false), "120.6 kB");
        assert_eq!(bytes(755.0 * 1024.0 * 1024.0, false), "755.0 MB");
        assert_eq!(bytes(1_500_000_000.0, false), "1.40 GB");
    }

    #[test]
    fn speed_eta_pct_ratio() {
        assert_eq!(speed(0.0, true), "");
        assert_eq!(speed(0.5, false), "0 B/s");
        assert_eq!(speed(2048.0, true), "2.00 kB/s");
        assert_eq!(eta(f64::INFINITY), "∞");
        assert_eq!(eta(400.0), "<1s");
        assert_eq!(eta(59_000.0), "59s");
        assert_eq!(eta(61_000.0), "1m 1s");
        assert_eq!(eta(3_720_000.0), "1h 2m");
        assert_eq!(eta(90_000_000.0), "1d 1h");
        assert_eq!(pct(0.4217, 1), "42.2%");
        assert_eq!(pct(2.0, 2), "100.00%");
        assert_eq!(ratio(1.0 / 3.0), "0.333");
        assert_eq!(ratio(f64::INFINITY), "∞");
        assert_eq!(duration(0), "");
        assert_eq!(duration(3_660_000), "1h 1m");
    }

    #[test]
    fn natural_order() {
        assert_eq!(compare_text("File 2", "File 10"), Ordering::Less);
        assert_eq!(compare_text("abc", "ABD"), Ordering::Less);
        assert_eq!(compare_text("Sintel", "sintel"), Ordering::Equal);
        assert_eq!(compare_text("", "a"), Ordering::Less);
    }
}
