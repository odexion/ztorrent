//! What the window knows, independent of how it is drawn: the rows the engine
//! last sent, what the user is looking at, and the rules for filtering, sorting
//! and selecting that renderer/app.js implemented. Nothing here touches GPUI,
//! so all of it is tested directly.

use std::collections::{HashMap, HashSet, VecDeque};
use ztorrent_core::columns::{self, Columns, MIN_WIDTH};
use ztorrent_core::command::Bootstrap;
use ztorrent_core::*;

pub const HISTORY_LEN: usize = 150;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    General,
    Trackers,
    Peers,
    Pieces,
    Files,
    Speed,
    Logger,
}

impl Tab {
    pub const ALL: [Tab; 7] = [Tab::General, Tab::Trackers, Tab::Peers, Tab::Pieces, Tab::Files, Tab::Speed, Tab::Logger];

    pub fn label(self) -> &'static str {
        match self {
            Tab::General => "General",
            Tab::Trackers => "Trackers",
            Tab::Peers => "Peers",
            Tab::Pieces => "Pieces",
            Tab::Files => "Files",
            Tab::Speed => "Speed",
            Tab::Logger => "Logger",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Tab::General => "info",
            Tab::Trackers => "tracker",
            Tab::Peers => "peers",
            Tab::Pieces => "pieces",
            Tab::Files => "files",
            Tab::Speed => "speed",
            Tab::Logger => "logger",
        }
    }

    pub fn parse(s: &str) -> Option<Tab> {
        Tab::ALL.into_iter().find(|t| t.label().eq_ignore_ascii_case(s))
    }
}

#[derive(Clone, Debug, Default)]
pub struct History {
    pub down: VecDeque<f64>,
    pub up: VecDeque<f64>,
}

impl History {
    pub fn push(&mut self, down: f64, up: f64) {
        self.down.push_back(down);
        self.up.push_back(up);
        while self.down.len() > HISTORY_LEN {
            self.down.pop_front();
            self.up.pop_front();
        }
    }
}

pub struct AppState {
    pub rows: Vec<Row>,
    pub globals: Globals,
    pub details: Option<Details>,
    pub settings: Settings,
    pub labels: Vec<String>,
    pub label_styles: LabelStyles,
    pub log: VecDeque<LogLine>,
    pub selection: Vec<String>,
    pub anchor: Option<String>,
    pub category: Category,
    pub search: String,
    pub sort_key: String,
    pub sort_desc: bool,
    pub tab: Tab,
    pub columns: Vec<String>,
    pub widths: HashMap<String, f32>,
    pub history: HashMap<String, History>,
    pub global_history: History,
    pub did_auto_select: bool,
    pub list_focused: bool,
}

impl AppState {
    pub fn new(boot: Bootstrap) -> (AppState, bool) {
        let (columns, changed) = columns::initial_order(boot.columns.as_ref());
        let widths = boot
            .columns
            .as_ref()
            .map(|c| c.widths.iter().map(|(k, v)| (k.clone(), *v as f32)).collect())
            .unwrap_or_default();
        let state = AppState {
            rows: boot.rows,
            globals: boot.globals,
            details: None,
            settings: boot.settings,
            labels: boot.labels,
            label_styles: boot.label_styles,
            log: boot.log.into_iter().collect(),
            selection: Vec::new(),
            anchor: None,
            category: Category::All,
            search: String::new(),
            sort_key: "#".into(),
            sort_desc: false,
            tab: Tab::General,
            columns,
            widths,
            history: HashMap::new(),
            global_history: History::default(),
            did_auto_select: false,
            list_focused: true,
        };
        (state, changed)
    }

    pub fn columns_record(&self) -> Columns {
        Columns {
            order: self.columns.clone(),
            widths: self.widths.iter().map(|(k, v)| (k.clone(), *v as f64)).collect(),
        }
    }

    pub fn width(&self, key: &str) -> f32 {
        self.widths.get(key).copied().unwrap_or_else(|| columns::find(key).map(|c| c.width).unwrap_or(80.))
    }

    pub fn set_width(&mut self, key: &str, w: f32) {
        self.widths.insert(key.to_string(), w.max(MIN_WIDTH));
    }

    /// Takes a tick. Returns true when the selection changed because a torrent
    /// went away, or when the first rows arrived and one was auto-selected --
    /// either way the caller has to ask the engine for new details.
    pub fn apply_tick(&mut self, rows: Vec<Row>, globals: Globals, details: Option<Details>) -> bool {
        self.global_history.push(globals.download_speed, globals.upload_speed);
        for r in &rows {
            self.history.entry(r.id.clone()).or_default().push(r.download_speed, r.upload_speed);
        }
        let live: HashSet<&str> = rows.iter().map(|r| r.id.as_str()).collect();
        self.history.retain(|id, _| live.contains(id.as_str()));

        // A tick in flight while the selection moves still carries the old
        // torrent's detail; showing it would paint one torrent's pieces under
        // another's name.
        match &details {
            Some(d) if self.selection.first() == Some(&d.id) => self.details = details,
            None if self.selection.is_empty() => self.details = None,
            _ => {}
        }

        self.rows = rows;
        self.globals = globals;

        let before = self.selection.len();
        let ids: HashSet<String> = self.rows.iter().map(|r| r.id.clone()).collect();
        self.selection.retain(|id| ids.contains(id));
        let mut changed = self.selection.len() != before;

        if !self.did_auto_select && !self.rows.is_empty() {
            self.did_auto_select = true;
            if self.selection.is_empty() {
                let first = self.visible_rows().first().map(|r| r.id.clone()).or_else(|| self.rows.first().map(|r| r.id.clone()));
                if let Some(id) = first {
                    self.select(&id, false, false);
                    changed = true;
                }
            }
        }
        changed
    }

    pub fn push_log(&mut self, line: LogLine) {
        self.log.push_back(line);
        while self.log.len() > LOG_LIMIT {
            self.log.pop_front();
        }
    }

    /// The rows the list shows, in the order it shows them.
    pub fn visible_rows(&self) -> Vec<&Row> {
        let q = self.search.trim().to_lowercase();
        let mut rows: Vec<&Row> = self
            .rows
            .iter()
            .filter(|r| self.category.matches(r))
            .filter(|r| q.is_empty() || r.name.to_lowercase().contains(&q) || r.label.to_lowercase().contains(&q))
            .collect();
        let key = if columns::find(&self.sort_key).is_some() { self.sort_key.as_str() } else { "#" };
        rows.sort_by(|a, b| {
            let o = columns::compare_rows(key, a, b);
            if self.sort_desc { o.reverse() } else { o }
        });
        rows
    }

    pub fn count(&self, cat: &Category) -> usize {
        self.rows.iter().filter(|r| cat.matches(r)).count()
    }

    pub fn toggle_sort(&mut self, key: &str) {
        if self.sort_key == key {
            self.sort_desc = !self.sort_desc;
        } else {
            self.sort_key = key.to_string();
            self.sort_desc = false;
        }
    }

    pub fn is_selected(&self, id: &str) -> bool {
        self.selection.iter().any(|s| s == id)
    }

    pub fn first_selected(&self) -> Option<&Row> {
        let id = self.selection.first()?;
        self.rows.iter().find(|r| &r.id == id)
    }

    pub fn row(&self, id: &str) -> Option<&Row> {
        self.rows.iter().find(|r| r.id == id)
    }

    /// Plain click selects one; Cmd/Ctrl toggles; Shift extends from the anchor
    /// across the rows as they are currently shown.
    pub fn select(&mut self, id: &str, additive: bool, range: bool) {
        if range {
            if let Some(anchor) = self.anchor.clone() {
                let visible: Vec<String> = self.visible_rows().iter().map(|r| r.id.clone()).collect();
                if let (Some(a), Some(b)) = (visible.iter().position(|x| *x == anchor), visible.iter().position(|x| x == id)) {
                    if !additive {
                        self.selection.clear();
                    }
                    for vid in &visible[a.min(b)..=a.max(b)] {
                        if !self.is_selected(vid) {
                            self.selection.push(vid.clone());
                        }
                    }
                    return;
                }
            }
        }
        if additive {
            if let Some(i) = self.selection.iter().position(|s| s == id) {
                self.selection.remove(i);
            } else {
                self.selection.push(id.to_string());
            }
        } else {
            self.selection = vec![id.to_string()];
        }
        self.anchor = Some(id.to_string());
    }

    pub fn select_all_visible(&mut self) {
        let ids: Vec<String> = self.visible_rows().iter().map(|r| r.id.clone()).collect();
        for id in ids {
            if !self.is_selected(&id) {
                self.selection.push(id);
            }
        }
    }

    /// Arrow keys: moves from the anchor, and extends with Shift.
    pub fn move_selection(&mut self, down: bool, extend: bool) -> Option<usize> {
        let visible: Vec<String> = self.visible_rows().iter().map(|r| r.id.clone()).collect();
        if visible.is_empty() {
            return None;
        }
        let cur = self.anchor.as_ref().and_then(|a| visible.iter().position(|v| v == a));
        let next = match cur {
            None => 0,
            Some(c) if down => (c + 1).min(visible.len() - 1),
            Some(c) => c.saturating_sub(1),
        };
        if extend && self.anchor.is_some() {
            let anchor = self.anchor.clone();
            self.select(&visible[next], false, true);
            // A range keeps its anchor where it started.
            self.anchor = anchor;
            // ...but the next arrow press moves from the far end.
            self.anchor = Some(visible[next].clone());
        } else {
            self.select(&visible[next], false, false);
        }
        Some(next)
    }

    /// The icons and wording of what a status bar limit pill says.
    pub fn speed_cap_label(&self) -> String {
        let s = &self.settings;
        let alt = s.alt_speed_enabled;
        let (dn, up) = if alt { (s.alt_download_rate, s.alt_upload_rate) } else { (s.max_download_rate, s.max_upload_rate) };
        if dn == 0 && up == 0 {
            return if alt { "Alt: no limit".into() } else { "No limit".into() };
        }
        let part = |v: u64, sym: &str| if v > 0 { format!("{sym}{v} kB/s") } else { format!("{sym}∞") };
        format!("{}: {} {}", if alt { "Alt" } else { "Limit" }, part(dn, "↓"), part(up, "↑"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, order: i64, name: &str, state: State, done: f64) -> Row {
        Row { id: id.into(), order, name: name.into(), state, done, eta: f64::INFINITY, ..Default::default() }
    }

    fn app(rows: Vec<Row>) -> AppState {
        let (mut s, _) = AppState::new(Bootstrap::default());
        s.rows = rows;
        s
    }

    #[test]
    fn filters_sorts_and_searches() {
        let mut s = app(vec![
            row("a", 0, "File 10", State::Seeding, 1.0),
            row("b", 1, "File 2", State::Downloading, 0.3),
            row("c", 2, "Other", State::Stopped, 1.0),
        ]);
        s.toggle_sort("name");
        assert_eq!(s.visible_rows().iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["b", "a", "c"]);
        s.toggle_sort("name");
        assert_eq!(s.visible_rows()[0].id, "c");
        s.category = Category::Completed;
        assert_eq!(s.visible_rows().len(), 2);
        s.search = "file".into();
        assert_eq!(s.visible_rows().len(), 1);
        assert_eq!(s.count(&Category::Seeding), 1);
    }

    #[test]
    fn selection_like_finder() {
        let mut s = app((0..5).map(|i| row(&i.to_string(), i, "x", State::Paused, 0.0)).collect());
        s.select("1", false, false);
        s.select("3", false, true);
        assert_eq!(s.selection, ["1", "2", "3"]);
        s.select("2", true, false);
        assert_eq!(s.selection, ["1", "3"]);
        s.select("4", false, false);
        assert_eq!(s.selection, ["4"]);
        s.move_selection(false, false);
        assert_eq!(s.selection, ["3"]);
        s.select_all_visible();
        assert_eq!(s.selection.len(), 5);
    }

    #[test]
    fn ticks_drop_stale_details_and_vanished_selections() {
        let mut s = app(vec![]);
        let changed = s.apply_tick(vec![row("a", 0, "A", State::Paused, 0.0), row("b", 1, "B", State::Paused, 0.0)], Globals::default(), None);
        assert!(changed, "first rows auto-select");
        assert_eq!(s.selection, ["a"]);
        let stale = Details { id: "b".into(), ..Default::default() };
        s.apply_tick(s.rows.clone(), Globals::default(), Some(stale));
        assert!(s.details.is_none(), "details for another torrent are ignored");
        let changed = s.apply_tick(vec![row("b", 1, "B", State::Paused, 0.0)], Globals::default(), None);
        assert!(changed && s.selection.is_empty());
        assert!(!s.history.contains_key("a"), "history of a removed torrent is dropped");
    }

    #[test]
    fn cap_label_matches_utorrent_readout() {
        let mut s = app(vec![]);
        assert_eq!(s.speed_cap_label(), "No limit");
        s.settings.max_download_rate = 100;
        assert_eq!(s.speed_cap_label(), "Limit: ↓100 kB/s ↑∞");
        s.settings.alt_speed_enabled = true;
        assert_eq!(s.speed_cap_label(), "Alt: ↓100 kB/s ↑20 kB/s");
    }
}
