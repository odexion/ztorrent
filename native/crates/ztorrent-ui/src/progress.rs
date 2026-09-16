use crate::theme::Palette;
use gpui::prelude::*;
use gpui::*;
use ztorrent_core::BarKind;

pub fn fill(p: &Palette, kind: BarKind) -> Hsla {
    match kind {
        BarKind::Active => p.bar,
        BarKind::Seed => p.bar_seed,
        BarKind::Finished => p.bar_fin,
        BarKind::Idle => p.bar_idle,
        BarKind::Error => p.bar_err,
    }
}

/// The progress bar. The label is drawn twice -- once over the empty track, once
/// clipped to the filled part -- each in the colour that reads against what is
/// behind it, so contrast holds at every percentage. `width` is the bar's own
/// width, which the clipped copy needs to line up with the first.
pub fn progress_bar(p: &Palette, kind: BarKind, done: f64, label: Option<String>, width: Pixels, height: f32) -> Div {
    let done = if done.is_nan() { 0.0 } else { done.clamp(0.0, 1.0) } as f32;
    let filled = width * done;
    let mut bar = div()
        .relative()
        .flex_none()
        .w(width)
        .h(px(height))
        .overflow_hidden()
        .bg(p.bar_track)
        .child(div().absolute().top_0().left_0().h_full().w(filled).bg(fill(p, kind)));
    if let Some(text) = label {
        let centred = |color: Hsla| {
            div()
                .absolute()
                .top_0()
                .left_0()
                .h_full()
                .w(width)
                .flex()
                .items_center()
                .justify_center()
                .whitespace_nowrap()
                .text_color(color)
                .child(text.clone())
        };
        bar = bar
            .child(centred(p.bar_label))
            .child(div().absolute().top_0().left_0().h_full().w(filled).overflow_hidden().child(centred(p.bar_label_on)));
    }
    bar
}
