//! The palette, token for token with renderer/styles.css: `:root` is Classic
//! (light), `[data-theme="graphite"]` is Graphite (dark). Names are the CSS
//! custom properties in snake_case, so a comment or lesson written about the
//! stylesheet still points at the right colour here.
//!
//! Aliases stay aliases: the sidebar icon colours are the state colours by
//! another name, computed from them rather than repeated.

use gpui::{App, FontFeatures, Global, Hsla, Pixels, Rgba, px};
use std::sync::Arc;

fn c(r: u8, g: u8, b: u8, a: f32) -> Hsla {
    Rgba { r: r as f32 / 255., g: g as f32 / 255., b: b as f32 / 255., a }.into()
}

fn hex(v: u32) -> Hsla {
    c((v >> 16) as u8, (v >> 8) as u8, v as u8, 1.0)
}

#[derive(Clone, Debug)]
pub struct Palette {
    pub bg: Hsla,
    pub chrome: Hsla,
    pub sidebar: Hsla,
    pub sunken: Hsla,
    pub raised: Hsla,

    pub line: Hsla,
    pub line_soft: Hsla,
    pub line_hard: Hsla,
    pub divider: Hsla,

    pub ink: Hsla,
    pub ink_dim: Hsla,
    pub ink_faint: Hsla,

    pub accent: Hsla,
    pub accent_hover: Hsla,
    pub accent_soft: Hsla,
    pub hover: Hsla,
    pub pressed: Hsla,
    pub sel: Hsla,
    pub sel_blur: Hsla,
    pub sel_quiet: Hsla,

    pub bar_track: Hsla,
    pub bar: Hsla,
    pub bar_seed: Hsla,
    pub bar_fin: Hsla,
    pub bar_idle: Hsla,
    pub bar_err: Hsla,
    pub bar_label: Hsla,
    pub bar_label_on: Hsla,
    pub piece_partial: Hsla,

    pub ok: Hsla,
    pub fin: Hsla,
    pub warn: Hsla,
    pub warn_soft: Hsla,
    pub alt_wash: Hsla,
    pub alt_edge: Hsla,
    pub err: Hsla,
    pub down: Hsla,
    pub up: Hsla,

    pub tag_slate: Hsla,
    pub tag_blue: Hsla,
    pub tag_teal: Hsla,
    pub tag_green: Hsla,
    pub tag_amber: Hsla,
    pub tag_red: Hsla,
    pub tag_violet: Hsla,
    pub tag_pink: Hsla,

    pub chart_down_fill: Hsla,
    pub chart_up_fill: Hsla,

    pub overlay: Hsla,
    pub shadow: Hsla,
    pub scrollbar: Hsla,
}

impl Palette {
    pub fn classic() -> Palette {
        Palette {
            bg: hex(0xffffff),
            chrome: hex(0xffffff),
            sidebar: hex(0xf7f7f8),
            sunken: hex(0xf4f4f5),
            raised: hex(0xffffff),
            line: c(0, 0, 0, 0.08),
            line_soft: c(0, 0, 0, 0.05),
            line_hard: c(0, 0, 0, 0.13),
            divider: c(0, 0, 0, 0.22),
            ink: hex(0x17181c),
            ink_dim: hex(0x6b7076),
            ink_faint: hex(0x9ba0a7),
            accent: hex(0x2f7bf6),
            accent_hover: hex(0x1f6be8),
            accent_soft: c(47, 123, 246, 0.12),
            hover: c(0, 0, 0, 0.045),
            pressed: c(0, 0, 0, 0.085),
            sel: hex(0xdce8f8),
            sel_blur: c(0, 0, 0, 0.05),
            sel_quiet: c(0, 0, 0, 0.10),
            bar_track: c(0, 0, 0, 0.07),
            bar: hex(0x0091ff),
            bar_seed: hex(0x46a758),
            bar_fin: hex(0x6e56cf),
            bar_idle: hex(0x889096),
            bar_err: hex(0xe5484d),
            bar_label: hex(0x5f666e),
            bar_label_on: hex(0xffffff),
            piece_partial: hex(0xe8a317),
            ok: hex(0x1a8a5a),
            fin: hex(0x6e56cf),
            warn: hex(0xb07500),
            warn_soft: c(176, 117, 0, 0.13),
            alt_wash: c(214, 69, 69, 0.055),
            alt_edge: c(214, 69, 69, 0.34),
            err: hex(0xd64545),
            down: hex(0x2f7bf6),
            up: hex(0x22a06b),
            tag_slate: hex(0x6b7076),
            tag_blue: hex(0x2f7bf6),
            tag_teal: hex(0x0d87a8),
            tag_green: hex(0x22a06b),
            tag_amber: hex(0xb07500),
            tag_red: hex(0xd64545),
            tag_violet: hex(0x6e56cf),
            tag_pink: hex(0xc2298a),
            chart_down_fill: c(47, 123, 246, 0.16),
            chart_up_fill: c(34, 160, 107, 0.16),
            overlay: c(0, 0, 0, 0.32),
            shadow: c(0, 0, 0, 0.22),
            scrollbar: c(0, 0, 0, 0.18),
        }
    }

    pub fn graphite() -> Palette {
        let ink = hex(0xe6e8ec);
        let ink_dim = hex(0x9aa0a8);
        Palette {
            bg: hex(0x1b1d21),
            chrome: hex(0x1b1d21),
            sidebar: hex(0x1f2126),
            sunken: hex(0x16181b),
            raised: hex(0x24262b),
            line: c(255, 255, 255, 0.09),
            line_soft: c(255, 255, 255, 0.06),
            line_hard: c(255, 255, 255, 0.14),
            divider: c(255, 255, 255, 0.24),
            ink,
            ink_dim,
            ink_faint: hex(0x6c727a),
            accent: hex(0x4b90ff),
            accent_hover: hex(0x629dff),
            accent_soft: c(75, 144, 255, 0.16),
            hover: c(255, 255, 255, 0.055),
            pressed: c(255, 255, 255, 0.10),
            sel: c(75, 144, 255, 0.20),
            sel_blur: c(255, 255, 255, 0.07),
            sel_quiet: c(255, 255, 255, 0.11),
            bar_track: c(255, 255, 255, 0.10),
            bar: hex(0x0091ff),
            bar_seed: hex(0x46a758),
            bar_fin: hex(0x6e56cf),
            bar_idle: hex(0x889096),
            bar_err: hex(0xe5484d),
            bar_label: ink_dim,
            bar_label_on: hex(0xffffff),
            piece_partial: hex(0xe0aa3c),
            ok: hex(0x3fc38a),
            fin: hex(0x9d8cdf),
            warn: hex(0xe0aa3c),
            warn_soft: c(224, 170, 60, 0.17),
            alt_wash: c(240, 104, 95, 0.075),
            alt_edge: c(240, 104, 95, 0.30),
            err: hex(0xf0685f),
            down: hex(0x4b90ff),
            up: hex(0x3fc38a),
            tag_slate: hex(0x9aa0a8),
            tag_blue: hex(0x4b90ff),
            tag_teal: hex(0x3ab6cf),
            tag_green: hex(0x3fc38a),
            tag_amber: hex(0xe0aa3c),
            tag_red: hex(0xf0685f),
            tag_violet: hex(0x9d8cdf),
            tag_pink: hex(0xee7bbb),
            chart_down_fill: c(75, 144, 255, 0.20),
            chart_up_fill: c(47, 189, 130, 0.20),
            overlay: c(0, 0, 0, 0.32),
            shadow: c(0, 0, 0, 0.55),
            scrollbar: c(255, 255, 255, 0.18),
        }
    }

    // Sidebar category icons: each state wears the colour it wears everywhere
    // else, so the sidebar reads as a key to the list.
    pub fn ic_downloading(&self) -> Hsla {
        self.down
    }
    pub fn ic_seeding(&self) -> Hsla {
        self.up
    }
    pub fn ic_completed(&self) -> Hsla {
        self.fin
    }
    pub fn ic_active(&self) -> Hsla {
        self.warn
    }
    pub fn ic_inactive(&self) -> Hsla {
        self.bar_idle
    }

    pub fn tag(&self, name: &str) -> Hsla {
        match name {
            "blue" => self.tag_blue,
            "teal" => self.tag_teal,
            "green" => self.tag_green,
            "amber" => self.tag_amber,
            "red" => self.tag_red,
            "violet" => self.tag_violet,
            "pink" => self.tag_pink,
            _ => self.tag_slate,
        }
    }
}

/// The active palette, as a GPUI global every view reads.
#[derive(Clone)]
pub struct Theme {
    pub dark: bool,
    pub p: Palette,
}

impl Global for Theme {}

impl Theme {
    pub fn for_setting(theme: &str) -> Theme {
        let dark = theme == "graphite";
        Theme { dark, p: if dark { Palette::graphite() } else { Palette::classic() } }
    }

    pub fn get(cx: &App) -> &Palette {
        &cx.global::<Theme>().p
    }
}

// ------------------------------------------------------------------ metrics

pub const FONT_SIZE: Pixels = px(13.);
pub const ROW_H: Pixels = px(26.);
pub const RADIUS: Pixels = px(6.);
pub const RADIUS_LG: Pixels = px(10.);

#[cfg(target_os = "macos")]
pub const MONO: &str = "Menlo";
#[cfg(target_os = "windows")]
pub const MONO: &str = "Consolas";
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub const MONO: &str = "DejaVu Sans Mono";

/// Digits that line up in columns.
pub fn tabular() -> FontFeatures {
    FontFeatures(Arc::new(vec![("tnum".into(), 1)]))
}
