//! Startup and the app around the island (the C# app's `App`): one instance at a time, the tray icon and menu,
//! global shortcuts, live settings (restarting for the few that need it), update checks, crash reports, and opening
//! the settings app.

use crate::activities::api::IslandApi;
use crate::activities::api::server::Server;
use crate::activities::focus::FocusActivity;
use crate::activities::media::MediaActivity;
use crate::activities::notifications::service::{self as notifications, Filter};
use crate::activities::notifications::Presenter;
use crate::activities::stopwatch::StopwatchActivity;
use crate::activities::timer::{TimerActivity, picker, recent};
use crate::addons::{Context, Host};
use crate::island::{Island, Panel};
use crate::model::{Alert, Manager, power};
use crate::rt::{self, Change, Timer, WideNull, Watcher};
use crate::ui::colors;
use crate::ui::menu::{self, Item};
use crate::ui::{self, ICON_SIZE, icons};
use crate::{interop, tray};
use di4w_core::updates::{self, Version};
use di4w_core::{AppSettings, l, paths, theme};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_ABANDONED, WAIT_OBJECT_0};
use windows::Win32::Storage::FileSystem::{FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE, FILE_NOTIFY_CHANGE_SIZE};
use windows::Win32::System::Threading::{CreateEventW, CreateMutexW, INFINITE, ReleaseMutex, WaitForSingleObject};
use windows::Win32::UI::WindowsAndMessaging::{MB_ICONWARNING, MB_OK, MessageBoxW};
use windows::core::{PCWSTR, w};

const SETTINGS_PROCESSES: [&str; 2] = ["DynamicIsland.Settings", "DynamicIsland-Settings"];

pub struct App {
    settings: Rc<RefCell<AppSettings>>,
    manager: Rc<Manager>,
    island: Rc<Island>,
    timer: Rc<TimerActivity>,
    focus: Rc<FocusActivity>,
    stopwatch: Rc<StopwatchActivity>,
    media: Rc<MediaActivity>,
    addons: Rc<Host>,
    notification_filter: Arc<Mutex<Filter>>,
    paused: Cell<bool>,
    restart: Cell<bool>,
    light: bool,
    pending_update: Cell<Option<Version>>,
    announced_update: Cell<Option<Version>>,
    server: RefCell<Option<(Rc<IslandApi>, Server)>>,
    timers: RefCell<Vec<Timer>>,
    watcher: RefCell<Option<Watcher>>,
}

thread_local! {
    static APP: RefCell<Option<Rc<App>>> = const { RefCell::new(None) };
}

fn app() -> Option<Rc<App>> {
    APP.with(|a| a.borrow().clone())
}

/// The single-instance mutex, released when dropped.
struct Instance(HANDLE);

impl Drop for Instance {
    fn drop(&mut self) {
        unsafe {
            let _ = ReleaseMutex(self.0);
            let _ = CloseHandle(self.0);
        }
    }
}

/// Runs the island until it quits. Returns false when another island already runs.
pub fn run(restarted: bool) -> bool {
    // After a self-restart the old process may still be shutting down; give it a moment.
    let instance = unsafe {
        let Ok(mutex) = CreateMutexW(None, false, w!("DynamicIsland4Windows.SingleInstance")) else { return false };
        let wait = WaitForSingleObject(mutex, if restarted { 10_000 } else { 0 });
        if wait != WAIT_OBJECT_0 && wait != WAIT_ABANDONED {
            let _ = CloseHandle(mutex);
            return false;
        }
        Instance(mutex)
    };

    rt::init();
    // The settings app sets this before installing an update, so the island exits cleanly (tray icon, port, mutex).
    listen_for_quit();

    di4w_core::log::info(format!("Starting Dynamic Island {}", updates::current_version()));
    let mut loaded = AppSettings::load();
    l::use_language(&loaded.language);
    // Before any view is built: the colors are read once.
    let light = theme::is_light(&loaded.theme);
    colors::set_light(light);
    let exe = std::env::current_exe().ok().map(|p| p.to_string_lossy().to_string());
    if loaded.island_path != exe {
        loaded = AppSettings::update(|s| s.island_path = exe.clone());
    }
    power::start();
    let settings = Rc::new(RefCell::new(loaded));
    let manager = Manager::new();
    manager.set_order(&settings.borrow().activity_order);

    // Activities
    let timer = TimerActivity::new(manager.clone(), settings.clone());
    manager.register(timer.clone());
    let focus = FocusActivity::new(manager.clone(), settings.clone());
    manager.register(focus.clone());
    let stopwatch = StopwatchActivity::new(manager.clone());
    manager.register(stopwatch.clone());
    let media = MediaActivity::new(manager.clone(), settings.clone());
    manager.register(media.clone());

    // Window
    let island = Island::create(manager.clone(), settings.clone());
    let ctx = Context::new(settings.clone(), manager.clone(), island.clone());
    let addons = Host::new(ctx);
    let filter = Arc::new(Mutex::new(Filter { enabled: settings.borrow().notifications_enabled, apps: settings.borrow().notification_apps.clone() }));

    let this = Rc::new(App {
        settings: settings.clone(),
        manager: manager.clone(),
        island: island.clone(),
        timer,
        focus,
        stopwatch,
        media: media.clone(),
        addons: addons.clone(),
        notification_filter: filter.clone(),
        paused: Cell::new(false),
        restart: Cell::new(false),
        light,
        pending_update: Cell::new(None),
        announced_update: Cell::new(None),
        server: RefCell::new(None),
        timers: RefCell::new(Vec::new()),
        watcher: RefCell::new(None),
    });
    APP.with(|a| *a.borrow_mut() = Some(this.clone()));

    *island.idle_panel.borrow_mut() = Some(Rc::new(|| app().map(|a| a.picker(0)).unwrap()));
    *island.context_menu.borrow_mut() = Some(Rc::new(|x, y| {
        if let Some(a) = app() {
            menu::show(a.menu_items(), x, y);
        }
    }));
    *island.hotkey_pressed.borrow_mut() = Some(Rc::new(|| {
        if let Some(a) = app() {
            a.set_paused(!a.paused.get(), true);
        }
    }));
    // Shown again: an update found while it was hidden is announced now (after the window has caught up).
    *island.on_shown.borrow_mut() = Some(Rc::new(|| {
        if let Some(a) = app() {
            a.announce_update();
        }
    }));
    *island.on_close.borrow_mut() = Some(Rc::new(|| rt::quit("the island window was closed")));
    // The settings app's Preview buttons send "preview:Battery" (WM_COPYDATA) to show a sample pop-up.
    island.add_hook(|msg, _, lparam| {
        const WM_COPYDATA: u32 = 0x004A;
        if msg != WM_COPYDATA {
            return;
        }
        let Some(text) = (unsafe { di4w_core::messages::read(lparam) }) else { return };
        if let Some(key) = text.strip_prefix(di4w_core::messages::PREVIEW_PREFIX) {
            let key = key.to_string();
            // Not while handling SendMessage: the settings app waits for it to return.
            rt::defer(move || {
                if let Some(a) = app() {
                    a.addons.preview(&key);
                }
            });
        }
    });
    island.show();
    island.register_hide_hotkey(&settings.borrow().hide_hotkey);
    island.register_expand_hotkey(&settings.borrow().expand_hotkey);

    addons.sync();

    tray::create(l::t("Dynamic Island"), |event| {
        let Some(a) = app() else { return };
        match event {
            // One click brings a hidden island back; a double-click opens the settings.
            tray::Event::Click => {
                if a.paused.get() {
                    a.set_paused(false, true);
                }
            }
            tray::Event::DoubleClick => open_settings(&[]),
            tray::Event::Menu(x, y) => menu::show(a.menu_items(), x, y),
            tray::Event::ThemeChanged => a.windows_theme_changed(),
        }
    });
    if settings.borrow().paused {
        this.set_paused(true, false); // hidden before the restart: stay hidden
    }
    this.watch_settings();
    this.schedule_update_checks();
    this.offer_crash_report();

    // A fresh install: show the welcome tour in the settings app once the island is up (the app marks it shown).
    if !settings.borrow().welcome_shown {
        rt::after(Duration::from_millis(500), || open_settings(&["--welcome"]));
    }

    // The Island API: scripts on this PC show pop-ups and progress (/alert, /progress).
    if settings.borrow().addons.island_api {
        let port = settings.borrow().addons.island_api_port.clamp(1, 65535) as u16;
        *this.server.borrow_mut() = IslandApi::start(manager.clone(), port);
    }

    let presenter = Rc::new(Presenter::new(manager.clone(), settings.clone()));
    if settings.borrow().media_enabled {
        media.start();
    }
    let host = addons.clone();
    notifications::start(filter, move |n| presenter.show(n), move |status| host.set_notification_access(status));

    rt::run();

    // Shutting down: out of sight at once, and never stuck (a hung driver call can't keep a zombie island around).
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(5));
        di4w_core::log::error("Shutting down took too long; exiting");
        unsafe {
            let _ = windows::Win32::System::Threading::TerminateProcess(windows::Win32::System::Threading::GetCurrentProcess(), 0);
        }
    });
    let restart = this.restart.get();
    island.hide();
    tray::remove();
    menu::close_all();
    this.watcher.borrow_mut().take();
    this.server.borrow_mut().take();
    this.addons.shutdown();
    crate::activities::media::meter::stop();
    di4w_core::log::info("Stopped");
    APP.with(|a| a.borrow_mut().take());
    drop(instance);
    if restart && let Ok(exe) = std::env::current_exe() {
        interop::run(&exe.to_string_lossy(), "--restart", None, false);
    }
    true
}

/// Waits for the named event the settings app signals before an update; then quits cleanly.
fn listen_for_quit() {
    let name = updates::ISLAND_QUIT_EVENT.encode_wide_null();
    let Ok(event) = (unsafe { CreateEventW(None, false, false, PCWSTR(name.as_ptr())) }) else { return };
    let raw = event.0 as isize;
    std::thread::spawn(move || {
        let event = HANDLE(raw as *mut _);
        if unsafe { WaitForSingleObject(event, INFINITE) } == WAIT_OBJECT_0 {
            di4w_core::log::info("Asked to quit (update)");
            rt::post(|| rt::quit("asked to by the settings app (update)"));
        }
    });
}

impl App {
    fn start_timer(&self, duration: Duration) {
        self.timer.start(duration);
        self.island.close_panel();
    }

    fn start_focus(&self) {
        self.focus.start();
        self.island.close_panel();
    }

    fn start_stopwatch(&self) {
        self.stopwatch.start();
        self.island.close_panel();
    }

    /// The panel that opens from the idle island (`tab` 3: Recent).
    fn picker(&self, tab: usize) -> Panel {
        recent::reset_scroll();
        let manager = self.manager.clone();
        let actions = picker::Actions {
            start_timer: Rc::new(|d| {
                if let Some(a) = app() {
                    a.start_timer(d);
                }
            }),
            start_focus: Rc::new(|| {
                if let Some(a) = app() {
                    a.start_focus();
                }
            }),
            start_stopwatch: Rc::new(|| {
                if let Some(a) = app() {
                    a.start_stopwatch();
                }
            }),
            recent: Rc::new(move || {
                recent::build(&manager.history(), Rc::new(|| {
                    if let Some(a) = app() {
                        a.island.close_panel();
                    }
                }))
            }),
        };
        picker::build(actions, self.settings.borrow().addons.clone(), tab)
    }

    /// The panel on its Recent tab (from the menu).
    fn show_recent(&self) {
        if self.paused.get() {
            self.set_paused(false, true);
        }
        self.island.open_panel(self.picker(3));
    }

    // ───────────────────────────── menu ─────────────────────────────

    /// One menu used by both the tray icon and right-clicking the island, built when it opens so it's current.
    fn menu_items(&self) -> Vec<Item> {
        let call = |f: fn(&App)| {
            move || {
                if let Some(a) = app() {
                    f(&a);
                }
            }
        };
        let mut items = Vec::new();
        // The island itself: open the settings (also a double-click on the tray icon), and show or hide it.
        items.push(Item::new(l::t("Open settings"), Some(icons::SETTINGS), || open_settings(&[])).bold());
        let shortcut = { let s = self.settings.borrow(); if di4w_core::hotkey::parse(&s.hide_hotkey).is_some() { s.hide_hotkey.clone() } else { String::new() } };
        let visibility = if self.paused.get() {
            Item::new(l::t("Show island"), Some(icons::VIEW), call(|a| a.set_paused(false, true)))
        } else {
            Item::new(l::t("Hide island"), Some(icons::HIDE), call(|a| a.set_paused(true, true)))
        };
        items.push(visibility.gesture(&shortcut));
        items.push(Item::Separator);

        // Timers.
        let mut timer: Vec<Item> = [1u64, 3, 5, 10, 15, 25, 45, 60]
            .iter()
            .map(|&m| {
                Item::new(l::f("{0} minutes", &[&m]), None, move || {
                    if let Some(a) = app() {
                        a.start_timer(Duration::from_secs(m * 60));
                    }
                })
            })
            .collect();
        timer.push(Item::Separator);
        timer.push(Item::new(l::t("Custom…"), None, call(|a| {
            if a.paused.get() {
                a.set_paused(false, true);
            }
            a.island.open_panel(a.picker(0));
        })));
        items.push(Item::submenu(l::t("Start timer"), Some(icons::TIMER), timer));
        use crate::model::Activity;
        if self.timer.is_active() {
            items.push(Item::new(l::t("Cancel timer"), Some(icons::CANCEL), call(|a| a.timer.cancel())));
        }
        items.push(if self.focus.is_active() {
            Item::new(l::t("Stop focus session"), Some(icons::STOP), call(|a| a.focus.stop()))
        } else {
            Item::new(l::t("Start focus session"), Some(icons::PLAY_GLYPH), call(|a| a.start_focus()))
        });
        items.push(if self.stopwatch.is_active() {
            Item::new(l::t("Stop stopwatch"), Some(icons::STOP), call(|a| a.stopwatch.stop()))
        } else {
            Item::new(l::t("Start stopwatch"), Some(icons::TIMER), call(|a| a.start_stopwatch()))
        });
        items.push(Item::new(l::t("Recent alerts…"), Some(icons::RECENT), call(|a| a.show_recent())));
        items.push(Item::Separator);
        items.push(Item::new(l::t("Check for updates…"), Some(icons::SYNC), || open_settings(&["--page", "About"])));
        items.push(Item::new(l::t("Quit"), Some(icons::POWER), || rt::quit("Quit in the menu")));
        items
    }

    /// Hides or shows the island. Used by the menu and the hide shortcut; saved, so a hidden island stays hidden
    /// after a restart; the tray tooltip says so.
    fn set_paused(&self, paused: bool, save: bool) {
        self.paused.set(paused);
        self.island.paused.set(paused);
        if paused {
            self.island.hide();
        } else {
            self.island.show();
        }
        tray::set_paused(paused, if paused { l::t("Dynamic Island is hidden. Click to show it.") } else { l::t("Dynamic Island") });
        if save && self.settings.borrow().paused != paused {
            AppSettings::update(|s| s.paused = paused);
        }
        self.settings.borrow_mut().paused = paused;
    }

    // ───────────────────────────── live settings ─────────────────────────────

    /// The settings app writes settings.json; pick up its changes without a restart.
    fn watch_settings(self: &Rc<Self>) {
        let folder = paths::data_folder();
        let _ = std::fs::create_dir_all(&folder);
        let file = paths::settings_file();
        let name = file.file_name().map(|n| n.to_os_string()).unwrap_or_default();
        let debounce: Rc<RefCell<Option<Timer>>> = Rc::new(RefCell::new(None));
        let watcher = Watcher::new(&folder, FILE_NOTIFY_CHANGE_LAST_WRITE | FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_SIZE, move |changes| {
            let touched = changes.iter().any(|c| {
                let p: &PathBuf = match c {
                    Change::Created(p) | Change::Modified(p) | Change::Deleted(p) => p,
                    Change::Renamed(_, p) => p,
                };
                p.file_name() == Some(name.as_os_str())
            });
            if touched {
                *debounce.borrow_mut() = Some(Timer::once(Duration::from_millis(400), || {
                    if let Some(a) = app() {
                        a.reload_settings();
                    }
                }));
            }
        });
        match watcher {
            Ok(w) => *self.watcher.borrow_mut() = Some(w),
            Err(e) => di4w_core::log::error_with("Could not watch the settings", e),
        }
    }

    fn reload_settings(&self) {
        let Some(updated) = AppSettings::try_load() else { return }; // half-written or locked; the next change retries
        let (needs_restart, hide_changed, expand_changed) = {
            let current = self.settings.borrow();
            // These are wired up once at startup, so changing them restarts the island.
            (
                updated.addons.island_api_port != current.addons.island_api_port
                    || updated.media_enabled != current.media_enabled
                    || updated.addons.island_api != current.addons.island_api
                    || updated.language != current.language
                    || theme::is_light(&updated.theme) != self.light,
                updated.hide_hotkey != current.hide_hotkey,
                updated.expand_hotkey != current.expand_hotkey,
            )
        };
        if updated.paused != self.paused.get() {
            self.set_paused(updated.paused, false);
        }
        *self.settings.borrow_mut() = updated;
        let s = self.settings.borrow().clone();
        self.manager.set_order(&s.activity_order);
        self.island.apply_scale();
        if hide_changed {
            self.island.register_hide_hotkey(&s.hide_hotkey);
        }
        if expand_changed {
            self.island.register_expand_hotkey(&s.expand_hotkey);
        }
        *self.notification_filter.lock().unwrap_or_else(|e| e.into_inner()) = Filter { enabled: s.notifications_enabled, apps: s.notification_apps.clone() };
        self.media.settings_changed();
        self.addons.sync();
        if needs_restart {
            di4w_core::log::info("Restarting to apply settings");
            self.restart.set(true);
            rt::quit("restarting for new settings");
        }
    }

    /// With the theme set to follow Windows, switching Windows between light and dark restarts the island in the
    /// new colors.
    fn windows_theme_changed(&self) {
        let setting = self.settings.borrow().theme.clone();
        if setting != theme::SYSTEM || theme::is_light(&setting) == self.light || self.restart.get() {
            return;
        }
        di4w_core::log::info("Windows switched between light and dark; restarting in the new colors");
        self.restart.set(true);
        rt::quit("restarting in the new colors");
    }

    // ───────────────────────────── crash reports ─────────────────────────────

    /// The island crashed last time: say so, and offer the settings app's report form with the details filled in.
    /// Only the user can send it (they see it all in the form first); ignoring the alert forgets the crash.
    fn offer_crash_report(&self) {
        if di4w_core::crash::read().is_none() {
            return;
        }
        if !self.settings.borrow().offer_crash_reports {
            di4w_core::crash::clear();
            return;
        }
        let reporting = Rc::new(Cell::new(false));
        let r = reporting.clone();
        let alert = Alert::new(|| {
            ui::alert_view(
                ui::glyph_circle(icons::WARNING, colors::orange(), ICON_SIZE, 22.0),
                l::t("Dynamic Island closed unexpectedly"),
                l::t("Click to report it. You see everything before anything is sent."),
                None,
                None,
            )
        })
        .seconds(10.0)
        .sticky_until_hover(true)
        .on_click(move || {
            r.set(true);
            open_settings(&["--report-crash"]); // the settings app clears the crash once it has read it
        })
        .on_dismissed(move || {
            if !reporting.get() {
                di4w_core::crash::clear();
            }
        })
        .rc();
        // A little after startup, so it isn't lost among the first activities.
        let manager = self.manager.clone();
        rt::after(Duration::from_secs(4), move || manager.show_alert(alert));
    }

    // ───────────────────────────── updates ─────────────────────────────

    /// First check shortly after startup (not to slow it down), then every few hours.
    fn schedule_update_checks(&self) {
        let first = Timer::once(Duration::from_secs(30), || {
            if let Some(a) = app() {
                a.check_for_updates();
                a.timers.borrow_mut().push(Timer::every(Duration::from_secs(6 * 3600), || {
                    if let Some(a) = app() {
                        a.check_for_updates();
                    }
                }));
            }
        });
        self.timers.borrow_mut().push(first);
    }

    fn check_for_updates(&self) {
        if !self.settings.borrow().auto_check_updates {
            return;
        }
        rt::spawn(updates::check, |result| {
            let Some(a) = app() else { return };
            match result {
                Ok(Some(update)) if Some(update.version) != a.announced_update.get() => {
                    // Announce each version once per run.
                    di4w_core::log::info(format!("Update available: {}", update.version));
                    a.pending_update.set(Some(update.version));
                    a.announce_update();
                }
                Ok(_) => {}
                Err(e) => di4w_core::log::info(format!("Update check failed: {e}")),
            }
        });
    }

    /// "Update available", once per version. While the island is hidden it waits (it would only land in Recent);
    /// showing the island announces it then.
    fn announce_update(&self) {
        let Some(version) = self.pending_update.get() else { return };
        if self.manager.suppressed() {
            return;
        }
        self.pending_update.set(None);
        self.announced_update.set(Some(version));
        let alert = Alert::new(move || {
            ui::alert_view(ui::glyph_circle(icons::UPDATE, colors::blue(), ICON_SIZE, 22.0), l::t("Update available"), &l::f("Version {0} is ready. Click to install.", &[&version]), None, None)
        })
        .seconds(8.0)
        .sticky_until_hover(true)
        .on_click(|| open_settings(&["--page", "About"]));
        self.manager.show_alert(alert.rc());
    }
}

// ───────────────────────────── settings app ─────────────────────────────

/// The settings app: `<island folder>\Settings\DynamicIsland.Settings.exe` when installed, or next to the island
/// (a development build).
fn find_settings_app() -> Option<PathBuf> {
    let dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    [dir.join("Settings").join("DynamicIsland.Settings.exe"), dir.join("DynamicIsland.Settings.exe"), dir.join("DynamicIsland-Settings.exe")]
        .into_iter()
        .find(|p| p.exists())
}

/// Opens the settings app, or brings it to the front when it's already open. `args`: e.g. `--page About` to open a
/// specific page (only when the app isn't open yet).
pub fn open_settings(args: &[&str]) {
    for name in SETTINGS_PROCESSES {
        if interop::is_running(name) {
            interop::focus_process(name);
            return;
        }
    }
    let Some(exe) = find_settings_app() else {
        di4w_core::log::error("Settings app not found next to the island");
        let text = l::t("The settings app (DynamicIsland.Settings.exe) was not found. Reinstall Dynamic Island, or publish both apps with: dotnet publish src/DynamicIsland -c Release -o publish").encode_wide_null();
        let caption = l::t("Dynamic Island").encode_wide_null();
        unsafe {
            MessageBoxW(None, PCWSTR(text.as_ptr()), PCWSTR(caption.as_ptr()), MB_OK | MB_ICONWARNING);
        }
        return;
    };
    let quoted: Vec<String> = args.iter().map(|a| interop::quote_arg(a)).collect();
    let dir = exe.parent().map(|d| d.to_string_lossy().to_string());
    interop::run(&exe.to_string_lossy(), &quoted.join(" "), dir.as_deref(), false);
}
