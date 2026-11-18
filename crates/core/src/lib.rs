//! Shared by the island (`DynamicIsland.exe`) and the settings app (`DynamicIsland.Settings.exe`).
//!
//! The settings app writes `%AppData%\DynamicIsland\settings.json`; the island watches it and applies changes
//! live. The island writes `status.json` back (add-on states) and listens for small commands the settings app
//! sends as `WM_COPYDATA` (see [`messages`]).

pub mod crash;
pub mod displays;
pub mod hotkey;
pub mod l;
pub mod log;
pub mod messages;
pub mod paths;
pub mod registry;
pub mod settings;
pub mod status;
pub mod theme;
pub mod updates;
pub mod wide;

pub use settings::{ActivityKinds, AddonQuiet, AddonSettings, AppSettings, ChatApp, ChatApps};
