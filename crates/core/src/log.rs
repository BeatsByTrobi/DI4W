//! A daily log file in `%AppData%\DynamicIsland\logs` (`2026-11-18.log`). Logging never fails loudly.

use std::fmt::Display;
use std::io::Write;
use std::sync::Mutex;

static GATE: Mutex<()> = Mutex::new(());

pub fn info(message: impl Display) {
    write("INFO", &message.to_string());
}

pub fn error(message: impl Display) {
    write("ERROR", &message.to_string());
}

/// An error with what caused it: "Failed to read settings: expected value at line 1".
pub fn error_with(message: impl Display, cause: impl Display) {
    write("ERROR", &format!("{message}: {cause}"));
}

fn write(level: &str, message: &str) {
    let _gate = GATE.lock().unwrap_or_else(|e| e.into_inner());
    let now = chrono::Local::now();
    let folder = crate::paths::log_folder();
    let _ = std::fs::create_dir_all(&folder);
    let file = folder.join(format!("{}.log", now.format("%Y-%m-%d")));
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(file) {
        let _ = writeln!(f, "{} [{level}] {message}", now.format("%H:%M:%S%.3f"));
    }
}
