//! Fills the pages' globals from the settings and handles what they ask for.

pub mod about;
pub mod addons;
pub mod general;
pub mod media;
pub mod notifications;
pub mod welcome;

use crate::MainWindow;
use di4w_core::AddonSettings;
use slint::{ModelRc, SharedString, VecModel};

pub const GENERAL: i32 = 0;
pub const MEDIA: i32 = 1;
pub const NOTIFICATIONS: i32 = 2;
pub const ADDONS: i32 = 3;
pub const ABOUT: i32 = 4;

/// "--page About" and friends (the island's tags) -> page number.
pub fn from_tag(tag: &str) -> i32 {
    match tag.to_ascii_lowercase().as_str() {
        "media" | "music" => MEDIA,
        "notifications" => NOTIFICATIONS,
        "addons" | "add-ons" => ADDONS,
        "about" => ABOUT,
        _ => GENERAL,
    }
}

pub fn tag(page: i32) -> &'static str {
    match page {
        MEDIA => "Media",
        NOTIFICATIONS => "Notifications",
        ADDONS => "Addons",
        ABOUT => "About",
        _ => "General",
    }
}

/// Shows a page's current values (when it's opened, and after the tour or an import changed them).
pub fn load(app: &MainWindow, page: i32) {
    match page {
        MEDIA => media::load(app),
        NOTIFICATIONS => notifications::load(app),
        ADDONS => addons::load(app),
        ABOUT => about::load(app),
        _ => general::load(app),
    }
}

pub fn wire(app: &MainWindow) {
    general::wire(app);
    media::wire(app);
    notifications::wire(app);
    addons::wire(app);
    about::wire(app);
    welcome::wire(app);
}

pub fn strings<S: Into<SharedString>>(items: impl IntoIterator<Item = S>) -> ModelRc<SharedString> {
    ModelRc::new(VecModel::from(items.into_iter().map(Into::into).collect::<Vec<_>>()))
}

pub fn model<T: Clone + 'static>(items: Vec<T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(items))
}

/// An add-on's on/off switch or yes/no option by its settings name ("Battery", "WeatherFahrenheit").
pub fn addon_bool<'a>(addons: &'a mut AddonSettings, key: &str) -> Option<&'a mut bool> {
    Some(match key {
        "Volume" => &mut addons.volume,
        "Brightness" => &mut addons.brightness,
        "Battery" => &mut addons.battery,
        "Bluetooth" => &mut addons.bluetooth,
        "UsbDrives" => &mut addons.usb_drives,
        "Network" => &mut addons.network,
        "PrivacyIndicators" => &mut addons.privacy_indicators,
        "LockKeys" => &mut addons.lock_keys,
        "PerformanceWarnings" => &mut addons.performance_warnings,
        "Calendar" => &mut addons.calendar,
        "Clipboard" => &mut addons.clipboard,
        "Screenshots" => &mut addons.screenshots,
        "Downloads" => &mut addons.downloads,
        "FileCopy" => &mut addons.file_copy,
        "Calls" => &mut addons.calls,
        "DoNotDisturb" => &mut addons.do_not_disturb,
        "WindowsUpdate" => &mut addons.windows_update,
        "Weather" => &mut addons.weather,
        "WeatherFahrenheit" => &mut addons.weather_fahrenheit,
        "RainAlerts" => &mut addons.rain_alerts,
        "GameMode" => &mut addons.game_mode,
        "IslandApi" => &mut addons.island_api,
        _ => return None,
    })
}

/// An add-on's number option by its settings name ("BatteryLowPercent").
pub fn addon_int<'a>(addons: &'a mut AddonSettings, key: &str) -> Option<&'a mut i32> {
    Some(match key {
        "BatteryLowPercent" => &mut addons.battery_low_percent,
        "BatteryCriticalPercent" => &mut addons.battery_critical_percent,
        "CalendarReminderMinutes" => &mut addons.calendar_reminder_minutes,
        "FocusMinutes" => &mut addons.focus_minutes,
        "BreakMinutes" => &mut addons.break_minutes,
        "LongBreakMinutes" => &mut addons.long_break_minutes,
        "SessionsBeforeLongBreak" => &mut addons.sessions_before_long_break,
        "IslandApiPort" => &mut addons.island_api_port,
        _ => return None,
    })
}

pub fn get_bool(addons: &AddonSettings, key: &str) -> bool {
    let mut copy = addons.clone();
    addon_bool(&mut copy, key).is_some_and(|v| *v)
}

pub fn get_int(addons: &AddonSettings, key: &str) -> i32 {
    let mut copy = addons.clone();
    addon_int(&mut copy, key).map(|v| *v).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_switch_and_number_has_a_field() {
        let mut addons = AddonSettings::default();
        for key in addons::switch_keys() {
            assert!(addon_bool(&mut addons, key).is_some(), "{key}");
        }
        for key in addons::number_keys() {
            assert!(addon_int(&mut addons, key).is_some(), "{key}");
        }
        *addon_bool(&mut addons, "GameMode").unwrap() = true;
        assert!(addons.game_mode);
        *addon_int(&mut addons, "IslandApiPort").unwrap() = 5000;
        assert_eq!(addons.island_api_port, 5000);
    }

    #[test]
    fn page_tags_round_trip() {
        for page in [GENERAL, MEDIA, NOTIFICATIONS, ADDONS, ABOUT] {
            assert_eq!(from_tag(tag(page)), page);
        }
        assert_eq!(from_tag("about"), ABOUT);
        assert_eq!(from_tag("nonsense"), GENERAL);
    }
}
