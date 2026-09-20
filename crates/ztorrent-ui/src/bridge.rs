//! The window's end of the command set. Everything the window asks of the
//! engine goes through `send` or `ask`; there is no other way to reach it.

use futures_channel::oneshot;
use gpui::{App, Global};
use ztorrent_core::command::{Command, Reply};

#[derive(Clone)]
pub struct Bridge(pub flume::Sender<Command>);

impl Global for Bridge {}

/// The updater's command channel.
#[derive(Clone)]
pub struct Updates(pub flume::Sender<ztorrent_core::update::UpdateCommand>);

impl Global for Updates {}

pub fn update(cx: &App, cmd: ztorrent_core::update::UpdateCommand) {
    let _ = cx.global::<Updates>().0.send(cmd);
}

pub fn send(cx: &App, cmd: Command) {
    let _ = cx.global::<Bridge>().0.send(cmd);
}

/// Sends a command carrying a reply channel and returns the receiving end.
pub fn ask<T>(cx: &App, make: impl FnOnce(Reply<T>) -> Command) -> oneshot::Receiver<T> {
    let (tx, rx) = oneshot::channel();
    send(cx, make(tx));
    rx
}

/// An empty element; keeps the bridge module's items referenced from render.
pub fn marker() -> gpui::Empty {
    gpui::Empty
}
