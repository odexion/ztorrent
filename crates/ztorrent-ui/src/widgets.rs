//! The small pieces every list and sheet is built from, so the search
//! results, the torrent list and the sheets look and behave alike: native
//! pop-up menus, filter pills, list rows with a tick box, the seed-health dot,
//! tags, and the bar that acts on what is ticked.

use crate::theme::{CHIP_H, CONTROL_H, Palette, RADIUS, TEXT_META};
use gpui::prelude::*;
use gpui::*;
use gpui_component::native_menu::NativeMenu;

/// A choice from one of the native pop-up menus, routed back to the view
/// that opened it.
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = ztorrent, no_json)]
pub struct PickOption {
    pub field: String,
    pub value: String,
}

/// Opens a native menu of `options` (value, label) from a click on `el`; the
/// choice arrives as a [`PickOption`] for `field`.
///
/// Actions travel from the focused element up, and while a sheet is open the
/// focus is usually on the dialog around it, above the view's handler. So the
/// view's own focus handle is focused first, putting the handler on that path.
pub fn with_menu(el: Stateful<Div>, field: &'static str, options: Vec<(String, String)>, selected: String, focus: &FocusHandle) -> Stateful<Div> {
    let focus = focus.clone();
    el.cursor_pointer().on_mouse_down(MouseButton::Left, move |ev, window, cx| {
        window.focus(&focus, cx);
        let mut menu = NativeMenu::new();
        for (value, label) in &options {
            menu = menu.menu_with_check(label.clone(), *value == selected, Box::new(PickOption { field: field.into(), value: value.clone() }));
        }
        menu.show(ev.position, window, cx);
        cx.stop_propagation();
    })
}

/// A dropdown drawn as a field that opens a native menu, as a <select> does
/// on macOS.
pub fn dropdown(p: &Palette, id: &str, current: String, field: &'static str, options: Vec<(String, String)>, selected: String, focus: &FocusHandle) -> Stateful<Div> {
    let hover = p.hover;
    let selector = format!("dd-{id}");
    let el = div()
        .id(SharedString::from(selector.clone()))
        .debug_selector(move || selector)
        .flex()
        .items_center()
        .justify_between()
        .gap(px(8.))
        .h(CONTROL_H)
        .px(px(9.))
        .rounded(RADIUS)
        .border_1()
        .border_color(p.line_hard)
        .bg(p.bg)
        .hover(move |s| s.bg(hover))
        .child(div().min_w_0().truncate().child(current))
        .child(div().text_size(px(9.)).text_color(p.ink_faint).child("▼"));
    with_menu(el, field, options, selected, focus)
}

/// A filter as a pill: set, it names its value and carries an × that calls
/// `clear`; unset, a dashed "+ Season". A click opens the menu of `options`,
/// headed by "Any season", whose choice arrives as a [`PickOption`].
#[allow(clippy::too_many_arguments)]
pub fn filter_pill<V: 'static>(
    p: &Palette,
    field: &'static str,
    name: &str,
    value: Option<String>,
    options: Vec<(String, String)>,
    focus: &FocusHandle,
    cx: &mut Context<V>,
    clear: impl Fn(&mut V, &mut Context<V>) + 'static,
) -> Stateful<Div> {
    let selected = options.iter().find(|(_, l)| Some(l) == value.as_ref()).map(|(v, _)| v.clone()).unwrap_or_default();
    let mut options = options;
    options.insert(0, (String::new(), format!("Any {}", name.to_lowercase())));
    let hover = p.hover;
    let pill = div()
        .id(SharedString::from(format!("pill-{field}")))
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.))
        .h(CHIP_H)
        .px(px(10.))
        .rounded_full()
        .text_size(TEXT_META);
    let pill = match value {
        Some(v) => pill.bg(p.accent_soft).text_color(p.accent).child(v).child(
            div()
                .id(SharedString::from(format!("clear-{field}")))
                .flex()
                .items_center()
                .justify_center()
                .size(px(14.))
                .rounded_full()
                .hover(move |s| s.bg(hover))
                .child("×")
                .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| {
                    clear(this, cx);
                    cx.stop_propagation();
                    cx.notify();
                })),
        ),
        None => pill.border_1().border_dashed().border_color(p.line_hard).text_color(p.ink_dim).hover(move |s| s.bg(hover)).child(format!("+ {name}")),
    };
    with_menu(pill, field, options, selected, focus)
}

/// The frame of a list row: room for a tick box, a body and buttons, tinted
/// when ticked and on hover otherwise.
pub fn list_row(p: &Palette, id: impl Into<ElementId>, ticked: bool) -> Stateful<Div> {
    let hover = p.hover;
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(12.))
        .px(px(8.))
        .py(px(8.))
        .rounded(RADIUS)
        .cursor_pointer()
        .when(ticked, |d| d.bg(p.accent_soft))
        .when(!ticked, |d| d.hover(move |s| s.bg(hover)))
}

/// A row's second line: small, quiet text with its parts spaced apart.
pub fn meta_line(p: &Palette) -> Div {
    div().flex().items_center().gap(px(10.)).text_size(TEXT_META).text_color(p.ink_dim)
}

/// A coloured dot before a status: how well seeded, or what state a torrent
/// is in.
pub fn dot(color: Hsla) -> Div {
    div().flex_none().size(px(7.)).rounded_full().bg(color)
}

/// Green when a swarm is healthy, amber when thin, grey when empty or unknown.
pub fn seed_health(p: &Palette, seeds: Option<u32>) -> Hsla {
    match seeds {
        Some(n) if n >= 5 => p.ok,
        Some(n) if n >= 1 => p.warn,
        _ => p.ink_faint,
    }
}

/// A small grey tag, such as `S06E11`, `1080p` or a label name.
pub fn tag(p: &Palette, text: impl Into<SharedString>) -> Div {
    div().flex_none().px(px(6.)).rounded(px(4.)).bg(p.sunken).text_color(p.ink_dim).child(text.into())
}

/// The bar that appears once rows are ticked: what is ticked on the left,
/// the caller's buttons on the right.
pub fn selection_bar(p: &Palette, summary: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .py(px(8.))
        .px(px(12.))
        .rounded(RADIUS)
        .bg(p.sunken)
        .border_1()
        .border_color(p.line)
        .child(div().flex_1().min_w_0().truncate().child(summary.into()))
}

/// "1 seed", "3 seeds".
pub fn count(n: impl Into<u64>, one: &str, many: &str) -> String {
    let n = n.into();
    format!("{n} {}", if n == 1 { one } else { many })
}

/// A path with the home folder written as `~`.
pub fn tilde(path: &str) -> String {
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && path.starts_with(&home) => format!("~{}", &path[home.len()..]),
        _ => path.to_string(),
    }
}

#[cfg(test)]
mod tests {
    // Only what is tested: gpui's prelude brings its own `test` attribute.
    use super::count;

    #[test]
    fn counts_read_naturally() {
        assert_eq!(count(1u32, "seed", "seeds"), "1 seed");
        assert_eq!(count(0u32, "seed", "seeds"), "0 seeds");
        assert_eq!(count(12u32, "peer", "peers"), "12 peers");
    }
}
