//! `settings.json`, shared by the island and the settings app.
//!
//! The settings app writes it; the island watches it and applies changes live. Field names are PascalCase so
//! files written by the 1.x (C#) apps load unchanged, and unknown fields are ignored.

use crate::{log, paths};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct AppSettings {
    /// Distance from the top of the screen to the island, in DIPs.
    pub top_offset: f64,
    /// Overall size multiplier (1.0 = default).
    pub scale: f64,
    /// "" = dark (black pill), "Light", or "System" to follow Windows. See [`crate::theme`].
    pub theme: String,
    pub hide_in_fullscreen: bool,
    pub media_enabled: bool,
    /// When false only Spotify is shown; when true any app using Windows media controls is.
    pub media_any_player: bool,
    pub media_hide_after_pause_seconds: i32,
    /// Settings files from 1.x kept the Island API's port here (it was shared with Claude Code); read once and
    /// moved to [`AddonSettings::island_api_port`], never written.
    #[serde(rename = "ClaudePort", skip_serializing)]
    pub legacy_port: Option<i32>,
    pub notifications_enabled: bool,
    /// App display names whose Windows notifications pop out of the island.
    pub notification_apps: Vec<String>,
    pub sounds: bool,
    /// Global shortcut that hides/shows the island, e.g. "Ctrl+Alt+I". Empty = no shortcut.
    pub hide_hotkey: String,
    /// Global shortcut that opens/closes the island's expanded view (keyboard and screen reader use).
    pub expand_hotkey: String,
    /// Hidden with the shortcut or "Hide island"; remembered so it stays hidden after a restart.
    pub paused: bool,
    /// "" = Windows' language, or "en", "nl", "de", "fr".
    pub language: String,
    /// Which display: "" = main display, "Mouse" = the one with the mouse, or a device name (\\.\DISPLAY2).
    pub monitor: String,
    /// Which activity gets the pill when several run, most important first ("Timer", "Media"…).
    /// Empty: the built-in order. See [`ActivityKinds`].
    pub activity_order: Vec<String>,
    /// The welcome tour has been shown. A fresh install starts with false (see [`AppSettings::load`]); files
    /// from before the tour existed read as true, so updating users don't get it.
    pub welcome_shown: bool,
    /// After a crash, offer to report it on the next start.
    pub offer_crash_reports: bool,
    /// Look for new releases on GitHub now and then and announce them in the island.
    pub auto_check_updates: bool,
    /// On/off switches and options for the add-ons (volume, battery, calendar, weather…).
    pub addons: AddonSettings,
    /// Written by the island on startup so the settings app can find it (Start with Windows).
    pub island_path: Option<String>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            top_offset: 8.0,
            scale: 1.0,
            theme: crate::theme::DARK.into(),
            hide_in_fullscreen: true,
            media_enabled: true,
            media_any_player: false,
            media_hide_after_pause_seconds: 10,
            legacy_port: None,
            notifications_enabled: true,
            notification_apps: vec!["Discord".into()],
            sounds: true,
            hide_hotkey: "Ctrl+Alt+I".into(),
            expand_hotkey: "Ctrl+Alt+O".into(),
            paused: false,
            language: String::new(),
            monitor: String::new(),
            activity_order: Vec::new(),
            welcome_shown: true,
            offer_crash_reports: true,
            auto_check_updates: true,
            addons: AddonSettings::default(),
            island_path: None,
        }
    }
}

/// Add-on switches. Everything that shows up uninvited (system pop-ups) defaults to on; things that need setup
/// (calendar feeds, weather location) or cost resources (game overlay) say so in the settings app.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct AddonSettings {
    // System
    pub volume: bool,
    pub brightness: bool,
    pub battery: bool,
    pub battery_low_percent: i32,
    pub battery_critical_percent: i32,
    pub bluetooth: bool,
    pub usb_drives: bool,
    pub network: bool,
    pub privacy_indicators: bool,
    pub lock_keys: bool,
    pub performance_warnings: bool,

    // Productivity
    pub calendar: bool,
    /// iCalendar (.ics) links, e.g. Outlook's or Google's "secret address" for a calendar.
    pub calendar_feeds: Vec<String>,
    pub calendar_reminder_minutes: i32,
    pub focus_minutes: i32,
    pub break_minutes: i32,
    pub long_break_minutes: i32,
    pub sessions_before_long_break: i32,
    pub clipboard: bool,
    pub screenshots: bool,
    pub downloads: bool,
    pub file_copy: bool,

    // Apps and alerts
    pub calls: bool,
    pub do_not_disturb: bool,
    pub windows_update: bool,

    // Info
    pub weather: bool,
    /// City name for the weather. Empty: use Windows location if allowed.
    pub weather_location: String,
    pub weather_fahrenheit: bool,
    pub rain_alerts: bool,
    /// Keep the island visible over fullscreen games and show GPU/CPU load. Off by default.
    pub game_mode: bool,

    // Developers
    /// Local API (127.0.0.1) so scripts can show alerts and progress.
    pub island_api: bool,
    /// The Island API's port on 127.0.0.1 (scripts can read DYNAMIC_ISLAND_PORT).
    pub island_api_port: i32,

    /// Per add-on (keyed by its switch, e.g. "Clipboard"): when its pop-ups stay away.
    pub quiet: BTreeMap<String, AddonQuiet>,
}

impl Default for AddonSettings {
    fn default() -> Self {
        Self {
            volume: true,
            brightness: true,
            battery: true,
            battery_low_percent: 20,
            battery_critical_percent: 10,
            bluetooth: true,
            usb_drives: true,
            network: true,
            privacy_indicators: true,
            lock_keys: true,
            performance_warnings: true,
            calendar: true,
            calendar_feeds: Vec::new(),
            calendar_reminder_minutes: 5,
            focus_minutes: 25,
            break_minutes: 5,
            long_break_minutes: 15,
            sessions_before_long_break: 4,
            clipboard: true,
            screenshots: true,
            downloads: true,
            file_copy: true,
            calls: true,
            do_not_disturb: true,
            windows_update: true,
            weather: true,
            weather_location: String::new(),
            weather_fahrenheit: false,
            rain_alerts: true,
            game_mode: false,
            island_api: true,
            island_api_port: 47823,
            quiet: BTreeMap::new(),
        }
    }
}

/// When one add-on's pop-ups stay away: during set hours (e.g. 22:00–08:00, across midnight) and/or while one of
/// the listed apps is in front (process names like "obs64"). Held-back pop-ups still land in Recent.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct AddonQuiet {
    pub hours_on: bool,
    pub from: String,
    pub to: String,
    pub apps: Vec<String>,
}

impl Default for AddonQuiet {
    fn default() -> Self {
        Self { hours_on: false, from: "22:00".into(), to: "08:00".into(), apps: Vec::new() }
    }
}

impl AddonQuiet {
    pub fn is_empty(&self) -> bool {
        !self.hours_on && self.apps.is_empty()
    }

    /// `foreground_app`: process name of the app in front, without ".exe"; `None` if unknown.
    pub fn is_quiet(&self, time_of_day: chrono::NaiveTime, foreground_app: Option<&str>) -> bool {
        if let Some(app) = foreground_app
            && self.apps.iter().any(|a| Self::normalize(a).eq_ignore_ascii_case(app)) {
                return true;
            }
        let (Some(from), Some(to)) = (parse_time(&self.from), parse_time(&self.to)) else { return false };
        if !self.hours_on || from == to {
            return false;
        }
        if from < to {
            time_of_day >= from && time_of_day < to // 13:00–14:00
        } else {
            time_of_day >= from || time_of_day < to // 22:00–08:00, across midnight
        }
    }

    /// "OBS64.exe" -> "OBS64".
    pub fn normalize(app: &str) -> &str {
        let app = app.trim();
        match app.len().checked_sub(4) {
            Some(cut) if app.is_char_boundary(cut) && app[cut..].eq_ignore_ascii_case(".exe") => &app[..cut],
            _ => app,
        }
    }
}

/// "22:00" or "22:00:30".
pub fn parse_time(text: &str) -> Option<chrono::NaiveTime> {
    let text = text.trim();
    chrono::NaiveTime::parse_from_str(text, "%H:%M:%S")
        .or_else(|_| chrono::NaiveTime::parse_from_str(text, "%H:%M"))
        .ok()
}

impl AppSettings {
    /// Reads settings.json. A broken file (usually a typo from a hand edit) is kept as `settings.json.bad` and
    /// defaults are used; with no file at all (a fresh install) the welcome tour is due. Always saves.
    pub fn load() -> Self {
        let path = paths::settings_file();
        let existed = path.exists();
        if existed {
            match std::fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|j| Self::parse(&j)) {
                Ok(settings) => return settings,
                Err(e) => {
                    log::error_with("Failed to read settings; kept a copy as settings.json.bad and using defaults", e);
                    let mut bad = path.as_os_str().to_owned();
                    bad.push(".bad");
                    if let Err(e) = std::fs::copy(&path, bad) {
                        log::error_with("Could not keep a copy of the broken settings", e);
                    }
                }
            }
        }
        let settings = Self { welcome_shown: existed, ..Self::default() };
        settings.save();
        settings
    }

    /// Reads the file without falling back to defaults; `None` if it is missing, locked or half-written.
    pub fn try_load() -> Option<Self> {
        Self::parse(&std::fs::read_to_string(paths::settings_file()).ok()?).ok()
    }

    /// Reads settings JSON (a file, or an imported backup), moving values older versions kept elsewhere.
    pub fn parse(json: &str) -> Result<Self, String> {
        let mut settings: Self = serde_json::from_str(json).map_err(|e| e.to_string())?;
        if let Some(port) = settings.legacy_port.take() {
            settings.addons.island_api_port = port;
        }
        Ok(settings)
    }

    /// Pretty JSON, as saved to disk (and exported as a backup).
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    pub fn save(&self) {
        if let Err(e) = paths::write_atomic(&paths::settings_file(), self.to_json().as_bytes()) {
            log::error_with("Failed to save settings", e);
        }
    }

    /// Changes settings on disk without overwriting what the other app changed: under a lock shared by the
    /// island and the settings app, reads the latest file, applies `change` and saves. Returns the saved settings.
    pub fn update(change: impl FnOnce(&mut AppSettings)) -> AppSettings {
        let _gate = SettingsLock::acquire();
        let mut latest = if paths::settings_file().exists() {
            Self::try_load().unwrap_or_else(Self::load)
        } else {
            Self { welcome_shown: false, ..Self::default() }
        };
        change(&mut latest);
        latest.save();
        latest
    }
}

/// The named mutex both apps take around read-change-save, so neither overwrites the other's change.
struct SettingsLock(Option<windows::Win32::Foundation::HANDLE>);

impl SettingsLock {
    fn acquire() -> Self {
        use windows::Win32::Foundation::{WAIT_ABANDONED, WAIT_OBJECT_0};
        use windows::Win32::System::Threading::{CreateMutexW, WaitForSingleObject};
        use windows::core::w;
        unsafe {
            let Ok(handle) = CreateMutexW(None, false, w!("DynamicIsland4Windows.SettingsFile")) else {
                return Self(None);
            };
            // Abandoned: the other app quit while holding it; the file itself is always whole (atomic swap).
            let wait = WaitForSingleObject(handle, 3000);
            if wait == WAIT_OBJECT_0 || wait == WAIT_ABANDONED {
                Self(Some(handle))
            } else {
                let _ = windows::Win32::Foundation::CloseHandle(handle);
                Self(None)
            }
        }
    }
}

impl Drop for SettingsLock {
    fn drop(&mut self) {
        if let Some(handle) = self.0 {
            unsafe {
                let _ = windows::Win32::System::Threading::ReleaseMutex(handle);
                let _ = windows::Win32::Foundation::CloseHandle(handle);
            }
        }
    }
}

/// The activities that can take the pill, in the built-in order (most important first).
pub struct ActivityKinds;

impl ActivityKinds {
    /// `(kind, English label)`; labels go through [`crate::l::t`] when shown.
    pub const ALL: [(&'static str, &'static str); 8] = [
        ("Focus", "Focus timer"),
        ("Timer", "Timer"),
        ("Stopwatch", "Stopwatch"),
        ("Downloads", "Downloads"),
        ("Api", "Island API progress"),
        ("FileCopy", "File copies"),
        ("Media", "Music"),
        ("Game", "Game mode"),
    ];

    /// The saved order, completed with kinds it doesn't mention yet (in their built-in place at the end).
    pub fn ordered<S: AsRef<str>>(saved: &[S]) -> Vec<String> {
        let mut order: Vec<String> = Vec::new();
        for kind in saved.iter().map(AsRef::as_ref) {
            if Self::ALL.iter().any(|(k, _)| *k == kind) && !order.iter().any(|o| o == kind) {
                order.push(kind.to_string());
            }
        }
        for (kind, _) in Self::ALL {
            if !order.iter().any(|o| o == kind) {
                order.push(kind.to_string());
            }
        }
        order
    }
}

/// A chat app with built-in support: matched on the name Windows shows for its notifications, with a brand color
/// for the fallback icon and the process names used to bring the app forward when an alert is clicked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChatApp {
    pub name: &'static str,
    pub color: &'static str,
    pub processes: &'static [&'static str],
    pub has_calls: bool,
}

pub struct ChatApps;

impl ChatApps {
    pub const ALL: [ChatApp; 8] = [
        ChatApp { name: "Discord", color: "#5865F2", processes: &["Discord", "DiscordPTB", "DiscordCanary"], has_calls: true },
        ChatApp { name: "WhatsApp", color: "#25D366", processes: &["WhatsApp", "WhatsApp.Root"], has_calls: true },
        ChatApp { name: "Telegram", color: "#2AABEE", processes: &["Telegram"], has_calls: true },
        ChatApp { name: "Microsoft Teams", color: "#5B5FC7", processes: &["ms-teams", "Teams"], has_calls: true },
        ChatApp { name: "Slack", color: "#4A154B", processes: &["slack"], has_calls: true },
        ChatApp { name: "Signal", color: "#3A76F0", processes: &["Signal"], has_calls: true },
        ChatApp { name: "Outlook", color: "#0F6CBD", processes: &["olk", "OUTLOOK"], has_calls: false },
        ChatApp { name: "Mail", color: "#0F6CBD", processes: &["HxOutlook"], has_calls: false },
    ];

    /// The preset whose name appears in a notification's app name ("Telegram Desktop" -> Telegram).
    pub fn find(app_display_name: &str) -> Option<&'static ChatApp> {
        let name = app_display_name.to_lowercase();
        Self::ALL.iter().find(|a| name.contains(&a.name.to_lowercase()))
    }
}

/// Start with Windows: the island's entry under HKCU\…\Run.
pub mod autostart {
    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const VALUE_NAME: &str = "DynamicIsland";

    pub fn is_enabled() -> bool {
        crate::registry::read_string(RUN_KEY, VALUE_NAME).is_some()
    }

    /// `island_exe`: path of DynamicIsland.exe (not the settings app).
    pub fn set(enabled: bool, island_exe: &str) -> windows::core::Result<()> {
        if enabled {
            crate::registry::write_string(RUN_KEY, VALUE_NAME, &format!("\"{island_exe}\""))
        } else {
            crate::registry::delete_value(RUN_KEY, VALUE_NAME);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveTime;

    fn t(text: &str) -> NaiveTime {
        parse_time(text).unwrap()
    }

    #[test]
    fn quiet_hours() {
        for (from, to, now, quiet) in [
            ("22:00", "08:00", "23:30", true), // across midnight
            ("22:00", "08:00", "07:59", true),
            ("22:00", "08:00", "08:00", false),
            ("22:00", "08:00", "12:00", false),
            ("13:00", "14:00", "13:30", true), // same day
            ("13:00", "14:00", "14:30", false),
        ] {
            let rule = AddonQuiet { hours_on: true, from: from.into(), to: to.into(), ..Default::default() };
            assert_eq!(rule.is_quiet(t(now), None), quiet, "{from}-{to} at {now}");
        }
    }

    #[test]
    fn quiet_while_an_app_is_in_front() {
        let rule = AddonQuiet { apps: vec!["OBS64.exe".into()], ..Default::default() };
        assert!(rule.is_quiet(t("12:00"), Some("obs64")));
        assert!(!rule.is_quiet(t("12:00"), Some("chrome")));
        assert!(!rule.is_quiet(t("12:00"), None));
        assert!(!AddonQuiet::default().is_quiet(t("23:00"), None)); // hours switched off
    }

    #[test]
    fn saved_order_is_completed_with_new_kinds() {
        let order = ActivityKinds::ordered(&["Media", "Unknown", "Timer", "Media"]);
        assert_eq!(order[0], "Media");
        assert_eq!(order[1], "Timer");
        assert!(!order.iter().any(|k| k == "Unknown"));
        assert_eq!(order.len(), ActivityKinds::ALL.len());
    }

    #[test]
    fn finds_chat_app_presets() {
        for (display, preset) in [
            ("Discord", "Discord"),
            ("Telegram Desktop", "Telegram"),
            ("Microsoft Teams (work or school)", "Microsoft Teams"),
            ("whatsapp", "WhatsApp"),
        ] {
            assert_eq!(ChatApps::find(display).map(|a| a.name), Some(preset));
        }
        assert!(ChatApps::find("Spotify").is_none());
        assert!(ChatApps::find("Steam").is_none());
    }
}
