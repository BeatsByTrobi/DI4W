//! The settings app (DynamicIsland.Settings.exe): a Windows 11 Settings-style window over settings.json.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

slint::include_modules!();

use di4w_core::{l, theme};
use slint::{ComponentHandle, Model, SharedString};

fn main() -> Result<(), slint::PlatformError> {
    let app = MainWindow::new()?;
    app.global::<L>().on_t(|english| l::t(&english).into());
    app.global::<L>().on_f(|english, args| {
        let values: Vec<SharedString> = args.iter().collect();
        let refs: Vec<&dyn std::fmt::Display> = values.iter().map(|v| v as &dyn std::fmt::Display).collect();
        l::f(&english, &refs).into()
    });
    app.global::<Theme>().set_dark(!theme::apps_use_light_theme());
    app.invoke_apply_theme();
    app.run()
}
