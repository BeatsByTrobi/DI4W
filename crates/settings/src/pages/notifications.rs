//! Notifications: the on/off switch, Windows' notification access, calls, the chat apps and any other app.

use super::{model, strings};
use crate::{ChatAppRow, MainWindow, Notifications, store, system};
use di4w_core::status::IslandStatus;
use di4w_core::{ChatApps, l};
use slint::{Color, ComponentHandle};

fn is_listed(apps: &[String], name: &str) -> bool {
    apps.iter().any(|a| a.eq_ignore_ascii_case(name))
}

/// "#5865F2" -> the color.
fn parse_color(hex: &str) -> Color {
    let v = u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap_or(0x808080);
    Color::from_rgb_u8((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

/// Apps in the list that aren't one of the chat apps (those have their own switches).
fn other_apps() -> Vec<String> {
    store::with(|s| s.notification_apps.iter().filter(|a| ChatApps::find(a).is_none()).cloned().collect())
}

pub fn load(app: &MainWindow) {
    let n = app.global::<Notifications>();
    let s = store::current();
    n.set_enabled(s.notifications_enabled);
    n.set_calls(s.addons.calls);
    let access = if system::island_running() { IslandStatus::try_load().and_then(|st| st.notification_access) } else { None };
    n.set_access_blocked(access.is_some_and(|a| a != "Allowed"));
    render_chat_apps(app);
    n.set_other_apps(strings(other_apps()));
}

fn render_chat_apps(app: &MainWindow) {
    let listed = store::with(|s| s.notification_apps.clone());
    let rows: Vec<ChatAppRow> = ChatApps::ALL
        .iter()
        .map(|a| ChatAppRow {
            name: a.name.into(),
            color: parse_color(a.color),
            description: if a.has_calls { l::t("Messages and calls") } else { l::t("New mail") }.into(),
            on: is_listed(&listed, a.name),
        })
        .collect();
    app.global::<Notifications>().set_chat_apps(model(rows));
}

pub fn wire(app: &MainWindow) {
    let n = app.global::<Notifications>();
    n.on_open_privacy(|| system::open("ms-settings:privacy-notifications"));
    n.on_open_windows_settings(|| system::open("ms-settings:notifications"));
    n.on_enabled_toggled(|on| store::change(move |s| s.notifications_enabled = on));
    n.on_calls_toggled(|on| store::change(move |s| s.addons.calls = on));

    // One switch per built-in chat app; it adds or removes the app from the notification list.
    n.on_chat_app_toggled(|index, on| {
        let Some(app) = ChatApps::ALL.get(index as usize) else { return };
        let name = app.name;
        store::change(move |s| {
            s.notification_apps.retain(|a| !a.eq_ignore_ascii_case(name));
            if on {
                s.notification_apps.push(name.into());
            }
        });
    });

    let weak = app.as_weak();
    n.on_add_app(move |text| {
        let name = text.trim().to_string();
        if name.is_empty() || store::with(|s| is_listed(&s.notification_apps, &name)) {
            return;
        }
        store::change(move |s| {
            if !is_listed(&s.notification_apps, &name) {
                s.notification_apps.push(name.clone());
            }
        });
        if let Some(app) = weak.upgrade() {
            app.global::<Notifications>().set_other_apps(strings(other_apps()));
        }
    });

    let weak = app.as_weak();
    n.on_remove_app(move |index| {
        let Some(name) = other_apps().into_iter().nth(index as usize) else { return };
        store::change(move |s| {
            if let Some(i) = s.notification_apps.iter().position(|a| *a == name) {
                s.notification_apps.remove(i);
            }
        });
        if let Some(app) = weak.upgrade() {
            app.global::<Notifications>().set_other_apps(strings(other_apps()));
        }
    });
}
