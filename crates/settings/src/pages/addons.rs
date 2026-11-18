//! Add-ons: switches and options for every add-on, grouped like the list they came from, with search, an
//! on/off filter, Preview buttons, quiet times and a status line the island fills in.

use super::{addon_bool, addon_int, get_bool, get_int, model, strings};
use crate::{AddonEntry, AddonItem, Addons, ItemKind, MainWindow, store, system};
use di4w_core::status::IslandStatus;
use di4w_core::{AddonQuiet, AppSettings, l, messages};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

/// One setting inside an add-on's expander.
#[derive(Clone)]
enum Spec {
    Number { header: String, key: &'static str, min: i32, max: i32, unit: String },
    Switch { header: String, description: String, key: &'static str },
    City,
    Feeds,
    Api,
    Quiet,
}

/// An add-on card (or the focus timer's lengths, which have no switch).
#[derive(Clone)]
struct Def {
    section: String,
    key: &'static str,
    header: String,
    description: String,
    glyph: char,
    items: Vec<Spec>,
}

/// Add-ons without pop-ups of their own (a moon, a live view, the API) have no quiet times.
const NO_QUIET: [&str; 3] = ["DoNotDisturb", "GameMode", "IslandApi"];
/// Add-ons without a pop-up of their own to show.
const NO_PREVIEW: [&str; 1] = ["IslandApi"];

fn number(header: &str, key: &'static str, min: i32, max: i32, unit: &str) -> Spec {
    Spec::Number { header: header.into(), key, min, max, unit: unit.into() }
}

fn switch(header: &str, description: &str, key: &'static str) -> Spec {
    Spec::Switch { header: header.into(), description: description.into(), key }
}

/// Every add-on, in the order of the page.
fn definitions() -> Vec<Def> {
    let mut defs = Vec::new();
    let mut add = |section: &str, key: &'static str, header: &str, description: &str, glyph: char, mut items: Vec<Spec>| {
        if !key.is_empty() && !NO_QUIET.contains(&key) {
            items.push(Spec::Quiet); // it opens to show its quiet times
        }
        defs.push(Def { section: section.into(), key, header: header.into(), description: description.into(), glyph, items });
    };

    let mut section: String = l::t("System").into();
    add(&section, "Volume", l::t("Volume"), l::t("A bar in the island when you change the volume."), '\u{E767}', vec![]);
    add(&section, "Brightness", l::t("Brightness"), l::t("A bar when the screen brightness changes. Works on built-in laptop and tablet screens."), '\u{E706}', vec![]);
    add(&section, "Battery", l::t("Battery"), l::t("Charger plugged in or unplugged, and warnings when the battery runs low. Laptops only."), '\u{E83F}', vec![
        number(l::t("Low battery warning at"), "BatteryLowPercent", 5, 50, l::t("%")),
        number(l::t("Very low battery warning at"), "BatteryCriticalPercent", 1, 30, l::t("%")),
    ]);
    add(&section, "Bluetooth", l::t("Bluetooth devices"), l::t("Headphones, mice and controllers connecting or disconnecting, with their battery level when available."), '\u{E702}', vec![]);
    add(&section, "UsbDrives", l::t("USB drives"), l::t("USB sticks and external drives plugged in or removed. Click the pop-up to open the drive."), '\u{E88E}', vec![]);
    add(&section, "Network", l::t("Wi-Fi and internet"), l::t("When the internet drops or comes back, and which network you joined."), '\u{E701}', vec![]);
    add(&section, "PrivacyIndicators", l::t("Microphone and camera in use"), l::t("An orange (microphone) or green (camera) dot next to the island while an app uses them."), '\u{E720}', vec![]);
    add(&section, "LockKeys", l::t("Caps Lock and Num Lock"), l::t("A short message when you turn them on or off."), '\u{E765}', vec![]);
    add(&section, "PerformanceWarnings", l::t("Heavy load warnings"), l::t("When the CPU or memory stays nearly full, or an NVIDIA graphics card runs hot."), '\u{E9D9}', vec![]);

    section = l::t("Productivity").into();
    add(&section, "Calendar", l::t("Calendar"), l::t("Meeting reminders with a Join button for Teams, Zoom, Google Meet and Webex."), '\u{E787}', vec![
        Spec::Feeds,
        number(l::t("Remind me before a meeting"), "CalendarReminderMinutes", 1, 60, l::t("min")),
    ]);
    add(&section, "", l::t("Focus timer"), l::t("Lengths for the Pomodoro timer. Start it by clicking the empty island and choosing Focus."), '\u{E7C1}', vec![
        number(l::t("Focus"), "FocusMinutes", 1, 180, l::t("min")),
        number(l::t("Break"), "BreakMinutes", 1, 60, l::t("min")),
        number(l::t("Long break"), "LongBreakMinutes", 1, 120, l::t("min")),
        number(l::t("Long break after"), "SessionsBeforeLongBreak", 1, 12, l::t("sessions")),
    ]);
    add(&section, "Clipboard", l::t("Clipboard"), l::t("A short preview of what you copy. Passwords from password managers are never shown."), '\u{E8C8}', vec![]);
    add(&section, "Screenshots", l::t("Screenshots"), l::t("A thumbnail when a screenshot is saved. Click it to open the image."), '\u{E722}', vec![]);
    add(&section, "Downloads", l::t("Downloads"), l::t("Download progress in the island and \"Download complete\" for files saved to your Downloads folder."), '\u{E896}', vec![]);
    add(&section, "FileCopy", l::t("File copies"), l::t("Progress of copies and moves in File Explorer."), '\u{E8C8}', vec![]);

    section = l::t("Alerts").into();
    add(&section, "DoNotDisturb", l::t("Follow Do Not Disturb"), l::t("While Windows' Do Not Disturb is on, only alarms and timers pop up, and a moon shows in the island."), '\u{E708}', vec![]);
    add(&section, "WindowsUpdate", l::t("Windows Update"), l::t("Tells you when a restart is needed to finish installing updates."), '\u{E895}', vec![]);

    section = l::t("Info").into();
    add(&section, "Weather", l::t("Weather"), l::t("The temperature in the empty island, and a warning before it starts raining. From Open-Meteo, no account needed."), '\u{E753}', vec![
        Spec::City,
        switch(l::t("Fahrenheit"), l::t("Show °F instead of °C."), "WeatherFahrenheit"),
        switch(l::t("Rain alerts"), l::t("\"Rain in about 15 min\" when it's dry now."), "RainAlerts"),
    ]);
    add(&section, "GameMode", l::t("Game mode"),
        l::t("Keeps the island visible over fullscreen games, with GPU load and temperature. Frame rate isn't available: Windows only shares it with programs running as administrator."),
        '\u{E7FC}', vec![]);

    section = l::t("For developers").into();
    add(&section, "IslandApi", l::t("Island API"),
        l::t("Lets scripts and apps on this PC show their own pop-ups and progress in the island. Web pages can't use it. The island restarts to apply this."),
        '\u{E943}', vec![number(l::t("Port"), "IslandApiPort", 1024, 65535, ""), Spec::Api]);
    defs
}

/// The settings names of every on/off switch and number on this page (for the tests).
#[cfg(test)]
pub fn switch_keys() -> Vec<&'static str> {
    let mut keys = Vec::new();
    for def in definitions() {
        if !def.key.is_empty() {
            keys.push(def.key);
        }
        keys.extend(def.items.iter().filter_map(|i| if let Spec::Switch { key, .. } = i { Some(*key) } else { None }));
    }
    keys
}

#[cfg(test)]
pub fn number_keys() -> Vec<&'static str> {
    definitions().into_iter().flat_map(|d| d.items).filter_map(|i| if let Spec::Number { key, .. } = i { Some(key) } else { None }).collect()
}

/// One card on the page, as built when the page opened.
struct Card {
    def: Def,
    expanded: bool,
}

#[derive(Default)]
struct PageState {
    cards: Vec<Card>,
    /// What the page shows: `None` for a section header (of the next card), `Some(i)` for card i.
    rows: Vec<(String, Option<usize>)>,
    model: Option<Rc<VecModel<AddonEntry>>>,
    /// Runs when the info bar's button is clicked.
    info_action: Option<Rc<dyn Fn()>>,
    status: Option<IslandStatus>,
}

thread_local! {
    static STATE: RefCell<PageState> = RefCell::new(PageState::default());
    static STATUS_TIMER: slint::Timer = slint::Timer::default();
}

fn quiet_of(settings: &AppSettings, key: &str) -> AddonQuiet {
    settings.addons.quiet.get(key).cloned().unwrap_or_default()
}

/// "22:00" -> (22, 0).
fn split_time(text: &str, fallback_hour: i32) -> (i32, i32) {
    di4w_core::settings::parse_time(text)
        .map(|t| {
            use chrono::Timelike;
            (t.hour() as i32, t.minute() as i32)
        })
        .unwrap_or((fallback_hour, 0))
}

fn is_on(settings: &AppSettings, key: &str) -> bool {
    key.is_empty() || get_bool(&settings.addons, key)
}

fn entry(card: &Card, settings: &AppSettings, status: Option<&IslandStatus>) -> AddonEntry {
    let def = &card.def;
    let items: Vec<AddonItem> = def
        .items
        .iter()
        .map(|spec| match spec {
            Spec::Number { header, key, min, max, unit } => AddonItem {
                kind: ItemKind::Number,
                key: (*key).into(),
                header: header.into(),
                value: get_int(&settings.addons, key),
                minimum: *min,
                maximum: *max,
                unit: unit.into(),
                ..Default::default()
            },
            Spec::Switch { header, description, key } => AddonItem {
                kind: ItemKind::Switch,
                key: (*key).into(),
                header: header.into(),
                description: description.into(),
                on: get_bool(&settings.addons, key),
                ..Default::default()
            },
            Spec::City => AddonItem { kind: ItemKind::City, ..Default::default() },
            Spec::Feeds => AddonItem { kind: ItemKind::Feeds, ..Default::default() },
            Spec::Api => AddonItem { kind: ItemKind::Api, ..Default::default() },
            Spec::Quiet => AddonItem { kind: ItemKind::Quiet, ..Default::default() },
        })
        .collect();
    let quiet = quiet_of(settings, def.key);
    let (from_hour, from_minute) = split_time(&quiet.from, 22);
    let (to_hour, to_minute) = split_time(&quiet.to, 8);
    // The status line: shown while the add-on is on and the island has something to say.
    let state = status.and_then(|s| s.addons.get(def.key));
    let message = state.and_then(|s| s.message.clone()).filter(|m| !m.is_empty() && !def.key.is_empty() && is_on(settings, def.key));
    AddonEntry {
        section: SharedString::new(),
        key: def.key.into(),
        header: def.header.as_str().into(),
        description: def.description.as_str().into(),
        glyph: def.glyph.to_string().into(),
        on: is_on(settings, def.key),
        has_preview: !def.key.is_empty() && !NO_PREVIEW.contains(&def.key),
        status: message.unwrap_or_default().into(),
        status_problem: state.is_some_and(|s| s.problem),
        expanded: card.expanded,
        items: model(items),
        quiet_hours: quiet.hours_on,
        quiet_from_hour: from_hour,
        quiet_from_minute: from_minute,
        quiet_to_hour: to_hour,
        quiet_to_minute: to_minute,
        quiet_apps: strings(quiet.apps.iter().cloned()),
    }
}

fn section_row(title: &str) -> AddonEntry {
    AddonEntry { section: title.into(), items: model(Vec::new()), quiet_apps: strings(Vec::<String>::new()), ..Default::default() }
}

/// The island's status, unless it isn't running (a status file left behind would be stale).
fn load_status() -> Option<IslandStatus> {
    if system::island_running() { IslandStatus::try_load() } else { None }
}

/// Builds the page: the sections, with the add-ons that are on first in each (the order is fixed while the
/// page is open, so a card doesn't jump away from the mouse when it's switched).
pub fn load(app: &MainWindow) {
    let settings = store::current();
    let mut cards: Vec<Card> = Vec::new();
    let defs = definitions();
    let mut sections: Vec<String> = Vec::new();
    for def in &defs {
        if !sections.contains(&def.section) {
            sections.push(def.section.clone());
        }
    }
    for section in &sections {
        let mut group: Vec<&Def> = defs.iter().filter(|d| &d.section == section).collect();
        group.sort_by_key(|d| if is_on(&settings, d.key) { 0 } else { 1 });
        cards.extend(group.into_iter().map(|d| Card { def: d.clone(), expanded: false }));
    }
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.cards = cards;
        s.status = load_status();
        s.info_action = None;
    });
    let a = app.global::<Addons>();
    a.set_search(SharedString::new());
    a.set_filter(0);
    a.set_info_open(false);
    a.set_city(settings.addons.weather_location.as_str().into());
    render_feeds(app);
    a.set_api_example(api_example(settings.addons.island_api_port).into());
    apply_filter(app);
    watch_status(app);
}

/// Which cards match the search and the on/off filter; rebuilds the list.
fn apply_filter(app: &MainWindow) {
    let a = app.global::<Addons>();
    let words: Vec<String> = a.get_search().split_whitespace().map(str::to_lowercase).collect();
    let filter = a.get_filter();
    let settings = store::current();
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        let mut rows: Vec<(String, Option<usize>)> = Vec::new();
        let mut last_section = String::new();
        for (i, card) in state.cards.iter().enumerate() {
            let def = &card.def;
            let text = format!("{} {} {}", def.section, def.header, def.description).to_lowercase();
            let visible = words.iter().all(|w| text.contains(w.as_str()))
                && match filter {
                    1 => !def.key.is_empty() && is_on(&settings, def.key),
                    2 => !def.key.is_empty() && !is_on(&settings, def.key),
                    _ => true,
                };
            if !visible {
                continue;
            }
            if def.section != last_section {
                rows.push((def.section.clone(), None));
                last_section = def.section.clone();
            }
            rows.push((String::new(), Some(i)));
        }
        let entries: Vec<AddonEntry> = rows
            .iter()
            .map(|(section, card)| match card {
                Some(i) => entry(&state.cards[*i], &settings, state.status.as_ref()),
                None => section_row(section),
            })
            .collect();
        a.set_no_matches(rows.is_empty());
        let vec_model = Rc::new(VecModel::from(entries));
        a.set_entries(ModelRc::from(vec_model.clone()));
        state.model = Some(vec_model);
        state.rows = rows;
    });
    update_summary(app);
}

fn update_summary(app: &MainWindow) {
    let settings = store::current();
    let keys: Vec<&str> = definitions().into_iter().map(|d| d.key).filter(|k| !k.is_empty()).collect();
    let on = keys.iter().filter(|k| is_on(&settings, k)).count();
    app.global::<Addons>().set_summary(l::f("{0} of {1} add-ons are on", &[&on, &keys.len()]).into());
}

/// The card behind a row of the page, if that row is a card.
fn card_at(row: i32) -> Option<usize> {
    STATE.with(|s| s.borrow().rows.get(row as usize).and_then(|(_, card)| *card))
}

fn card_key(row: i32) -> Option<&'static str> {
    let card = card_at(row)?;
    STATE.with(|s| s.borrow().cards.get(card).map(|c| c.def.key))
}

/// Shows a card's row again with the current settings.
fn refresh_row(row: i32) {
    let settings = store::current();
    STATE.with(|state| {
        let state = state.borrow();
        let Some((_, Some(card))) = state.rows.get(row as usize) else { return };
        if let Some(model) = &state.model {
            model.set_row_data(row as usize, entry(&state.cards[*card], &settings, state.status.as_ref()));
        }
    });
}

/// Updates every card's status line from the island.
fn refresh_status() {
    let status = load_status();
    let changed = STATE.with(|s| s.borrow().status != status);
    if !changed {
        return;
    }
    STATE.with(|s| s.borrow_mut().status = status);
    let rows = STATE.with(|s| s.borrow().rows.len());
    for row in 0..rows {
        refresh_row(row as i32);
    }
}

/// The island rewrites status.json as add-ons change state; keep the status lines current while the page is open.
fn watch_status(app: &MainWindow) {
    let weak = app.as_weak();
    STATUS_TIMER.with(|t| {
        t.start(slint::TimerMode::Repeated, Duration::from_millis(1500), move || {
            match weak.upgrade() {
                Some(app) if app.global::<crate::App>().get_page() == super::ADDONS => refresh_status(),
                _ => STATUS_TIMER.with(|t| t.stop()),
            }
        })
    });
}

// ───────────────────────────── previews ─────────────────────────────

fn explain(app: &MainWindow, title: &str, message: &str, action: Option<&str>, on_action: Option<Rc<dyn Fn()>>) {
    let a = app.global::<Addons>();
    a.set_info_title(title.into());
    a.set_info_message(message.into());
    a.set_info_action(action.unwrap_or_default().into());
    STATE.with(|s| s.borrow_mut().info_action = on_action);
    a.set_info_open(true);
}

fn send_preview(key: String) -> std::thread::JoinHandle<bool> {
    std::thread::spawn(move || messages::send(&format!("{}{key}", messages::PREVIEW_PREFIX)))
}

fn show_preview(app: &MainWindow, key: &'static str) {
    let weak = app.as_weak();
    if !system::island_running() {
        explain(app, l::t("The island isn't running"), l::t("Start it to see previews."), Some(l::t("Start island")), Some(Rc::new(move || {
            crate::start_island();
            if let Some(app) = weak.upgrade() {
                app.global::<Addons>().set_info_open(false);
            }
        })));
        return;
    }
    // Read the file: the island may have been hidden or shown with the shortcut since this window opened.
    if AppSettings::try_load().unwrap_or_else(store::current).paused {
        explain(app, l::t("The island is hidden"), l::t("Show it again to see the preview."), Some(l::t("Show island")), Some(Rc::new(move || {
            if let Some(app) = weak.upgrade() {
                app.global::<Addons>().set_info_open(false);
            }
            store::change(|s| s.paused = false);
            store::flush();
            // The island picks the change up, then fades in.
            slint::Timer::single_shot(Duration::from_millis(1200), move || {
                send_preview(key.into());
            });
        })));
        return;
    }
    app.global::<Addons>().set_info_open(false);
    let handle = send_preview(key.into());
    // Wait for the answer off the UI thread, then explain if the island didn't respond.
    std::thread::spawn(move || {
        if !handle.join().unwrap_or(false) {
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(app) = weak.upgrade() {
                    explain(&app, l::t("The island didn't respond"), l::t("Try again in a moment, or restart it from the tray icon."), None, None);
                }
            });
        }
    });
}

// ───────────────────────────── calendar links, weather, API ─────────────────────────────

/// "https://outlook.office365.com/owa/calendar/abc…/calendar.ics" -> "outlook.office365.com/…/calendar.ics"
pub fn shorten(url: &str) -> String {
    let normalized = url.replacen("webcal://", "https://", 1);
    let Some((_, rest)) = normalized.split_once("://") else { return url.into() };
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    if host.is_empty() {
        return url.into();
    }
    let path = path.split(['?', '#']).next().unwrap_or_default();
    let file = if path.is_empty() { "/" } else { path.rsplit('/').next().unwrap_or_default() };
    let file = if file.is_empty() { "/" } else { file };
    format!("{host}/…/{file}")
}

fn render_feeds(app: &MainWindow) {
    let feeds = store::with(|s| s.addons.calendar_feeds.iter().map(|f| shorten(f)).collect::<Vec<_>>());
    app.global::<Addons>().set_feeds(strings(feeds));
}

fn api_example(port: i32) -> String {
    format!(
        "curl -X POST http://127.0.0.1:{port}/alert -H \"Content-Type: application/json\" -d \"{{\\\"title\\\":\\\"Backup done\\\",\\\"icon\\\":\\\"success\\\"}}\"\n\n{}\nisland.ps1 -Title \"Backup done\" -Icon success\nisland.ps1 -Title \"Build failed\" -Icon error -Button \"log=Open log\" -Wait\nisland.ps1 -Progress backup -Title \"Backing up\" -Value 0.4",
        l::t("PowerShell (the helper script is in the app's tools folder):")
    )
}

/// Saves a changed copy (the change may run twice, see store::change, so it sets a whole copy).
fn save_quiet(key: &'static str, quiet: AddonQuiet) {
    store::change(move |s| {
        if quiet.is_empty() {
            s.addons.quiet.remove(key);
        } else {
            s.addons.quiet.insert(key.into(), quiet.clone());
        }
    });
}

fn change_quiet(row: i32, edit: impl FnOnce(&mut AddonQuiet)) {
    let Some(key) = card_key(row).filter(|k| !k.is_empty()) else { return };
    let mut quiet = store::with(|s| quiet_of(s, key));
    edit(&mut quiet);
    save_quiet(key, quiet);
    refresh_row(row);
}

pub fn wire(app: &MainWindow) {
    let a = app.global::<Addons>();

    let weak = app.as_weak();
    a.on_search_changed(move |_| {
        if let Some(app) = weak.upgrade() {
            apply_filter(&app);
        }
    });
    let weak = app.as_weak();
    a.on_filter_changed(move |_| {
        if let Some(app) = weak.upgrade() {
            apply_filter(&app);
        }
    });

    let weak = app.as_weak();
    a.on_toggled(move |row, on| {
        let Some(key) = card_key(row).filter(|k| !k.is_empty()) else { return };
        store::change(move |s| {
            if let Some(value) = addon_bool(&mut s.addons, key) {
                *value = on;
            }
        });
        refresh_row(row);
        if let Some(app) = weak.upgrade() {
            update_summary(&app);
        }
    });
    a.on_expanded(|row, open| {
        if let Some(card) = card_at(row) {
            STATE.with(|s| s.borrow_mut().cards[card].expanded = open);
            refresh_row(row);
        }
    });

    let weak = app.as_weak();
    a.on_preview(move |row| {
        if let (Some(app), Some(key)) = (weak.upgrade(), card_key(row).filter(|k| !k.is_empty())) {
            show_preview(&app, key);
        }
    });
    a.on_info_action_clicked(|| {
        if let Some(action) = STATE.with(|s| s.borrow().info_action.clone()) {
            action();
        }
    });
    let weak = app.as_weak();
    a.on_info_closed(move || {
        if let Some(app) = weak.upgrade() {
            app.global::<Addons>().set_info_open(false);
        }
    });

    let weak = app.as_weak();
    a.on_number_changed(move |row, key, value| {
        let key = key.to_string();
        store::change(move |s| {
            if let Some(field) = addon_int(&mut s.addons, &key) {
                *field = value;
            }
        });
        refresh_row(row);
        if let Some(app) = weak.upgrade() {
            app.global::<Addons>().set_api_example(api_example(store::with(|s| s.addons.island_api_port)).into());
        }
    });
    a.on_switch_changed(|row, key, on| {
        let key = key.to_string();
        store::change(move |s| {
            if let Some(field) = addon_bool(&mut s.addons, &key) {
                *field = on;
            }
        });
        refresh_row(row);
    });

    a.on_city_changed(|text| {
        let city = text.trim().to_string();
        if store::with(|s| s.addons.weather_location == city) {
            return;
        }
        store::change(move |s| s.addons.weather_location = city.clone());
    });

    let weak = app.as_weak();
    a.on_add_feed(move |text| {
        let url = text.trim().to_string();
        let lower = url.to_lowercase();
        if !(lower.starts_with("https://") || lower.starts_with("webcal")) || store::with(|s| s.addons.calendar_feeds.contains(&url)) {
            return;
        }
        store::change(move |s| {
            if !s.addons.calendar_feeds.contains(&url) {
                s.addons.calendar_feeds.push(url.clone());
            }
        });
        if let Some(app) = weak.upgrade() {
            render_feeds(&app);
        }
    });
    let weak = app.as_weak();
    a.on_remove_feed(move |index| {
        let Some(feed) = store::with(|s| s.addons.calendar_feeds.get(index as usize).cloned()) else { return };
        store::change(move |s| s.addons.calendar_feeds.retain(|f| *f != feed));
        if let Some(app) = weak.upgrade() {
            render_feeds(&app);
        }
    });

    // Quiet times.
    a.on_quiet_hours_toggled(|row, on| change_quiet(row, |q| q.hours_on = on));
    a.on_quiet_from_changed(|row, hour, minute| change_quiet(row, |q| q.from = format!("{hour:02}:{minute:02}")));
    a.on_quiet_to_changed(|row, hour, minute| change_quiet(row, |q| q.to = format!("{hour:02}:{minute:02}")));
    a.on_quiet_remove_app(|row, app| change_quiet(row, |q| q.apps.retain(|a| *a != app.as_str())));
    a.on_quiet_add_app(|row, app| {
        change_quiet(row, |q| {
            if !q.apps.iter().any(|a| a.eq_ignore_ascii_case(&app)) {
                q.apps.push(app.to_string());
            }
        })
    });
    let weak = app.as_weak();
    a.on_refresh_open_apps(move |row| {
        let Some(app) = weak.upgrade() else { return };
        let key = card_key(row).unwrap_or_default();
        let listed = store::with(|s| quiet_of(s, key).apps);
        let open: Vec<String> = system::open_apps().into_iter().filter(|n| !listed.iter().any(|l| l.eq_ignore_ascii_case(n))).collect();
        app.global::<Addons>().set_open_apps(strings(open));
    });

    let weak = app.as_weak();
    a.on_copy_example(move || {
        if let Some(app) = weak.upgrade() {
            system::copy_text(&app.global::<Addons>().get_api_example());
        }
    });
    a.on_open_api_docs(|| system::open("https://github.com/aangeraakt/DI4W#island-api"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortens_calendar_links() {
        assert_eq!(shorten("https://outlook.office365.com/owa/calendar/abc/def/calendar.ics"), "outlook.office365.com/…/calendar.ics");
        assert_eq!(shorten("webcal://p01-calendars.icloud.com/published/2/xyz"), "p01-calendars.icloud.com/…/xyz");
        assert_eq!(shorten("https://example.com"), "example.com/…//");
        assert_eq!(shorten("not a link"), "not a link");
    }

    #[test]
    fn every_addon_with_pop_ups_has_quiet_times() {
        for def in definitions() {
            let has_quiet = def.items.iter().any(|i| matches!(i, Spec::Quiet));
            assert_eq!(has_quiet, !def.key.is_empty() && !NO_QUIET.contains(&def.key), "{}", def.header);
        }
    }

    #[test]
    fn times_split_into_hours_and_minutes() {
        assert_eq!(split_time("22:05", 1), (22, 5));
        assert_eq!(split_time("nonsense", 8), (8, 0));
    }
}
