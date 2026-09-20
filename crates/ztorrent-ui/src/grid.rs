//! The torrent list: a virtualised grid whose columns sort, resize and hide.

use crate::actions::*;
use crate::icons::icon;
use crate::progress::progress_bar;
use crate::theme::{Palette, ROW_H, Theme, tabular};
use crate::workspace::{DraggedTorrents, Resize, Workspace};
use gpui::prelude::*;
use gpui::*;
use gpui_component::native_menu::NativeMenu;
use std::ops::Range;
use ztorrent_core::columns::{self, ALL_COLUMNS, ColumnDef};
use ztorrent_core::{Row, State, bar_kind, fmt, state_icon, status_text};

const HEADER_H: f32 = 32.;

pub struct DragChip(pub SharedString);

impl Render for DragChip {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Theme::get(cx);
        div()
            .px(px(11.))
            .py(px(5.))
            .rounded(px(20.))
            .bg(p.accent)
            .text_color(gpui::white())
            .text_size(px(12.))
            .font_weight(FontWeight::MEDIUM)
            .whitespace_nowrap()
            .child(self.0.clone())
    }
}

impl Workspace {
    pub fn visible_columns(&self) -> Vec<&'static ColumnDef> {
        self.state.columns.iter().filter_map(|k| columns::find(k)).collect()
    }

    pub fn render_grid(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Theme::get(cx).clone();
        let cols = self.visible_columns();
        let total: f32 = cols.iter().map(|c| self.state.width(c.key)).sum::<f32>() + 8.;
        let count = self.state.visible_rows().len();
        self.state.list_focused = self.list_focus.contains_focused(window, cx);

        let body = (count > 0).then(|| {
            uniform_list("torrents", count, cx.processor(|this, range: Range<usize>, _window, cx| this.render_rows(range, cx)))
                .track_scroll(&self.list_scroll)
                .size_full()
        });
        // The empty state sits over the visible pane, below the header, not in
        // the scrolled area: that is as wide as every column together, and
        // centring in it put the message off to the right.
        let empty = (count == 0).then(|| {
            div().absolute().top(px(HEADER_H)).left_0().right_0().bottom_0().child(self.render_empty(&p))
        });

        div()
            .id("grid")
            .relative()
            .track_focus(&self.list_focus)
            .key_context("TorrentList")
            .flex()
            .flex_col()
            .size_full()
            .bg(p.bg)
            .on_action(cx.listener(|this, _: &SelectUp, _, cx| this.move_selection(false, false, cx)))
            .on_action(cx.listener(|this, _: &SelectDown, _, cx| this.move_selection(true, false, cx)))
            .on_action(cx.listener(|this, _: &ExtendUp, _, cx| this.move_selection(false, true, cx)))
            .on_action(cx.listener(|this, _: &ExtendDown, _, cx| this.move_selection(true, true, cx)))
            .on_action(cx.listener(|this, _: &SelectAll, _, cx| {
                this.state.select_all_visible();
                this.sync_selection(cx);
            }))
            .on_action(cx.listener(|this, _: &ClearSelection, _, cx| {
                this.state.selection.clear();
                this.sync_selection(cx);
            }))
            .on_action(cx.listener(|this, _: &DeleteKey, window, cx| this.remove(false, window, cx)))
            .on_action(cx.listener(|this, _: &DeleteDataKey, window, cx| this.remove(true, window, cx)))
            .on_action(cx.listener(|this, _: &TogglePause, _, cx| this.toggle_pause(cx)))
            .on_action(cx.listener(|this, _: &EnterKey, _, cx| this.reveal_selected(cx)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    window.focus(&this.list_focus, cx);
                    this.state.selection.clear();
                    this.sync_selection(cx);
                }),
            )
            .child(
                div()
                    .id("grid-x")
                    .flex_1()
                    .min_h_0()
                    .overflow_x_scroll()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .h_full()
                            .min_w(px(total))
                            .w_full()
                            .child(self.render_header(&p, &cols, cx))
                            .child(div().flex_1().min_h_0().pt(px(4.)).children(body)),
                    ),
            )
            .children(empty)
    }

    fn render_empty(&self, p: &Palette) -> Div {
        let message = if self.state.rows.is_empty() { "No torrents yet." } else { "No torrents match this view." };
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(10.))
            .text_color(p.ink_faint)
            .child(div().opacity(0.4).mb(px(4.)).child(icon("logo", 40., p.ink_faint)))
            .child(message)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .child("Drop a")
                    .child(div().font_weight(FontWeight::BOLD).child(".torrent"))
                    .child("file here, or press")
                    .child(
                        div()
                            .px(px(6.))
                            .py(px(1.))
                            .rounded(px(4.))
                            .bg(p.sunken)
                            .border_1()
                            .border_color(p.line)
                            .text_size(px(11.))
                            .text_color(p.ink_dim)
                            .child(accel("O", false)),
                    )
                    .child("to add one."),
            )
    }

    fn render_header(&self, p: &Palette, cols: &[&'static ColumnDef], cx: &mut Context<Self>) -> impl IntoElement {
        let mut head = div().flex().flex_none().h(px(HEADER_H)).border_b_1().border_color(p.line).bg(p.bg);
        for col in cols {
            let key = col.key;
            let w = self.state.width(key);
            let arrow = if self.state.sort_key == key { if self.state.sort_desc { "▼" } else { "▲" } } else { "" };
            let label = div().min_w_0().truncate().child(col.label);
            let sort = div().flex_none().text_size(px(8.)).text_color(p.accent).child(arrow);
            let mut cell = div()
                .id(SharedString::from(format!("gh-{key}")))
                .relative()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(4.))
                .w(px(w))
                .h_full()
                .px(px(10.))
                .text_size(px(12.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(p.ink_dim)
                .whitespace_nowrap()
                .overflow_hidden()
                .hover(|s| s.text_color(p.ink));
            // Right-aligned headers put the arrow on the left, so the label stays
            // flush with the digits below it.
            cell = if col.numeric { cell.justify_end().child(sort).child(label) } else { cell.child(label).child(sort) };
            let divider = p.divider;
            let accent = p.accent;
            cell = cell
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.state.toggle_sort(key);
                    cx.notify();
                }))
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                        let mut menu = NativeMenu::new();
                        for c in ALL_COLUMNS {
                            let on = this.state.columns.iter().any(|k| k == c.key);
                            let label = if c.label == "#" { "Order (#)" } else { c.label };
                            let action = Box::new(ToggleColumn { key: c.key.to_string() });
                            menu = if c.key == "#" || c.key == "name" {
                                menu.menu_with_disabled(label, true, action)
                            } else {
                                menu.menu_with_check(label, on, action)
                            };
                        }
                        menu.show(ev.position, window, cx);
                        cx.stop_propagation();
                    }),
                )
                .child(
                    div()
                        .id(SharedString::from(format!("grip-{key}")))
                        .absolute()
                        .top_0()
                        .right_0()
                        .w(px(9.))
                        .h_full()
                        .cursor_col_resize()
                        .flex()
                        .justify_end()
                        .items_center()
                        .pr(px(4.))
                        .child(div().w(px(1.)).h(px(16.)).bg(divider))
                        .hover(move |s| s.bg(accent.opacity(0.0)))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                                this.resize = Some(Resize::Column { key, start_x: f32::from(ev.position.x), start_w: this.state.width(key) });
                                cx.stop_propagation();
                            }),
                        ),
                );
            head = head.child(cell);
        }
        head
    }

    fn render_rows(&mut self, range: Range<usize>, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let p = Theme::get(cx).clone();
        let cols = self.visible_columns();
        let rows: Vec<Row> = self.state.visible_rows().into_iter().skip(range.start).take(range.len()).cloned().collect();
        let focused = self.state.list_focused;
        rows.into_iter()
            .enumerate()
            .map(|(k, r)| {
                let ix = range.start + k;
                let selected = self.state.is_selected(&r.id);
                let drag_ids = if selected { self.state.selection.clone() } else { vec![r.id.clone()] };
                let chip = if drag_ids.len() > 1 { format!("{} torrents", drag_ids.len()) } else { r.name.clone() };
                let hover = p.hover;
                let mut row = div()
                    .id(("row", ix))
                    .flex()
                    .items_center()
                    .h(ROW_H)
                    .w_full()
                    .whitespace_nowrap()
                    .when(selected, |d| d.bg(if focused { p.sel } else { p.sel_blur }))
                    .when(!selected, move |d| d.hover(move |s| s.bg(hover)));
                for col in &cols {
                    row = row.child(self.render_cell(&p, col, &r, ix));
                }
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
                        this.reveal(id_click.clone(), None, false, cx);
                    }
                }))
                .on_drag(DraggedTorrents { ids: drag_ids }, move |_, _, _, cx| cx.new(|_| DragChip(chip.clone().into())))
                .into_any_element()
            })
            .collect()
    }

    fn render_cell(&self, p: &Palette, col: &ColumnDef, r: &Row, ix: usize) -> Div {
        let w = self.state.width(col.key);
        let cell = div().flex().flex_none().items_center().w(px(w)).h_full().px(px(10.)).overflow_hidden();
        let cell = if col.numeric { cell.justify_end().font_features(tabular()) } else { cell };
        let text = |s: String| div().min_w_0().truncate().child(s);
        match col.key {
            "#" => cell.child((ix + 1).to_string()),
            "name" => {
                let ic = state_icon(r);
                let color = match ic {
                    "downloading" => p.down,
                    "seeding" => p.ok,
                    "completed" => p.fin,
                    "error" => p.err,
                    _ => p.ink_faint,
                };
                cell.gap(px(8.)).child(icon(ic, 15., color)).child(text(r.name.clone()))
            }
            "size" => cell.child(fmt::bytes((if r.wanted_size > 0 { r.wanted_size } else { r.size }) as f64, false)),
            "status" => {
                // Progress reads from the bar, so the number shows only while it moves.
                let label = if r.done >= 1.0 || r.state == State::Error {
                    status_text(r)
                } else {
                    format!("{} {}", status_text(r), fmt::pct(r.done, 1))
                };
                cell.child(progress_bar(p, bar_kind(r), r.done, Some(label), px((w - 20.).max(10.)), 22.))
            }
            "seeds" => cell.child(if r.state == State::Stopped { String::new() } else { r.seeds.to_string() }),
            "peers" => cell.child(if r.state == State::Stopped { String::new() } else { r.peers.to_string() }),
            "downloadSpeed" => cell.child(fmt::speed(r.download_speed, true)),
            "uploadSpeed" => cell.child(fmt::speed(r.upload_speed, true)),
            "eta" => cell.child(if r.done >= 1.0 || r.download_speed == 0.0 { String::new() } else { fmt::eta(r.eta) }),
            "downloaded" => cell.child(fmt::bytes(r.downloaded as f64, true)),
            "uploaded" => cell.child(fmt::bytes(r.uploaded as f64, true)),
            "ratio" => cell.child(fmt::ratio(r.ratio)),
            "availability" => cell.child(if r.availability > 0.0 { format!("{:.3}", r.availability) } else { String::new() }),
            "label" => cell.child(text(r.label.clone())),
            "addedOn" => cell.child(fmt::datetime(r.added_on)),
            "completedOn" => cell.child(fmt::datetime(r.completed_on)),
            "savePath" => cell.child(text(r.save_path.clone())),
            _ => cell,
        }
    }
}
