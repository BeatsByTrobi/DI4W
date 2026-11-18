//! The settings app (DynamicIsland.Settings.exe): a Windows 11 Settings-style window over settings.json.
//!
//! Command line (the island passes these):
//!   --page <General|Media|Notifications|Addons|About>   open on that page
//!   --welcome                                           show the welcome tour
//!   --report-crash                                      report the island's last crash (crash.txt)
//!   --apply-update …                                    the update's install step (see updater.rs)
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

slint::include_modules!();

mod pages;
mod report;
mod shortcut;
mod store;
mod system;
mod updater;
mod window;

use di4w_core::{crash, l, log, theme};
use slint::{ComponentHandle, Model, SharedString};
use std::cell::RefCell;

thread_local! {
    static INSTANCE: RefCell<Option<system::SingleInstance>> = const { RefCell::new(None) };
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).is_some_and(|a| a == "--apply-update") {
        std::process::exit(updater::apply_update(&args[1..]));
    }

    let Some(instance) = system::SingleInstance::acquire() else {
        return; // already open: that window was brought forward
    };
    INSTANCE.with(|i| *i.borrow_mut() = Some(instance));

    // "--page About" opens a specific page (the island uses it for "Update available").
    let value_after = |flag: &str| args.iter().position(|a| a.eq_ignore_ascii_case(flag)).and_then(|i| args.get(i + 1));
    let mut page = value_after("--page").map(|p| pages::from_tag(p)).unwrap_or(pages::GENERAL);
    let has = |flag: &str| args.iter().any(|a| a.eq_ignore_ascii_case(flag));

    // Language first: pages are translated as they are built.
    let settings = store::current();
    l::use_language(&settings.language);

    // "--report-crash": the island crashed last time and the user clicked its offer to report it.
    let crash = if has("--report-crash") { crash::read() } else { None };
    if crash.is_some() {
        crash::clear();
        page = pages::ABOUT;
    }
    // The island asks for the tour after a fresh install; also show it if this app is opened first.
    let welcome = crash.is_none() && (has("--welcome") || !settings.welcome_shown);

    if let Err(e) = run(page, welcome, crash) {
        log::error_with("The settings app failed", e);
    }
    store::flush();
}

fn run(page: i32, welcome: bool, crash: Option<String>) -> Result<(), slint::PlatformError> {
    let app = MainWindow::new()?;
    app.global::<L>().on_t(|english| l::t(&english).into());
    app.global::<L>().on_f(|english, args| {
        let values: Vec<SharedString> = args.iter().collect();
        let refs: Vec<&dyn std::fmt::Display> = values.iter().map(|v| v as &dyn std::fmt::Display).collect();
        l::f(&english, &refs).into()
    });

    let dark = apps_dark();
    let mica = window::supports_mica();
    let t = app.global::<Theme>();
    t.set_dark(dark);
    t.set_mica(mica);
    t.set_icon_font(icon_font().into());
    t.set_mono_font(mono_font().into());
    app.invoke_apply_theme();

    wire_window(&app);
    pages::wire(&app);
    navigate(&app, page);

    app.window().on_close_requested(|| {
        store::flush();
        slint::CloseRequestResponse::HideWindow
    });
    window::place_on_main_display(app.window());
    app.show()?;
    customize_window(app.as_weak(), dark, mica, 0);

    if welcome {
        pages::welcome::show(&app);
    } else if let Some(crash) = crash {
        pages::about::open_report(&app, Some(&crash));
    }

    slint::run_event_loop()?;
    shortcut::stop();
    Ok(())
}

/// The window exists once the event loop runs; then its title bar, frame and backdrop are set up.
fn customize_window(weak: slint::Weak<MainWindow>, dark: bool, mica: bool, attempt: u32) {
    let Some(app) = weak.upgrade() else { return };
    let Some(hwnd) = window::hwnd(app.window()) else {
        if attempt < 100 {
            slint::Timer::single_shot(std::time::Duration::from_millis(10), move || customize_window(weak, dark, mica, attempt + 1));
        }
        return;
    };
    let weak_active = weak.clone();
    window::customize(
        hwnd,
        dark,
        mica,
        move |active| {
            if let Some(app) = weak_active.upgrade() {
                app.global::<App>().set_window_active(active);
            }
        },
        move || {
            if let Some(app) = weak.upgrade() {
                let dark = apps_dark();
                app.global::<Theme>().set_dark(dark);
                app.invoke_apply_theme();
                if let Some(hwnd) = window::hwnd(app.window()) {
                    window::set_dark_frame(hwnd, dark);
                }
            }
        },
    );
}

/// Windows' app mode ("Choose your default app mode"). A debug build can be told otherwise with
/// DI4W_THEME=light|dark, to check both looks.
fn apps_dark() -> bool {
    match std::env::var("DI4W_THEME").ok().filter(|_| cfg!(debug_assertions)).as_deref() {
        Some("light") => false,
        Some("dark") => true,
        _ => !theme::apps_use_light_theme(),
    }
}

/// "Segoe Fluent Icons" on Windows 11, "Segoe MDL2 Assets" on Windows 10.
fn icon_font() -> &'static str {
    let fonts = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into());
    if std::path::Path::new(&fonts).join("Fonts").join("SegoeIcons.ttf").is_file() { "Segoe Fluent Icons" } else { "Segoe MDL2 Assets" }
}

fn mono_font() -> &'static str {
    let fonts = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into());
    if std::path::Path::new(&fonts).join("Fonts").join("CascadiaMono.ttf").is_file() { "Cascadia Mono" } else { "Consolas" }
}

fn wire_window(app: &MainWindow) {
    let a = app.global::<App>();
    let weak = app.as_weak();
    a.on_navigate(move |page| {
        if let Some(app) = weak.upgrade() {
            navigate(&app, page);
        }
    });
    a.on_close_window(|| {
        store::flush();
        quit();
    });
    let weak = app.as_weak();
    a.on_minimize_window(move || {
        if let Some(app) = weak.upgrade() {
            app.window().set_minimized(true);
        }
    });
    let weak = app.as_weak();
    a.on_zoom_window(move || {
        if let Some(app) = weak.upgrade() {
            let maximized = app.window().is_maximized();
            app.window().set_maximized(!maximized);
        }
    });
}

fn navigate(app: &MainWindow, page: i32) {
    let a = app.global::<App>();
    if a.get_page() != page {
        shortcut::stop();
        if page != pages::ABOUT {
            pages::about::cancel_download();
        }
    }
    a.set_page(page);
    pages::load(app, page);
}

pub fn start_island() {
    if let Some(exe) = store::with(|s| system::island_exe(s.island_path.as_deref())) {
        system::open(&exe.display().to_string());
    }
}

/// Closes the app (the settings are saved first).
pub fn quit() {
    store::flush();
    let _ = slint::quit_event_loop();
}

/// Restarts the settings app on a page (after a language change or an import, so every text is redrawn).
pub fn restart(page: i32) {
    store::flush();
    INSTANCE.with(|i| {
        if let Some(instance) = i.borrow_mut().take() {
            instance.release();
        }
    });
    match std::env::current_exe() {
        Ok(exe) => {
            if let Err(e) = std::process::Command::new(exe).args(["--page", pages::tag(page)]).spawn() {
                log::error_with("Could not restart the settings app", e);
            }
        }
        Err(e) => log::error_with("Could not restart the settings app", e),
    }
    quit();
}
