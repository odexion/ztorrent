use crate::actions::{self, accel};
use crate::icons::icon;
use crate::theme::{RADIUS, Theme};
use crate::workspace::Workspace;
use gpui::prelude::*;
use gpui::*;
use gpui_component::input::Input;
use gpui_component::tooltip::Tooltip;

struct Button {
    act: &'static str,
    hint: String,
    action: Box<dyn Action>,
    enabled: bool,
    active: bool,
}

impl Workspace {
    fn toolbar_buttons(&self) -> Vec<Option<Button>> {
        let n = self.state.selection.len();
        let alt = self.state.settings.alt_speed_enabled;
        let delete_key = if cfg!(target_os = "macos") { "⌘⌫".to_string() } else { "Del".to_string() };
        let up = if cfg!(target_os = "macos") { "⌘↑" } else { "Ctrl+Up" };
        let down = if cfg!(target_os = "macos") { "⌘↓" } else { "Ctrl+Down" };
        let b = |act, name: &str, key: String, action: Box<dyn Action>, enabled| {
            Some(Button { act, hint: format!("{name}  ({key})"), action, enabled, active: false })
        };
        vec![
            b("add-file", "Add Torrent", accel("O", false), Box::new(actions::AddTorrent), true),
            b("add-url", "Add Torrent from URL", accel("U", false), Box::new(actions::AddUrl), true),
            b("create", "Create New Torrent", accel("N", false), Box::new(actions::CreateTorrent), true),
            None,
            b("remove", "Remove", delete_key, Box::new(actions::Remove), n > 0),
            None,
            b("start", "Start", accel("R", false), Box::new(actions::Start), n > 0),
            b("pause", "Pause", accel("P", false), Box::new(actions::Pause), n > 0),
            b("stop", "Stop", accel(".", false), Box::new(actions::Stop), n > 0),
            None,
            b("queue-up", "Move Up Queue", up.into(), Box::new(actions::QueueUp), n == 1),
            b("queue-down", "Move Down Queue", down.into(), Box::new(actions::QueueDown), n == 1),
            None,
            Some(Button {
                act: "alt-speed",
                hint: alt_speed_hint(alt),
                action: Box::new(actions::ToggleAltSpeed),
                enabled: true,
                active: alt,
            }),
            b("preferences", "Preferences", accel(",", false), Box::new(actions::Preferences), true),
        ]
    }

    pub fn render_toolbar(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Theme::get(cx).clone();
        let mut bar = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(2.))
            .h(px(48.))
            .px(px(12.))
            .bg(p.chrome)
            .border_b_1()
            .border_color(p.line);

        for button in self.toolbar_buttons() {
            let Some(button) = button else {
                bar = bar.child(div().w(px(1.)).h(px(20.)).mx(px(8.)).bg(p.line));
                continue;
            };
            let Button { act, hint, action, enabled, active } = button;
            // Alternate limits take amber, not the soft accent every other toggle
            // wears: it has to read as "you asked for this" from across the room.
            let (bg, fg) = if active { (p.warn_soft, p.warn) } else { (gpui::transparent_black(), p.ink_dim) };
            let hover_bg = if active { p.warn_soft } else { p.hover };
            let hover_fg = if active { p.warn } else { p.ink };
            let el = div()
                .id(act)
                .flex()
                .items_center()
                .justify_center()
                .size(px(32.))
                .rounded(RADIUS)
                .bg(bg)
                .text_color(fg)
                .child(icon(act, 18., fg))
                .tooltip(move |window, cx| Tooltip::new(hint.clone()).build(window, cx));
            let el = if enabled {
                el.hover(move |s| s.bg(hover_bg).text_color(hover_fg))
                    .active(move |s| s.bg(p.pressed))
                    .on_click(move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx))
            } else {
                el.opacity(0.3)
            };
            bar = bar.child(el);
        }

        bar.child(div().flex_1()).child(
            div()
                .w(px(200.))
                .child(Input::new(&self.search).prefix(icon("search", 15., p.ink_faint)).cleanable(true)),
        )
    }
}

/// Both the toolbar button and the status-bar pill say which way the click goes,
/// so the state is readable without decoding the colour.
pub fn alt_speed_hint(on: bool) -> String {
    let key = accel("L", true);
    if on {
        format!("Alternate speed limits are ON -- click for the normal limits  ({key})")
    } else {
        format!("Alternate Speed Limits  ({key})")
    }
}
