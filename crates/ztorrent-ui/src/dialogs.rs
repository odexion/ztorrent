//! The sheets: Add New Torrent, Add Torrent from URL, Create New Torrent,
//! Preferences, Properties, Customize Label, and the generic prompt. Each is a
//! view shown inside gpui-component's dialog, which supplies the overlay,
//! Escape and click-outside-to-cancel the Electron modal() implemented by hand.

use crate::actions::accel;
use crate::bridge::{ask, send};
use crate::icons::icon;
use crate::theme::{MONO, Palette, RADIUS, Theme, tabular};
use gpui::prelude::*;
use gpui::*;
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::checkbox::Checkbox;
use gpui_component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_component::native_menu::NativeMenu;
use gpui_component::{Disableable, Sizable, WindowExt};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;
use ztorrent_core::command::*;
use ztorrent_core::egress::suppressed_in_sheet;
use ztorrent_core::settings::{SettingsPatch, patch};
use ztorrent_core::*;

/// What the sheets need to know about the library, kept current by the workspace.
#[derive(Clone, Default)]
pub struct Library {
    pub settings: Settings,
    pub labels: Vec<String>,
}

impl Global for Library {}

/// A choice from one of the native dropdown menus, routed back to the sheet
/// that opened it.
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = ztorrent, no_json)]
pub struct PickOption {
    pub field: String,
    pub value: String,
}

// ---------------------------------------------------------------- plumbing

/// What Enter does in a sheet: the same as its primary button.
enum Confirm {
    /// Runs the sheet's own confirm, which closes the sheet when it is done.
    With(Rc<dyn Fn(&mut Window, &mut App)>),
    /// A sheet with nothing to confirm: Enter closes it.
    Close,
}

fn open<V: Render>(window: &mut Window, cx: &mut App, width: f32, view: Entity<V>, confirm: Confirm) {
    open_with(window, cx, width, view, confirm, true);
}

/// A sheet a click beside it does not close. For one the user works in for a
/// while and leaves to fetch something from another app: the click that brings
/// the window back lands beside the sheet, and must not throw away their work.
fn open_sticky<V: Render>(window: &mut Window, cx: &mut App, width: f32, view: Entity<V>, confirm: Confirm) {
    open_with(window, cx, width, view, confirm, false);
}

fn open_with<V: Render>(window: &mut Window, cx: &mut App, width: f32, view: Entity<V>, confirm: Confirm, backdrop_closes: bool) {
    crate::debug(format!("open dialog ({width}px)"));
    let confirm = Rc::new(confirm);
    window.open_dialog(cx, move |dialog, _, _| {
        let confirm = confirm.clone();
        dialog.w(px(width)).close_button(false).overlay_closable(backdrop_closes).child(view.clone()).on_ok(move |_, window, cx| match confirm.as_ref() {
            // Closed the way a sheet closes itself, which is the path that gives
            // focus back to the window -- closing from inside the key handler left
            // nothing focused, and every menu command after it went nowhere.
            Confirm::Close => {
                window.defer(cx, |window, cx| window.close_dialog(cx));
                false
            }
            Confirm::With(f) => {
                // Run after the dialog layer has finished handling the key, since
                // confirming usually closes the sheet.
                let f = f.clone();
                window.defer(cx, move |window, cx| f(window, cx));
                false
            }
        })
    });
}

fn title(p: &Palette, text: impl Into<SharedString>) -> Div {
    div().pb(px(14.)).text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).text_color(p.ink).child(text.into())
}

fn legend(p: &Palette, text: &str) -> Div {
    div().mb(px(10.)).text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).text_color(p.ink_faint).child(text.to_uppercase())
}

fn frow(p: &Palette, label: &str, content: impl IntoElement) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(12.))
        .mb(px(10.))
        .child(div().w(px(168.)).flex_none().text_color(p.ink_dim).child(label.to_string()))
        .child(div().flex_1().min_w_0().child(content))
}

fn hint(p: &Palette, text: &str) -> Div {
    div().ml(px(180.)).mt(px(-4.)).mb(px(10.)).text_size(px(12.)).text_color(p.ink_faint).child(text.to_string())
}

fn error_line(p: &Palette, error: &Option<String>) -> Option<Div> {
    error.as_ref().map(|e| div().mt(px(6.)).text_color(p.err).child(e.clone()))
}

fn footer(p: &Palette, extra: Option<AnyElement>) -> Div {
    div().flex().items_center().gap(px(8.)).mt(px(16.)).pt(px(14.)).border_t_1().border_color(p.line).children(extra).child(div().flex_1())
}

fn text_input(window: &mut Window, cx: &mut App, value: &str, placeholder: &str) -> Entity<InputState> {
    let value = value.to_string();
    let placeholder = placeholder.to_string();
    cx.new(|cx| {
        let mut s = InputState::new(window, cx).placeholder(placeholder);
        s.set_value(value, window, cx);
        s
    })
}

fn read(input: &Entity<InputState>, cx: &App) -> String {
    input.read(cx).value().to_string()
}

/// A dropdown drawn as a button that opens a native menu, as a <select> does on macOS.
/// A native pop-up menu whose choice arrives as a [`PickOption`] action.
///
/// Actions travel from the focused element up, and while a sheet is open the
/// focus is usually on the dialog around it, above the sheet's handler. So the
/// sheet's own focus handle is focused first, putting the handler on that path.
fn dropdown(p: &Palette, id: &str, current: String, field: &'static str, options: Vec<(String, String)>, selected: String, sheet: &FocusHandle) -> Stateful<Div> {
    let sheet = sheet.clone();
    let hover = p.hover;
    let selector = format!("dd-{id}");
    div()
        .id(SharedString::from(selector.clone()))
        .debug_selector(move || selector)
        .flex()
        .items_center()
        .justify_between()
        .gap(px(8.))
        .h(px(30.))
        .px(px(9.))
        .rounded(RADIUS)
        .border_1()
        .border_color(p.line_hard)
        .bg(p.bg)
        .hover(move |s| s.bg(hover))
        .child(div().min_w_0().truncate().child(current))
        .child(div().text_size(px(9.)).text_color(p.ink_faint).child("▼"))
        .on_mouse_down(MouseButton::Left, move |ev, window, cx| {
            window.focus(&sheet, cx);
            let mut menu = NativeMenu::new();
            for (value, label) in &options {
                menu = menu.menu_with_check(label.clone(), *value == selected, Box::new(PickOption { field: field.into(), value: value.clone() }));
            }
            menu.show(ev.position, window, cx);
            cx.stop_propagation();
        })
}

/// Where a folder sheet should open: a file is taken as the folder holding it,
/// and a path since deleted or unplugged walks up to one that exists.
fn starting_dir(p: &str) -> Option<PathBuf> {
    let mut path = PathBuf::from(p.trim());
    if p.trim().is_empty() {
        return None;
    }
    loop {
        if path.is_dir() {
            return Some(path);
        }
        if !path.pop() {
            return None;
        }
    }
}

/// What a file panel is asked to pick.
#[derive(Clone, Copy)]
pub enum Pick {
    File,
    Files,
    Folder,
    Save,
}

/// Opens a file panel as a sheet on the window and hands the choice back
/// later. It never blocks: a modal panel run from inside an event handler spins
/// its own event loop while GPUI is still in the middle of that handler, and
/// GPUI panics as soon as the loop delivers it anything. Nothing is called when
/// the panel is cancelled.
pub fn pick(
    window: &mut Window,
    cx: &mut App,
    kind: Pick,
    dialog: rfd::AsyncFileDialog,
    done: impl FnOnce(Vec<PathBuf>, &mut Window, &mut App) + 'static,
) {
    let dialog = dialog.set_parent(&*window);
    let handle = window.window_handle();
    cx.spawn(async move |cx| {
        let picked: Vec<PathBuf> = match kind {
            Pick::File => dialog.pick_file().await.map(|f| vec![f.path().to_path_buf()]).unwrap_or_default(),
            Pick::Files => dialog.pick_files().await.map(|fs| fs.iter().map(|f| f.path().to_path_buf()).collect()).unwrap_or_default(),
            Pick::Folder => dialog.pick_folder().await.map(|f| vec![f.path().to_path_buf()]).unwrap_or_default(),
            Pick::Save => dialog.save_file().await.map(|f| vec![f.path().to_path_buf()]).unwrap_or_default(),
        };
        if picked.is_empty() {
            return;
        }
        let _ = handle.update(cx, |_, window, cx| done(picked, window, cx));
    })
    .detach();
}

/// A panel that opens where the field already points.
pub fn panel(title: &str, start: &str) -> rfd::AsyncFileDialog {
    let mut d = rfd::AsyncFileDialog::new().set_title(title);
    if let Some(dir) = starting_dir(start) {
        d = d.set_directory(dir);
    }
    d
}

/// Picks a folder into a text field.
fn browse_into(window: &mut Window, cx: &mut App, title: &str, input: Entity<InputState>) {
    let start = read(&input, cx);
    pick(window, cx, Pick::Folder, panel(title, &start), move |paths, window, cx| {
        let dir = paths[0].to_string_lossy().into_owned();
        input.update(cx, |s, cx| s.set_value(dir, window, cx));
    });
}

fn browse_button(id: &'static str, title: &'static str, input: Entity<InputState>) -> Button {
    Button::new(id).label("Browse…").small().on_click(move |_, window, cx: &mut App| browse_into(window, cx, title, input.clone()))
}

// ------------------------------------------------------------------ prompt

pub struct PromptSheet {
    title: String,
    label: String,
    input: Entity<InputState>,
    on_ok: Rc<dyn Fn(String, &mut App)>,
}

impl PromptSheet {
    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let value = read(&self.input, cx);
        if value.trim().is_empty() {
            return;
        }
        (self.on_ok)(value, cx);
        window.close_dialog(cx);
    }
}

impl Render for PromptSheet {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Theme::get(cx).clone();
        div()
            .flex()
            .flex_col()
            .child(title(&p, self.title.clone()))
            .child(div().mb(px(6.)).text_color(p.ink_dim).child(self.label.clone()))
            .child(Input::new(&self.input))
            .child(
                footer(&p, None)
                    .child(Button::new("cancel").label("Cancel").on_click(|_, window, cx| window.close_dialog(cx)))
                    .child(div().debug_selector(|| "prompt-ok".into()).child(Button::new("ok").label("OK").primary().on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))))),
            )
    }
}

pub fn prompt<V: 'static>(window: &mut Window, cx: &mut Context<V>, title: &str, label: &str, value: &str, ok: impl Fn(String, &mut App) + 'static) {
    let input = text_input(window, cx, value, "");
    let (title, label) = (title.to_string(), label.to_string());
    let sheet = cx.new(|_| PromptSheet { title, label, input: input.clone(), on_ok: Rc::new(ok) });
    let target = sheet.clone();
    open(window, cx, 400., sheet, Confirm::With(Rc::new(move |window, cx| target.update(cx, |s, cx| s.submit(window, cx)))));
    input.update(cx, |s, cx| s.focus(window, cx));
}

// ------------------------------------------------------------- notice sheet

pub struct NoticeSheet {
    title: String,
    message: String,
    detail: String,
}

impl Render for NoticeSheet {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Theme::get(cx).clone();
        div()
            .flex()
            .flex_col()
            .child(title(&p, self.title.clone()))
            .child(div().text_color(p.err).child(self.message.clone()))
            .child(div().mt(px(6.)).text_color(p.ink_dim).child(self.detail.clone()))
            .child(footer(&p, None).child(div().debug_selector(|| "close".into()).child(Button::new("close").label("Close").on_click(|_, window, cx| window.close_dialog(cx)))))
    }
}

// --------------------------------------------------------------------- Add

pub struct AddSheet {
    source: TorrentSource,
    info: InspectInfo,
    path: Entity<InputState>,
    labels: Vec<String>,
    /// None: no label; Some(""): New label…; Some(name): that label.
    label: Option<String>,
    new_label: Entity<InputState>,
    checked: Vec<bool>,
    start: bool,
    sequential: bool,
    last_save_path: String,
    error: Option<String>,
    busy: bool,
    focus: FocusHandle,
}

/// The "Add New Torrent" sheet: destination, contents, and start options.
/// `save_path` overrides the offered folder for this sheet only; otherwise it
/// offers wherever the last torrent went, falling back to the default folder.
pub fn open_add<V: 'static>(window: &mut Window, cx: &mut Context<V>, source: TorrentSource, save_path: Option<String>) {
    crate::debug(format!("open_add: inspecting {source:?}"));
    let rx = ask(cx, |reply| Command::Inspect { source: source.clone(), reply });
    cx.spawn_in(window, async move |_, cx| {
        let result = rx.await.unwrap_or_else(|_| Err("the engine is not running".into()));
        crate::debug(format!("open_add: inspect answered ok={}", result.is_ok()));
        let _ = cx.update(move |window, cx| match result {
            Err(err) => {
                let sheet = cx.new(|_| NoticeSheet { title: "Add Torrent".into(), message: "Could not read this torrent.".into(), detail: err });
                open(window, cx, 420., sheet, Confirm::Close);
            }
            Ok(info) => {
                let lib = cx.global::<Library>().clone();
                let offered = save_path.clone().filter(|p| !p.is_empty()).unwrap_or_else(|| lib.settings.offered_save_path().to_string());
                let path = text_input(window, cx, &offered, "");
                let new_label = text_input(window, cx, "", "Label name");
                let checked = vec![true; info.files.len()];
                let sheet = cx.new(|cx| AddSheet {
                    source,
                    info,
                    path,
                    labels: lib.labels.clone(),
                    label: None,
                    new_label,
                    checked,
                    start: lib.settings.start_torrents_automatically,
                    sequential: lib.settings.sequential_download,
                    last_save_path: lib.settings.last_save_path.clone(),
                    error: None,
                    busy: false,
                    focus: cx.focus_handle(),
                });
                let target = sheet.clone();
                open(window, cx, 560., sheet, Confirm::With(Rc::new(move |window, cx| target.update(cx, |s, cx| s.confirm(window, cx)))));
            }
        });
    })
    .detach();
}

impl AddSheet {
    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let save_path = read(&self.path, cx).trim().to_string();
        if save_path.is_empty() {
            self.error = Some("Choose a download folder.".into());
            return cx.notify();
        }
        let label = match &self.label {
            None => String::new(),
            Some(l) if l.is_empty() => read(&self.new_label, cx).trim().to_string(),
            Some(l) => l.clone(),
        };
        let mut priorities = BTreeMap::new();
        let mut wanted = None;
        if !self.info.files.is_empty() {
            let mut w = Vec::new();
            for (f, on) in self.info.files.iter().zip(&self.checked) {
                priorities.insert(f.index, if *on { 1 } else { 0 });
                if *on {
                    w.push(f.index);
                }
            }
            if w.is_empty() {
                self.error = Some("Select at least one file.".into());
                return cx.notify();
            }
            wanted = Some(w);
        }
        let options = AddOptions { save_path: Some(save_path.clone()), label, sequential: Some(self.sequential), paused: !self.start, wanted, priorities };
        let rx = ask(cx, |reply| Command::Add { source: self.source.clone(), options, reply: Some(reply) });
        self.busy = true;
        self.error = None;
        cx.notify();
        let last = self.last_save_path.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = rx.await.unwrap_or_else(|_| Err("the engine is not running".into()));
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(AddOutcome::Duplicate(_)) => this.error = Some("This torrent is already in the list.".into()),
                    Err(err) => this.error = Some(err),
                    Ok(AddOutcome::Added(_)) => {
                        // Remember where it went, so the next torrent is offered the same folder.
                        if save_path != last {
                            send(cx, Command::SetSettings(patch("lastSavePath", save_path.clone())));
                        }
                        window.close_dialog(cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}

impl Render for AddSheet {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Theme::get(cx).clone();
        let total: u64 = self.info.files.iter().zip(&self.checked).filter(|(_, c)| **c).map(|(f, _)| f.length).sum();
        let size_text = if self.info.files.is_empty() || total == self.info.length {
            fmt::bytes(self.info.length as f64, false)
        } else {
            format!("{} of {}", fmt::bytes(total as f64, false), fmt::bytes(self.info.length as f64, false))
        };
        let label_text = match &self.label {
            None => "(none)".to_string(),
            Some(l) if l.is_empty() => "New label…".to_string(),
            Some(l) => l.clone(),
        };
        let mut options = vec![("=".to_string(), "(none)".to_string())];
        options.extend(self.labels.iter().map(|l| (format!("+{l}"), l.clone())));
        options.push(("new".into(), "New label…".into()));
        let selected = match &self.label {
            None => "=".to_string(),
            Some(l) if l.is_empty() => "new".to_string(),
            Some(l) => format!("+{l}"),
        };

        let contents = if self.info.files.is_empty() {
            div().text_color(p.ink_dim).child(if self.source.is_magnet() {
                "This is a magnet link — the file list arrives once metadata is fetched from the swarm."
            } else {
                "Single-file torrent."
            })
            .into_any_element()
        } else {
            let all = self.checked.iter().all(|c| *c);
            let mut table = div().flex().flex_col().child(
                div()
                    .flex()
                    .items_center()
                    .h(px(30.))
                    .border_b_1()
                    .border_color(p.line)
                    .text_size(px(12.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(p.ink_dim)
                    .child(div().w(px(34.)).pl(px(10.)).child(Checkbox::new("all").checked(all).on_click(cx.listener(|this, on: &bool, _, cx| {
                        this.checked.iter_mut().for_each(|c| *c = *on);
                        cx.notify();
                    }))))
                    .child(div().flex_1().child("Name"))
                    .child(div().w(px(88.)).pr(px(10.)).flex().justify_end().child("Size")),
            );
            for (i, f) in self.info.files.iter().enumerate() {
                table = table.child(
                    div()
                        .flex()
                        .items_center()
                        .h(px(26.))
                        .child(div().w(px(34.)).pl(px(10.)).child(Checkbox::new(("file", i)).checked(self.checked[i]).on_click(cx.listener(move |this, on: &bool, _, cx| {
                            this.checked[i] = *on;
                            cx.notify();
                        }))))
                        .child(div().flex_1().min_w_0().truncate().child(if f.path.is_empty() { f.name.clone() } else { f.path.clone() }))
                        .child(div().w(px(88.)).pr(px(10.)).flex().justify_end().font_features(tabular()).child(fmt::bytes(f.length as f64, false))),
                );
            }
            div().id("filelist").max_h(px(240.)).overflow_y_scroll().border_1().border_color(p.line).rounded(RADIUS).child(table).into_any_element()
        };

        div()
            .id("add-sheet")
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .on_action(cx.listener(|this, a: &PickOption, window, cx| {
                if a.field == "label" {
                    this.label = match a.value.as_str() {
                        "=" => None,
                        "new" => {
                            this.new_label.update(cx, |s, cx| s.focus(window, cx));
                            Some(String::new())
                        }
                        v => Some(v.trim_start_matches('+').to_string()),
                    };
                    cx.notify();
                }
            }))
            .child(title(&p, "Add New Torrent"))
            .child(legend(&p, "Save In"))
            .child(div().flex().gap(px(8.)).mb(px(10.)).child(div().flex_1().child(Input::new(&self.path))).child(browse_button("browse", "Choose Download Folder", self.path.clone())))
            .child(frow(
                &p,
                "Label:",
                div()
                    .flex()
                    .gap(px(8.))
                    .child(div().w(px(170.)).child(dropdown(&p, "label", label_text, "label", options, selected, &self.focus)))
                    .when(self.label.as_deref() == Some(""), |d| d.child(div().w(px(170.)).child(Input::new(&self.new_label)))),
            ))
            .child(div().mt(px(8.)).child(legend(&p, "Torrent Contents")))
            .child(
                div()
                    .flex()
                    .justify_between()
                    .gap(px(12.))
                    .mb(px(6.))
                    .child(div().min_w_0().truncate().font_weight(FontWeight::BOLD).child(self.info.name.clone()))
                    .child(div().text_color(p.ink_dim).child(size_text)),
            )
            .child(contents)
            .child(div().mt(px(8.)).child(frow(&p, "Info Hash:", div().font_family(MONO).text_size(px(10.)).child(self.info.info_hash.clone()))))
            .when(!self.info.comment.is_empty(), |d| d.child(frow(&p, "Comment:", div().child(self.info.comment.clone()))))
            .child(frow(&p, "Trackers:", div().child(format!("{} announce URL(s)", self.info.announce.len()))))
            .child(
                div()
                    .flex()
                    .gap(px(16.))
                    .child(Checkbox::new("start").label("Start torrent").checked(self.start).on_click(cx.listener(|this, on: &bool, _, cx| {
                        this.start = *on;
                        cx.notify();
                    })))
                    .child(Checkbox::new("seq").label("Download sequentially").checked(self.sequential).on_click(cx.listener(|this, on: &bool, _, cx| {
                        this.sequential = *on;
                        cx.notify();
                    }))),
            )
            .children(error_line(&p, &self.error))
            .child(
                footer(&p, None)
                    .child(Button::new("cancel").label("Cancel").on_click(|_, window, cx| window.close_dialog(cx)))
                    .child(div().debug_selector(|| "add-ok".into()).child(Button::new("ok").label("OK").primary().disabled(self.busy).on_click(cx.listener(|this, _, window, cx| this.confirm(window, cx))))),
            )
    }
}

// ------------------------------------------------------------------ URL

pub struct UrlSheet {
    url: Entity<InputState>,
    path: Entity<InputState>,
    error: Option<String>,
}

pub fn open_url<V: 'static>(window: &mut Window, cx: &mut Context<V>) {
    let lib = cx.global::<Library>().clone();
    // Offer whatever magnet link is already on the clipboard, like uTorrent does.
    let clip = cx.read_from_clipboard().and_then(|c| c.text()).map(|t| t.trim().to_string()).unwrap_or_default();
    let offered = clip.starts_with("magnet:") || ((clip.starts_with("http://") || clip.starts_with("https://")) && clip.to_ascii_lowercase().contains(".torrent"));
    let url = text_input(window, cx, if offered { &clip } else { "" }, "magnet:?xt=urn:btih:… or https://example.org/file.torrent");
    let path = text_input(window, cx, lib.settings.offered_save_path(), "");
    let sheet = cx.new(|_| UrlSheet { url: url.clone(), path, error: None });
    let target = sheet.clone();
    open(window, cx, 480., sheet, Confirm::With(Rc::new(move |window, cx| target.update(cx, |s, cx| s.confirm(window, cx)))));
    url.update(cx, |s, cx| s.focus(window, cx));
}

impl UrlSheet {
    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let v = read(&self.url, cx).trim().to_string();
        if v.is_empty() {
            self.error = Some("Enter a link.".into());
            return cx.notify();
        }
        let save_path = read(&self.path, cx).trim().to_string();
        window.close_dialog(cx);
        open_add(window, cx, TorrentSource::parse(&v), Some(save_path));
    }
}

impl Render for UrlSheet {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Theme::get(cx).clone();
        div()
            .flex()
            .flex_col()
            .child(title(&p, "Add Torrent from URL"))
            .child(div().mb(px(6.)).text_color(p.ink_dim).child("Enter a magnet link, an info hash, or an http(s) link to a .torrent file:"))
            .child(Input::new(&self.url))
            .child(div().mt(px(8.)).child(frow(&p, "Save In:", div().flex().gap(px(8.)).child(div().flex_1().child(Input::new(&self.path))).child(browse_button("browse", "Choose Download Folder", self.path.clone())))))
            .children(error_line(&p, &self.error))
            .child(
                footer(&p, None)
                    .child(Button::new("cancel").label("Cancel").on_click(|_, window, cx| window.close_dialog(cx)))
                    .child(div().debug_selector(|| "url-ok".into()).child(Button::new("ok").label("Continue").primary().on_click(cx.listener(|this, _, window, cx| this.confirm(window, cx))))),
            )
    }
}

// --------------------------------------------------------------- Search

#[derive(Clone, Copy, PartialEq)]
enum SortBy {
    Name,
    Source,
    Size,
    Seeds,
}

const RESULT_COUNTS: [u64; 4] = [20, 40, 100, 200];

/// The fields under Advanced in the source editor, by their key in a source
/// definition, with what each one names.
const MAPPING_FIELDS: [(&str, &str); 9] = [
    ("results", "Results list:"),
    ("items", "Versions list:"),
    ("title", "Title:"),
    ("hash", "Info hash:"),
    ("magnet", "Magnet link:"),
    ("link", "Torrent file link:"),
    ("size", "Size:"),
    ("seeders", "Seeders:"),
    ("leechers", "Leechers:"),
];

const SOURCE_EXAMPLE: &str = "https://example.org/api/search?q={query}";
const RESPONSIBILITY: &str = "ztorrent comes with no third-party torrent sites. You choose the sources you add, and you are responsible for them and for what you download.";

/// Adding or editing one of the user's own sources.
struct SourceEditor {
    /// The source being edited, by its place in the list; None when adding.
    index: Option<usize>,
    name: Entity<InputState>,
    url: Entity<InputState>,
    test_query: Entity<InputState>,
    mapping: BTreeMap<&'static str, Entity<InputState>>,
    trackers: Entity<InputState>,
    mirrors: Entity<InputState>,
    advanced: bool,
    /// The step-by-step guide is unfolded.
    guide: bool,
    testing: bool,
    preview: Vec<SearchResult>,
    error: Option<String>,
    /// The question for an AI assistant was just copied.
    asked: bool,
    /// Where an AI assistant's answer, or a shared definition, is pasted.
    answer: Entity<TextareaState>,
    /// What came of the last paste there: filled in, or why not.
    filled: Option<Result<String, String>>,
    generation: u32,
    _answer_changed: Subscription,
    /// Return in the address or test-search field runs Test.
    _enter: Vec<Subscription>,
}

impl SourceEditor {
    /// Puts a definition's every field into the form.
    fn fill(&mut self, shared: &CustomSource, window: &mut Window, cx: &mut App) {
        self.name.update(cx, |s, cx| s.set_value(shared.name.clone(), window, cx));
        self.url.update(cx, |s, cx| s.set_value(shared.url.clone(), window, cx));
        self.trackers.update(cx, |s, cx| s.set_value(shared.trackers.join(", "), window, cx));
        self.mirrors.update(cx, |s, cx| s.set_value(shared.mirrors.join(", "), window, cx));
        let values = [&shared.results, &shared.items, &shared.title, &shared.hash, &shared.magnet, &shared.link, &shared.size, &shared.seeders, &shared.leechers];
        for ((key, _), value) in MAPPING_FIELDS.iter().zip(values) {
            self.mapping[key].update(cx, |s, cx| s.set_value(value.clone(), window, cx));
        }
        self.advanced |= values.iter().any(|v| !v.is_empty()) || !shared.trackers.is_empty() || !shared.mirrors.is_empty();
        self.preview.clear();
        self.error = None;
    }
}

pub struct SearchSheet {
    query: Entity<InputState>,
    indexer_url: Entity<InputState>,
    indexer_key: Entity<InputState>,
    /// The search settings as this sheet last saw or set them.
    settings: Settings,
    show_settings: bool,
    editor: Option<SourceEditor>,
    /// A passing word under the source list, such as "Copied".
    flash: Option<String>,
    results: Vec<SearchResult>,
    notes: Vec<String>,
    sort: SortBy,
    descending: bool,
    busy: bool,
    searched: bool,
    /// Bumped by every search, so a slow answer to an earlier one is ignored.
    generation: u32,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

pub fn open_search<V: 'static>(window: &mut Window, cx: &mut Context<V>) {
    let settings = cx.global::<Library>().settings.clone();
    let query = text_input(window, cx, "", "Search for torrents");
    let indexer_url = text_input(window, cx, &settings.search_indexer_url, "http://127.0.0.1:9117");
    let indexer_key = cx.new(|cx| {
        let mut s = InputState::new(window, cx).masked(true);
        s.set_value(settings.search_indexer_key.clone(), window, cx);
        s
    });
    let sheet = cx.new(|cx| {
        // The address and key are saved when they lose focus, not per keystroke.
        let save_on_blur = |cx: &mut Context<SearchSheet>, input: &Entity<InputState>| {
            cx.subscribe(input, |this: &mut SearchSheet, _, ev: &InputEvent, cx| {
                if matches!(ev, InputEvent::Blur) {
                    this.save_indexer_fields(cx);
                }
            })
        };
        let mut subs = vec![save_on_blur(cx, &indexer_url), save_on_blur(cx, &indexer_key)];
        // Return in the search box searches. The input takes the key itself, so
        // the dialog's own Enter handling never sees it.
        subs.push(cx.subscribe_in(&query, window, |this: &mut SearchSheet, _, ev: &InputEvent, window, cx| {
            if matches!(ev, InputEvent::PressEnter { .. }) {
                this.run(window, cx);
            }
        }));
        SearchSheet {
            query: query.clone(),
            indexer_url,
            indexer_key,
            settings,
            show_settings: false,
            editor: None,
            flash: None,
            results: Vec::new(),
            notes: Vec::new(),
            sort: SortBy::Seeds,
            descending: true,
            busy: false,
            searched: false,
            generation: 0,
            focus: cx.focus_handle(),
            _subscriptions: subs,
        }
    });
    let target = sheet.clone();
    // Wide enough for a result's name, size and seeds side by side; long
    // names wrap rather than widen the sheet.
    let width = (f32::from(window.viewport_size().width) - 80.).clamp(520., 720.);
    open_sticky(window, cx, width, sheet, Confirm::With(Rc::new(move |window, cx| target.update(cx, |s, cx| s.confirm(window, cx)))));
    query.update(cx, |s, cx| s.focus(window, cx));
}

impl SearchSheet {
    /// Saves one search setting at once: there is no OK button to wait for.
    fn set(&mut self, key: &str, value: impl Into<serde_json::Value>, cx: &mut Context<Self>) {
        let change = patch(key, value);
        self.settings = self.settings.patched(&change);
        send(cx, Command::SetSettings(change));
        cx.notify();
    }

    fn set_custom(&mut self, list: Vec<CustomSource>, cx: &mut Context<Self>) {
        self.set("searchCustomSources", serde_json::to_value(list).unwrap_or_default(), cx);
    }

    fn save_indexer_fields(&mut self, cx: &mut Context<Self>) {
        let url = read(&self.indexer_url, cx).trim().to_string();
        let key = read(&self.indexer_key, cx).trim().to_string();
        if url != self.settings.search_indexer_url {
            self.set("searchIndexerUrl", url, cx);
        }
        if key != self.settings.search_indexer_key {
            self.set("searchIndexerKey", key, cx);
        }
    }

    /// Enter: tests the source being edited, or searches.
    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.editor.is_some() {
            self.test_source(window, cx);
        } else {
            self.run(window, cx);
        }
    }

    fn run(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.save_indexer_fields(cx);
        let query = read(&self.query, cx).trim().to_string();
        if query.is_empty() || self.busy {
            return;
        }
        let options = self.settings.search_options();
        if options.sources.is_empty() && options.custom.is_empty() {
            self.notes = vec!["Every source is switched off. Turn one on, or add your own, in the search settings.".into()];
            self.show_settings = true;
            return cx.notify();
        }
        self.generation += 1;
        let generation = self.generation;
        self.busy = true;
        self.searched = true;
        self.results.clear();
        self.notes.clear();
        cx.notify();
        let rx = ask(cx, |reply| Command::Search { query, options, reply });
        cx.spawn_in(window, async move |this, cx| {
            let outcome = rx.await.unwrap_or_else(|_| SearchOutcome { results: Vec::new(), failures: vec!["the engine is not running".into()] });
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.busy = false;
                this.results = outcome.results;
                this.notes = outcome.failures;
                this.sort_results();
                cx.notify();
            });
        })
        .detach();
    }

    fn sort_by(&mut self, by: SortBy, cx: &mut Context<Self>) {
        if self.sort == by {
            self.descending = !self.descending;
        } else {
            self.sort = by;
            // Most seeded and largest first; names and sources A to Z.
            self.descending = matches!(by, SortBy::Seeds | SortBy::Size);
        }
        self.sort_results();
        cx.notify();
    }

    fn sort_results(&mut self) {
        let by = self.sort;
        // Stable, so hits that tie keep the order their source ranked them in.
        self.results.sort_by(|a, b| {
            let o = match by {
                SortBy::Name => fmt::compare_text(&a.title, &b.title),
                SortBy::Source => fmt::compare_text(&a.source, &b.source),
                SortBy::Size => a.size.cmp(&b.size),
                SortBy::Seeds => a.seeders.cmp(&b.seeders),
            };
            if self.descending { o.reverse() } else { o }
        });
    }

    // ------------------------------------------------------ source editor

    fn open_editor(&mut self, index: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let source = index.and_then(|i| self.settings.search_custom_sources.get(i).cloned()).unwrap_or_default();
        let field = |key: &str| match key {
            "results" => source.results.clone(),
            "items" => source.items.clone(),
            "title" => source.title.clone(),
            "hash" => source.hash.clone(),
            "magnet" => source.magnet.clone(),
            "link" => source.link.clone(),
            "size" => source.size.clone(),
            "seeders" => source.seeders.clone(),
            "leechers" => source.leechers.clone(),
            _ => String::new(),
        };
        let mapping = MAPPING_FIELDS.iter().map(|(key, _)| (*key, text_input(window, cx, &field(key), "found automatically"))).collect::<BTreeMap<_, _>>();
        let advanced = mapping.keys().any(|k| !field(k).is_empty()) || !source.trackers.is_empty() || !source.mirrors.is_empty();
        let typed = read(&self.query, cx).trim().to_string();
        let answer = cx.new(|cx| TextareaState::new(window, cx).rows(5).placeholder("Paste the assistant's whole answer here, or a definition someone shared"));
        let editor = SourceEditor {
            index,
            name: text_input(window, cx, &source.name, "e.g. My favourite site"),
            url: text_input(window, cx, &source.url, SOURCE_EXAMPLE),
            // A word every catalogue has titles with, unless a search is typed.
            test_query: text_input(window, cx, if typed.is_empty() { "the" } else { &typed }, ""),
            mapping,
            trackers: text_input(window, cx, &source.trackers.join(", "), "udp://tracker.example.org:1337/announce, …"),
            mirrors: text_input(window, cx, &source.mirrors.join(", "), "https://mirror.example.org/api?q={query}, …"),
            advanced,
            guide: false,
            testing: false,
            preview: Vec::new(),
            error: None,
            asked: false,
            filled: None,
            generation: 0,
            _answer_changed: cx.subscribe_in(&answer, window, |this, state, ev: &InputEvent, window, cx| {
                if matches!(ev, InputEvent::Change) {
                    this.answer_pasted(state.read(cx).value().to_string(), window, cx);
                }
            }),
            answer,
            _enter: Vec::new(),
        };
        let mut editor = editor;
        for input in [editor.url.clone(), editor.test_query.clone()] {
            editor._enter.push(cx.subscribe_in(&input, window, |this: &mut SearchSheet, _, ev: &InputEvent, window, cx| {
                if matches!(ev, InputEvent::PressEnter { .. }) {
                    this.test_source(window, cx);
                }
            }));
        }
        let focus = if index.is_some() { editor.url.clone() } else { editor.name.clone() };
        self.editor = Some(editor);
        self.show_settings = true;
        self.flash = None;
        focus.update(cx, |s, cx| s.focus(window, cx));
        cx.notify();
    }

    /// The source as the editor's fields describe it. A definition pasted into
    /// the address field fills in every field first.
    fn editor_source(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Result<CustomSource, String> {
        let Some(ed) = &mut self.editor else { return Err(String::new()) };
        let pasted = read(&ed.url, cx).trim().to_string();
        // A definition pasted into Address instead of the answer box still works.
        if CustomSource::looks_shared(&pasted) {
            let shared = CustomSource::from_shared(&pasted)?;
            ed.fill(&shared, window, cx);
        }
        let m = |key: &str| read(&ed.mapping[key], cx).trim().to_string();
        let enabled = ed.index.and_then(|i| self.settings.search_custom_sources.get(i)).is_none_or(|s| s.enabled);
        let source = CustomSource {
            name: read(&ed.name, cx).trim().to_string(),
            url: read(&ed.url, cx).trim().to_string(),
            enabled,
            results: m("results"),
            items: m("items"),
            title: m("title"),
            hash: m("hash"),
            magnet: m("magnet"),
            link: m("link"),
            size: m("size"),
            seeders: m("seeders"),
            leechers: m("leechers"),
            trackers: read(&ed.trackers, cx).split([',', '\n', ' ']).map(str::trim).filter(|t| !t.is_empty()).map(String::from).collect(),
            mirrors: read(&ed.mirrors, cx).split([',', '\n', ' ']).map(str::trim).filter(|t| !t.is_empty()).map(String::from).collect(),
        };
        source.check()?;
        if SEARCH_SOURCES.iter().any(|(_, n)| n.eq_ignore_ascii_case(&source.name)) {
            return Err("That is the name of a built-in source: choose another.".into());
        }
        Ok(source)
    }

    /// Fills the form from an answer as soon as it is pasted, so the next step
    /// is only Test.
    fn answer_pasted(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ed) = &mut self.editor else { return };
        let text = text.trim();
        ed.filled = if text.is_empty() {
            None
        } else {
            match CustomSource::from_shared(text) {
                Ok(shared) => {
                    ed.fill(&shared, window, cx);
                    let note = CustomSource::shared_note(text).map(|n| format!(" The assistant notes: {n}")).unwrap_or_default();
                    Some(Ok(format!("Filled in \u{201c}{}\u{201d} from the answer. Now click Test to check that it finds results.{note}", shared.name)))
                }
                // Still being typed or pasted: say nothing until it could be whole.
                Err(_) if !text.contains('}') => None,
                Err(err) => Some(Err(err)),
            }
        };
        cx.notify();
    }

    fn test_source(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let source = self.editor_source(window, cx);
        let Some(ed) = &mut self.editor else { return };
        ed.preview.clear();
        let source = match source {
            Ok(s) => s,
            Err(err) => {
                ed.error = Some(err);
                return cx.notify();
            }
        };
        let query = read(&ed.test_query, cx).trim().to_string();
        let query = if query.is_empty() { "the".to_string() } else { query };
        ed.generation += 1;
        let generation = ed.generation;
        ed.testing = true;
        ed.error = None;
        cx.notify();
        let rx = ask(cx, |reply| Command::TestSource { source, query, reply });
        cx.spawn_in(window, async move |this, cx| {
            let result = rx.await.unwrap_or_else(|_| Err("the engine is not running".into()));
            let _ = this.update(cx, |this, cx| {
                let Some(ed) = &mut this.editor else { return };
                if ed.generation != generation {
                    return;
                }
                ed.testing = false;
                match result {
                    Ok(hits) => ed.preview = hits,
                    Err(err) => ed.error = Some(err),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn save_source(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let source = match self.editor_source(window, cx) {
            Ok(s) => s,
            Err(err) => {
                if let Some(ed) = &mut self.editor {
                    ed.error = Some(err);
                }
                return cx.notify();
            }
        };
        let mut list = self.settings.search_custom_sources.clone();
        let name = source.name.clone();
        let editing = self.editor.as_ref().and_then(|e| e.index).filter(|i| *i < list.len());
        let namesake = self.namesake(&name);
        match (editing, namesake) {
            // Renamed onto another source: two of one name could not be told apart.
            (Some(i), Some(other)) if other != i => {
                if let Some(ed) = &mut self.editor {
                    ed.error = Some(format!("There is already a source called \u{201c}{name}\u{201d}. Choose another name, or remove that one first."));
                }
                return cx.notify();
            }
            (Some(i), _) => list[i] = source,
            // Adding one under a name already taken replaces it: the usual reason
            // is a fresh definition for a source that stopped working.
            (None, Some(other)) => list[other] = CustomSource { enabled: list[other].enabled, ..source },
            (None, None) => list.push(source),
        }
        self.set_custom(list, cx);
        self.editor = None;
        self.flash = Some(format!("Saved \u{201c}{name}\u{201d}. It is searched along with the others."));
        cx.notify();
    }

    /// The user's source of this name, if there is one.
    fn namesake(&self, name: &str) -> Option<usize> {
        let name = name.trim();
        self.settings.search_custom_sources.iter().position(|s| !name.is_empty() && s.name.eq_ignore_ascii_case(name))
    }

    fn render_editor(&self, ed: &SourceEditor, p: &Palette, cx: &mut Context<Self>) -> Div {
        let step = |n: &str, text: &str| {
            div()
                .flex()
                .gap(px(8.))
                .mb(px(6.))
                .child(div().flex_none().w(px(18.)).h(px(18.)).rounded_full().bg(p.accent_soft).text_color(p.accent).text_size(px(11.)).flex().items_center().justify_center().child(n.to_string()))
                .child(div().flex_1().min_w_0().text_color(p.ink_dim).child(text.to_string()))
        };
        let guide = div()
            .mb(px(14.))
            .p(px(12.))
            .rounded(RADIUS)
            .bg(p.bg)
            .border_1()
            .border_color(p.line)
            .child(step("1", "Find the site's search API: an address that answers a search with JSON, not a web page. Sites usually describe it on an \u{201c}API\u{201d} page."))
            .child(step("2", "Copy the address of a search and put {query} where your search words are, e.g."))
            .child(div().ml(px(26.)).mb(px(8.)).min_w_0().font_family(MONO).text_size(px(12.)).text_color(p.ink).child(SOURCE_EXAMPLE))
            .child(step("3", "Click Test. ztorrent finds the results and their fields by itself. If the preview looks right, click Save."))
            .child(div().ml(px(26.)).text_size(px(12.)).text_color(p.ink_faint).child("Got a definition someone shared? Paste it into the answer box below: it fills in everything."));
        let ask_ai = div()
            .mb(px(14.))
            .p(px(12.))
            .rounded(RADIUS)
            .bg(p.accent_soft)
            .child(div().mb(px(4.)).font_weight(FontWeight::SEMIBOLD).child("Don't know the address? Ask an AI assistant."))
            .child(div().mb(px(8.)).min_w_0().text_color(p.ink_dim).child("Copy this question, paste it into an AI assistant and type the site's name right after it. The assistant replies with a short piece of JSON: paste it into the box here, and Name and Address fill themselves in."))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(Button::new("ask-ai").label("Copy question for an AI").small().on_click(cx.listener(|this, _, _, cx| {
                        let Some(ed) = &mut this.editor else { return };
                        let prompt = CustomSource::assistant_prompt();
                        cx.write_to_clipboard(ClipboardItem::new_string(prompt));
                        ed.asked = true;
                        cx.notify();
                    })))
                    .children(ed.asked.then(|| div().flex_1().min_w_0().text_size(px(12.)).text_color(p.ok).child("Copied. Now paste it into an AI assistant and type the site's name at the end."))),
            )
            .child(div().mt(px(10.)).font_family(MONO).text_size(px(12.)).child(Textarea::new(&ed.answer).h(px(110.))))
            .children(ed.filled.clone().map(|f| match f {
                Ok(msg) => div().mt(px(6.)).min_w_0().text_size(px(12.)).text_color(p.ok).child(msg),
                Err(err) => div().mt(px(6.)).min_w_0().text_size(px(12.)).text_color(p.err).child(err),
            }));
        let mut form = div()
            .child(legend(p, if ed.index.is_some() { "Edit source" } else { "Add a source" }))
            .child(frow(p, "Name:", Input::new(&ed.name)))
            .children(self.namesake(&read(&ed.name, cx)).filter(|i| Some(*i) != ed.index).map(|_| {
                let note = if ed.index.is_some() { "Another source already has this name." } else { "You already have a source with this name: saving replaces it." };
                hint(p, note).text_color(p.warn)
            }))
            .child(frow(p, "Address:", Input::new(&ed.url)))
            .child(frow(p, "Test search:", div().flex().gap(px(8.)).child(div().flex_1().child(Input::new(&ed.test_query))).child(Button::new("test-source").label(if ed.testing { "Testing…" } else { "Test" }).small().disabled(ed.testing).on_click(cx.listener(|this, _, window, cx| this.test_source(window, cx))))));
        if let Some(err) = &ed.error {
            form = form.child(div().mb(px(10.)).text_color(p.err).child(err.clone()));
        }
        if !ed.preview.is_empty() {
            let mut rows = div().mb(px(10.)).p(px(8.)).rounded(RADIUS).border_1().border_color(p.line).bg(p.bg);
            rows = rows.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .mb(px(6.))
                    .child(div().flex_1().min_w_0().text_size(px(12.)).text_color(p.ok).child(format!("It works: {} results. The first few:", ed.preview.len())))
                    .child(div().debug_selector(|| "save-tested".into()).child(Button::new("save-tested").label("Save this source").small().primary().on_click(cx.listener(|this, _, window, cx| this.save_source(window, cx))))),
            );
            for hit in ed.preview.iter().take(5) {
                let seeds = hit.seeders.map(|n| format!("{n} seeds")).unwrap_or_default();
                rows = rows.child(
                    div()
                        .flex()
                        .gap(px(10.))
                        .text_size(px(12.))
                        .mb(px(3.))
                        .child(div().flex_1().min_w_0().line_clamp(2).child(hit.title.clone()))
                        .child(div().flex_none().text_color(p.ink_faint).font_features(tabular()).child(format!("{} {seeds}", fmt::bytes(hit.size as f64, true)))),
                );
            }
            form = form.child(rows);
        }
        form = form.child(ask_ai);
        let guide_toggle = div()
            .id("guide")
            .mb(px(10.))
            .text_color(p.accent)
            .cursor_pointer()
            .child(if ed.guide { "▾ How do I find a site's search API?" } else { "▸ How do I find a site's search API?" })
            .on_click(cx.listener(|this, _, _, cx| {
                if let Some(ed) = &mut this.editor {
                    ed.guide = !ed.guide;
                }
                cx.notify();
            }));
        form = form.child(guide_toggle);
        if ed.guide {
            form = form.child(guide);
        }
        let toggle = div()
            .id("advanced")
            .mb(px(10.))
            .text_color(p.accent)
            .cursor_pointer()
            .child(if ed.advanced { "▾ Advanced: which fields to read" } else { "▸ Advanced: which fields to read" })
            .on_click(cx.listener(|this, _, _, cx| {
                if let Some(ed) = &mut this.editor {
                    ed.advanced = !ed.advanced;
                }
                cx.notify();
            }));
        form = form.child(toggle);
        if ed.advanced {
            form = form.child(hint(p, "Only if Test finds nothing or the wrong thing. Name a field as a dotted path, e.g. data.results or torrents.").ml(px(0.)).mt(px(0.)));
            for (key, label) in MAPPING_FIELDS {
                form = form.child(frow(p, label, Input::new(&ed.mapping[key])));
            }
            form = form.child(frow(p, "Trackers for hashes:", Input::new(&ed.trackers)));
            form = form.child(frow(p, "Backup addresses:", Input::new(&ed.mirrors)));
        }
        form.child(
            div()
                .flex()
                .items_center()
                .justify_end()
                .gap(px(8.))
                .child(Button::new("cancel-source").label("Cancel").small().on_click(cx.listener(|this, _, _, cx| {
                    this.editor = None;
                    cx.notify();
                })))
                .child(Button::new("save-source").label("Save").small().primary().on_click(cx.listener(|this, _, window, cx| this.save_source(window, cx)))),
        )
    }

    fn render_settings(&self, p: &Palette, cx: &mut Context<Self>) -> Div {
        let panel = div().mt(px(12.)).p(px(14.)).border_1().border_color(p.line).rounded(RADIUS).bg(p.chrome);
        if let Some(ed) = &self.editor {
            return panel.child(self.render_editor(ed, p, cx));
        }
        let s = &self.settings;
        let mut built_in = div().flex().flex_col().gap(px(10.)).mb(px(20.));
        for (id, name) in SEARCH_SOURCES {
            let on = s.search_source_enabled(id);
            let about = match id {
                "archive" => "Free, legal films, music, books and software.",
                _ => "Only if you run Jackett or Prowlarr: free programs that search every site you add to them. Leave this off otherwise.",
            };
            built_in = built_in.child(
                div()
                    .child(Checkbox::new(SharedString::from(format!("src-{id}"))).label(name).checked(on).on_click(cx.listener(move |this, on: &bool, _, cx| {
                        let list = this.settings.search_sources_with(id, *on);
                        this.set("searchDisabledSources", list, cx);
                    })))
                    .child(div().ml(px(26.)).text_size(px(12.)).text_color(p.ink_faint).child(about)),
            );
        }
        let mut yours = div().mb(px(8.)).rounded(RADIUS).border_1().border_color(p.line).bg(p.bg);
        if s.search_custom_sources.is_empty() {
            yours = yours.child(div().px(px(10.)).py(px(10.)).text_color(p.ink_faint).child("None yet. Add any site whose search answers in JSON: paste its address, or let an AI assistant find it for you."));
        }
        let has_any = !s.search_custom_sources.is_empty();
        for (i, src) in s.search_custom_sources.iter().enumerate() {
            let host = src.url.split("://").nth(1).and_then(|r| r.split(['/', '?', '#']).next()).unwrap_or_default().to_string();
            let shared = src.to_shared();
            yours = yours.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .h(px(38.))
                    .px(px(10.))
                    .when(i > 0, |d| d.border_t_1().border_color(p.line))
                    .child(Checkbox::new(("custom", i)).checked(src.enabled).on_click(cx.listener(move |this, on: &bool, _, cx| {
                        let mut list = this.settings.search_custom_sources.clone();
                        if let Some(s) = list.get_mut(i) {
                            s.enabled = *on;
                        }
                        this.set_custom(list, cx);
                    })))
                    .child(div().max_w(px(260.)).min_w_0().truncate().child(src.name.clone()))
                    .child(div().flex_1().min_w_0().truncate().text_size(px(12.)).text_color(p.ink_faint).child(host))
                    .child(Button::new(("edit-src", i)).label("Edit").small().on_click(cx.listener(move |this, _, window, cx| this.open_editor(Some(i), window, cx))))
                    .child(Button::new(("copy-src", i)).label("Copy").small().tooltip("Copy its definition, to share or keep").on_click(cx.listener(move |this, _, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(shared.clone()));
                        this.flash = Some("Definition copied. Paste it into the answer box when adding a source.".into());
                        cx.notify();
                    })))
                    .child(Button::new(("remove-src", i)).label("Remove").small().on_click(cx.listener(move |this, _, _, cx| {
                        let mut list = this.settings.search_custom_sources.clone();
                        if i < list.len() {
                            let gone = list.remove(i);
                            this.flash = Some(format!("Removed \u{201c}{}\u{201d}.", gone.name));
                        }
                        this.set_custom(list, cx);
                    }))),
            );
        }
        let max = s.search_max_results;
        let counts = RESULT_COUNTS.iter().map(|n| (n.to_string(), n.to_string())).collect();
        let kind = if s.search_indexer_kind == "prowlarr" { "prowlarr" } else { "jackett" };
        let kinds = vec![("jackett".to_string(), "Jackett".to_string()), ("prowlarr".to_string(), "Prowlarr".to_string())];
        let mut panel = panel
            .child(legend(p, "Built in"))
            .child(built_in)
            .child(legend(p, "Your sources"))
            .child(yours.child({
                let hover = p.hover;
                div()
                    .id("add-src")
                    .debug_selector(|| "add-source".into())
                    .flex()
                    .items_center()
                    .h(px(36.))
                    .px(px(10.))
                    .border_t_1()
                    .border_color(p.line)
                    .text_color(p.accent)
                    .cursor_pointer()
                    .hover(move |s| s.bg(hover))
                    .child(if has_any { "+ Add another source" } else { "+ Add a source" })
                    .on_click(cx.listener(|this, _, window, cx| this.open_editor(None, window, cx)))
            }))
            .children(self.flash.clone().map(|f| div().text_size(px(12.)).text_color(p.ink_dim).child(f)))
            .child(div().mt(px(20.)).child(legend(p, "Results")))
            .child(frow(p, "Results per source:", div().w(px(100.)).child(dropdown(p, "max", max.to_string(), "max", counts, max.to_string(), &self.focus))))
            .child(div().child(Checkbox::new("hide-dead").label("Hide torrents with no seeders").checked(s.search_hide_dead).on_click(cx.listener(|this, on: &bool, _, cx| {
                this.set("searchHideDead", *on, cx);
            }))));
        if s.search_source_enabled("indexer") {
            panel = panel
                .child(div().mt(px(20.)).child(legend(p, "Your own search server")))
                .child(frow(p, "Server:", div().w(px(140.)).child(dropdown(p, "kind", if kind == "prowlarr" { "Prowlarr" } else { "Jackett" }.into(), "kind", kinds, kind.into(), &self.focus))))
                .child(frow(p, "Address:", Input::new(&self.indexer_url)))
                .child(frow(p, "API key:", Input::new(&self.indexer_key)))
                .child(hint(p, "Searches every site you have set up in your own Jackett or Prowlarr. The key is on its dashboard or under Settings → General."));
        }
        panel
    }

    /// The line under the search bar: where a search goes, and the way to add
    /// somewhere else.
    fn render_searching(&self, p: &Palette, cx: &mut Context<Self>) -> Div {
        let s = &self.settings;
        let mut names: Vec<String> = SEARCH_SOURCES
            .iter()
            // One switched on but not set up yet is not searched, so not named.
            .filter(|(id, _)| s.search_source_ready(id))
            .map(|(_, n)| n.to_string())
            .collect();
        names.extend(s.search_custom_sources.iter().filter(|c| c.enabled).map(|c| c.name.clone()));
        let what = if names.is_empty() { "No sources are on.".to_string() } else { format!("Searching {}", names.join(" · ")) };
        let accent = p.accent;
        div()
            .flex()
            .items_center()
            .gap(px(6.))
            .h(px(20.))
            .mt(px(6.))
            .text_size(px(12.))
            .child(div().min_w_0().truncate().text_color(p.ink_faint).child(what))
            .child(div().flex_none().text_color(p.ink_faint).child("·"))
            .child(
                div()
                    .id("add-source-link")
                    .flex_none()
                    .text_color(accent)
                    .cursor_pointer()
                    .hover(|s| s.underline())
                    .child("Add a source")
                    .on_click(cx.listener(|this, _, window, cx| this.open_editor(None, window, cx))),
            )
    }

    /// What the sheet says before the first search, when nothing but the
    /// Internet Archive would be searched.
    fn render_welcome(&self, p: &Palette, cx: &mut Context<Self>) -> Option<Div> {
        let s = &self.settings;
        let only_archive = s.search_custom_sources.iter().all(|c| !c.enabled) && !s.search_source_ready("indexer");
        if self.searched || self.show_settings || !only_archive {
            return None;
        }
        Some(
            div()
                .mt(px(12.))
                .p(px(14.))
                .rounded(RADIUS)
                .border_1()
                .border_color(p.line)
                .child(div().mb(px(6.)).font_weight(FontWeight::SEMIBOLD).child("Choose where to search"))
                .child(div().mb(px(10.)).text_color(p.ink_dim).child("Out of the box ztorrent searches the Internet Archive, a library of free, legal films, music, books and software. Add any other site you use by pasting its search address, or let an AI assistant find it for you."))
                .child(
                    div()
                        .flex()
                        .gap(px(8.))
                        .child(Button::new("welcome-add").label("Add a source…").small().primary().on_click(cx.listener(|this, _, window, cx| this.open_editor(None, window, cx))))
                        .child(Button::new("welcome-settings").label("Search settings").small().on_click(cx.listener(|this, _, _, cx| {
                            this.show_settings = true;
                            cx.notify();
                        }))),
                ),
        )
    }
}

impl Render for SearchSheet {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Theme::get(cx).clone();
        // What is left of the window once the dialog's own title, search row and
        // footer are drawn, shared between the settings panel and the results.
        // The sheet sits ~120px down the window; its title, search row, status
        // lines, table header and footer take ~340px more.
        let room = (f32::from(window.viewport_size().height) - 120. - 340. - 22. * self.notes.len() as f32).max(160.);
        let (panel_h, results_h) = if self.show_settings && !self.results.is_empty() { (room * 0.6, room * 0.4) } else { (room, room) };
        let status = if self.busy {
            Some("Searching…".to_string())
        } else if self.searched && self.results.is_empty() && self.notes.is_empty() {
            Some("Nothing found. Try fewer words, another category, or more sources.".to_string())
        } else if self.searched && !self.results.is_empty() {
            Some(format!("{} results", self.results.len()))
        } else {
            None
        };
        let mut list = div().flex().flex_col();
        for (i, r) in self.results.iter().enumerate() {
            let seeds = r.seeders.map(|n| n.to_string()).unwrap_or_else(|| "–".into());
            let peers = r.leechers.map(|n| n.to_string()).unwrap_or_else(|| "–".into());
            let full = format!("{}\n{}", r.title, r.source);
            list = list.child(
                div()
                    .id(("hit", i))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .min_h(px(34.))
                    .py(px(5.))
                    .px(px(10.))
                    .border_b_1()
                    .border_color(p.line)
                    .tooltip(move |window, cx| gpui_component::tooltip::Tooltip::new(full.clone()).build(window, cx))
                    // Long release names wrap onto a second line rather than vanish.
                    .child(div().flex_1().min_w_0().line_clamp(2).child(r.title.clone()))
                    .child(div().w(px(96.)).flex_none().truncate().text_color(p.ink_faint).child(r.source.clone()))
                    .child(div().w(px(76.)).flex_none().flex().justify_end().font_features(tabular()).child(fmt::bytes(r.size as f64, true)))
                    .child(div().w(px(48.)).flex_none().flex().justify_end().font_features(tabular()).child(seeds))
                    .child(div().w(px(48.)).flex_none().flex().justify_end().font_features(tabular()).text_color(p.ink_dim).child(peers))
                    .child(Button::new(("add", i)).label("Add").small().on_click(cx.listener(move |this, _, window, cx| {
                        let link = this.results[i].link.clone();
                        window.close_dialog(cx);
                        open_add(window, cx, TorrentSource::parse(&link), None);
                    }))),
            );
        }
        let (sort, descending) = (self.sort, self.descending);
        let heading = |text: &'static str, by: SortBy, w: Option<f32>, right: bool| {
            let arrow = if sort == by { if descending { " ▼" } else { " ▲" } } else { "" };
            let hover = p.ink;
            div()
                .id(text)
                .flex()
                .when(right, |d| d.justify_end())
                .map(|d| match w {
                    Some(w) => d.w(px(w)).flex_none(),
                    None => d.flex_1(),
                })
                .cursor_pointer()
                .hover(move |s| s.text_color(hover))
                .child(format!("{text}{arrow}"))
                .on_click(cx.listener(move |this, _, _, cx| this.sort_by(by, cx)))
        };
        let header = div()
            .flex()
            .items_center()
            .gap(px(10.))
            .h(px(26.))
            .px(px(10.))
            .border_b_1()
            .border_color(p.line)
            .text_size(px(11.))
            .text_color(p.ink_faint)
            .child(heading("NAME", SortBy::Name, None, false))
            .child(heading("SOURCE", SortBy::Source, Some(96.), false))
            .child(heading("SIZE", SortBy::Size, Some(76.), true))
            .child(heading("SEEDS", SortBy::Seeds, Some(48.), true))
            .child(div().w(px(48.)).flex_none().flex().justify_end().child("PEERS"))
            .child(div().w(px(52.)).flex_none());

        let category = self.settings.search_category.clone();
        let category_label = SEARCH_CATEGORIES.iter().find(|(id, _)| *id == category).map_or("All", |(_, n)| n).to_string();
        let categories = SEARCH_CATEGORIES.iter().map(|(id, n)| (id.to_string(), n.to_string())).collect();
        let settings_on = self.show_settings;
        let (gear_bg, gear_fg) = if settings_on { (p.accent_soft, p.accent) } else { (gpui::transparent_black(), p.ink_dim) };
        let hover_bg = p.hover;
        let welcome = self.render_welcome(&p, cx);
        let settings = settings_on.then(|| self.render_settings(&p, cx));

        div()
            .id("search-sheet")
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .on_action(cx.listener(|this, a: &PickOption, _, cx| match a.field.as_str() {
                "category" => this.set("searchCategory", a.value.clone(), cx),
                "kind" => this.set("searchIndexerKind", a.value.clone(), cx),
                "max" => {
                    if let Ok(n) = a.value.parse::<u64>() {
                        this.set("searchMaxResults", n, cx);
                    }
                }
                _ => {}
            }))
            .child(title(&p, "Search Online"))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(div().flex_1().child(Input::new(&self.query)))
                    .child(div().w(px(120.)).child(dropdown(&p, "category", category_label, "category", categories, category, &self.focus)))
                    .child(
                        div()
                            .id("search-settings")
                            .debug_selector(|| "search-settings".into())
                            .flex()
                            .items_center()
                            .justify_center()
                            .gap(px(6.))
                            .h(px(30.))
                            .px(px(10.))
                            .rounded(RADIUS)
                            .border_1()
                            .border_color(if settings_on { p.accent } else { p.line_hard })
                            .bg(gear_bg)
                            .text_color(gear_fg)
                            .hover(move |s| s.bg(hover_bg))
                            .child(icon("preferences", 15., gear_fg))
                            .child("Sources")
                            .on_click(cx.listener(|this, _, _, cx| {
                                if this.show_settings {
                                    this.save_indexer_fields(cx);
                                    this.editor = None;
                                }
                                this.show_settings = !this.show_settings;
                                cx.notify();
                            })),
                    )
                    .child(div().debug_selector(|| "search-go".into()).child(Button::new("go").label("Search").primary().disabled(self.busy).on_click(cx.listener(|this, _, window, cx| this.run(window, cx))))),
            )
            .child(self.render_searching(&p, cx))
            .child(div().id("search-settings-panel").max_h(px(panel_h)).overflow_y_scroll().children(settings))
            .children(welcome)
            .children(status.map(|s| div().mt(px(10.)).text_color(p.ink_dim).child(s)))
            .children(self.notes.iter().map(|n| div().mt(px(6.)).text_color(p.warn).child(n.clone())))
            .children((!self.results.is_empty()).then(|| {
                div()
                    .mt(px(10.))
                    .border_1()
                    .border_color(p.line)
                    .rounded(RADIUS)
                    .child(header)
                    .child(div().id("search-results").max_h(px(results_h)).overflow_y_scroll().child(list))
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(16.))
                    .mt(px(16.))
                    .pt(px(14.))
                    .border_t_1()
                    .border_color(p.line)
                    .child(div().flex_1().min_w_0().text_size(px(11.)).text_color(p.ink_faint).child(RESPONSIBILITY))
                    .child(Button::new("close").label("Close").on_click(|_, window, cx| window.close_dialog(cx))),
            )
    }
}

// --------------------------------------------------------------- Create

const PIECE_SIZES: [(&str, u32); 12] = [
    ("Auto", 0),
    ("16 kB", 16384),
    ("32 kB", 32768),
    ("64 kB", 65536),
    ("128 kB", 131072),
    ("256 kB", 262144),
    ("512 kB", 524288),
    ("1 MB", 1048576),
    ("2 MB", 2097152),
    ("4 MB", 4194304),
    ("8 MB", 8388608),
    ("16 MB", 16777216),
];

const DEFAULT_TRACKERS: &str = "udp://tracker.opentrackr.org:1337/announce\nudp://open.demonii.com:1337/announce\nudp://tracker.torrent.eu.org:451/announce";

pub struct CreateSheet {
    src: Entity<InputState>,
    trackers: Entity<TextareaState>,
    web_seeds: Entity<TextareaState>,
    comment: Entity<InputState>,
    piece: u32,
    private: bool,
    seed: bool,
    error: Option<String>,
    status: Option<String>,
    busy: bool,
    focus: FocusHandle,
}

pub fn open_create<V: 'static>(window: &mut Window, cx: &mut Context<V>) {
    let src = text_input(window, cx, "", "Choose a file or folder to share");
    let trackers = cx.new(|cx| {
        let mut s = TextareaState::new(window, cx).rows(4);
        s.set_value(DEFAULT_TRACKERS, window, cx);
        s
    });
    let web_seeds = cx.new(|cx| TextareaState::new(window, cx).rows(2));
    let comment = text_input(window, cx, "", "");
    let sheet = cx.new(|cx| CreateSheet { src, trackers, web_seeds, comment, piece: 0, private: false, seed: true, error: None, status: None, busy: false, focus: cx.focus_handle() });
    let target = sheet.clone();
    open(window, cx, 560., sheet, Confirm::With(Rc::new(move |window, cx| target.update(cx, |s, cx| s.confirm(window, cx)))));
}

/// Tracker lines as tiers: one URL per line, a blank line starts the next tier --
/// what the sheet has always said it does.
pub fn tracker_tiers(text: &str) -> Vec<Vec<String>> {
    let mut tiers = vec![Vec::new()];
    for line in text.lines().map(str::trim) {
        if line.is_empty() {
            if !tiers.last().unwrap().is_empty() {
                tiers.push(Vec::new());
            }
        } else {
            tiers.last_mut().unwrap().push(line.to_string());
        }
    }
    tiers.retain(|t| !t.is_empty());
    tiers
}

impl CreateSheet {
    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input = read(&self.src, cx).trim().to_string();
        if input.is_empty() {
            self.error = Some("Choose a file or folder first.".into());
            return cx.notify();
        }
        let input_path = PathBuf::from(&input);
        let name = input_path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "torrent".into());
        let dialog = rfd::AsyncFileDialog::new().set_file_name(format!("{name}.torrent")).add_filter("Torrent Files", &["torrent"]);
        let sheet = cx.entity();
        pick(window, cx, Pick::Save, dialog, move |paths, window, cx| {
            let dest = paths[0].clone();
            sheet.update(cx, |this, cx| this.create(input_path, name, dest, window, cx));
        });
    }

    fn create(&mut self, input_path: PathBuf, name: String, dest: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let options = CreateTorrentOptions {
            input_path,
            output_path: dest,
            name,
            comment: read(&self.comment, cx).trim().to_string(),
            trackers: tracker_tiers(&self.trackers.read(cx).value()),
            web_seeds: self.web_seeds.read(cx).value().lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect(),
            piece_length: self.piece,
            private: self.private,
            start_seeding: self.seed,
        };
        let (progress_tx, mut progress_rx) = futures_channel::mpsc::unbounded::<f32>();
        let rx = ask(cx, |reply| Command::CreateTorrent { options, progress: Some(progress_tx), reply });
        self.busy = true;
        self.error = None;
        self.status = Some("Hashing… this can take a moment for large folders.".into());
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            use futures_channel::mpsc::UnboundedReceiver;
            async fn next(rx: &mut UnboundedReceiver<f32>) -> Option<f32> {
                std::future::poll_fn(|c| {
                    use std::pin::Pin;
                    futures_core_poll(Pin::new(rx), c)
                })
                .await
            }
            fn futures_core_poll(rx: std::pin::Pin<&mut UnboundedReceiver<f32>>, c: &mut std::task::Context) -> std::task::Poll<Option<f32>> {
                use futures_core::Stream;
                rx.poll_next(c)
            }
            while let Some(fraction) = next(&mut progress_rx).await {
                let _ = this.update(cx, |this, cx| {
                    this.status = Some(format!("Hashing… {:.0}%", fraction * 100.0));
                    cx.notify();
                });
            }
            let result = rx.await.unwrap_or_else(|_| Err("the engine is not running".into()));
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(_) => window.close_dialog(cx),
                    Err(e) => {
                        this.status = None;
                        this.error = Some(e);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}

impl Render for CreateSheet {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Theme::get(cx).clone();
        let src = self.src.clone();
        let src2 = self.src.clone();
        let piece_label = PIECE_SIZES.iter().find(|(_, v)| *v == self.piece).map(|(l, _)| l.to_string()).unwrap_or_default();
        let options = PIECE_SIZES.iter().map(|(l, v)| (v.to_string(), l.to_string())).collect();
        div()
            .id("create-sheet")
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .on_action(cx.listener(|this, a: &PickOption, _, cx| {
                if a.field == "piece" {
                    this.piece = a.value.parse().unwrap_or(0);
                    cx.notify();
                }
            }))
            .child(title(&p, "Create New Torrent"))
            .child(legend(&p, "Select Source"))
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .mb(px(16.))
                    .child(div().flex_1().child(Input::new(&self.src)))
                    .child(Button::new("pick-file").label("Add File…").small().on_click(move |_, window, cx: &mut App| {
                        let start = read(&src, cx);
                        let src = src.clone();
                        pick(window, cx, Pick::File, panel("Choose a File to Share", &start), move |paths, window, cx| {
                            let path = paths[0].to_string_lossy().into_owned();
                            src.update(cx, |s, cx| s.set_value(path, window, cx));
                        });
                    }))
                    .child(Button::new("pick-dir").label("Add Folder…").small().on_click(move |_, window, cx: &mut App| {
                        browse_into(window, cx, "Choose a Folder to Share", src2.clone())
                    })),
            )
            .child(legend(&p, "Torrent Properties"))
            .child(div().mb(px(4.)).text_color(p.ink_dim).child("Trackers (one per line, blank line separates tiers):"))
            .child(div().mb(px(8.)).font_family(MONO).child(Textarea::new(&self.trackers).h(px(96.))))
            .child(div().mb(px(4.)).text_color(p.ink_dim).child("Web seeds (optional, one URL per line):"))
            .child(div().mb(px(8.)).font_family(MONO).child(Textarea::new(&self.web_seeds).h(px(52.))))
            .child(frow(&p, "Comment:", Input::new(&self.comment)))
            .child(frow(&p, "Piece size:", div().w(px(140.)).child(dropdown(&p, "piece", piece_label, "piece", options, self.piece.to_string(), &self.focus))))
            .child(
                div()
                    .flex()
                    .gap(px(16.))
                    .child(Checkbox::new("private").label("Private torrent (disable DHT and PEX)").checked(self.private).on_click(cx.listener(|this, on: &bool, _, cx| {
                        this.private = *on;
                        cx.notify();
                    })))
                    .child(Checkbox::new("seed").label("Start seeding").checked(self.seed).on_click(cx.listener(|this, on: &bool, _, cx| {
                        this.seed = *on;
                        cx.notify();
                    }))),
            )
            .children(error_line(&p, &self.error))
            .children(self.status.clone().map(|s| div().mt(px(6.)).text_color(p.ink_dim).child(s)))
            .child(
                footer(&p, None)
                    .child(Button::new("cancel").label("Cancel").on_click(|_, window, cx| window.close_dialog(cx)))
                    .child(div().debug_selector(|| "create-ok".into()).child(Button::new("ok").label("Create and Save As…").primary().disabled(self.busy).on_click(cx.listener(|this, _, window, cx| this.confirm(window, cx))))),
            )
    }
}

// ----------------------------------------------------------- Preferences

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    General,
    Dirs,
    Connection,
    Bandwidth,
    Queueing,
    Appearance,
}

const PAGES: [(Page, &str, &str); 6] = [
    (Page::General, "General", "preferences"),
    (Page::Dirs, "Directories", "folder"),
    (Page::Connection, "Connection", "tracker"),
    (Page::Bandwidth, "Bandwidth", "speed"),
    (Page::Queueing, "Queueing", "files"),
    (Page::Appearance, "Appearance", "info"),
];

/// Text and number fields, by their camelCase settings key.
const TEXT_KEYS: [&str; 4] = ["downloadPath", "proxyHost", "proxyUsername", "proxyPassword"];
const NUMBER_KEYS: [(&str, u64); 12] = [
    ("listenPort", 0),
    ("proxyPort", 1),
    ("maxDownloadRate", 0),
    ("maxUploadRate", 0),
    ("altDownloadRate", 0),
    ("altUploadRate", 0),
    ("globalMaxConnections", 10),
    ("maxUploadSlots", 1),
    ("maxActiveTorrents", 1),
    ("maxActiveDownloads", 1),
    ("seedRatioLimit", 0),
    ("seedTimeLimit", 0),
];

pub struct PrefsSheet {
    page: Page,
    original: Settings,
    draft: serde_json::Map<String, serde_json::Value>,
    inputs: BTreeMap<&'static str, Entity<InputState>>,
    interfaces: Vec<NetInterface>,
    applied: bool,
    focus: FocusHandle,
}

pub fn open_preferences<V: 'static>(settings: Settings, window: &mut Window, cx: &mut Context<V>) {
    let draft = match serde_json::to_value(&settings) {
        Ok(serde_json::Value::Object(m)) => m,
        _ => Default::default(),
    };
    let mut inputs = BTreeMap::new();
    for key in TEXT_KEYS {
        let value = draft.get(key).and_then(|v| v.as_str()).unwrap_or_default().to_string();
        let input = cx.new(|cx| {
            let mut s = InputState::new(window, cx);
            if key == "proxyPassword" {
                s = s.masked(true);
            }
            if key == "proxyHost" {
                s = s.placeholder("127.0.0.1");
            }
            if key == "proxyUsername" {
                s = s.placeholder("(optional)");
            }
            s.set_value(value, window, cx);
            s
        });
        inputs.insert(key, input);
    }
    for (key, _) in NUMBER_KEYS {
        let value = draft.get(key).map(|v| v.to_string()).unwrap_or_default();
        inputs.insert(key, text_input(window, cx, &value, ""));
    }
    let sheet = cx.new(|cx| PrefsSheet { page: Page::General, original: settings, draft, inputs, interfaces: Vec::new(), applied: false, focus: cx.focus_handle() });
    let rx = ask(cx, Command::Interfaces);
    let weak = sheet.downgrade();
    cx.spawn(async move |_, cx| {
        if let Ok(list) = rx.await {
            let _ = weak.update(cx, |this, cx| {
                this.interfaces = list;
                cx.notify();
            });
        }
    })
    .detach();
    let revert = sheet.downgrade();
    window.open_dialog(cx, move |dialog, _, _| {
        let revert = revert.clone();
        let apply = sheet.clone();
        dialog
            .w(px(620.))
            .close_button(false)
            .child(sheet.clone())
            .on_ok(move |_, window, cx| {
                let apply = apply.clone();
                window.defer(cx, move |window, cx| apply.update(cx, |s, cx| s.apply(window, cx)));
                false
            })
            .on_close(move |_, _, cx| {
            // The theme was previewed live; Cancel puts back the saved one --
            // as it is now, since Cmd+L may have changed it under the sheet.
            if revert.upgrade().is_some_and(|s| !s.read(cx).applied) {
                let saved = cx.global::<Library>().settings.clone();
                crate::apply_theme(&saved, cx);
            }
        })
    });
}

impl PrefsSheet {
    fn flag(&self, key: &str) -> bool {
        self.draft.get(key).and_then(|v| v.as_bool()).unwrap_or(false)
    }

    fn string(&self, key: &str) -> String {
        self.draft.get(key).and_then(|v| v.as_str()).unwrap_or_default().to_string()
    }

    fn apply(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut patch: SettingsPatch = self.draft.clone();
        for key in TEXT_KEYS {
            patch.insert(key.into(), read(&self.inputs[key], cx).into());
        }
        for (key, min) in NUMBER_KEYS {
            let n = read(&self.inputs[key], cx).trim().parse::<f64>().ok().filter(|f| f.is_finite() && *f >= 0.0).map(|f| f as u64).unwrap_or(0);
            patch.insert(key.into(), n.max(min).into());
        }
        patch.remove("proxyPasswordEnc");
        // Only what was changed here: the rest of the draft is the settings as
        // they were when the sheet opened, and saving it would undo whatever
        // changed since -- Cmd+Shift+L's alternate speeds, Cmd+L's theme.
        let original = match serde_json::to_value(&self.original) {
            Ok(serde_json::Value::Object(m)) => m,
            _ => Default::default(),
        };
        patch.retain(|key, value| match (original.get(key), value) {
            // 2 typed into a field is 2.0 in a float setting.
            (Some(a), b) if a.is_number() && b.is_number() => a.as_f64() != b.as_f64(),
            (a, b) => a != Some(b),
        });
        self.applied = true;
        if !patch.is_empty() {
            send(cx, Command::SetSettings(patch));
        }
        window.close_dialog(cx);
    }

    fn check(&self, cx: &mut Context<Self>, key: &'static str, label: &str, disabled: bool) -> impl IntoElement {
        div().mb(px(10.)).when(disabled, |d| d.opacity(0.45)).child(
            Checkbox::new(key).label(label.to_string()).checked(self.flag(key)).disabled(disabled).on_click(cx.listener(move |this, on: &bool, _, cx| {
                this.draft.insert(key.into(), (*on).into());
                cx.notify();
            })),
        )
    }

    fn number(&self, p: &Palette, key: &'static str, label: &str, suffix: &str) -> impl IntoElement {
        frow(p, label, div().flex().items_center().gap(px(8.)).child(div().w(px(92.)).debug_selector(move || format!("field-{key}")).child(Input::new(&self.inputs[key]))).child(div().text_color(p.ink_dim).child(suffix.to_string())))
    }
}

impl Render for PrefsSheet {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Theme::get(cx).clone();
        let mut nav = div().flex().flex_col().gap(px(1.)).w(px(184.)).flex_none().py(px(4.)).pr(px(12.)).border_r_1().border_color(p.line);
        for (page, label, ic) in PAGES {
            let active = self.page == page;
            nav = nav.child(
                div()
                    .id(label)
                    .debug_selector(move || format!("nav-{label}"))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .h(px(30.))
                    .px(px(10.))
                    .rounded(RADIUS)
                    .when(active, |d| d.bg(p.sel_quiet).font_weight(FontWeight::MEDIUM))
                    .child(icon(ic, 16., if active { p.accent } else { p.ink_dim }))
                    .child(label)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.page = page;
                        cx.notify();
                    })),
            );
        }

        let proxied = self.flag("proxyEnabled");
        let bind = self.string("bindInterface");
        let off = suppressed_in_sheet(proxied, &bind);

        let body: AnyElement = match self.page {
            Page::General => div()
                .child(legend(&p, "When adding torrents"))
                .child(self.check(cx, "startTorrentsAutomatically", "Start torrents automatically", false))
                .child(self.check(cx, "askWhereToSave", "Show the Add Torrent dialog", false))
                .child(self.check(cx, "sequentialDownload", "Download pieces in order by default", false))
                .child(self.check(cx, "partFiles", "Append .part to incomplete files", false))
                .child(div().mt(px(-6.)).mb(px(14.)).text_size(px(12.)).text_color(p.ink_faint).child("Renamed once the torrent finishes, so unfinished downloads are never mistaken for complete ones."))
                .child(legend(&p, "Notifications"))
                .child(self.check(cx, "notifyOnComplete", "Show a notification when a download finishes", false))
                .child(self.check(cx, "confirmOnDelete", "Confirm before removing a torrent", false))
                .child(self.check(cx, "showSpeedInDock", "Show download progress on the Dock icon", false))
                .child(legend(&p, "Updates"))
                .child(self.check(cx, "autoUpdate", "Check for new versions automatically", false))
                .child(div().mt(px(-6.)).text_size(px(12.)).text_color(p.ink_faint).child("Downloads the new build in the background and offers a restart when it is ready. Nothing is installed until you say so."))
                .into_any_element(),
            Page::Dirs => div()
                .child(legend(&p, "Location of downloaded files"))
                .child(div().mb(px(6.)).text_color(p.ink_dim).child("Put new downloads in:"))
                .child(div().flex().gap(px(8.)).child(div().flex_1().child(Input::new(&self.inputs["downloadPath"]))).child(browse_button("browse-dl", "Choose Download Folder", self.inputs["downloadPath"].clone())))
                .into_any_element(),
            Page::Connection => {
                let mut ifaces = vec![(String::new(), "Any (follow the routing table)".to_string())];
                ifaces.extend(self.interfaces.iter().map(|i| (i.name.clone(), format!("{} — {}", i.name, i.address))));
                if !bind.is_empty() && !self.interfaces.iter().any(|i| i.name == bind) {
                    ifaces.push((bind.clone(), format!("{bind} — not present")));
                }
                let bind_label = ifaces.iter().find(|(v, _)| *v == bind).map(|(_, l)| l.clone()).unwrap_or_default();
                let enc = self.draft.get("encryption").and_then(|v| v.as_u64()).unwrap_or(1);
                let enc_options = vec![
                    ("0".to_string(), "Disabled — plaintext handshakes".to_string()),
                    ("1".to_string(), "Enabled — encrypt when the peer supports it".to_string()),
                    ("2".to_string(), "Required — refuse peers that will not encrypt".to_string()),
                ];
                let enc_label = enc_options[enc.min(2) as usize].1.clone();
                div()
                    .child(legend(&p, "Listening port"))
                    .child(self.check(cx, "randomizePort", "Randomize the port each time ztorrent starts", false))
                    .child(self.number(&p, "listenPort", "Port used for incoming connections:", ""))
                    .child(self.check(cx, "enableUPnP", "Map the port with UPnP / NAT-PMP", off.upnp))
                    .child(div().mt(px(8.)).child(legend(&p, "Proxy")))
                    .child(self.check(cx, "proxyEnabled", "Route traffic through a SOCKS5 proxy", false))
                    .child(frow(&p, "Proxy host:", Input::new(&self.inputs["proxyHost"])))
                    .child(self.number(&p, "proxyPort", "Proxy port:", ""))
                    .child(frow(&p, "Username:", Input::new(&self.inputs["proxyUsername"])))
                    .child(frow(&p, "Password:", Input::new(&self.inputs["proxyPassword"])))
                    .child(hint(&p, "Covers tracker announces, web seeds and outgoing peer connections. Anything that cannot be routed is switched off rather than sent around the proxy: DHT, local discovery, µTP, port mapping and udp:// trackers all stop while this is on. Takes effect after a restart."))
                    .child(div().mt(px(8.)).child(legend(&p, "Network interface")))
                    .child(frow(&p, "Send traffic from:", dropdown(&p, "bind", bind_label, "bindInterface", ifaces, bind.clone(), &self.focus)))
                    .child(hint(&p, "Pin every outgoing connection to one interface — a VPN's, typically. If it goes away, connections fail instead of falling back to your normal one, and resume by themselves when it returns. Local discovery, µTP, port mapping and udp:// trackers stop while this is set; DHT keeps working, bound to the same interface. Takes effect after a restart."))
                    .child(div().mt(px(8.)).child(legend(&p, "Protocol encryption")))
                    .child(frow(&p, "Peer connections:", dropdown(&p, "enc", enc_label, "encryption", enc_options, enc.to_string(), &self.focus)))
                    .child(hint(&p, "Hides the handshake from traffic inspection. It does not hide tracker or DHT activity, and your address is still public to the swarm."))
                    .child(div().mt(px(8.)).child(legend(&p, "Peer discovery")))
                    .child(self.check(cx, "enableDHT", "Enable DHT (distributed hash table)", off.dht))
                    .child(self.check(cx, "enablePEX", "Enable peer exchange", false))
                    .child(self.check(cx, "enableLSD", "Enable local peer discovery", off.lsd))
                    .child(hint(&p, "Local discovery broadcasts each torrent's infohash in the clear to every device on your network, and only finds peers on that same network. Off by default."))
                    .child(self.check(cx, "enableUTP", "Enable µTP (micro transport protocol)", off.utp))
                    .child(hint(&p, "Discovery changes take effect after a restart."))
                    .into_any_element()
            }
            Page::Bandwidth => div()
                .child(legend(&p, "Global rate limits"))
                .child(self.number(&p, "maxDownloadRate", "Maximum download rate:", "kB/s  (0 = unlimited)"))
                .child(self.number(&p, "maxUploadRate", "Maximum upload rate:", "kB/s  (0 = unlimited)"))
                .child(div().mt(px(8.)).child(legend(&p, "Alternate rate limits")))
                .child(self.number(&p, "altDownloadRate", "Alternate download rate:", "kB/s  (0 = unlimited)"))
                .child(self.number(&p, "altUploadRate", "Alternate upload rate:", "kB/s  (0 = unlimited)"))
                .child(self.check(cx, "altSpeedEnabled", "Use alternate limits now", false))
                .child(div().mt(px(8.)).child(legend(&p, "Number of connections")))
                .child(self.number(&p, "globalMaxConnections", "Global maximum connections:", "shared out between active torrents"))
                .child(self.number(&p, "maxUploadSlots", "Upload slots per torrent:", ""))
                .into_any_element(),
            Page::Queueing => div()
                .child(legend(&p, "Queue settings"))
                .child(self.number(&p, "maxActiveTorrents", "Maximum active torrents:", ""))
                .child(self.number(&p, "maxActiveDownloads", "Maximum active downloads:", ""))
                .child(div().mt(px(8.)).child(legend(&p, "Seeding goal")))
                .child(self.number(&p, "seedRatioLimit", "Seed until ratio reaches:", "%  (0 = forever)"))
                .child(self.number(&p, "seedTimeLimit", "Seed for at least:", "minutes  (0 = forever)"))
                .into_any_element(),
            Page::Appearance => {
                let theme = self.string("theme");
                let options = vec![("classic".to_string(), "Light".to_string()), ("graphite".to_string(), "Dark".to_string())];
                let label = if theme == "graphite" { "Dark" } else { "Light" }.to_string();
                div().child(legend(&p, "Theme")).child(frow(&p, "Appearance:", div().w(px(160.)).child(dropdown(&p, "theme", label, "theme", options, theme, &self.focus)))).into_any_element()
            }
        };

        div()
            .id("prefs")
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .on_action(cx.listener(|this, a: &PickOption, _, cx| {
                match a.field.as_str() {
                    "encryption" => {
                        this.draft.insert("encryption".into(), a.value.parse::<u64>().unwrap_or(1).into());
                    }
                    "bindInterface" => {
                        this.draft.insert("bindInterface".into(), a.value.clone().into());
                    }
                    "theme" => {
                        this.draft.insert("theme".into(), a.value.clone().into());
                        // Live preview while the sheet is open.
                        let mut preview = this.original.clone();
                        preview.theme = a.value.clone();
                        crate::apply_theme(&preview, cx);
                    }
                    _ => {}
                }
                cx.notify();
            }))
            .child(title(&p, "Preferences"))
            .child(
                div()
                    .flex()
                    .min_h(px(400.))
                    .child(nav)
                    .child(div().id("prefs-page").flex_1().min_w_0().max_h(px(520.)).overflow_y_scroll().pl(px(20.)).child(body)),
            )
            .child(
                footer(&p, None)
                    .child(Button::new("cancel").label("Cancel").on_click(|_, window, cx| window.close_dialog(cx)))
                    .child(div().debug_selector(|| "prefs-apply".into()).child(Button::new("ok").label("Apply").primary().on_click(cx.listener(|this, _, window, cx| this.apply(window, cx))))),
            )
    }
}

// ------------------------------------------------------------ Properties

pub struct PropertiesSheet {
    row: Row,
    details: Option<Details>,
}

pub fn open_properties<V: 'static>(row: Row, details: Option<Details>, window: &mut Window, cx: &mut Context<V>) {
    let sheet = cx.new(|_| PropertiesSheet { row, details });
    open(window, cx, 520., sheet, Confirm::Close);
}

impl Render for PropertiesSheet {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Theme::get(cx).clone();
        let r = &self.row;
        let d = self.details.as_ref();
        let dash = |s: String| if s.is_empty() { "—".to_string() } else { s };
        let lines: Vec<(&str, String)> = vec![
            ("Name", r.name.clone()),
            ("Info Hash", r.info_hash.clone()),
            ("Save Path", r.save_path.clone()),
            ("Total Size", fmt::bytes(r.size as f64, false)),
            ("Selected Size", fmt::bytes(r.wanted_size as f64, false)),
            ("Pieces", d.map(|d| format!("{} × {}", d.piece_count, fmt::bytes(d.piece_length as f64, false))).unwrap_or("—".into())),
            ("Private", d.map(|d| if d.private { "Yes" } else { "No" }.to_string()).unwrap_or("—".into())),
            ("Comment", dash(d.map(|d| d.comment.clone()).unwrap_or_default())),
            ("Created By", dash(d.map(|d| d.created_by.clone()).unwrap_or_default())),
            ("Created On", d.filter(|d| d.created_on > 0).map(|d| fmt::datetime(d.created_on)).unwrap_or("—".into())),
            ("Added On", fmt::datetime(r.added_on)),
            ("Completed On", if r.completed_on > 0 { fmt::datetime(r.completed_on) } else { "—".into() }),
            ("Downloaded", fmt::bytes(r.downloaded as f64, false)),
            ("Uploaded", fmt::bytes(r.uploaded as f64, false)),
            ("Ratio", fmt::ratio(r.ratio)),
            ("Label", dash(r.label.clone())),
            ("Magnet URI", r.magnet_uri.clone().unwrap_or("—".into())),
        ];
        let mut grid = div().flex().flex_col().gap(px(7.));
        for (k, v) in lines {
            grid = grid.child(
                div()
                    .flex()
                    .gap(px(12.))
                    .child(div().w(px(130.)).flex_none().text_color(p.ink_dim).child(k))
                    .child(div().flex_1().min_w_0().font_family(MONO).text_size(px(12.)).overflow_hidden().child(v)),
            );
        }
        div()
            .flex()
            .flex_col()
            .child(title(&p, format!("Properties — {}", r.name)))
            .child(grid)
            .child(footer(&p, None).child(div().debug_selector(|| "close".into()).child(Button::new("close").label("Close").on_click(|_, window, cx| window.close_dialog(cx)))))
    }
}

// --------------------------------------------------------- Customize Label

pub struct LabelStyleSheet {
    name: String,
    symbol: String,
    color: String,
}

pub fn open_label_style<V: 'static>(name: String, current: Option<LabelStyle>, window: &mut Window, cx: &mut Context<V>) {
    let (symbol, color) = tag_style(current.as_ref());
    let sheet = cx.new(|_| LabelStyleSheet { name, symbol: symbol.into(), color: color.into() });
    let target = sheet.clone();
    open(window, cx, 400., sheet, Confirm::With(Rc::new(move |window, cx| target.update(cx, |s, cx| s.ok(window, cx)))));
}

impl LabelStyleSheet {
    fn ok(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        send(cx, Command::SetLabelStyle { name: self.name.clone(), style: Some(LabelStyle { symbol: self.symbol.clone(), color: self.color.clone() }) });
        window.close_dialog(cx);
    }
}

impl Render for LabelStyleSheet {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Theme::get(cx).clone();
        let colors = color_row(&p, &self.color, cx, |this: &mut Self, c| this.color = c.into());
        let symbols = symbol_grid(&p, &self.symbol, cx, |this: &mut Self, s| this.symbol = s.into());
        let small = |t: &str| small_heading(&p, t);
        div()
            .flex()
            .flex_col()
            .child(title(&p, "Customize Label"))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .h(px(34.))
                    .px(px(10.))
                    .mb(px(14.))
                    .rounded(RADIUS)
                    .bg(p.sunken)
                    .font_weight(FontWeight::MEDIUM)
                    .child(icon(&self.symbol, 16., p.tag(&self.color)))
                    .child(self.name.clone()),
            )
            .child(small("COLOR"))
            .child(colors)
            .child(small("SYMBOL"))
            .child(symbols)
            .child(
                footer(&p, None)
                    .child(Button::new("cancel").label("Cancel").on_click(|_, window, cx| window.close_dialog(cx)))
                    .child(div().debug_selector(|| "label-ok".into()).child(Button::new("ok").label("OK").primary().on_click(cx.listener(|this, _, window, cx| this.ok(window, cx))))),
            )
    }
}

fn small_heading(p: &Palette, text: &str) -> Div {
    div().mb(px(8.)).text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).text_color(p.ink_faint).child(text.to_string())
}

/// The label hues as a row of swatches, `selected` ringed; a click hands the
/// hue to `set`.
fn color_row<T: 'static>(p: &Palette, selected: &str, cx: &mut Context<T>, set: fn(&mut T, &'static str)) -> Div {
    let mut colors = div().flex().justify_between().mb(px(18.));
    for c in TAG_COLORS {
        let on = selected == c;
        let swatch = div().id(c).debug_selector(move || format!("color-{c}")).size(px(26.)).rounded_full();
        // Ink follows the theme, so its swatch shows both faces, dark | light.
        // Drawn in layers rather than with a border, which would take room from
        // the halves and leave it smaller than its neighbours. The half that
        // matches the sheet gets a hairline edge, or it melts into the sheet
        // and reads smaller than the other.
        let swatch = if c == "ink" {
            let (dark, light) = Palette::ink_pair();
            let layer = |name: &str, color: Hsla| icon(name, 26., color).absolute().top_0().left_0();
            let edge = if p.ink == dark { "arc-right" } else { "arc-left" };
            swatch
                .relative()
                .child(layer("half-left", dark))
                .child(layer("half-right", light))
                .child(if on { layer("ring", p.accent) } else { layer(edge, p.line_hard) })
        } else {
            swatch.bg(p.tag(c)).when(on, |d| d.border_2().border_color(p.accent))
        };
        colors = colors.child(
            swatch
                .hover(|s| s.opacity(0.85))
                .on_click(cx.listener(move |this, _, _, cx| {
                    set(this, c);
                    cx.notify();
                })),
        );
    }
    colors
}

/// The label symbols as rows of tiles that share the width, `selected`
/// outlined; a click hands the symbol to `set`.
fn symbol_grid<T: 'static>(p: &Palette, selected: &str, cx: &mut Context<T>, set: fn(&mut T, &'static str)) -> Div {
    let mut grid = div().flex().flex_col().gap(px(6.));
    for chunk in TAG_SYMBOLS.chunks(SYMBOLS_PER_ROW) {
        let mut row = div().flex().gap(px(6.));
        for &s in chunk {
            let on = selected == s;
            row = row.child(
                div()
                    .id(s)
                    .debug_selector(move || format!("symbol-{s}"))
                    .flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .h(px(36.))
                    .rounded(RADIUS)
                    .bg(if on { p.accent_soft } else { p.sunken })
                    .border_1()
                    .border_color(if on { p.accent } else { gpui::transparent_black() })
                    .child(icon(s, 18., p.ink))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        set(this, s);
                        cx.notify();
                    })),
            );
        }
        grid = grid.child(row);
    }
    grid
}

const SYMBOLS_PER_ROW: usize = 8;

// --------------------------------------------------------------- New Label

pub struct NewLabelSheet {
    input: Entity<InputState>,
    symbol: String,
    color: String,
    on_ok: Rc<dyn Fn(String, LabelStyle, &mut App)>,
}

/// Asks for a new label's name, colour and symbol.
pub fn open_new_label<V: 'static>(window: &mut Window, cx: &mut Context<V>, ok: impl Fn(String, LabelStyle, &mut App) + 'static) {
    let input = text_input(window, cx, "", "");
    let (symbol, color) = tag_style(None);
    let sheet = cx.new(|_| NewLabelSheet { input: input.clone(), symbol: symbol.into(), color: color.into(), on_ok: Rc::new(ok) });
    let target = sheet.clone();
    open(window, cx, 400., sheet, Confirm::With(Rc::new(move |window, cx| target.update(cx, |s, cx| s.submit(window, cx)))));
    input.update(cx, |s, cx| s.focus(window, cx));
}

impl NewLabelSheet {
    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = read(&self.input, cx).trim().to_string();
        if name.is_empty() {
            return;
        }
        (self.on_ok)(name, LabelStyle { symbol: self.symbol.clone(), color: self.color.clone() }, cx);
        window.close_dialog(cx);
    }
}

impl Render for NewLabelSheet {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Theme::get(cx).clone();
        let colors = color_row(&p, &self.color, cx, |this: &mut Self, c| this.color = c.into());
        let symbols = symbol_grid(&p, &self.symbol, cx, |this: &mut Self, s| this.symbol = s.into());
        div()
            .flex()
            .flex_col()
            .child(title(&p, "New Label"))
            .child(small_heading(&p, "NAME"))
            .child(div().mb(px(18.)).child(Input::new(&self.input).prefix(icon(&self.symbol, 16., p.tag(&self.color)))))
            .child(small_heading(&p, "COLOR"))
            .child(colors)
            .child(small_heading(&p, "SYMBOL"))
            .child(symbols)
            .child(
                footer(&p, None)
                    .child(Button::new("cancel").label("Cancel").on_click(|_, window, cx| window.close_dialog(cx)))
                    .child(div().debug_selector(|| "new-label-ok".into()).child(Button::new("ok").label("OK").primary().on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))))),
            )
    }
}

/// gpui-component's Root leaves its layers to the view it wraps: dialogs,
/// sheets and notifications are drawn here, over everything else in the window.
pub fn render_layer<V: 'static>(window: &mut Window, cx: &mut Context<V>) -> impl IntoElement {
    let _ = accel("", false);
    div()
        .children(gpui_component::Root::render_sheet_layer(window, cx))
        .children(gpui_component::Root::render_dialog_layer(window, cx))
        .children(gpui_component::Root::render_notification_layer(window, cx))
}

#[cfg(test)]
mod tests {
    use super::tracker_tiers;

    #[test]
    fn blank_lines_separate_tiers() {
        let t = tracker_tiers("a\nb\n\n\nc\n");
        assert_eq!(t, vec![vec!["a".to_string(), "b".into()], vec!["c".into()]]);
        assert!(tracker_tiers("\n\n").is_empty());
    }
}
