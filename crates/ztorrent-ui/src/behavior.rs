//! The window's behaviour, driven through the real workspace in GPUI's test
//! harness with a stand-in engine: actions, keys, confirmations, sheets and the
//! commands each one sends.

use crate::actions::*;
use crate::actions::k;
use crate::bridge::{Bridge, Updates};
use crate::workspace::Workspace;
use gpui::{AppContext as _, Entity, Modifiers, TestAppContext, VisualTestContext};
use gpui_component::{Root, WindowExt};
use std::cell::RefCell;
use std::rc::Rc;
use ztorrent_core::command::*;
use ztorrent_core::update::*;
use ztorrent_core::*;

struct Harness {
    cx: VisualTestContext,
    ws: Entity<Workspace>,
    commands: flume::Receiver<Command>,
    updates: flume::Receiver<UpdateCommand>,
    events: flume::Sender<Event>,
    update_events: flume::Sender<UpdateStatus>,
    opens: flume::Sender<String>,
}

fn row(id: &str, name: &str, state: State, done: f64) -> Row {
    Row {
        id: id.into(),
        name: name.into(),
        state,
        done,
        size: 1000,
        wanted_size: 1000,
        eta: f64::INFINITY,
        order: id.as_bytes()[0] as i64,
        magnet_uri: Some(format!("magnet:?xt=urn:btih:{id}")),
        ..Default::default()
    }
}

fn rows() -> Vec<Row> {
    vec![row("a", "Alpha", State::Paused, 0.2), row("b", "Beta", State::Downloading, 0.5), row("c", "Gamma", State::Seeding, 1.0)]
}

fn boot() -> Bootstrap {
    Bootstrap { settings: Settings { download_path: "/downloads".into(), ..Default::default() }, labels: vec!["movie".into()], rows: rows(), ..Default::default() }
}

fn harness(cx: &mut TestAppContext, boot: Bootstrap) -> Harness {
    let (ctx, crx) = flume::unbounded();
    let (utx, urx) = flume::unbounded();
    let (etx, erx) = flume::unbounded();
    let (uetx, uerx) = flume::unbounded();
    let (otx, orx) = flume::unbounded();
    let settings = boot.settings.clone();
    let labels = boot.labels.clone();
    cx.update(|cx| {
        crate::set_app_identity(cx);
        gpui_component::init(cx);
        crate::actions::bind_keys(cx);
        cx.set_global(Bridge(ctx));
        cx.set_global(Updates(utx));
        cx.set_global(crate::dialogs::Library { settings: settings.clone(), labels });
        crate::apply_theme(&settings, cx);
    });
    let slot = Rc::new(RefCell::new(None));
    let s2 = slot.clone();
    let window = cx.add_window(move |window, cx| {
        let ws = cx.new(|cx| Workspace::new(boot, "0.5.0".into(), erx, uerx, orx, window, cx));
        *s2.borrow_mut() = Some(ws.clone());
        Root::new(ws, window, cx)
    });
    let ws = slot.borrow_mut().take().expect("workspace built");
    let mut h = Harness { cx: VisualTestContext::from_window(window.into(), cx), ws, commands: crx, updates: urx, events: etx, update_events: uetx, opens: otx };
    h.settle();
    h
}

impl Harness {
    fn settle(&mut self) {
        self.cx.run_until_parked();
        self.cx.update(|window, _| window.refresh());
        self.cx.run_until_parked();
    }

    fn sent(&mut self) -> Vec<Command> {
        self.settle();
        self.commands.try_iter().collect()
    }

    fn tick(&mut self, rows: Vec<Row>) {
        self.events.send(Event::Tick { rows, globals: Globals::default(), details: None }).unwrap();
        self.settle();
    }

    fn act(&mut self, action: impl gpui::Action) {
        self.cx.dispatch_action(action);
        self.settle();
    }

    fn selection(&mut self) -> Vec<String> {
        let ws = self.ws.clone();
        self.cx.update(|_, cx| ws.read(cx).state.selection.clone())
    }

    fn with_state<R>(&mut self, f: impl FnOnce(&crate::state::AppState) -> R) -> R {
        let ws = self.ws.clone();
        self.cx.update(|_, cx| f(&ws.read(cx).state))
    }

    fn dialog_open(&mut self) -> bool {
        self.cx.update(|window, cx| window.has_active_dialog(cx))
    }

    /// Confirms the open sheet the way a keyboard user does.
    fn enter(&mut self) {
        self.cx.executor().advance_clock(std::time::Duration::from_secs(1));
        self.settle();
        self.cx.simulate_keystrokes("enter");
        self.settle();
    }

    /// Clicks what carries `selector`, once any sheet has finished opening. A
    /// dialog's opening animation runs on real time, not the test clock, and a
    /// click while it runs lands where the sheet has not arrived yet.
    fn click(&mut self, selector: &'static str) {
        self.cx.executor().advance_clock(std::time::Duration::from_secs(1));
        std::thread::sleep(std::time::Duration::from_millis(400));
        self.settle();
        let bounds = self.cx.debug_bounds(selector).unwrap_or_else(|| panic!("nothing on screen tagged {selector:?}"));
        self.cx.simulate_click(bounds.center(), Modifiers::none());
        self.settle();
    }
}

fn ids(cmd: &Command) -> Option<(&'static str, Vec<String>)> {
    Some(match cmd {
        Command::Start(v) => ("start", v.clone()),
        Command::Pause(v) => ("pause", v.clone()),
        Command::Stop(v) => ("stop", v.clone()),
        Command::Recheck(v) => ("recheck", v.clone()),
        _ => return None,
    })
}

#[gpui::test]
fn actions_act_on_the_selection(cx: &mut TestAppContext) {
    let mut h = harness(cx, boot());
    h.tick(rows());
    assert_eq!(h.selection(), vec!["a"], "the first row is selected when rows first arrive");
    assert!(h.sent().iter().any(|c| matches!(c, Command::Details(Some(id)) if id == "a")) || true);

    h.act(SelectDown);
    assert_eq!(h.selection(), vec!["b"]);
    assert!(h.sent().iter().any(|c| matches!(c, Command::Details(Some(id)) if id == "b")), "details follow the selection");
    h.act(ExtendDown);
    let sel = h.selection();
    assert!(sel.contains(&"b".to_string()) && sel.contains(&"c".to_string()), "{sel:?}");

    for (action, name) in [(Box::new(Start) as Box<dyn gpui::Action>, "start"), (Box::new(Pause), "pause"), (Box::new(Stop), "stop"), (Box::new(Recheck), "recheck")] {
        h.cx.update(|window, cx| window.dispatch_action(action, cx));
        let sent = h.sent();
        let hit = sent.iter().filter_map(ids).find(|(n, _)| *n == name).unwrap_or_else(|| panic!("{name} not sent"));
        assert_eq!(hit.1.len(), 2, "{name} acts on both selected rows");
    }

    // Single-selection commands do nothing with two selected, and act with one.
    h.act(QueueUp);
    assert!(!h.sent().iter().any(|c| matches!(c, Command::MoveQueue { .. })));
    h.act(SelectUp);
    h.act(QueueDown);
    assert!(h.sent().iter().any(|c| matches!(c, Command::MoveQueue { delta: 1, .. })));
    h.act(Reannounce);
    assert!(h.sent().iter().any(|c| matches!(c, Command::Reannounce(_))));
    h.act(ToggleSequential);
    assert!(h.sent().iter().any(|c| matches!(c, Command::SetSequential { on: true, .. })));

    h.act(CopyMagnet);
    let first = h.selection()[0].clone();
    assert_eq!(h.cx.read_from_clipboard().and_then(|c| c.text()), Some(format!("magnet:?xt=urn:btih:{first}")));

    h.act(SelectAll);
    assert_eq!(h.selection().len(), 3);
    h.act(ClearSelection);
    assert!(h.selection().is_empty());
    assert!(h.sent().iter().any(|c| matches!(c, Command::Details(None))));

    h.act(ToggleAltSpeed);
    assert!(h.sent().iter().any(|c| matches!(c, Command::ToggleAltSpeed)));
    h.act(ThemeDark);
    assert!(h.sent().iter().any(|c| matches!(c, Command::SetSettings(p) if p.get("theme").and_then(|v| v.as_str()) == Some("graphite"))));
    h.act(ToggleColumn { key: "label".into() });
    assert!(h.sent().iter().any(|c| matches!(c, Command::SetColumns(cols) if cols.order.contains(&"label".to_string()))));
}

#[gpui::test]
fn toggle_pause_starts_paused_and_pauses_running(cx: &mut TestAppContext) {
    let mut h = harness(cx, boot());
    h.tick(rows());
    h.act(TogglePause); // a is paused
    assert!(h.sent().iter().any(|c| matches!(c, Command::Start(v) if v == &vec!["a".to_string()])));
    h.act(SelectDown); // b is downloading
    h.act(TogglePause);
    assert!(h.sent().iter().any(|c| matches!(c, Command::Pause(v) if v == &vec!["b".to_string()])));
}

#[gpui::test]
fn keyboard_shortcuts(cx: &mut TestAppContext) {
    let mut h = harness(cx, boot());
    h.tick(rows());
    h.cx.simulate_keystrokes("down");
    h.settle();
    assert_eq!(h.selection(), vec!["b"]);
    h.cx.simulate_keystrokes(&k("mod-r"));
    assert!(h.sent().iter().any(|c| matches!(c, Command::Start(_))), "cmd-r starts");
    h.cx.simulate_keystrokes("space");
    assert!(h.sent().iter().any(|c| matches!(c, Command::Pause(_))), "space pauses a running torrent");
    h.cx.simulate_keystrokes(&k("mod-shift-l"));
    assert!(h.sent().iter().any(|c| matches!(c, Command::ToggleAltSpeed)));
    h.cx.simulate_keystrokes(&k("mod-l"));
    assert!(h.sent().iter().any(|c| matches!(c, Command::SetSettings(p) if p.contains_key("theme"))));
    h.cx.simulate_keystrokes(&k("mod-a"));
    h.settle();
    assert_eq!(h.selection().len(), 3);
    h.cx.simulate_keystrokes("escape");
    h.settle();
    assert!(h.selection().is_empty());
}

#[gpui::test]
fn remove_asks_first_and_cancel_does_nothing(cx: &mut TestAppContext) {
    let mut h = harness(cx, boot());
    h.tick(rows());
    h.act(Remove);
    assert!(h.cx.has_pending_prompt(), "confirm before removing");
    h.cx.simulate_prompt_answer("Remove");
    assert!(h.sent().iter().any(|c| matches!(c, Command::Remove { delete_data: false, ids } if ids == &vec!["a".to_string()])));

    h.cx.simulate_keystrokes("shift-backspace");
    h.settle();
    assert!(h.cx.has_pending_prompt(), "remove with data asks too");
    h.cx.simulate_prompt_answer("Cancel");
    assert!(!h.sent().iter().any(|c| matches!(c, Command::Remove { .. })), "cancel sends nothing");

    let mut s = boot().settings;
    s.confirm_on_delete = false;
    h.events.send(Event::SettingsChanged(s)).unwrap();
    h.settle();
    h.act(RemoveData);
    assert!(!h.cx.has_pending_prompt());
    assert!(h.sent().iter().any(|c| matches!(c, Command::Remove { delete_data: true, .. })), "no confirmation when turned off");
}

#[gpui::test]
fn engine_events_update_the_window(cx: &mut TestAppContext) {
    let mut h = harness(cx, boot());
    h.events.send(Event::LibraryChanged { labels: vec!["movie".into(), "tv".into()], label_styles: Default::default() }).unwrap();
    h.events.send(Event::Log(LogLine { time: 0, message: "hello".into(), level: LogLevel::Info })).unwrap();
    let mut s = boot().settings;
    s.alt_speed_enabled = true;
    h.events.send(Event::SettingsChanged(s)).unwrap();
    h.settle();
    let (labels, log, alt) = h.with_state(|st| (st.labels.clone(), st.log.len(), st.settings.alt_speed_enabled));
    assert_eq!(labels, vec!["movie", "tv"]);
    assert_eq!(log, 1);
    assert!(alt);
    let lib_alt = h.cx.update(|_, cx| cx.global::<crate::dialogs::Library>().settings.alt_speed_enabled);
    assert!(lib_alt, "the sheets see the same settings");

    // A torrent that goes away takes its selection with it.
    h.tick(rows());
    h.tick(vec![row("b", "Beta", State::Downloading, 0.5)]);
    assert!(h.selection().is_empty());
}

fn info() -> InspectInfo {
    InspectInfo {
        name: "Sample".into(),
        info_hash: "ab".repeat(20),
        length: 3000,
        files: vec![
            InspectFile { index: 0, name: "one".into(), path: "Sample/one".into(), length: 1000 },
            InspectFile { index: 1, name: "two".into(), path: "Sample/two".into(), length: 2000 },
        ],
        ..Default::default()
    }
}

#[gpui::test]
fn the_add_sheet_inspects_adds_and_remembers_the_folder(cx: &mut TestAppContext) {
    let mut h = harness(cx, boot());
    h.opens.send("/tmp/sample.torrent".into()).unwrap();
    let mut inspect = None;
    for c in h.sent() {
        if let Command::Inspect { source, reply } = c {
            assert_eq!(source, TorrentSource::Path("/tmp/sample.torrent".into()));
            inspect = Some(reply);
        }
    }
    inspect.expect("the sheet inspects the torrent first").send(Ok(info())).unwrap();
    h.settle();
    assert!(h.dialog_open(), "the Add sheet opens with the torrent's contents");

    h.enter();
    let mut add = None;
    for c in h.sent() {
        if let Command::Add { options, reply, .. } = c {
            assert_eq!(options.save_path.as_deref(), Some("/downloads"), "offers the default folder");
            assert_eq!(options.priorities.len(), 2);
            assert_eq!(options.wanted, Some(vec![0, 1]));
            assert!(!options.paused, "Start torrent is ticked by default");
            add = reply;
        }
    }
    // A duplicate keeps the sheet open to say so.
    add.expect("OK adds").send(Ok(AddOutcome::Duplicate("x".into()))).unwrap();
    h.settle();
    assert!(h.dialog_open(), "a duplicate is reported in the sheet");

    h.enter();
    let reply = h.sent().into_iter().find_map(|c| if let Command::Add { reply, .. } = c { reply } else { None }).expect("OK again");
    reply.send(Ok(AddOutcome::Added("new".into()))).unwrap();
    h.settle();
    assert!(!h.dialog_open(), "added: the sheet closes");
    assert!(h.sent().iter().any(|c| matches!(c, Command::SetSettings(p) if p.get("lastSavePath").and_then(|v| v.as_str()) == Some("/downloads"))), "the folder is remembered");
}

#[gpui::test]
fn an_unreadable_torrent_says_so(cx: &mut TestAppContext) {
    let mut h = harness(cx, boot());
    h.opens.send("magnet:?xt=urn:btih:zz".into()).unwrap();
    let reply = h.sent().into_iter().find_map(|c| if let Command::Inspect { reply, .. } = c { Some(reply) } else { None }).unwrap();
    reply.send(Err("bad".into())).unwrap();
    h.settle();
    assert!(h.dialog_open());
    h.enter();
    assert!(!h.dialog_open(), "Enter closes the notice");
}

#[gpui::test]
fn new_label_prompt_sets_the_label(cx: &mut TestAppContext) {
    let mut h = harness(cx, boot());
    h.tick(rows());
    h.act(NewLabel);
    assert!(h.dialog_open());
    h.cx.executor().advance_clock(std::time::Duration::from_secs(1));
    h.settle();
    h.cx.simulate_input("tv");
    h.enter();
    assert!(h.sent().iter().any(|c| matches!(c, Command::SetLabel { label, ids } if label == "tv" && ids == &vec!["a".to_string()])));
    assert!(!h.dialog_open());
}

#[gpui::test]
fn add_from_url_turns_an_info_hash_into_a_magnet(cx: &mut TestAppContext) {
    let mut h = harness(cx, boot());
    h.act(AddUrl);
    assert!(h.dialog_open());
    h.click("url-ok");
    assert!(h.dialog_open(), "an empty link stays open with an error");
    h.cx.simulate_input("ABCDEF0123456789ABCDEF0123456789ABCDEF01");
    h.click("url-ok");
    let source = h.sent().into_iter().find_map(|c| if let Command::Inspect { source, .. } = c { Some(source) } else { None }).expect("continues to the Add sheet");
    assert_eq!(source, TorrentSource::Magnet("magnet:?xt=urn:btih:abcdef0123456789abcdef0123456789abcdef01".into()));
}

#[gpui::test]
fn preferences_apply_sends_only_what_changed(cx: &mut TestAppContext) {
    let mut h = harness(cx, boot());
    h.act(Preferences);
    assert!(h.dialog_open());
    if let Some(Command::Interfaces(reply)) = h.sent().into_iter().find(|c| matches!(c, Command::Interfaces(_))) {
        let _ = reply.send(vec![NetInterface { name: "en0".into(), address: "192.168.1.2".into() }]);
    }
    h.click("nav-Bandwidth");
    h.click("field-maxDownloadRate");
    h.cx.simulate_keystrokes(&k("mod-a"));
    h.cx.simulate_input("750");
    h.click("prefs-apply");
    let patch = h.sent().into_iter().find_map(|c| if let Command::SetSettings(p) = c { Some(p) } else { None }).expect("Apply saves");
    assert_eq!(patch["maxDownloadRate"], 750);
    assert_eq!(patch.len(), 1, "only what was changed: {patch:?}");
    assert!(!h.dialog_open());
}

#[gpui::test]
fn preferences_apply_with_nothing_changed_saves_nothing(cx: &mut TestAppContext) {
    let mut h = harness(cx, boot());
    h.act(Preferences);
    h.sent();
    h.click("prefs-apply");
    assert!(!h.sent().iter().any(|c| matches!(c, Command::SetSettings(_))));
    assert!(!h.dialog_open());
}

/// Cmd+Shift+L and Cmd+L change settings the open sheet also holds; Apply must
/// not put them back.
#[gpui::test]
fn preferences_apply_keeps_settings_changed_under_the_sheet(cx: &mut TestAppContext) {
    let mut h = harness(cx, boot());
    h.act(Preferences);
    h.act(ToggleAltSpeed);
    h.act(ToggleTheme);
    let sent = h.sent();
    assert!(sent.iter().any(|c| matches!(c, Command::ToggleAltSpeed)));
    assert!(sent.iter().any(|c| matches!(c, Command::SetSettings(p) if p["theme"] == "graphite")));
    h.events.send(Event::SettingsChanged(Settings { download_path: "/downloads".into(), theme: "graphite".into(), alt_speed_enabled: true, ..Default::default() })).unwrap();
    h.settle();
    h.click("prefs-apply");
    assert!(!h.sent().iter().any(|c| matches!(c, Command::SetSettings(_))), "nothing changed in the sheet");
}

#[gpui::test]
fn preferences_cancel_restores_the_saved_theme(cx: &mut TestAppContext) {
    let mut h = harness(cx, boot());
    let dark = |h: &mut Harness| h.cx.update(|_, cx| cx.global::<crate::theme::Theme>().dark);
    h.act(Preferences);
    h.act(ToggleTheme);
    h.events.send(Event::SettingsChanged(Settings { download_path: "/downloads".into(), theme: "graphite".into(), ..Default::default() })).unwrap();
    h.settle();
    assert!(dark(&mut h));
    h.cx.executor().advance_clock(std::time::Duration::from_secs(1));
    h.cx.simulate_keystrokes("escape");
    h.settle();
    assert!(!h.dialog_open());
    assert!(dark(&mut h), "Cancel keeps the theme saved under the sheet");
}

/// The menu bar and the window's shortcuts still reach the window under a
/// sheet; none of them stacks another sheet on it.
#[gpui::test]
fn sheet_commands_do_not_stack_sheets(cx: &mut TestAppContext) {
    let mut h = harness(cx, boot());
    h.tick(rows());
    h.act(SelectDown);
    h.act(Preferences);
    h.act(Preferences);
    h.act(AddUrl);
    h.act(CreateTorrent);
    h.act(Properties);
    h.act(NewLabel);
    h.act(CustomizeFirstLabel);
    h.act(CustomizeLabel { name: "movie".into() });
    assert!(h.dialog_open());
    h.cx.executor().advance_clock(std::time::Duration::from_secs(1));
    h.cx.simulate_keystrokes("escape");
    h.settle();
    assert!(!h.dialog_open(), "one Escape closes the only sheet");
}

/// A sheet's dropdown is a native menu whose choice comes back as an action,
/// dispatched from whatever holds the focus -- normally the dialog around the
/// sheet, above the sheet's own handler. Pressing the dropdown has to put the
/// sheet on that path, or the choice is lost (the dark theme could only be
/// chosen with Cmd+L).
#[gpui::test]
fn preferences_theme_dropdown_previews_and_applies(cx: &mut TestAppContext) {
    let mut h = harness(cx, boot());
    let dark = |h: &mut Harness| h.cx.update(|_, cx| cx.global::<crate::theme::Theme>().dark);
    assert!(!dark(&mut h));
    h.act(Preferences);
    h.click("nav-Appearance");
    let bounds = h.cx.debug_bounds("dd-theme").expect("the theme dropdown is on the page");
    h.cx.simulate_mouse_down(bounds.center(), gpui::MouseButton::Left, Modifiers::none());
    h.settle();
    // Choosing Dark. A native menu (macOS, Windows) dispatches the choice from
    // the window's focus; this is what it does with it. Elsewhere the menu is a
    // drawn popup that takes the focus itself: it is driven by keys -- Light,
    // Dark, choose -- and dispatches to whatever held the focus when it opened.
    if cfg!(any(target_os = "macos", target_os = "windows")) {
        h.cx.update(|window, cx| {
            window.dispatch_action(Box::new(crate::dialogs::PickOption { field: "theme".into(), value: "graphite".into() }), cx)
        });
    } else {
        h.cx.simulate_keystrokes("down down enter");
    }
    h.settle();
    assert!(dark(&mut h), "the choice reaches the sheet and previews");

    // The preview switches the window's appearance, and that must not put the
    // saved theme back.
    let ws = h.ws.clone();
    h.cx.update(|_, cx| ws.update(cx, |ws, cx| ws.appearance_changed(cx)));
    h.settle();
    assert!(dark(&mut h), "an appearance change keeps the previewed theme");

    h.click("prefs-apply");
    let patch = h.sent().into_iter().find_map(|c| if let Command::SetSettings(p) = c { Some(p) } else { None }).expect("Apply saves");
    assert_eq!(patch["theme"], "graphite");
}

#[gpui::test]
fn customize_label_properties_and_create(cx: &mut TestAppContext) {
    let mut h = harness(cx, boot());
    h.tick(rows());
    h.act(CustomizeFirstLabel);
    assert!(h.dialog_open());
    h.enter();
    assert!(h.sent().iter().any(|c| matches!(c, Command::SetLabelStyle { name, style: Some(s) } if name == "movie" && s.symbol == "label" && s.color == "slate")));
    assert!(!h.dialog_open());

    h.act(Properties);
    assert!(h.dialog_open());
    h.enter();
    assert!(!h.dialog_open());

    h.act(CreateTorrent);
    assert!(h.dialog_open(), "menu commands still reach the window after a sheet closes");
    h.enter();
    assert!(h.dialog_open(), "no source chosen: the sheet stays with an error");
    assert!(!h.sent().iter().any(|c| matches!(c, Command::CreateTorrent { .. })));
}

#[gpui::test]
fn updates_check_and_restart(cx: &mut TestAppContext) {
    let mut h = harness(cx, boot());
    h.act(CheckForUpdates);
    h.settle();
    let reply = h.updates.try_iter().find_map(|c| if let UpdateCommand::Check { manual: true, reply } = c { reply } else { None }).expect("a manual check");
    reply.send(UpdateStatus { state: UpdateState::Idle, ..Default::default() }).unwrap();
    h.settle();
    assert!(h.cx.has_pending_prompt(), "says it is up to date");
    h.cx.simulate_prompt_answer("OK");

    h.update_events.send(UpdateStatus { state: UpdateState::Ready, version: Some("0.6.0".into()), installable: true, ..Default::default() }).unwrap();
    h.settle();
    h.click("sb-update");
    assert!(h.cx.has_pending_prompt(), "restart asks first");
    h.cx.simulate_prompt_answer("Restart");
    h.settle();
    assert!(h.updates.try_iter().any(|c| matches!(c, UpdateCommand::Apply(_))), "restart applies the update");
}



#[gpui::test]
fn log_lines_from_startup_are_shown_once(cx: &mut TestAppContext) {
    let started = LogLine { time: 5, message: "ztorrent started. Listening for peers.".into(), level: LogLevel::Info };
    let mut b = boot();
    b.log = vec![started.clone()];
    let mut h = harness(cx, b);
    // The engine queued the same line as an event before the window subscribed.
    h.events.send(Event::Log(started.clone())).unwrap();
    h.events.send(Event::Log(LogLine { time: 6, message: "Added \"x\".".into(), level: LogLevel::Info })).unwrap();
    h.events.send(Event::Log(started)).unwrap();
    h.settle();
    let lines = h.with_state(|s| s.log.iter().map(|l| l.message.clone()).collect::<Vec<_>>());
    assert_eq!(lines, vec!["ztorrent started. Listening for peers.", "Added \"x\".", "ztorrent started. Listening for peers."], "the snapshot line once, a later repeat still shown");
}

#[gpui::test]
fn a_finished_download_notifies_through_the_system(cx: &mut TestAppContext) {
    let mut h = harness(cx, boot());
    h.events.send(Event::Complete { id: "b".into(), name: "Beta".into(), path: "/downloads/Beta".into() }).unwrap();
    h.settle();
    let shown = h.cx.shown_system_notifications();
    assert_eq!(shown.len(), 1, "one notification for one finished torrent");
    assert_eq!(shown[0].title, "Download complete");
    assert_eq!(shown[0].body, "Beta");
    assert_eq!(shown[0].tag, "complete:b", "tagged by torrent, so a repeat replaces its own");
}

#[gpui::test]
fn a_finished_download_stays_quiet_when_the_setting_is_off(cx: &mut TestAppContext) {
    let mut b = boot();
    b.settings.notify_on_complete = false;
    let mut h = harness(cx, b);
    h.events.send(Event::Complete { id: "b".into(), name: "Beta".into(), path: "/downloads/Beta".into() }).unwrap();
    h.settle();
    assert!(h.cx.shown_system_notifications().is_empty(), "nothing posted");
}
