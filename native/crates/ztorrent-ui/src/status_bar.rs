use crate::actions::{About, ToggleAltSpeed};
use crate::icons::icon;
use crate::theme::{Theme, tabular};
use crate::toolbar::alt_speed_hint;
use crate::workspace::Workspace;
use gpui::prelude::*;
use gpui::*;
use gpui_component::tooltip::Tooltip;
use ztorrent_core::fmt;

impl Workspace {
    pub fn render_status_bar(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Theme::get(cx).clone();
        let g = &self.state.globals;
        let pill = || div().flex().items_center().gap(px(6.)).h(px(22.)).px(px(10.)).rounded(px(5.));
        let dot = |c: Hsla| div().size(px(8.)).rounded_full().bg(c).flex_none();

        let dht = if g.dht_enabled {
            if g.dht_ready {
                pill().pl_0().child(dot(p.ok)).child(format!("DHT: {} nodes", g.dht_nodes))
            } else {
                pill().pl_0().child(dot(p.warn)).child("DHT: starting")
            }
        } else {
            pill().pl_0().child(dot(p.err)).child("DHT: disabled")
        };

        let alt = self.state.settings.alt_speed_enabled;
        let hint = alt_speed_hint(alt);
        let (alt_bg, alt_fg) = if alt { (p.warn_soft, p.warn) } else { (gpui::transparent_black(), p.ink_dim) };
        let alt_pill = pill()
            .id("sb-alt")
            .gap(px(7.))
            .bg(alt_bg)
            .text_color(alt_fg)
            .when(alt, |d| d.font_weight(FontWeight::MEDIUM))
            .child(icon("alt-speed", 13., alt_fg))
            .child(self.state.speed_cap_label())
            .hover(move |s| if alt { s } else { s.bg(p.hover).text_color(p.ink) })
            .tooltip(move |window, cx| Tooltip::new(hint.clone()).build(window, cx))
            .on_click(|_, window, cx| window.dispatch_action(Box::new(ToggleAltSpeed), cx));

        let version = self.version.clone();
        let brand = pill()
            .id("sb-brand")
            .opacity(0.75)
            .hover(|s| s.opacity(1.0).bg(p.hover))
            .child(icon("logo", 14., p.ink_dim))
            .child(version.clone())
            .tooltip(move |window, cx| Tooltip::new(format!("ztorrent {version} -- About ztorrent")).build(window, cx))
            .on_click(|_, window, cx| window.dispatch_action(Box::new(About), cx));

        div()
            .flex()
            .flex_none()
            .items_center()
            .h(px(30.))
            .px(px(14.))
            .text_size(px(12.))
            .text_color(p.ink_dim)
            .bg(p.chrome)
            .border_t_1()
            .border_color(p.line)
            .font_features(tabular())
            .child(dht)
            .child(pill().child(div().text_color(p.ink_faint).child("Torrents:")).child(self.state.rows.len().to_string()))
            .child(div().flex_1())
            .children(self.render_update_pill(cx))
            .child(alt_pill)
            .child(
                pill()
                    .child(icon("down", 13., p.down))
                    .child(format!("D: {}", fmt::speed(g.download_speed, false)))
                    .child(div().text_color(p.ink_faint).child(format!("T: {}", fmt::bytes(g.downloaded as f64, false)))),
            )
            .child(
                pill()
                    .child(icon("up", 13., p.up))
                    .child(format!("U: {}", fmt::speed(g.upload_speed, false)))
                    .child(div().text_color(p.ink_faint).child(format!("T: {}", fmt::bytes(g.uploaded as f64, false)))),
            )
            .child(brand)
    }

    /// The update pill. Anything the user has not asked about -- an idle check,
    /// a check that came back with nothing -- shows nothing at all.
    fn render_update_pill(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        use ztorrent_core::update::{UpdateState, UpdateStep};
        let p = Theme::get(cx).clone();
        let u = self.update.as_ref()?;
        let version = u.version.clone().unwrap_or_default();
        // (text, clickable, ready, busy)
        let (text, act, ready, busy) = match u.state {
            UpdateState::Checking if u.manual => ("Checking for updates…".to_string(), false, false, true),
            UpdateState::Available => (format!("Update available · {version}"), true, false, false),
            UpdateState::Downloading => {
                let pct = if u.size > 0 { (u.received as f64 / u.size as f64 * 100.0).round() as u64 } else { 0 };
                (format!("Downloading update · {pct}%"), false, false, true)
            }
            UpdateState::Staging => ("Preparing update…".to_string(), false, false, true),
            UpdateState::Ready => (format!("Restart to update · {version}"), true, true, false),
            // A download that died is the one failure that left something half-done.
            UpdateState::Error if u.error_from == Some(UpdateStep::Download) => ("Update download failed".to_string(), true, false, false),
            _ => return None,
        };
        let (bg, fg) = if ready { (p.accent_soft, p.accent) } else { (gpui::transparent_black(), p.ink_dim) };
        let hover = p.hover;
        let pill = div()
            .id("sb-update")
            .debug_selector(|| "sb-update".into())
            .flex()
            .items_center()
            .gap(px(7.))
            .h(px(22.))
            .px(px(10.))
            .rounded(px(5.))
            .bg(bg)
            .text_color(fg)
            .when(ready, |d| d.font_weight(FontWeight::MEDIUM))
            .child(div().when(busy, |d| d.opacity(0.6)).child(icon(if ready { "restart" } else { "download" }, 13., fg)))
            .child(text)
            .when(act, |d| {
                d.cursor_pointer()
                    .hover(move |s| if ready { s } else { s.bg(hover) })
                    .on_click(cx.listener(|this, _, window, cx| this.on_update_click(window, cx)))
            });
        Some(pill.into_any_element())
    }
}
