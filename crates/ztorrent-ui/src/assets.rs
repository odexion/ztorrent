//! The line icons, compiled into the binary. Every glyph is drawn with
//! currentColor, so GPUI tints it with the text colour at the call site --
//! colour stays a decision of the view, as it was of the stylesheet.

use gpui::{AssetSource, Result, SharedString};
use std::borrow::Cow;

macro_rules! icons {
    ($($name:literal),* $(,)?) => {
        const ICONS: &[(&str, &[u8])] = &[
            $(($name, include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/icons/", $name, ".svg")))),*
        ];
    };
}

icons!(
    "active", "add-file", "add-url", "alt-speed", "book", "bookmark", "box", "completed", "create", "disc",
    "down", "download", "downloading", "error", "feeds", "files", "film", "flag", "folder", "gamepad", "heart",
    "image", "inactive", "info", "label", "logger", "logo", "magnet", "monitor", "music", "pause", "peers",
    "pieces", "pin", "preferences", "queue-down", "queue-up", "remove", "restart", "search", "seeding", "speed",
    "star", "start", "stop", "terminal", "torrents", "tracker", "up",
);

/// Our icons under `icons/<name>.svg`; anything else is gpui-component's own.
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(name) = path.strip_prefix("ztorrent/").and_then(|p| p.strip_suffix(".svg")) {
            return Ok(ICONS.iter().find(|(n, _)| *n == name).map(|(_, bytes)| Cow::Borrowed(*bytes)));
        }
        gpui_kit_assets::AllAssets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        if path.starts_with("ztorrent") {
            return Ok(ICONS.iter().map(|(n, _)| SharedString::from(format!("ztorrent/{n}.svg"))).collect());
        }
        gpui_kit_assets::AllAssets.list(path)
    }
}

/// The asset path of one of our icons.
pub fn icon_path(name: &str) -> SharedString {
    SharedString::from(format!("ztorrent/{name}.svg"))
}
