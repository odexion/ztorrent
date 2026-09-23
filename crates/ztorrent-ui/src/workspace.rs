//! The main window: its layout, its reactions to the engine, and every action.

use crate::actions::*;
use crate::bridge::{self, ask, send};
use crate::state::AppState;
use crate::theme::{FONT_SIZE, Theme};
use gpui::prelude::*;
use gpui::*;
use gpui_component::input::{InputEvent, InputState};
use gpui_component::WindowExt as _;
use gpui_component::native_menu::NativeMenu;
use std::path::PathBuf;
use ztorrent_core::command::{Bootstrap, Command, TorrentSource};
use ztorrent_core::settings::patch;
use ztorrent_core::store::WindowBounds;
use ztorrent_core::{Event, columns, tag_style};

#[derive(Clone)]
pub struct DraggedTorrents {
    pub ids: Vec<String>,
}

pub enum Resize {
    Column { key: &'static str, start_x: f32, start_w: f32 },
    Sidebar { start_x: f32, start_w: f32 },
    Detail { start_y: f32, start_h: f32 },
}

pub struct Workspace {
    pub state: AppState,
    pub version: String,
    pub list_focus: FocusHandle,
    pub list_scroll: UniformListScrollHandle,
    pub log_scroll: ScrollHandle,
    pub search: Entity<InputState>,
    pub sidebar_w: f32,
    pub detail_h: f32,
    pub resize: Option<Resize>,
    pub pending_select: Option<String>,
    pub dragging: Option<Vec<String>>,
    pub update: Option<ztorrent_core::update::UpdateStatus>,
    /// Log lines the startup snapshot already carried; the same lines also
    /// arrive as events queued before the window existed, and are shown once.
    boot_log: std::collections::HashSet<(i64, String)>,
    _subscriptions: Vec<Subscription>,
    _tasks: Vec<Task<()>>,
}

impl Workspace {
    pub fn new(boot: Bootstrap, version: String, events: flume::Receiver<Event>, updates: flume::Receiver<ztorrent_core::update::UpdateStatus>, opens: flume::Receiver<String>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let boot_log = boot.log.iter().map(|l| (l.time, l.message.clone())).collect();
        let (state, columns_changed) = AppState::new(boot);
        if columns_changed {
            send(cx, Command::SetColumns(state.columns_record()));
        }
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Filter torrents"));
        let subscriptions = vec![
            cx.subscribe_in(&search, window, |this, input, ev: &InputEvent, _, cx| {
                if matches!(ev, InputEvent::Change) {
                    this.state.search = input.read(cx).value().to_string();
                    cx.notify();
                }
            }),
            cx.observe_window_bounds(window, |_, window, cx| {
                let b = window.bounds();
                send(
                    cx,
                    Command::SetWindow(WindowBounds {
                        x: Some(f32::from(b.origin.x) as f64),
                        y: Some(f32::from(b.origin.y) as f64),
                        width: f32::from(b.size.width) as f64,
                        height: f32::from(b.size.height) as f64,
                    }),
                );
            }),
        ];
        let tasks = vec![
            cx.spawn(async move |this, cx| {
                while let Ok(event) = events.recv_async().await {
                    if this.update(cx, |this, cx| this.on_event(event, cx)).is_err() {
                        break;
                    }
                }
            }),
            cx.spawn(async move |this, cx| {
                while let Ok(status) = updates.recv_async().await {
                    if this.update(cx, |this, cx| {
                        this.update = Some(status);
                        cx.notify();
                    }).is_err() {
                        break;
                    }
                }
            }),
            cx.spawn_in(window, async move |this, cx| {
                while let Ok(source) = opens.recv_async().await {
                    if this.update_in(cx, |this, window, cx| this.open_source(source, window, cx)).is_err() {
                        break;
                    }
                }
            }),
        ];
        // gpui-component re-syncs its theme to the system appearance as the window
        // opens, and again whenever that appearance changes; ztorrent's own palette
        // goes back on top each time, since the preference -- not the OS -- decides.
        let settings = state.settings.clone();
        cx.defer_in(window, move |_, _, cx| crate::apply_theme(&settings, cx));
        let appearance = cx.observe_window_appearance(window, |this: &mut Workspace, _, cx| this.appearance_changed(cx));
        let mut subscriptions = subscriptions;
        subscriptions.push(appearance);
        let list_focus = cx.focus_handle();
        window.focus(&list_focus, cx);
        Workspace {
            state,
            version,
            list_focus,
            list_scroll: UniformListScrollHandle::new(),
            log_scroll: ScrollHandle::new(),
            search,
            sidebar_w: 216.,
            detail_h: 260.,
            resize: None,
            pending_select: None,
            dragging: None,
            update: None,
            boot_log,
            _subscriptions: subscriptions,
            _tasks: tasks,
        }
    }

    fn on_event(&mut self, event: Event, cx: &mut Context<Self>) {
        match event {
            Event::Tick { rows, globals, details } => {
                if self.state.apply_tick(rows, globals, details.map(|d| *d)) {
                    self.sync_selection(cx);
                }
                self.update_badge();
            }
            Event::Log(line) => {
                if self.boot_log.remove(&(line.time, line.message.clone())) {
                    return;
                }
                let at_bottom = {
                    let (offset, max) = (self.log_scroll.offset(), self.log_scroll.max_offset());
                    f32::from(max.y) + f32::from(offset.y) < 24.
                };
                self.state.push_log(line);
                if at_bottom {
                    self.log_scroll.scroll_to_bottom();
                }
            }
            Event::LibraryChanged { labels, label_styles } => {
                cx.global_mut::<crate::dialogs::Library>().labels = labels.clone();
                self.state.labels = labels;
                self.state.label_styles = label_styles;
            }
            Event::SettingsChanged(settings) => {
                let theme_changed = settings.theme != self.state.settings.theme;
                if settings.auto_update != self.state.settings.auto_update {
                    bridge::update(cx, ztorrent_core::update::UpdateCommand::SetAutomatic(settings.auto_update));
                }
                self.state.settings = settings.clone();
                cx.global_mut::<crate::dialogs::Library>().settings = settings.clone();
                if theme_changed {
                    crate::apply_theme(&settings, cx);
                }
                crate::menus::set_menus(&settings, cx);
            }
            Event::Complete { id, name, .. } => {
                if self.state.settings.notify_on_complete {
                    crate::platform::notify_complete(&id, &name, cx);
                }
            }
        }
        cx.notify();
    }

    /// The Dock badge shows how far the running downloads have got, together.
    fn update_badge(&self) {
        use ztorrent_core::State;
        if !cfg!(target_os = "macos") {
            return;
        }
        let active: Vec<_> = self.state.rows.iter().filter(|r| r.state == State::Downloading).collect();
        if !self.state.settings.show_speed_in_dock || active.is_empty() {
            return crate::platform::set_badge(None);
        }
        let total: f64 = active.iter().map(|r| r.size as f64).sum();
        let got: f64 = active.iter().map(|r| r.size as f64 * r.done).sum();
        let pct = if total > 0.0 { (got / total * 100.0).round() } else { 0.0 };
        crate::platform::set_badge(Some(&format!("{pct}%")));
    }

    /// The menu's check: says what it found, where the automatic one says nothing.
    pub fn check_for_updates(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        use ztorrent_core::update::{UpdateCommand, UpdateState};
        let (tx, rx) = futures_channel::oneshot::channel();
        bridge::update(cx, UpdateCommand::Check { manual: true, reply: Some(tx) });
        let version = self.version.clone();
        cx.spawn_in(window, async move |_, cx| {
            let Ok(status) = rx.await else { return };
            let _ = cx.update(|window, cx| {
                if status.state == UpdateState::Idle {
                    let _ = window.prompt(PromptLevel::Info, &format!("ztorrent {version} is up to date."), None, &["OK"], cx);
                } else if let Some(err) = status.error.filter(|_| status.state == UpdateState::Error) {
                    let _ = window.prompt(PromptLevel::Warning, "Could not check for updates.", Some(&err), &["OK"], cx);
                }
            });
        })
        .detach();
    }

    /// One pill carries the whole update flow; clicking it does the next thing.
    pub fn on_update_click(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        use ztorrent_core::update::{UpdateCommand, UpdateState, UpdateStep};
        let Some(u) = self.update.clone() else { return };
        match (u.state, u.error_from) {
            (UpdateState::Available, _) if !u.installable => {
                let tag = u.version.as_deref().map(|v| format!("tag/v{v}")).unwrap_or_else(|| "latest".into());
                cx.open_url(&format!("https://github.com/odexion/ztorrent/releases/{tag}"));
            }
            (UpdateState::Available, _) => bridge::update(cx, UpdateCommand::Download),
            (UpdateState::Error, Some(UpdateStep::Download)) => bridge::update(cx, UpdateCommand::Check { manual: true, reply: None }),
            (UpdateState::Ready, _) => {
                let version = u.version.unwrap_or_default();
                let message = format!("ztorrent {version} is ready to install. ztorrent will close, update itself and open again. Downloads resume where they left off.");
                let answer = window.prompt(PromptLevel::Info, "Restart to update", Some(&message), &["Restart", "Cancel"], cx);
                cx.spawn(async move |_, cx| {
                    if answer.await != Ok(0) {
                        return;
                    }
                    let (tx, rx) = futures_channel::oneshot::channel();
                    let _ = cx.update(|cx| bridge::update(cx, UpdateCommand::Apply(tx)));
                    // The hand-off script is already waiting for this process to exit.
                    if rx.await == Ok(true) {
                        let _ = cx.update(|cx| cx.quit());
                    }
                })
                .detach();
            }
            _ => {}
        }
    }

    pub fn show_tab(&mut self, tab: crate::state::Tab, cx: &mut Context<Self>) {
        self.state.tab = tab;
        cx.notify();
    }

    pub fn sync_selection(&mut self, cx: &mut Context<Self>) {
        send(cx, Command::Details(self.state.selection.first().cloned()));
        cx.notify();
    }

    // ------------------------------------------------------------ selection

    pub fn row_mouse_down(&mut self, id: &str, ev: &MouseDownEvent, cx: &mut Context<Self>) {
        self.pending_select = None;
        let m = ev.modifiers;
        let plain = !m.secondary() && !m.shift;
        // A plain press on one of several selected rows may start a drag of all
        // of them, so the collapse to one row waits for the release.
        if plain && self.state.selection.len() > 1 && self.state.is_selected(id) {
            self.pending_select = Some(id.to_string());
            return;
        }
        self.state.select(id, m.secondary(), m.shift);
        self.sync_selection(cx);
    }

    pub fn row_mouse_up(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.pending_select.take().as_deref() == Some(id) {
            self.state.select(id, false, false);
            self.sync_selection(cx);
        }
    }

    pub fn move_selection(&mut self, down: bool, extend: bool, cx: &mut Context<Self>) {
        if let Some(ix) = self.state.move_selection(down, extend) {
            self.list_scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
            self.sync_selection(cx);
        }
    }

    pub fn row_context_menu(&mut self, id: &str, position: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        if !self.state.is_selected(id) {
            self.state.select(id, false, false);
            self.sync_selection(cx);
        }
        let single = self.state.selection.len() == 1;
        let Some(row) = self.state.row(id).cloned() else { return };
        let mut labels = NativeMenu::new().menu("Remove Label", Box::new(SetLabel { label: String::new() })).separator();
        for l in &self.state.labels {
            labels = labels.menu_with_check(l.clone(), row.label == *l, Box::new(SetLabel { label: l.clone() }));
        }
        labels = labels.separator().menu("New Label…", Box::new(NewLabel));
        let item = |menu: NativeMenu, label: &str, action: Box<dyn Action>, enabled: bool| {
            if enabled { menu.menu(label.to_string(), action) } else { menu.menu_with_disabled(label.to_string(), true, action) }
        };
        let mut bandwidth = NativeMenu::new();
        bandwidth = item(bandwidth, "Move Up Queue", Box::new(QueueUp), single);
        bandwidth = item(bandwidth, "Move Down Queue", Box::new(QueueDown), single).separator();
        bandwidth = if single {
            bandwidth.menu_with_check("Download Sequentially", row.sequential, Box::new(ToggleSequential))
        } else {
            bandwidth.menu_with_disabled("Download Sequentially", true, Box::new(ToggleSequential))
        };
        let mut menu = NativeMenu::new().menu("Start", Box::new(Start)).menu("Pause", Box::new(Pause)).menu("Stop", Box::new(Stop)).separator().menu("Force Re-Check", Box::new(Recheck));
        menu = item(menu, "Update Tracker", Box::new(Reannounce), single).separator().menu("Remove", Box::new(Remove)).menu("Remove And Delete Data…", Box::new(RemoveData)).separator();
        menu = menu.submenu("Bandwidth Allocation", bandwidth).submenu("Labels", labels).separator();
        menu = item(menu, "Open Containing Folder", Box::new(RevealFolder), single);
        menu = item(menu, "Copy Magnet URI", Box::new(CopyMagnet), single);
        menu = item(menu, "Save .torrent As…", Box::new(SaveTorrentAs), single).separator();
        menu = item(menu, "Properties…", Box::new(Properties), single);
        menu.show(position, window, cx);
    }

    fn ids(&self) -> Vec<String> {
        self.state.selection.clone()
    }

    fn single(&self) -> Option<String> {
        (self.state.selection.len() == 1).then(|| self.state.selection[0].clone())
    }

    pub fn toggle_pause(&mut self, cx: &mut Context<Self>) {
        let Some(r) = self.state.first_selected() else { return };
        let ids = self.ids();
        let cmd = if matches!(r.state, ztorrent_core::State::Paused | ztorrent_core::State::Stopped) { Command::Start(ids) } else { Command::Pause(ids) };
        send(cx, cmd);
    }

    pub fn reveal_selected(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.single() {
            self.reveal(id, None, false, cx);
        }
    }

    /// Opens (`open`) or reveals a torrent's content, or one file of it. A path
    /// that is not there yet reveals the save folder instead.
    pub fn reveal(&mut self, id: String, file: Option<usize>, open: bool, cx: &mut Context<Self>) {
        let rx = ask(cx, |reply| Command::ResolvePath { id, file, reply });
        cx.spawn(async move |_, cx| {
            if let Ok(Some(resolved)) = rx.await {
                let _ = cx.update(|cx| {
                    if resolved.exists {
                        if open { cx.open_with_system(&resolved.target) } else { cx.reveal_path(&resolved.target) }
                    } else if !open {
                        cx.open_with_system(&resolved.save_path)
                    }
                });
            }
        })
        .detach();
    }

    pub fn remove(&mut self, delete_data: bool, window: &mut Window, cx: &mut Context<Self>) {
        let ids = self.ids();
        if ids.is_empty() {
            return;
        }
        if !self.state.settings.confirm_on_delete {
            return send(cx, Command::Remove { ids, delete_data });
        }
        let names: Vec<String> = ids.iter().filter_map(|id| self.state.row(id).map(|r| r.name.clone())).collect();
        let what = if ids.len() == 1 { format!("\"{}\"", names.first().cloned().unwrap_or_default()) } else { format!("{} torrents", ids.len()) };
        let (message, detail, button) = if delete_data {
            (format!("Remove {what} and delete the downloaded data?"), "The files will be moved to the trash of no return — this cannot be undone.", "Delete Data")
        } else {
            (format!("Remove {what} from the list?"), "The downloaded files will be left on disk.", "Remove")
        };
        let answer = window.prompt(PromptLevel::Warning, &message, Some(detail), &[button, "Cancel"], cx);
        cx.spawn(async move |_, cx| {
            if answer.await == Ok(0) {
                let _ = cx.update(|cx| send(cx, Command::Remove { ids, delete_data }));
            }
        })
        .detach();
    }

    pub fn new_label(&mut self, ids: Option<Vec<String>>, window: &mut Window, cx: &mut Context<Self>) {
        if sheet_up(window, cx) {
            return;
        }
        let ids = ids.unwrap_or_else(|| self.ids());
        crate::dialogs::open_new_label(window, cx, move |name, style, cx| {
            // The style goes first, so the label appears already wearing it;
            // the default look is left unstored.
            if (style.symbol.as_str(), style.color.as_str()) != tag_style(None) {
                send(cx, Command::SetLabelStyle { name: name.clone(), style: Some(style) });
            }
            // With nothing selected this still creates the label, empty.
            send(cx, Command::SetLabel { ids: ids.clone(), label: name });
        });    }

    /// A torrent handed to the app from outside: a double-clicked file, a magnet
    /// link, a second launch, or a drop.
    pub fn open_source(&mut self, source: String, window: &mut Window, cx: &mut Context<Self>) {
        crate::debug(format!("open_source({source:?})"));
        cx.activate(true);
        if source.is_empty() {
            return;
        }
        let source = source.strip_prefix("file://").map(|p| percent_decode(p)).unwrap_or(source);
        crate::dialogs::open_add(window, cx, TorrentSource::parse(&source), None);
    }

    fn add_files(&mut self, paths: Vec<PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        let paths: Vec<PathBuf> = paths.into_iter().filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("torrent"))).collect();
        match paths.len() {
            0 => {}
            1 => crate::dialogs::open_add(window, cx, TorrentSource::Path(paths[0].clone()), None),
            _ => send(cx, Command::AddPaths(paths)),
        }
    }

    /// Puts ztorrent's palette back over the one gpui-component re-syncs to the
    /// system appearance. The theme on screen, not the saved one: Preferences
    /// previews a theme by switching the window's appearance, which lands here,
    /// and putting the saved theme back would undo the preview at once.
    pub(crate) fn appearance_changed(&mut self, cx: &mut Context<Self>) {
        let mut shown = self.state.settings.clone();
        shown.theme = if cx.global::<Theme>().dark { "graphite" } else { "classic" }.into();
        crate::apply_theme(&shown, cx);
    }

    fn set_theme(&mut self, theme: &str, cx: &mut Context<Self>) {
        send(cx, Command::SetSettings(patch("theme", theme)));
    }

    // --------------------------------------------------------------- render

    fn on_mouse_move(&mut self, ev: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(resize) = &self.resize else { return };
        let (x, y) = (f32::from(ev.position.x), f32::from(ev.position.y));
        match *resize {
            Resize::Column { key, start_x, start_w } => self.state.set_width(key, start_w + x - start_x),
            Resize::Sidebar { start_x, start_w } => self.sidebar_w = (start_w + x - start_x).clamp(120., 400.),
            Resize::Detail { start_y, start_h } => {
                let max = f32::from(window.viewport_size().height) - 190.;
                self.detail_h = (start_h - (y - start_y)).clamp(90., max.max(90.));
            }
        }
        cx.notify();
    }

    fn end_resize(&mut self, cx: &mut Context<Self>) {
        if let Some(Resize::Column { .. }) = self.resize.take() {
            send(cx, Command::SetColumns(self.state.columns_record()));
        }
        self.pending_select = None;
    }
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() + 0 && i + 2 <= bytes.len() - 1 {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Theme::get(cx).clone();
        let alt = self.state.settings.alt_speed_enabled;
        let line = p.line;
        let accent = p.accent;
        let splitter_v = div()
            .id("vsplit")
            .w(px(5.))
            .mx(px(-2.))
            .h_full()
            .flex_none()
            .flex()
            .justify_center()
            .cursor_col_resize()
            .child(div().w(px(1.)).h_full().bg(line))
            .hover(move |s| s.bg(accent.opacity(0.35)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseDownEvent, _, _| {
                    this.resize = Some(Resize::Sidebar { start_x: f32::from(ev.position.x), start_w: this.sidebar_w });
                }),
            );
        let splitter_h = div()
            .id("hsplit")
            .h(px(5.))
            .my(px(-2.))
            .w_full()
            .flex_none()
            .flex()
            .flex_col()
            .justify_center()
            .cursor_row_resize()
            .child(div().h(px(1.)).w_full().bg(line))
            .hover(move |s| s.bg(accent.opacity(0.35)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseDownEvent, _, _| {
                    this.resize = Some(Resize::Detail { start_y: f32::from(ev.position.y), start_h: this.detail_h });
                }),
            );

        let toolbar = self.render_toolbar(window, cx).into_any_element();
        let sidebar = self.render_sidebar(window, cx).into_any_element();
        let grid = self.render_grid(window, cx).into_any_element();
        let detail = self.render_detail(window, cx).into_any_element();
        let status = self.render_status_bar(window, cx).into_any_element();

        div()
            .id("workspace")
            .key_context("Workspace")
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(p.bg)
            .text_color(p.ink)
            .text_size(FONT_SIZE)
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, cx| this.end_resize(cx)))
            .on_action(cx.listener(|_, _: &AddTorrent, window, cx| {
                if sheet_up(window, cx) {
                    return;
                }
                let dialog = rfd::AsyncFileDialog::new().set_title("Add Torrent").add_filter("Torrent Files", &["torrent"]);
                let workspace = cx.entity();
                crate::dialogs::pick(window, cx, crate::dialogs::Pick::Files, dialog, move |files, window, cx| {
                    workspace.update(cx, |this, cx| this.add_files(files, window, cx));
                });
            }))
            .on_action(cx.listener(|_, _: &AddUrl, window, cx| {
                if !sheet_up(window, cx) {
                    crate::dialogs::open_url(window, cx)
                }
            }))
            .on_action(cx.listener(|_, _: &CreateTorrent, window, cx| {
                if !sheet_up(window, cx) {
                    crate::dialogs::open_create(window, cx)
                }
            }))
            .on_action(cx.listener(|this, _: &Preferences, window, cx| {
                if !sheet_up(window, cx) {
                    crate::dialogs::open_preferences(this.state.settings.clone(), window, cx)
                }
            }))
            .on_action(cx.listener(|this, _: &About, window, cx| {
                let _ = window.prompt(
                    PromptLevel::Info,
                    &format!("ztorrent {}", this.version),
                    Some("A BitTorrent client for macOS, Windows and Linux.\n\nBuilt on libtorrent -- DHT, PEX, LSD, µTP,\nHTTP/UDP trackers and web seeds."),
                    &["OK"],
                    cx,
                );
            }))
            .on_action(cx.listener(|this, _: &Start, _, cx| send(cx, Command::Start(this.ids()))))
            .on_action(cx.listener(|this, _: &Pause, _, cx| send(cx, Command::Pause(this.ids()))))
            .on_action(cx.listener(|this, _: &Stop, _, cx| send(cx, Command::Stop(this.ids()))))
            .on_action(cx.listener(|this, _: &Recheck, _, cx| send(cx, Command::Recheck(this.ids()))))
            .on_action(cx.listener(|this, _: &Reannounce, _, cx| {
                if let Some(id) = this.single() {
                    send(cx, Command::Reannounce(id))
                }
            }))
            .on_action(cx.listener(|this, _: &QueueUp, _, cx| {
                if let Some(id) = this.single() {
                    send(cx, Command::MoveQueue { id, delta: -1 })
                }
            }))
            .on_action(cx.listener(|this, _: &QueueDown, _, cx| {
                if let Some(id) = this.single() {
                    send(cx, Command::MoveQueue { id, delta: 1 })
                }
            }))
            .on_action(cx.listener(|this, _: &Remove, window, cx| this.remove(false, window, cx)))
            .on_action(cx.listener(|this, _: &RemoveData, window, cx| this.remove(true, window, cx)))
            .on_action(cx.listener(|this, _: &CopyMagnet, _, cx| {
                if let Some(m) = this.state.first_selected().and_then(|r| r.magnet_uri.clone()) {
                    cx.write_to_clipboard(ClipboardItem::new_string(m));
                }
            }))
            .on_action(cx.listener(|this, _: &RevealFolder, _, cx| this.reveal_selected(cx)))
            .on_action(cx.listener(|this, _: &SaveTorrentAs, window, cx| {
                let Some(r) = this.single().and_then(|id| this.state.row(&id).cloned()) else { return };
                let dialog = rfd::AsyncFileDialog::new().set_file_name(format!("{}.torrent", r.name)).add_filter("Torrent Files", &["torrent"]);
                crate::dialogs::pick(window, cx, crate::dialogs::Pick::Save, dialog, move |paths, _, cx| {
                    let _ = ask(cx, |reply| Command::SaveTorrentFile { id: r.id.clone(), dest: paths[0].clone(), reply });
                });
            }))
            .on_action(cx.listener(|this, _: &Properties, window, cx| {
                if sheet_up(window, cx) {
                    return;
                }
                if let Some(row) = this.single().and_then(|id| this.state.row(&id).cloned()) {
                    let details = this.state.details.clone().filter(|d| d.id == row.id);
                    crate::dialogs::open_properties(row, details, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &ToggleSequential, _, cx| {
                if let Some(r) = this.single().and_then(|id| this.state.row(&id).cloned()) {
                    send(cx, Command::SetSequential { id: r.id, on: !r.sequential });
                }
            }))
            .on_action(cx.listener(|this, _: &NewLabel, window, cx| this.new_label(None, window, cx)))
            .on_action(cx.listener(|this, a: &SetLabel, _, cx| send(cx, Command::SetLabel { ids: this.ids(), label: a.label.clone() })))
            .on_action(cx.listener(|this, a: &CustomizeLabel, window, cx| {
                if sheet_up(window, cx) {
                    return;
                }
                let current = this.state.label_styles.get(&a.name).cloned();
                crate::dialogs::open_label_style(a.name.clone(), current, window, cx);
            }))
            .on_action(cx.listener(|this, _: &CustomizeFirstLabel, window, cx| {
                if sheet_up(window, cx) {
                    return;
                }
                if let Some(name) = this.state.labels.first().cloned() {
                    let current = this.state.label_styles.get(&name).cloned();
                    crate::dialogs::open_label_style(name, current, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &ShowGeneral, _, cx| this.show_tab(crate::state::Tab::General, cx)))
            .on_action(cx.listener(|this, _: &ShowTrackers, _, cx| this.show_tab(crate::state::Tab::Trackers, cx)))
            .on_action(cx.listener(|this, _: &ShowPeers, _, cx| this.show_tab(crate::state::Tab::Peers, cx)))
            .on_action(cx.listener(|this, _: &ShowPieces, _, cx| this.show_tab(crate::state::Tab::Pieces, cx)))
            .on_action(cx.listener(|this, _: &ShowFiles, _, cx| this.show_tab(crate::state::Tab::Files, cx)))
            .on_action(cx.listener(|this, _: &ShowSpeed, _, cx| this.show_tab(crate::state::Tab::Speed, cx)))
            .on_action(cx.listener(|this, _: &ShowLogger, _, cx| this.show_tab(crate::state::Tab::Logger, cx)))
            .on_action(cx.listener(|_, _: &ToggleAltSpeed, _, cx| send(cx, Command::ToggleAltSpeed)))
            .on_action(cx.listener(|this, _: &CheckForUpdates, window, cx| this.check_for_updates(window, cx)))
            .on_action(cx.listener(|this, _: &ThemeLight, _, cx| this.set_theme("classic", cx)))
            .on_action(cx.listener(|this, _: &ThemeDark, _, cx| this.set_theme("graphite", cx)))
            .on_action(cx.listener(|this, _: &ToggleTheme, _, cx| {
                let next = if this.state.settings.is_dark() { "classic" } else { "graphite" };
                this.set_theme(next, cx)
            }))
            .on_action(cx.listener(|this, _: &Find, window, cx| this.search.update(cx, |s, cx| s.focus(window, cx))))
            .on_action(cx.listener(|_, _: &SampleTorrents, _, cx| cx.open_with_system(&crate::samples_dir())))
            .on_action(cx.listener(|_, _: &Minimize, window, _| window.minimize_window()))
            .on_action(cx.listener(|_, _: &Zoom, window, _| window.zoom_window()))
            .on_action(cx.listener(|_, _: &ToggleFullScreen, window, _| window.toggle_fullscreen()))
            .on_action(cx.listener(|_, _: &CloseWindow, window, _| window.remove_window()))
            .on_action(cx.listener(|this, a: &SetPriority, _, cx| {
                if let Some(id) = this.state.selection.first().cloned() {
                    send(cx, Command::SetFilePriority { id, index: a.index, priority: a.priority });
                }
            }))
            .on_action(cx.listener(|this, a: &OpenFile, _, cx| {
                if let Some(id) = this.state.selection.first().cloned() {
                    this.reveal(id, Some(a.index), true, cx);
                }
            }))
            .on_action(cx.listener(|this, a: &RevealFile, _, cx| {
                if let Some(id) = this.state.selection.first().cloned() {
                    this.reveal(id, Some(a.index), false, cx);
                }
            }))
            .on_action(cx.listener(|_, a: &CopyText, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(a.text.clone()))))
            .on_action(cx.listener(|this, a: &ToggleColumn, _, cx| {
                columns::toggle_column(&mut this.state.columns, &a.key);
                send(cx, Command::SetColumns(this.state.columns_record()));
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &AddTrackerPrompt, window, cx| {
                let Some(id) = this.state.selection.first().cloned() else { return };
                crate::dialogs::prompt(window, cx, "Add Tracker", "Tracker announce URL:", "udp://tracker.opentrackr.org:1337/announce", move |url, cx| {
                    send(cx, Command::AddTracker { id: id.clone(), url: url.trim().to_string() })
                });
            }))
            .on_action(cx.listener(|this, _: &AddPeerPrompt, window, cx| {
                let Some(id) = this.state.selection.first().cloned() else { return };
                crate::dialogs::prompt(window, cx, "Add Peer", "Peer address (ip:port):", "", move |addr, cx| {
                    send(cx, Command::AddPeer { id: id.clone(), address: addr.trim().to_string() })
                });
            }))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| this.add_files(paths.paths().to_vec(), window, cx)))
            .child(toolbar)
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(div().w(px(self.sidebar_w)).flex_none().h_full().child(sidebar))
                    .child(splitter_v)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .child(div().flex_1().min_h_0().child(grid))
                            .child(splitter_h)
                            .child(div().h(px(self.detail_h)).flex_none().child(detail)),
                    ),
            )
            .child(status)
            .child(crate::dialogs::render_layer(window, cx))
            // The throttled-window wash: above the dialogs, because the throttle
            // does not lift while one is open, and inert, so every click lands.
            .when(alt, |d| {
                d.child(deferred(div().absolute().inset_0().bg(p.alt_wash).border_1().border_color(p.alt_edge)).with_priority(10))
            })
            .drag_over::<ExternalPaths>(move |s, _, _, _| s.border_2().border_color(accent))
            .child(div().hidden().child(bridge::marker()))
    }
}

/// Whether a sheet is already up. A command that opens a sheet leaves it be,
/// the way a Mac app's Cmd+, brings back its one Settings window, instead of
/// stacking another copy on top: the menu bar and the window's shortcuts still
/// reach the window under a sheet. A torrent handed over from outside is the
/// exception -- its Add sheet stacks rather than being dropped.
fn sheet_up(window: &mut Window, cx: &mut App) -> bool {
    window.has_active_dialog(cx)
}
