use crate::actions::CustomizeLabel;
use crate::icons::icon;
use crate::theme::{Palette, RADIUS, Theme};
use crate::workspace::{DraggedTorrents, Workspace};
use gpui::prelude::*;
use gpui::*;
use gpui_component::native_menu::NativeMenu;
use ztorrent_core::command::Command;
use ztorrent_core::{Category, tag_style};

impl Workspace {
    pub fn render_sidebar(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Theme::get(cx).clone();
        let categories = [
            (Category::Downloading, "Downloading", "downloading", p.ic_downloading()),
            (Category::Seeding, "Seeding", "seeding", p.ic_seeding()),
            (Category::Completed, "Completed", "completed", p.ic_completed()),
            (Category::Active, "Active", "active", p.ic_active()),
            (Category::Inactive, "Inactive", "inactive", p.ic_inactive()),
        ];

        let mut torrents = div().flex().flex_col().gap(px(1.)).child(self.tree_item(&p, Category::All, "Torrents", "torrents", p.ink_dim, true, false, cx));
        for (cat, label, ic, color) in categories {
            torrents = torrents.child(self.tree_item(&p, cat, label, ic, color, false, true, cx));
        }

        let mut labels = div()
            .flex()
            .flex_col()
            .gap(px(1.))
            .child(
                div()
                    .h(px(26.))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .text_size(px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(p.ink_faint)
                    .child("LABELS"),
            )
            .child(self.tree_item(&p, Category::NoLabel, "No Label", "label", p.ink_dim, false, true, cx));
        for name in self.state.labels.clone() {
            let (symbol, color) = tag_style(self.state.label_styles.get(&name));
            let color = p.tag(color);
            labels = labels.child(self.tree_item(&p, Category::Label(name.clone()), &name, symbol, color, false, true, cx));
        }
        // Not a category, just an affordance -- quieter than the real labels.
        labels = labels.child(
            div()
                .id("new-label")
                .flex()
                .items_center()
                .gap(px(8.))
                .h(px(30.))
                .pl(px(16.))
                .pr(px(10.))
                .rounded(RADIUS)
                .text_color(p.ink_faint)
                .hover(|s| s.bg(p.hover).text_color(p.ink))
                .child(icon("create", 14., p.ink_faint))
                .child("New Label…")
                .on_click(cx.listener(|this, _, window, cx| this.new_label(None, window, cx)))
                .drag_over::<DraggedTorrents>({
                    let p = p.clone();
                    move |s, _, _, _| drop_target(s, &p)
                })
                .on_drop(cx.listener(|this, dragged: &DraggedTorrents, window, cx| {
                    this.new_label(Some(dragged.ids.clone()), window, cx)
                })),
        );

        div()
            .id("sidebar")
            .flex()
            .flex_col()
            .gap(px(16.))
            .h_full()
            .overflow_y_scroll()
            .py(px(10.))
            .px(px(8.))
            .bg(p.sidebar)
            .text_size(px(13.))
            .child(torrents)
            .child(labels)
    }

    #[allow(clippy::too_many_arguments)]
    fn tree_item(
        &self,
        p: &Palette,
        cat: Category,
        label: &str,
        ic: &str,
        color: Hsla,
        root: bool,
        child: bool,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let selected = self.state.category == cat;
        let count = self.state.count(&cat);
        let id = SharedString::from(format!("cat-{cat:?}"));
        let is_label_target = matches!(cat, Category::NoLabel | Category::Label(_));
        let p2 = p.clone();
        let mut el = div()
            .id(id)
            .flex()
            .items_center()
            .gap(px(8.))
            .h(px(30.))
            .pl(px(if child { 16. } else { 10. }))
            .pr(px(10.))
            .rounded(RADIUS)
            .text_color(p.ink)
            .when(selected || root, |d| d.font_weight(FontWeight::MEDIUM))
            .when(selected, |d| d.bg(p.sel_quiet))
            .when(!selected, |d| d.hover(|s| s.bg(p2.hover)))
            .child(icon(ic, 16., color))
            .child(div().flex_1().min_w_0().truncate().child(label.to_string()))
            .child(div().text_size(px(12.)).text_color(p.ink_faint).font_features(crate::theme::tabular()).child(format!("({count})")))
            .on_click(cx.listener({
                let cat = cat.clone();
                move |this, _, _, cx| {
                    this.state.category = cat.clone();
                    cx.notify();
                }
            }));
        if is_label_target {
            let p3 = p.clone();
            let target = match &cat {
                Category::Label(l) => l.clone(),
                _ => String::new(),
            };
            el = el
                .drag_over::<DraggedTorrents>(move |s, _, _, _| drop_target(s, &p3))
                .on_drop(cx.listener(move |this, dragged: &DraggedTorrents, _, cx| {
                    crate::bridge::send(cx, Command::SetLabel { ids: dragged.ids.clone(), label: target.clone() });
                    this.dragging = None;
                }));
        }
        if let Category::Label(name) = &cat {
            let name = name.clone();
            el = el.on_mouse_down(MouseButton::Right, move |ev, window, cx| {
                NativeMenu::new()
                    .menu(format!("Customize \"{name}\"…"), Box::new(CustomizeLabel { name: name.clone() }))
                    .show(ev.position, window, cx);
            });
        }
        el
    }
}

/// A label accepting a dropped torrent: an inset ring rather than a border, so
/// the row does not shift by a pixel as it lights up.
fn drop_target(s: StyleRefinement, p: &Palette) -> StyleRefinement {
    s.bg(p.accent_soft).border_2().border_color(p.accent)
}
