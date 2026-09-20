//! What the updater tells the window, and what the window can ask of it.

use crate::command::Reply;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UpdateState {
    #[default]
    Idle,
    Checking,
    Available,
    Downloading,
    Staging,
    Ready,
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateStep {
    Check,
    Download,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct UpdateStatus {
    pub state: UpdateState,
    pub version: Option<String>,
    pub url: Option<String>,
    pub size: u64,
    pub received: u64,
    pub file: Option<PathBuf>,
    pub error: Option<String>,
    /// Which step the error came from: a check nobody asked for has nothing to
    /// report, while a download that died left work the user started unfinished.
    pub error_from: Option<UpdateStep>,
    /// Whether the user asked for this check; the automatic one works in silence.
    pub manual: bool,
    /// False where this process cannot install the release itself -- Linux
    /// outside an AppImage, or no artifact for this platform.
    pub installable: bool,
    pub current: String,
}

pub enum UpdateCommand {
    Check { manual: bool, reply: Option<Reply<UpdateStatus>> },
    Download,
    /// Hands off to the swap script; true when the app should now quit.
    Apply(Reply<bool>),
    /// Automatic checks on or off, as the preference changes.
    SetAutomatic(bool),
    Status(Reply<UpdateStatus>),
}
