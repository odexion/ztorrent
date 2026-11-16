//! The torrent list as two-line rows, the default view: the name and size,
//! a thin progress bar, then the state, speeds, time left, swarm and label.
//! Above it, pills for what the list is narrowed to and a sort menu. The
//! one-line table in grid.rs stays as the Compact view.

use crate::grid::DragChip;
use crate::theme::{FONT_SIZE, Palette, RADIUS, TEXT_META, Theme, tabular};
use crate::widgets::{PickOption, count, dot, filter_pill, meta_line, tag, with_menu};
use crate::workspace::{DraggedTorrents, Workspace};
use gpui::prelude::*;
use gpui::*;
use gpui_component::checkbox::Checkbox;
use std::ops::Range;
use ztorrent_core::{Category, Row, State, bar_kind, fmt, state_icon, status_text};

/// Each row's height, gap included: the list is virtualised, so every row
/// has to be the same.
const ROW_TALL: f32 = 62.;
pub const BAR_H: f32 = 44.;

/// What the sort menu offers, as column keys the table sorts by too.
const SORTS: [(&str, &str); 10] = [
    ("#", "queue order"),
    ("name", "name"),
    ("size", "size"),
    ("status", "progress"),
    ("downloadSpeed", "download speed"),
    ("uploadSpeed", "upload speed"),
    ("eta", "time left"),
    ("ratio", "ratio"),
    ("addedOn", "date added"),
    ("completedOn", "date completed"),
];

const STATUSES: [(Category, &str, &str); 5] = [
    (Category::Downloading, "downloading", "Downloading"),
    (Category::Seeding, "seeding", "Seeding"),
    (Category::Completed, "completed", "Completed"),
    (Category::Active, "active", "Active"),
    (Category::Inactive, "inactive", "Inactive"),
];

/// The pill's menu value as the sidebar's choice.
fn category_from(v: &str) -> Category {
    if let Some(l) = v.strip_prefix("label:") {
        return Category::Label(l.to_string());
    }
    if v == "nolabel" {
        return Category::NoLabel;
    }
    STATUSES.iter().find(|(_, s, _)| *s == v).map(|(c, _, _)| c.clone()).unwrap_or(Category::All)
}

fn category_label(c: &Category) -> Option<String> {
    match c {
        Category::All => None,
        Category::NoLabel => Some("No label".into()),
        Category::Label(l) => Some(format!("Label: {l}")),
        other => STATUSES.iter().find(|(s, _, _)| s == other).map(|(_, _, l)| l.to_string()),
    }
}

/// The colour of a torrent's state, as its icon in the table has.
pub fn state_color(p: &Palette, r: &Row) -> Hsla {
    match state_icon(r) {
        "downloading" => p.down,
        "seeding" => p.ic_seeding(),
        "completed" => p.ic_completed(),
        "error" => p.err,
        _ => p.ink_faint,
    }
}

impl Workspace {
    pub fn pick_list_option(&mut self, a: &PickOption, cx: &mut Context<Self>) {
        match a.field.as_str() {
            "category" => self.state.category = category_from(&a.value),
            "sort" => {
                self.state.sort_key = a.value.clone();
                // Biggest, fastest and newest first; names and the queue A to Z.
                self.state.sort_desc = !matches!(a.value.as_str(), "#" | "name" | "eta");
            }
            "sortdir" => self.state.sort_desc = a.value == "desc",
            _ => return,
        }
        cx.notify();
    }

    /// The bar above the rows: tick all, the pills, the count and the sort.
    pub fn render_list_bar(&self, p: &Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let visible: Vec<String> = self.state.visible_rows().iter().map(|r| r.id.clone()).collect();
        let all = !visible.is_empty() && visible.iter().all(|id| self.state.is_selected(id));
        let focus = &self.list_focus;

        let mut options: Vec<(String, String)> = STATUSES.iter().map(|(_, v, l)| (v.to_string(), l.to_string())).collect();
        options.extend(self.state.labels.iter().map(|l| (format!("label:{l}"), format!("Label: {l}"))));
        options.push(("nolabel".into(), "No label".into()));
        let shown = category_label(&self.state.category);
        let pill = filter_pill(p, "category", "Filter", shown, options, focus, cx, |this: &mut Workspace, cx| {
            this.state.category = Category::All;
            cx.notify();
        });

        let search = self.state.search.trim().to_string();
        let search_pill = (!search.is_empty()).then(|| {
            div()
                .flex()
                .flex_none()
                .items_center()
                .h(crate::theme::CHIP_H)
                .px(px(10.))
                .rounded_full()
                .bg(p.accent_soft)
                .text_color(p.accent)
                .text_size(TEXT_META)
                .child(format!("Name: “{search}”"))
        });

        let key = self.state.sort_key.as_str();
        let name = SORTS.iter().find(|(k, _)| *k == key).map(|(_, l)| *l).unwrap_or("queue order");
        let sorts = SORTS.iter().map(|(k, l)| (k.to_string(), l.to_string())).collect();
        let sort = with_menu(
            div().id("sort-menu").hover(|s| s.text_color(p.ink)).child(format!("sorted by {name} {}", if self.state.sort_desc { "↓" } else { "↑" })),
            "sort",
            sorts,
            key.to_string(),
            focus,
        );
        let dir = with_menu(
            div().id("sort-dir").px(px(4.)).rounded(px(4.)).hover(|s| s.text_color(p.ink)).child("▾"),
            "sortdir",
            vec![("asc".into(), "Ascending".into()), ("desc".into(), "Descending".into())],
            if self.state.sort_desc { "desc".into() } else { "asc".into() },
            focus,
        );

        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.))
            .h(px(BAR_H))
            .px(px(16.))
            .border_b_1()
            .border_color(p.line)
            .text_size(TEXT_META)
            .text_color(p.ink_dim)
            .child(
                div().on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()).child(
                    Checkbox::new("tick-all").checked(all).on_click(cx.listener(|this, on: &bool, window, cx| {
                        window.focus(&this.list_focus, cx);
                        if *on {
                            this.state.select_all_visible();
                        } else {
                            this.state.selection.clear();
                        }
                        this.sync_selection(cx);
                    })),
                ),
            )
            .child(pill)
            .children(search_pill)
            .child(div().flex_1())
            .child(format!("{} ·", count(visible.len() as u64, "torrent", "torrents")))
            .child(sort)
            .child(dir)
    }

    pub fn render_tall_list(&mut self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let count = self.state.visible_rows().len();
        (count > 0).then(|| {
            uniform_list("torrent-rows", count, cx.processor(|this, range: Range<usize>, _window, cx| this.render_tall_rows(range, cx)))
                .track_scroll(&self.list_scroll)
                .size_full()
                .px(px(8.))
                .pt(px(4.))
        })
    }

    fn render_tall_rows(&mut self, range: Range<usize>, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let p = Theme::get(cx).clone();
        let rows: Vec<Row> = self.state.visible_rows().into_iter().skip(range.start).take(range.len()).cloned().collect();
        let focused = self.state.list_focused;
        rows.into_iter()
            .enumerate()
            .map(|(k, r)| {
                let ix = range.start + k;
                let selected = self.state.is_selected(&r.id);
                let hover = p.hover;
                let row = div()
                    .id(("trow", ix))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .h(px(ROW_TALL - 2.))
                    .px(px(8.))
                    .rounded(RADIUS)
                    .when(selected, |d| d.bg(if focused { p.sel } else { p.sel_blur }))
                    .when(!selected, move |d| d.hover(move |s| s.bg(hover)))
                    .child(self.render_tick(&r.id, selected, ix, cx))
                    .child(render_body(&p, &r));
                let row = self.wire_row(row, &r, selected, cx);
                div().h(px(ROW_TALL)).pb(px(2.)).child(row).into_any_element()
            })
            .collect()
    }

    /// A tick box that adds the row to the selection or takes it out, as a
    /// Cmd-click does.
    fn render_tick(&self, id: &str, selected: bool, ix: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let id = id.to_string();
        div().flex_none().on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()).child(
            Checkbox::new(("tick", ix)).checked(selected).on_click(cx.listener(move |this, _: &bool, window, cx| {
                window.focus(&this.list_focus, cx);
                this.state.select(&id, true, false);
                this.sync_selection(cx);
            })),
        )
    }

    /// What every torrent row answers to, in either view: click and
    /// Shift/Cmd-click to select, right-click for the menu, double-click to
    /// show the files, and drag onto a label.
    pub fn wire_row(&self, row: Stateful<Div>, r: &Row, selected: bool, cx: &mut Context<Self>) -> Stateful<Div> {
        let drag_ids = if selected { self.state.selection.clone() } else { vec![r.id.clone()] };
        let chip = if drag_ids.len() > 1 { format!("{} torrents", drag_ids.len()) } else { r.name.clone() };
        let id = r.id.clone();
        let id_up = r.id.clone();
        let id_menu = r.id.clone();
        let id_click = r.id.clone();
        row.on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                window.focus(&this.list_focus, cx);
                this.row_mouse_down(&id, ev, cx);
                cx.stop_propagation();
            }),
        )
        .on_mouse_up(MouseButton::Left, cx.listener(move |this, _: &MouseUpEvent, _, cx| this.row_mouse_up(&id_up, cx)))
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                window.focus(&this.list_focus, cx);
                this.row_context_menu(&id_menu, ev.position, window, cx);
                cx.stop_propagation();
            }),
        )
        .on_click(cx.listener(move |this, ev: &ClickEvent, _, cx| {
            if ev.click_count() == 2 {
                this.reveal(id_click.clone(), None, cx);
            }
        }))
        .on_drag(DraggedTorrents { ids: drag_ids }, move |_, _, _, cx| cx.new(|_| DragChip(chip.clone().into())))
    }
}

fn render_body(p: &Palette, r: &Row) -> Div {
    let size = fmt::bytes((if r.wanted_size > 0 { r.wanted_size } else { r.size }) as f64, false);
    let color = state_color(p, r);
    let finished = r.done >= 1.0;
    let done = if r.done.is_nan() { 0.0 } else { r.done.clamp(0.0, 1.0) } as f32;
    let status = if finished || r.state == State::Error { status_text(r) } else { format!("{} {}", status_text(r), fmt::pct(r.done, 1)) };
    let stopped = matches!(r.state, State::Stopped | State::Paused);

    let mut meta = meta_line(p).font_features(tabular()).child(
        div().flex().items_center().gap(px(5.)).min_w_0().text_color(color).child(dot(color)).child(div().min_w_0().truncate().child(status)),
    );
    let speeds = [(r.download_speed, "↓"), (r.upload_speed, "↑")]
        .iter()
        .filter(|(v, _)| *v > 0.0)
        .map(|(v, s)| format!("{s} {}", fmt::speed(*v, false)))
        .collect::<Vec<_>>()
        .join("  ");
    if !speeds.is_empty() {
        meta = meta.child(div().flex_none().child(speeds));
    }
    if !finished && r.download_speed > 0.0 {
        meta = meta.child(div().flex_none().child(format!("{} left", fmt::eta(r.eta))));
    } else if finished && r.uploaded > 0 {
        meta = meta.child(div().flex_none().child(format!("ratio {}", fmt::ratio(r.ratio))));
    }
    if !stopped && (r.seeds > 0 || r.peers > 0) {
        meta = meta.child(div().flex_none().child(format!("{} · {}", count(r.seeds, "seed", "seeds"), count(r.peers, "peer", "peers"))));
    }
    if !r.label.is_empty() {
        meta = meta.child(tag(p, r.label.clone()));
    }

    div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(5.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .child(div().flex_1().min_w_0().truncate().text_size(FONT_SIZE).font_weight(FontWeight::MEDIUM).text_color(p.ink).child(r.name.clone()))
                .child(div().flex_none().text_size(TEXT_META).text_color(p.ink_dim).font_features(tabular()).child(size)),
        )
        .child(
            div()
                .h(px(4.))
                .w_full()
                .rounded(px(2.))
                .overflow_hidden()
                .bg(p.bar_track)
                .child(div().h_full().w(relative(done)).bg(crate::progress::fill(p, bar_kind(r)))),
        )
        .child(meta.whitespace_nowrap().overflow_hidden())
}
