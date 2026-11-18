//! The island (DynamicIsland.exe): the iPhone's Dynamic Island for Windows.
//!
//! The island window and everything it shows live on one UI thread ([`rt`]); [`app`] starts it all. The settings
//! app (DynamicIsland.Settings.exe) writes `settings.json`, which the island watches (see `di4w_core`).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod accessibility;
mod activities;
mod addons;
mod animation;
mod app;
mod interop;
mod island;
mod model;
mod reg;
mod rt;
mod tooltip;
mod tray;
mod ui;

use std::sync::Mutex;

/// The last panic's message and where it happened (for crash.txt).
static LAST_PANIC: Mutex<Option<String>> = Mutex::new(None);

fn main() {
    // Every panic is logged; one that takes the island down is kept in crash.txt and offered for reporting on the
    // next start (see app::App::offer_crash_report).
    std::panic::set_hook(Box::new(|info| {
        let thread = std::thread::current().name().unwrap_or("background").to_string();
        let details = format!("{info}\nThread: {thread}\n{}", std::backtrace::Backtrace::force_capture());
        di4w_core::log::error_with("Unhandled exception", &details);
        *LAST_PANIC.lock().unwrap_or_else(|e| e.into_inner()) = Some(details);
    }));

    let restarted = std::env::args().any(|a| a == "--restart");
    if std::panic::catch_unwind(|| app::run(restarted)).is_err() {
        let details = LAST_PANIC.lock().unwrap_or_else(|e| e.into_inner()).take().unwrap_or_else(|| "Unknown error".into());
        di4w_core::crash::write(&details);
        std::process::exit(1);
    }
}
