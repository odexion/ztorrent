//! The application menu, rebuilt from the settings whenever a checked item can
//! change, so a checkbox never disagrees with the window.

use crate::actions::*;
use gpui::{App, Menu, MenuItem, OsAction, SystemMenuType};
use ztorrent_core::Settings;

pub fn set_menus(settings: &Settings, cx: &mut App) {
    let mac = cfg!(target_os = "macos");
    let dark = settings.is_dark();
    let mut menus = Vec::new();

    if mac {
        menus.push(Menu {
            name: "ztorrent".into(),
            items: vec![
                MenuItem::action("About ztorrent", About),
                MenuItem::action("Check for Updates…", CheckForUpdates),
                MenuItem::separator(),
                MenuItem::action("Preferences…", Preferences),
                MenuItem::separator(),
                MenuItem::os_submenu("Services", SystemMenuType::Services),
                MenuItem::separator(),
                MenuItem::action("Quit ztorrent", Quit),
            ],
            disabled: false,
        });
    }

    let mut file = vec![
        MenuItem::action("Add Torrent…", AddTorrent),
        MenuItem::action("Add Torrent from URL…", AddUrl),
        MenuItem::separator(),
        MenuItem::action("Create New Torrent…", CreateTorrent),
        MenuItem::separator(),
        MenuItem::action("Close Window", CloseWindow),
    ];
    if !mac {
        file.extend([
            MenuItem::separator(),
            MenuItem::action("Preferences…", Preferences),
            MenuItem::separator(),
            MenuItem::action("Exit", Quit),
        ]);
    }
    menus.push(Menu { name: "File".into(), items: file, disabled: false });

    use gpui_component::input::{Copy, Cut, Paste, Redo, SelectAll as InputSelectAll, Undo};
    menus.push(Menu {
        name: "Edit".into(),
        items: vec![
            MenuItem::os_action("Undo", Undo, OsAction::Undo),
            MenuItem::os_action("Redo", Redo, OsAction::Redo),
            MenuItem::separator(),
            MenuItem::os_action("Cut", Cut, OsAction::Cut),
            MenuItem::os_action("Copy", Copy, OsAction::Copy),
            MenuItem::os_action("Paste", Paste, OsAction::Paste),
            MenuItem::os_action("Select All", InputSelectAll, OsAction::SelectAll),
            MenuItem::separator(),
            MenuItem::action("Find", Find),
        ],
        disabled: false,
    });

    menus.push(Menu {
        name: "Torrent".into(),
        items: vec![
            MenuItem::action("Start", Start),
            MenuItem::action("Pause", Pause),
            MenuItem::action("Stop", Stop),
            MenuItem::separator(),
            MenuItem::action("Force Re-Check", Recheck),
            MenuItem::action("Update Tracker", Reannounce),
            MenuItem::separator(),
            MenuItem::action("Move Up Queue", QueueUp),
            MenuItem::action("Move Down Queue", QueueDown),
            MenuItem::separator(),
            MenuItem::action("Remove", Remove),
            MenuItem::action("Remove And Delete Data…", RemoveData),
            MenuItem::separator(),
            MenuItem::action("Copy Magnet URI", CopyMagnet),
            MenuItem::action("Open Containing Folder", RevealFolder),
        ],
        disabled: false,
    });

    menus.push(Menu {
        name: "Options".into(),
        items: vec![
            MenuItem::action("Alternate Speed Limits", ToggleAltSpeed).checked(settings.alt_speed_enabled),
            MenuItem::separator(),
            MenuItem::action("Preferences…", Preferences),
        ],
        disabled: false,
    });

    menus.push(Menu {
        name: "View".into(),
        items: vec![
            MenuItem::submenu(Menu {
                name: "Appearance".into(),
                items: vec![
                    MenuItem::action("Light", ThemeLight).checked(!dark),
                    MenuItem::action("Dark", ThemeDark).checked(dark),
                    MenuItem::separator(),
                    MenuItem::action("Toggle Light/Dark", ToggleTheme),
                ],
                disabled: false,
            }),
            MenuItem::separator(),
            MenuItem::action("Toggle Full Screen", ToggleFullScreen),
        ],
        disabled: false,
    });

    menus.push(Menu {
        name: "Window".into(),
        items: if mac {
            vec![MenuItem::action("Minimize", Minimize), MenuItem::action("Zoom", Zoom)]
        } else {
            vec![MenuItem::action("Minimize", Minimize), MenuItem::action("Close", CloseWindow)]
        },
        disabled: false,
    });

    let mut help = vec![MenuItem::action("ztorrent Help", About), MenuItem::action("Sample Torrents Folder", SampleTorrents)];
    if !mac {
        help.extend([MenuItem::separator(), MenuItem::action("Check for Updates…", CheckForUpdates)]);
    }
    menus.push(Menu { name: "Help".into(), items: help, disabled: false });

    cx.set_menus(menus);
}
