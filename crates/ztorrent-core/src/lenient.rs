//! The state file was written by JavaScript, where every number is a double and
//! any field may be missing or null. These readers take what is there and fall
//! back rather than refusing the whole file over one odd value.

use serde::{Deserialize, Deserializer};
use serde_json::Value;

pub fn u64<'de, D: Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::Number(n) => n
            .as_u64()
            .or_else(|| n.as_f64().filter(|f| f.is_finite() && *f >= 0.0).map(|f| f as u64))
            .unwrap_or(0),
        Value::String(s) => s.trim().parse::<f64>().map(|f| f.max(0.0) as u64).unwrap_or(0),
        Value::Bool(b) => b as u64,
        _ => 0,
    })
}

pub fn i64<'de, D: Deserializer<'de>>(d: D) -> Result<i64, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::Number(n) => n
            .as_i64()
            .or_else(|| n.as_f64().filter(|f| f.is_finite()).map(|f| f as i64))
            .unwrap_or(0),
        Value::String(s) => s.trim().parse::<f64>().map(|f| f as i64).unwrap_or(0),
        _ => 0,
    })
}

pub fn f64<'de, D: Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::Number(n) => n.as_f64().unwrap_or(0.0),
        Value::String(s) => s.trim().parse().unwrap_or(0.0),
        _ => 0.0,
    })
}

pub fn bool<'de, D: Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    // JavaScript truthiness, since that is how the old code read these.
    Ok(match Value::deserialize(d)? {
        Value::Bool(b) => b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Null => false,
        _ => true,
    })
}

pub fn string<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::String(s) => s,
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        _ => String::new(),
    })
}

pub fn opt_string<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::String(s) if !s.is_empty() => Some(s),
        _ => None,
    })
}
