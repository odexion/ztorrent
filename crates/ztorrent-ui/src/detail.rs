//! The detail pane: General, Trackers, Peers, Pieces, Files, Speed, Logger.

use crate::actions::*;
use crate::icons::icon;
use crate::progress::progress_bar;
use crate::state::{HISTORY_LEN, History, Tab};
use crate::theme::{MONO, Palette, RADIUS, ROW_H, Theme, tabular};
use crate::workspace::Workspace;
use gpui::prelude::*;
use gpui::*;
use gpui_component::native_menu::NativeMenu;
use ztorrent_core::{Details, Row, bar_kind, fmt, status_text};

fn dash(s: String) -> String {
    if s.is_empty() { "—".into() } else { s }
}

impl Workspace {
    pub fn render_detail(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Theme::get(cx).clone();
        let mut strip = div().flex().flex_none().items_center().gap(px(2.)).h(px(40.)).px(px(10.)).border_b_1().border_color(p.line);
        for tab in Tab::ALL {
            let active = self.state.tab == tab;
            strip = strip.child(
                div()
                    .id(tab.label())
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .h(px(28.))
                    .px(px(10.))
                    .rounded(RADIUS)
                    .text_size(px(12.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(if active { p.ink } else { p.ink_dim })
                    .when(active, |d| d.bg(p.sel_quiet))
                    .when(!active, |d| d.hover(|s| s.text_color(p.ink).bg(p.hover)))
                    .child(icon(tab.icon(), 15., if active { p.ink } else { p.ink_dim }))
                    .child(tab.label())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.state.tab = tab;
                        cx.notify();
                    })),
            );
        }

        let row = self.state.first_selected().cloned();
        let details = self.state.details.clone().filter(|d| row.as_ref().is_some_and(|r| r.id == d.id));
        let body: AnyElement = match (self.state.tab, &row) {
            (Tab::Speed, _) => self.render_speed(&p, row.as_ref()).into_any_element(),
            (Tab::Logger, _) => self.render_logger(&p).into_any_element(),
            (_, None) => empty(&p, "Select a torrent to see its details.").into_any_element(),
            (Tab::General, Some(r)) => render_general(&p, r, details.as_ref()).into_any_element(),
            (tab, Some(_)) => match details {
                None => div().into_any_element(),
                Some(d) => match tab {
                    Tab::Trackers => render_trackers(&p, &d).into_any_element(),
                    Tab::Peers => render_peers(&p, &d).into_any_element(),
                    Tab::Pieces => render_pieces(&p, &d).into_any_element(),
                    Tab::Files => self.render_files(&p, &d, cx).into_any_element(),
                    _ => div().into_any_element(),
                },
            },
        };
        let _ = window;
        div().flex().flex_col().size_full().bg(p.bg).child(strip).child(div().flex_1().min_h_0().child(body))
    }

    fn render_files(&self, p: &Palette, d: &Details, cx: &mut Context<Self>) -> impl IntoElement {
        if d.files.is_empty() {
            return empty(p, "File list appears once the torrent metadata arrives.").into_any_element();
        }
        let header = table_head(p, &[("#", px(36.), true), ("Name", px(0.), false), ("Size", px(84.), true), ("Progress", px(90.), false), ("%", px(56.), true), ("Priority", px(110.), false)]);
        let mut body = div().flex().flex_col();
        for f in &d.files {
            let index = f.index;
            let priority = f.priority;
            let (label, color, weight) = match f.priority {
                0 => ("Don't Download", p.ink_faint, FontWeight::NORMAL),
                2 => ("High", p.ok, FontWeight::MEDIUM),
                _ => ("Normal", p.ink, FontWeight::NORMAL),
            };
            let hover = p.hover;
            body = body.child(
                div()
                    .id(("file", index))
                    .flex()
                    .items_center()
                    .h(ROW_H)
                    .hover(move |s| s.bg(hover))
                    .child(cell(px(36.), true).text_color(p.ink_faint).child((index + 1).to_string()))
                    .child(div().flex_1().min_w_0().px(px(10.)).truncate().child(f.name.clone()))
                    .child(cell(px(84.), true).child(fmt::bytes(f.length as f64, false)))
                    .child(cell(px(90.), false).child(
                        div().h(px(12.)).w_full().bg(p.bar_track).overflow_hidden().child(div().h_full().w(relative(f.progress.clamp(0., 1.) as f32)).bg(p.bar)),
                    ))
                    .child(cell(px(56.), true).child(fmt::pct(f.progress, 1)))
                    .child(cell(px(110.), false).text_color(color).font_weight(weight).child(label))
                    .on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| {
                        if ev.click_count() == 2 {
                            if let Some(id) = this.state.selection.first().cloned() {
                                this.open_file(id, index, true, window, cx);
                            }
                        }
                    }))
                    .on_mouse_down(MouseButton::Right, move |ev, window, cx| {
                        let reveal = crate::workspace::reveal_label();
                        NativeMenu::new()
                            .menu("Open", Box::new(OpenFile { index }))
                            .menu(reveal, Box::new(RevealFile { index }))
                            .separator()
                            .menu_with_check("High Priority", priority == 2, Box::new(SetPriority { index, priority: 2 }))
                            .menu_with_check("Normal Priority", priority == 1, Box::new(SetPriority { index, priority: 1 }))
                            .menu_with_check("Don't Download", priority == 0, Box::new(SetPriority { index, priority: 0 }))
                            .show(ev.position, window, cx);
                    }),
            );
        }
        div().id("files").size_full().overflow_y_scroll().font_features(tabular()).child(header).child(body).into_any_element()
    }

    fn render_speed(&self, p: &Palette, row: Option<&Row>) -> impl IntoElement {
        let history = match row {
            Some(r) => self.state.history.get(&r.id).cloned().unwrap_or_default(),
            None => self.state.global_history.clone(),
        };
        let title = row.map(|r| r.name.clone()).unwrap_or_else(|| "All torrents".into());
        let now_d = history.down.back().copied().unwrap_or(0.0);
        let now_u = history.up.back().copied().unwrap_or(0.0);
        let peak = history.down.iter().chain(history.up.iter()).copied().fold(1024.0_f64, f64::max) * 1.15;

        let head = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(18.))
            .px(px(18.))
            .py(px(10.))
            .text_size(px(12.))
            .text_color(p.ink_dim)
            .border_b_1()
            .border_color(p.line)
            .child(legend(p.down, "Download"))
            .child(legend(p.up, "Upload"))
            .child(div().flex().gap(px(4.)).child("Showing:").child(div().text_color(p.ink).font_weight(FontWeight::MEDIUM).child(title)))
            .child(
                div()
                    .flex()
                    .gap(px(4.))
                    .child("D")
                    .child(div().text_color(p.ink).child(fmt::speed(now_d, false)))
                    .child("· U")
                    .child(div().text_color(p.ink).child(fmt::speed(now_u, false))),
            );

        let (pl, pr, pt, pb) = (58.0_f32, 8.0_f32, 8.0_f32, 16.0_f32);
        let colors = p.clone();
        let chart = canvas(|_, _, _| {}, move |bounds, _, window, _| paint_speed(&colors, &history, peak, bounds, (pl, pr, pt, pb), window));

        let mut labels = div().absolute().left_0().top(px(pt)).bottom(px(pb)).w(px(pl - 6.));
        for i in 0..=4 {
            let frac = i as f32 / 4.0;
            labels = labels.child(
                div()
                    .absolute()
                    .right_0()
                    .top(relative(frac))
                    .mt(px(-8.))
                    .text_size(px(11.))
                    .text_color(p.ink_faint)
                    .child(fmt::speed(peak * (1.0 - frac as f64), false)),
            );
        }
        let axis = div()
            .absolute()
            .bottom_0()
            .left(px(pl))
            .right(px(pr))
            .h(px(pb))
            .flex()
            .justify_between()
            .text_size(px(11.))
            .text_color(p.ink_faint)
            .child(format!("{HISTORY_LEN}s ago"))
            .child("now");

        div()
            .flex()
            .flex_col()
            .size_full()
            .child(head)
            .child(div().relative().flex_1().min_h_0().child(chart.size_full()).child(labels).child(axis))
    }

    fn render_logger(&self, p: &Palette) -> impl IntoElement {
        let mut list = div().flex().flex_col().py(px(8.)).font_family(MONO).text_size(px(12.)).line_height(px(20.));
        for line in &self.state.log {
            let color = match line.level {
                ztorrent_core::LogLevel::Warn => p.warn,
                ztorrent_core::LogLevel::Error => p.err,
                ztorrent_core::LogLevel::Info => p.ink,
            };
            list = list.child(
                div()
                    .px(px(18.))
                    .text_color(color)
                    .child(div().flex().gap(px(6.)).child(div().flex_none().text_color(p.ink_faint).child(format!("[{}]", fmt::clock(line.time)))).child(line.message.clone())),
            );
        }
        div().id("logger").size_full().overflow_y_scroll().track_scroll(&self.log_scroll).child(list)
    }
}

fn paint_speed(p: &Palette, h: &History, peak: f64, bounds: Bounds<Pixels>, pad: (f32, f32, f32, f32), window: &mut Window) {
    let (pl, pr, pt, pb) = pad;
    let (x0, y0) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
    let (w, ht) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    let (gw, gh) = (w - pl - pr, ht - pt - pb);
    if gw <= 0.0 || gh <= 0.0 {
        return;
    }
    for i in 0..=4 {
        let y = (y0 + pt + gh * i as f32 / 4.0).round();
        window.paint_quad(fill(Bounds::new(point(px(x0 + pl), px(y)), size(px(gw), px(1.))), p.line));
    }
    let n = HISTORY_LEN as f32;
    let x_at = |i: f32| x0 + pl + gw * i / (n - 1.0);
    let y_at = |v: f64| y0 + pt + gh - (v.min(peak) / peak) as f32 * gh;
    let mut series = |data: &std::collections::VecDeque<f64>, stroke: Hsla, area: Hsla| {
        if data.len() < 2 {
            return;
        }
        let off = n - data.len() as f32;
        let mut filled = PathBuilder::fill();
        filled.move_to(point(px(x_at(off)), px(y0 + pt + gh)));
        for (i, v) in data.iter().enumerate() {
            filled.line_to(point(px(x_at(off + i as f32)), px(y_at(*v))));
        }
        filled.line_to(point(px(x_at(off + data.len() as f32 - 1.0)), px(y0 + pt + gh)));
        filled.close();
        if let Ok(path) = filled.build() {
            window.paint_path(path, area);
        }
        let mut line = PathBuilder::stroke(px(1.4));
        for (i, v) in data.iter().enumerate() {
            let pt = point(px(x_at(off + i as f32)), px(y_at(*v)));
            if i == 0 { line.move_to(pt) } else { line.line_to(pt) }
        }
        if let Ok(path) = line.build() {
            window.paint_path(path, stroke);
        }
    };
    series(&h.down, p.down, p.chart_down_fill);
    series(&h.up, p.up, p.chart_up_fill);
    window.paint_quad(fill(Bounds::new(point(px(x0 + pl), px(y0 + pt)), size(px(1.), px(gh))), p.line_hard));
    window.paint_quad(fill(Bounds::new(point(px(x0 + pl), px(y0 + pt + gh)), size(px(gw), px(1.))), p.line_hard));
}

fn empty(p: &Palette, msg: &'static str) -> Div {
    div().p(px(18.)).text_color(p.ink_faint).child(msg)
}

fn legend(color: Hsla, label: &'static str) -> Div {
    div().flex().items_center().gap(px(6.)).child(div().size(px(9.)).rounded(px(2.)).bg(color)).child(label)
}

fn cell(width: Pixels, numeric: bool) -> Div {
    let d = div().flex().flex_none().items_center().w(width).px(px(10.)).overflow_hidden().whitespace_nowrap();
    if numeric { d.justify_end() } else { d }
}

/// A header row; a zero width means "take what is left".
fn table_head(p: &Palette, cols: &[(&'static str, Pixels, bool)]) -> Div {
    let mut head = div().flex().flex_none().items_center().h(px(30.)).border_b_1().border_color(p.line).text_size(px(12.)).font_weight(FontWeight::MEDIUM).text_color(p.ink_dim);
    for (label, width, numeric) in cols {
        head = head.child(if *width == px(0.) { div().flex_1().min_w_0().px(px(10.)).child(*label) } else { cell(*width, *numeric).child(*label) });
    }
    head
}

fn render_general(p: &Palette, r: &Row, d: Option<&Details>) -> impl IntoElement {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
    let grid = |title: &'static str, items: Vec<(&'static str, String, bool)>| {
        let mut g = div().flex().flex_col().gap(px(7.)).min_w_0();
        for (k, v, mono) in items {
            g = g.child(
                div()
                    .flex()
                    .gap(px(12.))
                    .child(div().w(px(128.)).flex_none().text_color(p.ink_dim).child(k))
                    .child(div().flex_1().min_w_0().truncate().when(mono, |d| d.font_family(MONO).text_size(px(12.))).child(v)),
            );
        }
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .child(div().mb(px(10.)).text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).text_color(p.ink_faint).child(title.to_uppercase()))
            .child(g)
    };
    let pieces = d.map(|d| format!("{} × {} (have {})", d.piece_count, fmt::bytes(d.piece_length as f64, false), d.have)).unwrap_or("—".into());
    let total = if r.wanted_size != r.size { format!("{} ({} selected)", fmt::bytes(r.size as f64, false), fmt::bytes(r.wanted_size as f64, false)) } else { fmt::bytes(r.size as f64, false) };
    let transfer = vec![
        ("Time Elapsed:", fmt::duration(now - r.added_on), false),
        ("Remaining:", if r.done >= 1.0 { "—".into() } else { fmt::eta(r.eta) }, false),
        ("Downloaded:", fmt::bytes(r.downloaded as f64, false), false),
        ("Uploaded:", fmt::bytes(r.uploaded as f64, false), false),
        ("Download Speed:", fmt::speed(r.download_speed, false), false),
        ("Upload Speed:", fmt::speed(r.upload_speed, false), false),
        ("Share Ratio:", fmt::ratio(r.ratio), false),
        ("Seeds:", format!("{} connected", r.seeds), false),
        ("Peers:", format!("{} connected", r.peers), false),
        ("Availability:", if r.availability > 0.0 { format!("{:.3}", r.availability) } else { "—".into() }, false),
        ("Status:", status_text(r), false),
    ];
    let torrent = vec![
        ("Name:", r.name.clone(), false),
        ("Save As:", r.save_path.clone(), false),
        ("Total Size:", total, false),
        ("Pieces:", pieces, false),
        ("Hash:", r.info_hash.clone(), true),
        ("Comment:", dash(d.map(|d| d.comment.clone()).unwrap_or_default()), false),
        ("Created By:", dash(d.map(|d| d.created_by.clone()).unwrap_or_default()), false),
        ("Created On:", d.filter(|d| d.created_on > 0).map(|d| fmt::datetime(d.created_on)).unwrap_or("—".into()), false),
        ("Added On:", fmt::datetime(r.added_on), false),
        ("Completed On:", if r.completed_on > 0 { fmt::datetime(r.completed_on) } else { "—".into() }, false),
        ("Private:", d.map(|d| if d.private { "Yes (DHT/PEX off)".to_string() } else { "No".into() }).unwrap_or("—".into()), false),
        ("Order:", d.map(|d| if d.sequential { "Sequential".to_string() } else { "Rarest first".into() }).unwrap_or("—".into()), false),
    ];
    div()
        .id("general")
        .size_full()
        .overflow_y_scroll()
        .px(px(18.))
        .py(px(16.))
        .font_features(tabular())
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(12.))
                .mb(px(22.))
                .child(progress_bar(p, bar_kind(r), r.done, None, px(460.), 28.))
                .child(div().font_weight(FontWeight::MEDIUM).child(fmt::pct(r.done, 2))),
        )
        .child(div().flex().gap(px(40.)).child(grid("Transfer", transfer)).child(grid("Torrent", torrent)))
}

fn render_trackers(p: &Palette, d: &Details) -> impl IntoElement {
    let mut body = div().flex().flex_col();
    for (i, t) in d.trackers.iter().enumerate() {
        let color = match t.status.as_str() {
            "Working" => p.ok,
            "Not working" => p.err,
            _ => p.ink_faint,
        };
        let url = t.url.clone();
        let hover = p.hover;
        body = body.child(
            div()
                .id(("tracker", i))
                .flex()
                .items_center()
                .h(ROW_H)
                .hover(move |s| s.bg(hover))
                .child(div().w(relative(0.52)).px(px(10.)).truncate().child(t.url.clone()))
                .child(div().w(relative(0.18)).px(px(10.)).truncate().text_color(color).child(t.status.clone()))
                .child(div().flex_1().flex().justify_end().px(px(10.)).child(if t.seeds >= 0 { t.seeds.to_string() } else { "—".into() }))
                .child(cell(px(70.), true).child(if t.peers >= 0 { t.peers.to_string() } else { "—".into() }))
                .child(cell(px(90.), true).child(if t.interval > 0 { format!("{}s", t.interval) } else { "—".into() }))
                .on_mouse_down(MouseButton::Right, move |ev, window, cx| {
                    NativeMenu::new()
                        .menu("Add Tracker…", Box::new(AddTrackerPrompt))
                        .menu("Update Tracker", Box::new(Reannounce))
                        .separator()
                        .menu("Copy Tracker URL", Box::new(CopyText { text: url.clone() }))
                        .show(ev.position, window, cx);
                }),
        );
    }
    let header = div()
        .flex()
        .flex_none()
        .items_center()
        .h(px(30.))
        .border_b_1()
        .border_color(p.line)
        .text_size(px(12.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(p.ink_dim)
        .child(div().w(relative(0.52)).px(px(10.)).child("Tracker"))
        .child(div().w(relative(0.18)).px(px(10.)).child("Status"))
        .child(div().flex_1().flex().justify_end().px(px(10.)).child("Seeds"))
        .child(cell(px(70.), true).child("Peers"))
        .child(cell(px(90.), true).child("Update In"));
    let empty_row = d.trackers.is_empty().then(|| div().p(px(10.)).text_color(p.ink_faint).child("No trackers."));
    div()
        .id("trackers")
        .size_full()
        .overflow_y_scroll()
        .font_features(tabular())
        .on_mouse_down(MouseButton::Right, |ev, window, cx| {
            NativeMenu::new().menu("Add Tracker…", Box::new(AddTrackerPrompt)).menu("Update Tracker", Box::new(Reannounce)).show(ev.position, window, cx);
        })
        .child(header)
        .child(body)
        .children(empty_row)
}

fn render_peers(p: &Palette, d: &Details) -> impl IntoElement {
    let mut peers = d.peers.clone();
    peers.sort_by(|a, b| (b.down_speed + b.up_speed).partial_cmp(&(a.down_speed + a.up_speed)).unwrap_or(std::cmp::Ordering::Equal));
    let cols: [(&str, Pixels, bool); 9] = [
        ("IP", px(140.), false),
        ("Conn", px(74.), false),
        ("Flags", px(58.), false),
        ("Client", px(0.), false),
        ("%", px(58.), true),
        ("Down Speed", px(94.), true),
        ("Up Speed", px(88.), true),
        ("Downloaded", px(96.), true),
        ("Uploaded", px(96.), true),
    ];
    let mut body = div().flex().flex_col();
    for (i, peer) in peers.iter().enumerate() {
        let addr = format!("{}:{}", peer.address, peer.port);
        let hover = p.hover;
        body = body.child(
            div()
                .id(("peer", i))
                .flex()
                .items_center()
                .h(ROW_H)
                .hover(move |s| s.bg(hover))
                .child(cell(px(140.), false).child(div().truncate().child(peer.address.clone())))
                .child(cell(px(74.), false).text_color(p.ink_faint).child(peer.kind.clone()))
                .child(cell(px(58.), false).font_family(MONO).text_size(px(11.)).child(peer.flags.clone()))
                .child(div().flex_1().min_w_0().px(px(10.)).truncate().child(peer.client.clone()))
                .child(cell(px(58.), true).child(fmt::pct(peer.progress, 1)))
                .child(cell(px(94.), true).child(fmt::speed(peer.down_speed, true)))
                .child(cell(px(88.), true).child(fmt::speed(peer.up_speed, true)))
                .child(cell(px(96.), true).child(fmt::bytes(peer.downloaded as f64, true)))
                .child(cell(px(96.), true).child(fmt::bytes(peer.uploaded as f64, true)))
                .on_mouse_down(MouseButton::Right, move |ev, window, cx| {
                    NativeMenu::new()
                        .menu("Add Peer…", Box::new(AddPeerPrompt))
                        .menu("Copy Peer Address", Box::new(CopyText { text: addr.clone() }))
                        .show(ev.position, window, cx);
                }),
        );
    }
    let empty_row = d.peers.is_empty().then(|| div().p(px(10.)).text_color(p.ink_faint).child("No peers connected."));
    div()
        .id("peers")
        .size_full()
        .overflow_y_scroll()
        .font_features(tabular())
        .on_mouse_down(MouseButton::Right, |ev, window, cx| {
            NativeMenu::new().menu("Add Peer…", Box::new(AddPeerPrompt)).show(ev.position, window, cx);
        })
        .child(table_head(p, &cols))
        .child(body)
        .children(empty_row)
}

fn render_pieces(p: &Palette, d: &Details) -> impl IntoElement {
    let head = div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(18.))
        .px(px(18.))
        .py(px(10.))
        .text_size(px(12.))
        .text_color(p.ink_dim)
        .border_b_1()
        .border_color(p.line)
        .child(legend(p.bar_seed, "Have"))
        .child(legend(p.piece_partial, "Downloading"))
        .child(legend(p.bar_track, "Missing"))
        .child(match &d.pieces {
            Some(map) => div()
                .flex()
                .gap(px(4.))
                .child(div().text_color(p.ink).child(d.have.to_string()))
                .child("of")
                .child(div().text_color(p.ink).child(map.len().to_string()))
                .child(format!("pieces · {} each", fmt::bytes(d.piece_length as f64, false))),
            None => div().child("Waiting for metadata…"),
        });
    let map = d.pieces.clone().unwrap_or_default();
    let colors = [p.bar_track, p.piece_partial, p.bar_seed];
    let chart = canvas(|_, _, _| {}, move |bounds, _, window, _| {
        let n = map.len();
        if n == 0 {
            return;
        }
        let pad = 6.0_f32;
        let (x0, y0) = (f32::from(bounds.origin.x) + pad, f32::from(bounds.origin.y) + pad);
        let (aw, ah) = (f32::from(bounds.size.width) - pad * 2.0, f32::from(bounds.size.height) - pad * 2.0);
        if aw <= 0.0 || ah <= 0.0 {
            return;
        }
        // A cell size that fits every piece into the box.
        let mut cell = ((aw * ah) / n as f32).sqrt().floor().clamp(2.0, 14.0);
        let mut per_row = (aw / cell).floor().max(1.0) as usize;
        while (n.div_ceil(per_row) as f32) * cell > ah && cell > 2.0 {
            cell -= 1.0;
            per_row = (aw / cell).floor().max(1.0) as usize;
        }
        let gap = if cell > 4.0 { 1.0 } else { 0.0 };
        for (i, v) in map.iter().enumerate() {
            let x = x0 + (i % per_row) as f32 * cell;
            let y = y0 + (i / per_row) as f32 * cell;
            if y > y0 + ah {
                break;
            }
            let color = colors[(*v as usize).min(2)];
            window.paint_quad(fill(Bounds::new(point(px(x), px(y)), size(px(cell - gap), px(cell - gap))), color));
        }
    });
    div().flex().flex_col().size_full().child(head).child(div().flex_1().min_h_0().child(chart.size_full()))
}
