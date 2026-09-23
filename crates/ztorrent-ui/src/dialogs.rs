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
use gpui_component::input::{Input, InputState, Textarea, TextareaState};
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
    crate::debug(format!("open dialog ({width}px)"));
    let confirm = Rc::new(confirm);
    window.open_dialog(cx, move |dialog, _, _| {
        let confirm = confirm.clone();
        dialog.w(px(width)).close_button(false).child(view.clone()).on_ok(move |_, window, cx| match confirm.as_ref() {
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
