//! settings.json on disk. Each test uses its own temporary folder; never touches the real %AppData%\DynamicIsland.
//! The data folder is global, so the tests here run one at a time.

use di4w_core::{AppSettings, paths, theme};
use std::path::PathBuf;
use std::sync::Mutex;

static SERIAL: Mutex<()> = Mutex::new(());

struct TempFolder(PathBuf, #[allow(dead_code)] std::sync::MutexGuard<'static, ()>);

impl TempFolder {
    fn new() -> Self {
        let guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let folder = std::env::temp_dir().join(format!("DI4W-tests-{}-{nanos}", std::process::id()));
        paths::set_data_folder(Some(folder.clone()));
        Self(folder, guard)
    }

    fn write(&self, json: &str) {
        std::fs::create_dir_all(&self.0).unwrap();
        std::fs::write(paths::settings_file(), json).unwrap();
    }
}

impl Drop for TempFolder {
    fn drop(&mut self) {
        paths::set_data_folder(None);
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn update_keeps_what_the_other_app_wrote() {
    let _folder = TempFolder::new();
    // The settings app opened earlier and holds an old copy...
    AppSettings::update(|s| s.sounds = true);
    let mut stale = AppSettings::load();
    // ...meanwhile the island records its path.
    AppSettings::update(|s| s.island_path = Some(r"C:\Apps\DynamicIsland.exe".into()));
    // The settings app changes one thing; the island's path must survive.
    stale.sounds = false;
    AppSettings::update(|s| s.sounds = stale.sounds);

    let saved = AppSettings::load();
    assert!(!saved.sounds);
    assert_eq!(saved.island_path.as_deref(), Some(r"C:\Apps\DynamicIsland.exe"));
}

#[test]
fn broken_file_is_kept_aside_and_defaults_are_used() {
    let folder = TempFolder::new();
    folder.write(r#"{ "Scale": 1.2, oops"#);

    let settings = AppSettings::load();

    assert_eq!(settings.scale, 1.0);
    let bad = std::fs::read_to_string(folder.0.join("settings.json.bad")).unwrap();
    assert!(bad.contains("oops"));
}

#[test]
fn try_load_returns_none_for_missing_or_broken_files() {
    let folder = TempFolder::new();
    assert!(AppSettings::try_load().is_none());
    folder.write("not json");
    assert!(AppSettings::try_load().is_none());
}

#[test]
fn old_files_without_add_on_settings_get_defaults() {
    let folder = TempFolder::new();
    folder.write(r#"{ "Scale": 1.2, "Sounds": false }"#);

    let settings = AppSettings::load();

    assert_eq!(settings.scale, 1.2);
    assert!(!settings.sounds);
    assert!(settings.addons.volume);
    assert_eq!(settings.hide_hotkey, "Ctrl+Alt+I");
    assert!(settings.welcome_shown);
}

#[test]
fn a_fresh_install_gets_the_welcome_tour() {
    let _folder = TempFolder::new();
    assert!(!AppSettings::load().welcome_shown);
}

#[test]
fn port_from_1x_moves_to_the_island_api() {
    let folder = TempFolder::new();
    folder.write(r#"{ "ClaudeEnabled": true, "ClaudePort": 50000 }"#);

    let settings = AppSettings::load();

    assert_eq!(settings.addons.island_api_port, 50000);
    settings.save();
    assert!(!std::fs::read_to_string(paths::settings_file()).unwrap().contains("ClaudePort"));
}

#[test]
fn files_from_the_csharp_apps_load() {
    let folder = TempFolder::new();
    folder.write(
        r#"{
  "TopOffset": 12,
  "Theme": "Light",
  "ActivityOrder": [ "Media", "Timer" ],
  "Addons": { "Weather": false, "IslandApiPort": 47900, "Quiet": { "Clipboard": { "HoursOn": true, "From": "22:00", "To": "08:00", "Apps": [ "obs64" ] } } },
  "IslandPath": null
}"#,
    );

    let settings = AppSettings::load();

    assert_eq!(settings.top_offset, 12.0);
    assert!(theme::is_light(&settings.theme));
    assert_eq!(settings.activity_order, ["Media", "Timer"]);
    assert!(!settings.addons.weather);
    assert_eq!(settings.addons.island_api_port, 47900);
    assert_eq!(settings.addons.quiet["Clipboard"].apps, ["obs64"]);
}

#[test]
fn shortcut_text_is_saved_readably() {
    let _folder = TempFolder::new();
    AppSettings::update(|s| s.hide_hotkey = "Ctrl+Alt+I".into());
    assert!(std::fs::read_to_string(paths::settings_file()).unwrap().contains(r#""Ctrl+Alt+I""#));
}

#[test]
fn dark_is_the_default_theme() {
    assert!(!theme::is_light(&AppSettings::default().theme));
    assert!(theme::is_light(theme::LIGHT));
    assert!(!theme::is_light("something else"));
}
