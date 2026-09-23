use crate::assets::{bold_icon_path, icon_path};
use gpui::{Hsla, Styled, Svg, px, svg};

/// One of the line icons, tinted by `color` the way currentColor tinted it.
pub fn icon(name: &str, size: f32, color: Hsla) -> Svg {
    svg().path(icon_path(name)).size(px(size)).flex_none().text_color(color)
}

/// A line icon with a heavier, softer stroke.
pub fn bold_icon(name: &str, size: f32, color: Hsla) -> Svg {
    svg().path(bold_icon_path(name)).size(px(size)).flex_none().text_color(color)
}
