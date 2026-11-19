//! The island (DynamicIsland.exe): the iPhone's Dynamic Island for Windows.
//!
//! The island window and everything it shows live on one UI thread ([`rt`]); [`app`] starts it all. The settings
//! app (DynamicIsland.Settings.exe) writes `settings.json`, which the island watches (see `di4w_core`).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// Callbacks stored for the UI thread (`Rc<dyn Fn(...)>` in maps and cells) are clearer spelled out than aliased.
#![allow(clippy::type_complexity)]

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
    let code = match std::panic::catch_unwind(|| app::run(restarted)) {
        Ok(_) => 0,
        Err(_) => {
            let details = LAST_PANIC.lock().unwrap_or_else(|e| e.into_inner()).take().unwrap_or_else(|| "Unknown error".into());
            di4w_core::crash::write(&details);
            1
        }
    };
    // Everything is released by now (tray icon, port, mutex, settings). End the process right here: the system DLLs'
    // own shutdown (audio, notifications, Direct3D) can leave a terminating process stuck in the kernel otherwise.
    unsafe {
        let _ = windows::Win32::System::Threading::TerminateProcess(windows::Win32::System::Threading::GetCurrentProcess(), code);
    }
}
