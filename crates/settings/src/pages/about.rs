//! About: version and island status, the welcome tour, updates (check, download and install), backups of the
//! settings, logs, and reporting a problem.

use crate::{About, App, Dialog, MainWindow, Report, Severity, report, store, system, updater};
use di4w_core::updates::{self, UpdateInfo};
use di4w_core::{AppSettings, l, log, paths};
use slint::ComponentHandle;
use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

thread_local! {
    static UPDATE: RefCell<Option<UpdateInfo>> = const { RefCell::new(None) };
    /// Set while downloading; setting the flag cancels the download (leaving the page).
    static DOWNLOAD: RefCell<Option<Arc<AtomicBool>>> = const { RefCell::new(None) };
}

fn busy() -> bool {
    DOWNLOAD.with(|d| d.borrow().is_some())
}

/// Leaving the page mid-download stops it; the next try replaces the temp folder.
pub fn cancel_download() {
    DOWNLOAD.with(|d| {
        if let Some(cancel) = d.borrow_mut().take() {
            cancel.store(true, Ordering::Relaxed);
        }
    });
}

pub fn load(app: &MainWindow) {
    let a = app.global::<About>();
    let s = store::current();
    a.set_version(l::f("Version {0}", &[&updates::current_version()]).into());
    a.set_settings_file(paths::settings_file().display().to_string().into());
    a.set_auto_check(s.auto_check_updates);
    a.set_crash_offers(s.offer_crash_reports);
    a.set_backup_open(false);
    a.set_update_open(false);
    a.set_update_description(l::t("Updates come from the releases on GitHub.").into());
    update_status(app);
    check(app);
}

fn update_status(app: &MainWindow) {
    let a = app.global::<About>();
    let running = system::island_running();
    a.set_status(if running { l::t("Running") } else { l::t("Not running") }.into());
    let island = store::with(|s| system::island_exe(s.island_path.as_deref()));
    a.set_can_start(!running && island.is_some());
}

/// "14:05" (or "2:05 PM" in English), like the C# short time format.
fn short_time(time: chrono::DateTime<chrono::Local>) -> String {
    if l::language() == "en" { time.format("%-I:%M %p").to_string() } else { time.format("%H:%M").to_string() }
}

/// One decimal, with the decimal comma where the language uses one: "12.3" / "12,3"; "12" for 12.0.
fn one_decimal(value: f64) -> String {
    let rounded = (value * 10.0).round() / 10.0;
    let text = if rounded.fract() == 0.0 { format!("{rounded:.0}") } else { format!("{rounded:.1}") };
    if l::language() == "en" { text } else { text.replace('.', ",") }
}

fn check(app: &MainWindow) {
    if busy() {
        return; // busy installing
    }
    let a = app.global::<About>();
    a.set_checking(true);
    let weak = app.as_weak();
    std::thread::spawn(move || {
        let result = updates::check();
        let _ = slint::invoke_from_event_loop(move || {
            let Some(app) = weak.upgrade() else { return };
            let a = app.global::<About>();
            a.set_checking(false);
            match result {
                Ok(None) => {
                    a.set_update_description(l::f("You're up to date. Last checked {0:t}.", &[&short_time(chrono::Local::now())]).into());
                    a.set_update_open(false);
                    UPDATE.with(|u| *u.borrow_mut() = None);
                }
                Ok(Some(update)) => show_update(&app, update),
                Err(e) => a.set_update_description(l::f("Couldn't check for updates: {0}", &[&e]).into()),
            }
        });
    });
}

fn show_update(app: &MainWindow, update: UpdateInfo) {
    let a = app.global::<About>();
    a.set_update_description(l::f("Version {0} is available.", &[&update.version]).into());
    a.set_update_title(l::f("Version {0} is available", &[&update.version]).into());
    let size = updater::pick_asset(&update).map(|asset| asset.size).unwrap_or(0);
    a.set_update_message(
        l::f("You have {0}. Download size {1:0.#} MB.", &[&updates::current_version(), &one_decimal(size as f64 / 1_048_576.0)]).into(),
    );
    a.set_update_severity(Severity::Informational);
    a.set_release_notes(updater::plain_text(&update.notes).into());
    a.set_downloading(false);
    if updater::install_folder().is_none() {
        a.set_install_enabled(false);
        a.set_install_text(l::t("Only available in the published app").into());
    } else {
        a.set_install_enabled(true);
        a.set_install_text(l::t("Download and install").into());
    }
    a.set_update_open(true);
    UPDATE.with(|u| *u.borrow_mut() = Some(update));
}

fn install(app: &MainWindow) {
    let (Some(update), Some(folder)) = (UPDATE.with(|u| u.borrow().clone()), updater::install_folder()) else { return };
    if busy() {
        return;
    }
    let cancel = Arc::new(AtomicBool::new(false));
    DOWNLOAD.with(|d| *d.borrow_mut() = Some(cancel.clone()));
    let a = app.global::<About>();
    a.set_busy(true);
    a.set_install_enabled(false);
    a.set_install_text(l::t("Downloading…").into());
    a.set_downloading(true);
    a.set_progress(0.0);

    let weak = app.as_weak();
    std::thread::spawn(move || {
        let progress_weak = weak.clone();
        let progress = move |p: f64| {
            let weak = progress_weak.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(app) = weak.upgrade() {
                    app.global::<About>().set_progress(p as f32);
                }
            });
        };
        let result = updater::download(&update, &progress, &cancel);
        let _ = slint::invoke_from_event_loop(move || {
            let cancelled = cancel.load(Ordering::Relaxed);
            DOWNLOAD.with(|d| *d.borrow_mut() = None);
            let Some(app) = weak.upgrade() else { return };
            let a = app.global::<About>();
            a.set_busy(false);
            if cancelled {
                return; // left the page mid-download
            }
            let failed = |message: String| {
                log::error_with("Update failed", &message);
                a.set_update_severity(Severity::Error);
                a.set_update_title(l::t("The update couldn't be installed").into());
                a.set_update_message(message.into());
                a.set_install_text(l::t("Try again").into());
                a.set_install_enabled(true);
                a.set_downloading(false);
            };
            match result {
                Ok(payload) => {
                    a.set_install_text(l::t("Installing…").into());
                    store::flush();
                    match updater::install(&payload, &folder, update.version) {
                        // This app closes; the island restarts on the new version.
                        Ok(()) => crate::quit(),
                        Err(updater::InstallError::Declined) => failed(l::t("Administrator permission is needed to update this folder.").into()),
                        Err(updater::InstallError::Other(e)) => failed(e),
                    }
                }
                Err(e) => failed(e),
            }
        });
    });
}

// ───────────────────────────── backup ─────────────────────────────

fn show_backup(app: &MainWindow, severity: Severity, message: String) {
    let a = app.global::<About>();
    a.set_backup_severity(severity);
    a.set_backup_message(message.into());
    a.set_backup_open(true);
}

fn documents_folder() -> Option<std::path::PathBuf> {
    std::env::var_os("USERPROFILE").map(|p| std::path::PathBuf::from(p).join("Documents")).filter(|p| p.is_dir())
}

fn export(app: &MainWindow) {
    let mut dialog = rfd::FileDialog::new()
        .add_filter(l::t("Settings file"), &["json"])
        .set_file_name(format!("Dynamic Island settings {}.json", chrono::Local::now().format("%Y-%m-%d")))
        .set_parent(&app.window().window_handle());
    if let Some(folder) = documents_folder() {
        dialog = dialog.set_directory(folder);
    }
    let Some(file) = dialog.save_file() else { return };
    store::flush();
    match std::fs::copy(paths::settings_file(), &file) {
        Ok(_) => show_backup(app, Severity::Success, l::f("Saved to {0}", &[&file.display()])),
        Err(e) => show_backup(app, Severity::Error, l::f("Couldn't save the file: {0}", &[&e])),
    }
}

/// Reads a backup; any JSON would read as "all defaults", so ours must have the add-ons section.
pub fn parse_backup(json: &str) -> Option<AppSettings> {
    if !json.contains("\"Addons\"") {
        return None;
    }
    AppSettings::parse(json).ok()
}

fn import(app: &MainWindow) {
    let mut dialog = rfd::FileDialog::new().add_filter(l::t("Settings file"), &["json"]).set_parent(&app.window().window_handle());
    if let Some(folder) = documents_folder() {
        dialog = dialog.set_directory(folder);
    }
    let Some(file) = dialog.pick_file() else { return };
    let imported = match std::fs::read_to_string(&file) {
        Ok(json) => parse_backup(&json),
        Err(e) => {
            log::info(format!("Settings import failed: {e}"));
            None
        }
    };
    let Some(imported) = imported else {
        show_backup(app, Severity::Error, l::t("That isn't a Dynamic Island settings file.").into());
        return;
    };
    store::flush();
    AppSettings::update(move |latest| {
        // This PC's own facts stay: where the island is installed, and that the tour was seen.
        let island_path = latest.island_path.take();
        *latest = imported.clone();
        latest.island_path = island_path;
        latest.welcome_shown = true;
    });
    crate::restart(super::ABOUT); // every page shows the imported values; the island picks them up by itself
}

// ───────────────────────────── report a problem ─────────────────────────────

/// The report form; with `crash` (from crash.txt) the crash details are filled in too.
pub fn open_report(app: &MainWindow, crash: Option<&str>) {
    let facts = report::Facts {
        install_type: updater::install_type(),
        monitor: store::with(|s| s.monitor.clone()),
        island_running: system::island_running(),
    };
    let r = app.global::<Report>();
    r.set_title(crash.map(report::crash_title).unwrap_or_default().into());
    r.set_body(report::build(crash, &facts).into());
    app.global::<App>().set_dialog(Dialog::Report);
}

pub fn wire(app: &MainWindow) {
    let a = app.global::<About>();

    let weak = app.as_weak();
    a.on_start_island(move || {
        crate::start_island();
        let weak = weak.clone();
        slint::Timer::single_shot(Duration::from_millis(1500), move || {
            if let Some(app) = weak.upgrade() {
                update_status(&app);
            }
        });
    });
    let weak = app.as_weak();
    a.on_show_welcome(move || {
        if let Some(app) = weak.upgrade() {
            super::welcome::show(&app);
        }
    });
    let weak = app.as_weak();
    a.on_check(move || {
        if let Some(app) = weak.upgrade() {
            check(&app);
        }
    });
    let weak = app.as_weak();
    a.on_install(move || {
        if let Some(app) = weak.upgrade() {
            install(&app);
        }
    });
    a.on_open_notes(|| {
        if let Some(url) = UPDATE.with(|u| u.borrow().as_ref().map(|u| u.page_url.clone())) {
            system::open(&url);
        }
    });
    a.on_auto_check_toggled(|on| store::change(move |s| s.auto_check_updates = on));
    a.on_crash_offers_toggled(|on| store::change(move |s| s.offer_crash_reports = on));
    a.on_open_settings_file(|| system::open(&paths::settings_file().display().to_string()));
    let weak = app.as_weak();
    a.on_export_settings(move || {
        if let Some(app) = weak.upgrade() {
            export(&app);
        }
    });
    let weak = app.as_weak();
    a.on_import_settings(move || {
        if let Some(app) = weak.upgrade() {
            import(&app);
        }
    });
    let weak = app.as_weak();
    a.on_backup_closed(move || {
        if let Some(app) = weak.upgrade() {
            app.global::<About>().set_backup_open(false);
        }
    });
    a.on_open_logs(|| {
        let folder = paths::log_folder();
        let _ = std::fs::create_dir_all(&folder);
        system::open(&folder.display().to_string());
    });
    let weak = app.as_weak();
    a.on_report(move || {
        if let Some(app) = weak.upgrade() {
            open_report(&app, None);
        }
    });

    let r = app.global::<Report>();
    r.on_open_on_github(|title, body| {
        // Long reports don't fit in a link: open an empty issue and let the user paste (it's on the clipboard).
        if body.encode_utf16().count() > report::MAX_BODY_IN_LINK {
            system::copy_text(&body);
        }
        system::open(&report::issue_link(&title, &body));
    });
    r.on_copy(|body| system::copy_text(&body));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_only_settings_files() {
        assert!(parse_backup("{}").is_none());
        assert!(parse_backup("{\"Addons\": nonsense").is_none());
        let settings = AppSettings { theme: "Light".into(), ..Default::default() };
        assert_eq!(parse_backup(&settings.to_json()).map(|s| s.theme), Some("Light".to_string()));
    }

    #[test]
    fn formats_sizes_with_one_decimal() {
        assert_eq!(one_decimal(12.34), "12.3");
        assert_eq!(one_decimal(12.0), "12");
    }
}
