//! Everything the engine and the window agree on, and nothing that does I/O on
//! either's behalf beyond the state file.
//!
//! The window depends on this crate and never on the engine, so the command set
//! in [`command`] is the whole of what the window can ask for -- the Rust
//! successor to the Electron preload's channel list.

pub mod columns;
pub mod command;
pub mod egress;
pub mod fmt;
pub mod launch;
pub mod model;
pub mod settings;
pub mod store;
pub mod update;
pub mod version;

mod lenient;

pub use command::{Command, Event};
pub use model::*;
pub use settings::Settings;
pub use store::{SecretCodec, StateData, Store};
