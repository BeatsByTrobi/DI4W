//! The welcome tour after a fresh install (and from About): what the island is, the two shortcuts, and which
//! add-ons to use. Choices apply right away, so closing early keeps what was picked.

use super::{addon_bool, get_bool, model, strings};
use crate::{App, Dialog, MainWindow, Pick, Welcome, store, system};
use di4w_core::{hotkey, l, theme};
use slint::ComponentHandle;

/// The add-ons offered in the last step: the ones people notice first.
const PICKS: [(&str, &str); 12] = [
    ("Volume", "Volume"),
    ("Brightness", "Brightness"),
    ("Battery", "Battery"),
    ("Bluetooth", "Bluetooth devices"),
    ("UsbDrives", "USB drives"),
    ("Network", "Wi-Fi and internet"),
    ("Clipboard", "Clipboard"),
    ("Screenshots", "Screenshots"),
    ("Downloads", "Downloads"),
    ("Calendar", "Calendar"),
    ("Weather", "Weather"),
    ("GameMode", "Game mode"),
];

/// "Ctrl" "Alt" "I" as key caps; "Not set" without a valid shortcut.
fn keys(shortcut: &str) -> Vec<String> {
    if hotkey::parse(shortcut).is_some() { shortcut.split('+').map(|k| k.trim().to_string()).collect() } else { vec![l::t("Not set").into()] }
}

/// Opens the tour on its first step.
pub fn show(app: &MainWindow) {
    if !store::with(|s| s.welcome_shown) {
        store::change(|s| s.welcome_shown = true);
    }
    let w = app.global::<Welcome>();
    let s = store::current();
    w.set_step(0);
    w.set_preview_scale(s.scale as f32);
    w.set_preview_offset(s.top_offset as f32);
    w.set_preview_light(theme::is_light(&s.theme));
    w.set_hide_keys(strings(keys(&s.hide_hotkey)));
    w.set_expand_keys(strings(keys(&s.expand_hotkey)));
    w.set_picks(model(
        PICKS.iter().map(|(key, label)| Pick { key: (*key).into(), label: l::t(label).into(), on: get_bool(&s.addons, key) }).collect(),
    ));
    w.set_autostart_available(system::island_exe(s.island_path.as_deref()).is_some());
    w.set_autostart(di4w_core::settings::autostart::is_enabled());
    app.global::<App>().set_dialog(Dialog::Welcome);
}

pub fn wire(app: &MainWindow) {
    let w = app.global::<Welcome>();
    w.on_pick_toggled(|index, on| {
        let Some((key, _)) = PICKS.get(index as usize) else { return };
        store::change(move |s| {
            if let Some(value) = addon_bool(&mut s.addons, key) {
                *value = on;
            }
        });
    });
    w.on_autostart_toggled(|on| {
        if let Some(exe) = store::with(|s| system::island_exe(s.island_path.as_deref())) {
            system::set_autostart(on, &exe);
        }
    });
    // Afterwards the current page is shown again, as the tour may have changed its settings.
    let weak = app.as_weak();
    w.on_finished(move || {
        if let Some(app) = weak.upgrade() {
            let page = app.global::<App>().get_page();
            super::load(&app, page);
        }
    });
}
