//! Every command the menus, toolbar, keyboard and context menus can trigger.
//! GPUI actions are named, so these names are also the scripting surface
//! `--shot` drives -- the successor of window.ztorrentUI.

use gpui::{Action, App, KeyBinding, actions};

actions!(
    ztorrent,
    [
        AddTorrent,
        AddUrl,
        CreateTorrent,
        Preferences,
        About,
        CheckForUpdates,
        Quit,
        CloseWindow,
        Find,
        Start,
        Pause,
        Stop,
        Recheck,
        Reannounce,
        QueueUp,
        QueueDown,
        Remove,
        RemoveData,
        CopyMagnet,
        RevealFolder,
        SaveTorrentAs,
        Properties,
        ToggleSequential,
        NewLabel,
        ToggleAltSpeed,
        ThemeLight,
        ThemeDark,
        ToggleTheme,
        SampleTorrents,
        Minimize,
        Zoom,
        ToggleFullScreen,
        SelectAll,
        ClearSelection,
        SelectUp,
        SelectDown,
        ExtendUp,
        ExtendDown,
        DeleteKey,
        DeleteDataKey,
        TogglePause,
        EnterKey,
        AddTrackerPrompt,
        AddPeerPrompt,
        /// Opens Customize Label for the first label; for scripted checks.
        CustomizeFirstLabel,
        ShowGeneral,
        ShowTrackers,
        ShowPeers,
        ShowPieces,
        ShowFiles,
        ShowSpeed,
        ShowLogger,
    ]
);

#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = ztorrent, no_json)]
pub struct SetLabel {
    pub label: String,
}

#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = ztorrent, no_json)]
pub struct CustomizeLabel {
    pub name: String,
}

#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = ztorrent, no_json)]
pub struct ToggleColumn {
    pub key: String,
}

#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = ztorrent, no_json)]
pub struct SetPriority {
    pub index: usize,
    pub priority: u8,
}

#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = ztorrent, no_json)]
pub struct OpenFile {
    pub index: usize,
}

#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = ztorrent, no_json)]
pub struct RevealFile {
    pub index: usize,
}

#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = ztorrent, no_json)]
pub struct CopyText {
    pub text: String,
}

/// Cmd on macOS, Ctrl elsewhere, the way CmdOrCtrl read in the Electron menus.
pub(crate) fn k(binding: &str) -> String {
    if cfg!(target_os = "macos") { binding.replace("mod-", "cmd-") } else { binding.replace("mod-", "ctrl-") }
}

pub fn bind_keys(cx: &mut App) {
    let list = Some("TorrentList");
    cx.bind_keys([
        KeyBinding::new(&k("mod-o"), AddTorrent, None),
        KeyBinding::new(&k("mod-u"), AddUrl, None),
        KeyBinding::new(&k("mod-n"), CreateTorrent, None),
        KeyBinding::new(&k("mod-,"), Preferences, None),
        KeyBinding::new(&k("mod-alt-,"), Preferences, None),
        KeyBinding::new(&k("mod-q"), Quit, None),
        KeyBinding::new(&k("mod-w"), CloseWindow, None),
        KeyBinding::new(&k("mod-f"), Find, None),
        KeyBinding::new(&k("mod-r"), Start, None),
        KeyBinding::new(&k("mod-p"), Pause, None),
        KeyBinding::new(&k("mod-."), Stop, None),
        KeyBinding::new(&k("mod-e"), Recheck, None),
        KeyBinding::new(&k("mod-t"), Reannounce, None),
        KeyBinding::new(&k("mod-up"), QueueUp, None),
        KeyBinding::new(&k("mod-down"), QueueDown, None),
        KeyBinding::new(&k("mod-backspace"), Remove, None),
        KeyBinding::new(&k("mod-shift-backspace"), RemoveData, None),
        KeyBinding::new(&k("mod-shift-c"), CopyMagnet, None),
        KeyBinding::new(&k("mod-shift-o"), RevealFolder, None),
        KeyBinding::new(&k("mod-shift-l"), ToggleAltSpeed, None),
        KeyBinding::new(&k("mod-l"), ToggleTheme, None),
        KeyBinding::new(if cfg!(target_os = "macos") { "cmd-ctrl-f" } else { "f11" }, ToggleFullScreen, None),
        KeyBinding::new("up", SelectUp, list),
        KeyBinding::new("down", SelectDown, list),
        KeyBinding::new("shift-up", ExtendUp, list),
        KeyBinding::new("shift-down", ExtendDown, list),
        KeyBinding::new(&k("mod-a"), SelectAll, list),
        KeyBinding::new("escape", ClearSelection, list),
        KeyBinding::new("delete", DeleteKey, list),
        KeyBinding::new("backspace", DeleteKey, list),
        KeyBinding::new("shift-delete", DeleteDataKey, list),
        KeyBinding::new("shift-backspace", DeleteDataKey, list),
        KeyBinding::new("space", TogglePause, list),
        KeyBinding::new("enter", EnterKey, list),
    ]);
}

/// Shortcut hints, written the way each platform writes them.
pub fn accel(key: &str, shift: bool) -> String {
    if cfg!(target_os = "macos") {
        format!("{}⌘{key}", if shift { "⇧" } else { "" })
    } else {
        format!("Ctrl+{}{key}", if shift { "Shift+" } else { "" })
    }
}
