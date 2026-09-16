//! The torrent list's column model, and the one migration saved layouts need.

use crate::fmt::compare_text;
use crate::model::Row;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

#[derive(Clone, Copy, Debug)]
pub struct ColumnDef {
    pub key: &'static str,
    pub label: &'static str,
    pub width: f32,
    pub numeric: bool,
}

const fn col(key: &'static str, label: &'static str, width: f32, numeric: bool) -> ColumnDef {
    ColumnDef { key, label, width, numeric }
}

pub const ALL_COLUMNS: [ColumnDef; 17] = [
    col("#", "#", 44., true),
    col("name", "Name", 360., false),
    col("size", "Size", 78., true),
    col("status", "Status", 220., false),
    col("downloadSpeed", "Down Speed", 110., true),
    col("uploadSpeed", "Up Speed", 88., true),
    col("eta", "ETA", 72., true),
    col("seeds", "Seeds", 68., true),
    col("peers", "Peers", 68., true),
    col("downloaded", "Downloaded", 86., true),
    col("uploaded", "Uploaded", 86., true),
    col("ratio", "Ratio", 68., true),
    col("availability", "Avail.", 68., true),
    col("label", "Label", 88., false),
    col("addedOn", "Added On", 116., false),
    col("completedOn", "Completed On", 116., false),
    col("savePath", "Save Path", 200., false),
];

pub const DEFAULT_VISIBLE: [&str; 11] = [
    "#", "name", "size", "status", "downloadSpeed", "uploadSpeed", "eta", "seeds", "peers", "ratio", "addedOn",
];

pub const MIN_WIDTH: f32 = 28.;

pub fn find(key: &str) -> Option<&'static ColumnDef> {
    ALL_COLUMNS.iter().find(|c| c.key == key)
}

/// What the state file keeps about the list: which columns, in what order,
/// and any width the user dragged.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Columns {
    pub order: Vec<String>,
    pub widths: IndexMap<String, f64>,
}

/// Seeds and Peers used to sit between Status and Down Speed; they now follow
/// the speeds and the ETA. An order saved by an older build is rewritten --
/// but only while the two are still exactly where that build left them, so a
/// hand-arranged layout is left alone and running this twice changes nothing.
pub fn migrate_columns(order: &[String]) -> Vec<String> {
    let pos = |k: &str| order.iter().position(|x| x == k);
    let (Some(seeds), Some(peers), Some(down)) = (pos("seeds"), pos("peers"), pos("downloadSpeed")) else {
        return order.to_vec();
    };
    if !(peers == seeds + 1 && down == peers + 1) {
        return order.to_vec();
    }
    let mut rest: Vec<String> = order.iter().filter(|k| *k != "seeds" && *k != "peers").cloned().collect();
    let anchor = ["eta", "uploadSpeed", "downloadSpeed"]
        .iter()
        .find_map(|k| rest.iter().position(|x| x == k));
    let Some(anchor) = anchor else { return order.to_vec() };
    rest.splice(anchor + 1..anchor + 1, ["seeds".to_string(), "peers".to_string()]);
    rest
}

/// The visible order to start from: the saved one (with the merged-away 'done'
/// column dropped and the migration applied), or the default. The flag says
/// whether that differs from what is saved, so the caller can write it back.
pub fn initial_order(saved: Option<&Columns>) -> (Vec<String>, bool) {
    match saved {
        Some(c) if !c.order.is_empty() => {
            let filtered: Vec<String> = c.order.iter().filter(|k| *k != "done").cloned().collect();
            let migrated = migrate_columns(&filtered);
            let changed = migrated != c.order;
            (migrated, changed)
        }
        _ => (DEFAULT_VISIBLE.iter().map(|s| s.to_string()).collect(), false),
    }
}

/// Toggles a column in or out, re-inserting it in canonical order so the
/// layout stays predictable. # and Name cannot be hidden.
pub fn toggle_column(order: &mut Vec<String>, key: &str) {
    if key == "#" || key == "name" || find(key).is_none() {
        return;
    }
    if let Some(i) = order.iter().position(|k| k == key) {
        order.remove(i);
    } else {
        order.push(key.to_string());
        let canon = |k: &String| ALL_COLUMNS.iter().position(|c| c.key == k).unwrap_or(usize::MAX);
        order.sort_by_key(canon);
    }
}

/// The comparison behind a click on a header.
pub fn compare_rows(key: &str, a: &Row, b: &Row) -> Ordering {
    let num = |x: f64, y: f64| x.partial_cmp(&y).unwrap_or(Ordering::Equal);
    match key {
        "#" => a.order.cmp(&b.order),
        "name" => compare_text(&a.name, &b.name),
        "size" => a.size.cmp(&b.size),
        "status" => num(a.done, b.done),
        "downloadSpeed" => num(a.download_speed, b.download_speed),
        "uploadSpeed" => num(a.upload_speed, b.upload_speed),
        "eta" => num(a.eta, b.eta),
        "seeds" => a.seeds.cmp(&b.seeds),
        "peers" => a.peers.cmp(&b.peers),
        "downloaded" => a.downloaded.cmp(&b.downloaded),
        "uploaded" => a.uploaded.cmp(&b.uploaded),
        "ratio" => num(a.ratio, b.ratio),
        "availability" => num(a.availability, b.availability),
        "label" => compare_text(&a.label, &b.label),
        "addedOn" => a.added_on.cmp(&b.added_on),
        "completedOn" => a.completed_on.cmp(&b.completed_on),
        "savePath" => compare_text(&a.save_path, &b.save_path),
        _ => a.order.cmp(&b.order),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn moves_seeds_and_peers_behind_eta_once() {
        let old = v(&["#", "name", "status", "seeds", "peers", "downloadSpeed", "uploadSpeed", "eta"]);
        let new = migrate_columns(&old);
        assert_eq!(new, v(&["#", "name", "status", "downloadSpeed", "uploadSpeed", "eta", "seeds", "peers"]));
        assert_eq!(migrate_columns(&new), new, "idempotent");
    }

    #[test]
    fn leaves_hand_arranged_layouts_alone() {
        let mine = v(&["#", "seeds", "name", "peers", "downloadSpeed"]);
        assert_eq!(migrate_columns(&mine), mine);
    }

    #[test]
    fn drops_done_and_reports_change() {
        let saved = Columns { order: v(&["#", "name", "done", "size"]), widths: Default::default() };
        let (order, changed) = initial_order(Some(&saved));
        assert_eq!(order, v(&["#", "name", "size"]));
        assert!(changed);
    }

    #[test]
    fn toggle_reinserts_canonically() {
        let mut order = v(&["#", "name", "eta"]);
        toggle_column(&mut order, "size");
        assert_eq!(order, v(&["#", "name", "size", "eta"]));
        toggle_column(&mut order, "name");
        assert_eq!(order.len(), 4);
    }
}
