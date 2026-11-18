//! General: start with Windows, fullscreen, sounds, the two shortcuts, the activity order, and the island's
//! look (theme, size, distance from the top, language, display).

use super::{model, strings};
use crate::{General, MainWindow, OrderItem, shortcut, store, system};
use di4w_core::{ActivityKinds, displays, hotkey, l, theme};
use slint::{ComponentHandle, Model, SharedString};
use std::cell::RefCell;

thread_local! {
    /// The settings behind the display list as shown.
    static DISPLAY_VALUES: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

const THEMES: [&str; 3] = [theme::DARK, theme::LIGHT, theme::SYSTEM];
const SIZES: [f64; 4] = [0.85, 1.0, 1.2, 1.4];
const LANGUAGES: [&str; 5] = ["", "en", "nl", "de", "fr"];

/// Main display, the display with the mouse, then each connected display: (setting, label).
fn display_choices(current: &str) -> Vec<(String, String)> {
    let mut choices = vec![
        (String::new(), l::t("Main display").to_string()),
        (displays::FOLLOW_MOUSE.to_string(), l::t("Display with the mouse").to_string()),
    ];
    choices.extend(displays::all().into_iter().map(|d| (d.device, d.label)));
    // A display that's not connected right now stays selectable, so the setting isn't silently lost.
    if !current.is_empty() && !choices.iter().any(|(value, _)| value == current) {
        choices.push((current.to_string(), l::f("{0} (not connected)", &[&current])));
    }
    choices
}

fn shortcut_label(text: &str) -> (SharedString, bool) {
    if hotkey::parse(text).is_some() { (text.replace('+', " + ").into(), true) } else { (l::t("None").into(), false) }
}

pub fn load(app: &MainWindow) {
    let g = app.global::<General>();
    let s = store::current();
    g.set_island_running(system::island_running());
    let island = system::island_exe(s.island_path.as_deref());
    g.set_autostart_available(island.is_some());
    g.set_autostart(di4w_core::settings::autostart::is_enabled());
    g.set_hide_in_fullscreen(s.hide_in_fullscreen);
    g.set_sounds(s.sounds);
    render_shortcuts(app);
    fill_order(app, &s.activity_order);
    g.set_theme_index(THEMES.iter().position(|t| *t == s.theme).unwrap_or(0) as i32);
    let size = SIZES.iter().enumerate().min_by(|a, b| (a.1 - s.scale).abs().total_cmp(&(b.1 - s.scale).abs())).map(|(i, _)| i).unwrap_or(1);
    g.set_size_index(size as i32);
    g.set_top_offset(s.top_offset as f32);
    g.set_language_index(LANGUAGES.iter().position(|code| *code == s.language).unwrap_or(0) as i32);
    let choices = display_choices(&s.monitor);
    g.set_display_index(choices.iter().position(|(value, _)| *value == s.monitor).unwrap_or(0) as i32);
    DISPLAY_VALUES.with(|v| *v.borrow_mut() = choices.iter().map(|(value, _)| value.clone()).collect());
    g.set_displays(strings(choices.into_iter().map(|(_, label)| label)));
    update_preview(app);
}

fn render_shortcuts(app: &MainWindow) {
    let g = app.global::<General>();
    let s = store::current();
    let (hide, hide_set) = shortcut_label(&s.hide_hotkey);
    let (expand, expand_set) = shortcut_label(&s.expand_hotkey);
    g.set_hide_shortcut(hide);
    g.set_hide_shortcut_set(hide_set);
    g.set_expand_shortcut(expand);
    g.set_expand_shortcut_set(expand_set);
    g.set_recording(if shortcut::is_recording() { g.get_recording() } else { -1 });
}

fn fill_order(app: &MainWindow, saved: &[String]) {
    let items: Vec<OrderItem> = ActivityKinds::ordered(saved)
        .into_iter()
        .map(|kind| {
            let label = ActivityKinds::ALL.iter().find(|(k, _)| *k == kind).map(|(_, label)| *label).unwrap_or_default();
            OrderItem { kind: kind.into(), label: l::t(label).into() }
        })
        .collect();
    app.global::<General>().set_order(model(items));
}

fn update_preview(app: &MainWindow) {
    let g = app.global::<General>();
    let s = store::current();
    g.set_preview_scale(s.scale as f32);
    g.set_preview_offset(s.top_offset as f32);
    g.set_preview_light(theme::is_light(&s.theme));
}

/// Which setting a shortcut box edits: 0 the hide shortcut, 1 the open shortcut.
fn get_shortcut(which: i32) -> String {
    store::with(|s| if which == 0 { s.hide_hotkey.clone() } else { s.expand_hotkey.clone() })
}

fn set_shortcut(which: i32, value: String) {
    store::change(move |s| {
        if which == 0 {
            s.hide_hotkey = value.clone();
        } else {
            s.expand_hotkey = value.clone();
        }
    });
}

pub fn wire(app: &MainWindow) {
    let g = app.global::<General>();

    let weak = app.as_weak();
    g.on_start_island(move || {
        crate::start_island();
        if let Some(app) = weak.upgrade() {
            app.global::<General>().set_island_running(true);
        }
    });

    g.on_autostart_toggled(|on| {
        if let Some(exe) = store::with(|s| system::island_exe(s.island_path.as_deref())) {
            system::set_autostart(on, &exe);
        }
    });
    g.on_hide_in_fullscreen_toggled(|on| store::change(move |s| s.hide_in_fullscreen = on));
    g.on_sounds_toggled(|on| store::change(move |s| s.sounds = on));

    // Shortcuts: click, press the keys, done.
    let weak = app.as_weak();
    g.on_record_shortcut(move |which| {
        let Some(app) = weak.upgrade() else { return };
        if shortcut::is_recording() {
            return;
        }
        app.global::<General>().set_recording(which);
        let weak = app.as_weak();
        shortcut::start(move |outcome| {
            let Some(app) = weak.upgrade() else { return };
            app.global::<General>().set_recording(-1);
            if let shortcut::Outcome::Recorded(modifiers, key) = outcome {
                let text = hotkey::format(modifiers, key);
                if text != get_shortcut(which) {
                    let taken = !shortcut::is_free(modifiers, key);
                    set_shortcut(which, text);
                    app.global::<General>().set_shortcut_warning(taken);
                }
            }
            render_shortcuts(&app);
        });
    });
    let weak = app.as_weak();
    g.on_stop_recording(move || {
        shortcut::stop();
        if let Some(app) = weak.upgrade() {
            app.global::<General>().set_recording(-1);
            render_shortcuts(&app);
        }
    });
    let weak = app.as_weak();
    g.on_clear_shortcut(move |which| {
        set_shortcut(which, String::new());
        if let Some(app) = weak.upgrade() {
            app.global::<General>().set_shortcut_warning(false);
            render_shortcuts(&app);
        }
    });

    // The activity order.
    let weak = app.as_weak();
    g.on_move_order(move |from, to| {
        let Some(app) = weak.upgrade() else { return };
        let order = app.global::<General>().get_order();
        let mut kinds: Vec<SharedString> = order.iter().map(|o| o.kind).collect();
        let (from, to) = (from as usize, to as usize);
        if from >= kinds.len() || to >= kinds.len() {
            return;
        }
        let kind = kinds.remove(from);
        kinds.insert(to, kind);
        let kinds: Vec<String> = kinds.into_iter().map(|k| k.to_string()).collect();
        fill_order(&app, &kinds);
        store::change(move |s| s.activity_order = kinds.clone());
    });
    let weak = app.as_weak();
    g.on_reset_order(move || {
        store::change(|s| s.activity_order = Vec::new());
        if let Some(app) = weak.upgrade() {
            fill_order(&app, &[]);
        }
    });

    // Appearance.
    let weak = app.as_weak();
    g.on_theme_selected(move |index| {
        let value = THEMES.get(index as usize).copied().unwrap_or_default();
        store::change(move |s| s.theme = value.into()); // the island restarts itself in the new colors
        if let Some(app) = weak.upgrade() {
            update_preview(&app);
        }
    });
    let weak = app.as_weak();
    g.on_size_selected(move |index| {
        let scale = SIZES.get(index as usize).copied().unwrap_or(1.0);
        store::change(move |s| s.scale = scale);
        if let Some(app) = weak.upgrade() {
            update_preview(&app);
        }
    });
    let weak = app.as_weak();
    g.on_top_offset_changed(move |value| {
        let value = value.round() as f64;
        if store::with(|s| s.top_offset) == value {
            return;
        }
        store::change(move |s| s.top_offset = value);
        if let Some(app) = weak.upgrade() {
            update_preview(&app);
        }
    });
    g.on_language_selected(|index| {
        let value = LANGUAGES.get(index as usize).copied().unwrap_or_default();
        if value == store::with(|s| s.language.clone()) {
            return;
        }
        store::change(move |s| s.language = value.into()); // the island restarts itself in the new language
        crate::restart(super::GENERAL); // and so does this window, so every text is redrawn
    });
    g.on_display_selected(|index| {
        if let Some(value) = DISPLAY_VALUES.with(|v| v.borrow().get(index as usize).cloned()) {
            store::change(move |s| s.monitor = value.clone());
        }
    });
}
