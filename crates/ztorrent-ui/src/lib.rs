//! The ztorrent window, on GPUI.

mod actions;
mod assets;
mod bridge;
#[cfg(test)]
mod behavior;
mod detail;
pub mod dialogs;
mod grid;
mod icons;
mod menus;
mod platform;
mod progress;
mod sidebar;
mod state;
mod status_bar;
mod theme;
mod toolbar;
mod workspace;

use gpui::*;
use gpui_component::Root;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use ztorrent_core::command::{Bootstrap, Command};
use ztorrent_core::{Event, Settings};

/// Traces the window's own flow to stderr when ZTORRENT_DEBUG_UI is set.
pub fn debug(message: String) {
    if std::env::var_os("ZTORRENT_DEBUG_UI").is_some() {
        eprintln!("[ui] {message}");
    }
}

pub struct RunOptions {
    pub commands: flume::Sender<Command>,
    pub events: flume::Receiver<Event>,
    pub updates: flume::Sender<ztorrent_core::update::UpdateCommand>,
    pub update_events: flume::Receiver<ztorrent_core::update::UpdateStatus>,
    /// Torrents handed to the app from outside: argv, a second launch, Finder.
    pub opens: (flume::Sender<String>, flume::Receiver<String>),
    pub version: String,
    pub args: Vec<String>,
}

/// Where the four sample torrents are: beside the executable in a packaged
/// build, in the repository in a development one.
pub fn samples_dir() -> PathBuf {
    let dev = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../sample-torrents"));
    if cfg!(debug_assertions) && dev.exists() {
        return dev;
    }
    let exe = std::env::current_exe().unwrap_or_default();
    let dir = exe.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    for candidate in [dir.join("../Resources/sample-torrents"), dir.join("sample-torrents"), dir.join("../share/ztorrent/sample-torrents")] {
        if candidate.exists() {
            return candidate;
        }
    }
    dev
}

/// Keeps gpui-component's widgets in the same palette as ztorrent's own views.
///
/// Its theme is not patched after the fact -- `Theme::change` derives several
/// layers from the active config, and a colour set afterwards reaches only some
/// of them. Instead Classic and Graphite are installed as its light and dark
/// configs, and the change is made through them.
pub fn apply_theme(settings: &Settings, cx: &mut App) {
    use gpui_component::theme::{Theme as ComponentTheme, ThemeConfig, ThemeMode};
    use std::rc::Rc;

    fn hex(c: Hsla) -> SharedString {
        let r: Rgba = c.into();
        let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        format!("#{:02x}{:02x}{:02x}{:02x}", b(r.r), b(r.g), b(r.b), b(r.a)).into()
    }
    fn config(p: &theme::Palette, dark: bool) -> Rc<ThemeConfig> {
        let mut c = ThemeConfig::default();
        c.name = if dark { "ztorrent Graphite" } else { "ztorrent Classic" }.into();
        c.mode = if dark { ThemeMode::Dark } else { ThemeMode::Light };
        c.font_size = Some(13.0);
        c.radius = Some(6);
        c.radius_lg = Some(10);
        let k = &mut c.colors;
        let white = hex(gpui::white());
        k.background = Some(hex(p.bg));
        k.foreground = Some(hex(p.ink));
        k.border = Some(hex(p.line_hard));
        k.input = Some(hex(p.line_hard));
        k.caret = Some(hex(p.accent));
        k.link = Some(hex(p.accent));
        k.muted_foreground = Some(hex(p.ink_faint));
        k.popover = Some(hex(p.raised));
        k.popover_foreground = Some(hex(p.ink));
        k.accent = Some(hex(p.hover));
        k.accent_foreground = Some(hex(p.ink));
        k.list_active = Some(hex(p.sel_quiet));
        k.primary = Some(hex(p.accent));
        k.primary_hover = Some(hex(p.accent_hover));
        k.primary_active = Some(hex(p.accent_hover));
        k.primary_foreground = Some(white.clone());
        k.button_primary = Some(hex(p.accent));
        k.button_primary_hover = Some(hex(p.accent_hover));
        k.button_primary_active = Some(hex(p.accent_hover));
        k.button_primary_foreground = Some(white);
        Rc::new(c)
    }

    let ours = theme::Theme::for_setting(&settings.theme);
    let dark = ours.dark;
    platform::set_appearance(dark);
    cx.set_global(ours);
    {
        let t = ComponentTheme::global_mut(cx);
        t.light_theme = config(&theme::Palette::classic(), false);
        t.dark_theme = config(&theme::Palette::graphite(), true);
    }
    ComponentTheme::change(if dark { ThemeMode::Dark } else { ThemeMode::Light }, None, cx);
    cx.refresh_windows();
}

fn wait_for<T>(mut rx: futures_channel::oneshot::Receiver<T>) -> Option<T> {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        match rx.try_recv() {
            Ok(Some(v)) => return Some(v),
            Ok(None) => std::thread::sleep(Duration::from_millis(5)),
            Err(_) => return None,
        }
    }
    None
}

fn bootstrap(commands: &flume::Sender<Command>) -> Bootstrap {
    let (tx, rx) = futures_channel::oneshot::channel();
    let _ = commands.send(Command::Bootstrap(tx));
    wait_for(rx).unwrap_or_default()
}

fn open_window(boot: Bootstrap, version: String, events: flume::Receiver<Event>, updates: flume::Receiver<ztorrent_core::update::UpdateStatus>, opens: flume::Receiver<String>, cx: &mut App) {
    let size_ = size(px(1180.), px(760.));
    let bounds = match &boot.window {
        Some(w) if w.width >= 820. && w.height >= 480. => {
            let s = size(px(w.width as f32), px(w.height as f32));
            match (w.x, w.y) {
                (Some(x), Some(y)) => Bounds::new(point(px(x as f32), px(y as f32)), s),
                _ => Bounds::centered(None, s, cx),
            }
        }
        _ => Bounds::centered(None, size_, cx),
    };
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions { title: Some("ztorrent".into()), ..Default::default() }),
        window_min_size: Some(size(px(820.), px(480.))),
        app_id: Some("dev.zaf4.ztorrent".into()),
        ..Default::default()
    };
    let result = cx.open_window(options, |window, cx| {
        let view = cx.new(|cx| workspace::Workspace::new(boot, version, events, updates, opens, window, cx));
        cx.new(|cx| Root::new(view, window, cx))
    });
    if let Err(err) = result {
        eprintln!("[ztorrent] could not open the window: {err:#}");
    }
}

pub fn run(options: RunOptions) {
    let RunOptions { commands, events, updates, update_events, opens, version, args } = options;
    let (open_tx, open_rx) = opens;
    for arg in args.iter().filter(|a| !a.starts_with('-')) {
        if arg.starts_with("magnet:") || arg.to_ascii_lowercase().ends_with(".torrent") {
            debug(format!("queued from argv: {arg}"));
            let _ = open_tx.send(arg.clone());
        }
    }

    let app = gpui_platform::application().with_assets(assets::Assets);
    let url_tx = open_tx.clone();
    app.on_open_urls(move |urls| {
        for url in urls {
            let _ = url_tx.send(url);
        }
    });

    let (reopen_commands, reopen_version, reopen_events, reopen_updates, reopen_opens) = (commands.clone(), version.clone(), events.clone(), update_events.clone(), open_rx.clone());
    app.on_reopen(move |cx| {
        if cx.windows().is_empty() {
            let boot = bootstrap(&reopen_commands);
            open_window(boot, reopen_version.clone(), reopen_events.clone(), reopen_updates.clone(), reopen_opens.clone(), cx);
        }
    });

    app.run(move |cx: &mut App| {
        // GPUI panics when no font resolves, and a minimal Linux install can
        // have none. Noto Sans is on its fallback list, so carrying it means
        // there is always one; a system that has fonts keeps using its own.
        #[cfg(target_os = "linux")]
        {
            let noto: &'static [u8] = include_bytes!("../assets/fonts/NotoSans-Regular.ttf");
            let _ = cx.text_system().add_fonts(vec![std::borrow::Cow::Borrowed(noto)]);
        }
        gpui_component::init(cx);
        cx.set_global(bridge::Bridge(commands.clone()));
        cx.set_global(bridge::Updates(updates.clone()));
        actions::bind_keys(cx);

        let boot = bootstrap(&commands);
        let _ = updates.send(ztorrent_core::update::UpdateCommand::SetAutomatic(boot.settings.auto_update));
        cx.set_global(dialogs::Library { settings: boot.settings.clone(), labels: boot.labels.clone() });
        apply_theme(&boot.settings, cx);
        menus::set_menus(&boot.settings, cx);

        cx.on_action(|_: &actions::Quit, cx| cx.quit());
        let shutdown = commands.clone();
        cx.on_app_quit(move |_| {
            let (tx, rx) = futures_channel::oneshot::channel();
            let _ = shutdown.send(Command::Shutdown(tx));
            async move {
                let _ = rx.await;
            }
        })
        .detach();
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() && !cfg!(target_os = "macos") {
                cx.quit();
            }
        })
        .detach();

        open_window(boot, version.clone(), events.clone(), update_events.clone(), open_rx.clone(), cx);
        cx.activate(true);

        // --action=ztorrent::Preferences dispatches a named action once the window
        // is up (after --action-delay=N seconds, so the first rows can arrive):
        // the hook scripted screenshots and UI checks drive the app with.
        let delay = args.iter().find_map(|a| a.strip_prefix("--action-delay=")).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
        let names: Vec<String> = args.iter().filter_map(|a| a.strip_prefix("--action=")).map(String::from).collect();
        if !names.is_empty() {
            cx.spawn(async move |cx| {
                if delay > 0 {
                    cx.background_executor().timer(Duration::from_secs(delay)).await;
                }
                let _ = cx.update(|cx| {
                    for name in &names {
                        match cx.build_action(name, None) {
                            Ok(action) => {
                                if let Some(window) = cx.windows().first().copied() {
                                    let _ = window.update(cx, |_, window, cx| window.dispatch_action(action, cx));
                                }
                            }
                            Err(err) => eprintln!("[ztorrent] unknown action {name}: {err}"),
                        }
                    }
                });
            })
            .detach();
        }
    });
}
