//! The island (DynamicIsland.exe): the iPhone's Dynamic Island for Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// Parts of the port are wired up step by step; until the app is complete some helpers are still unused.
#![allow(dead_code)]

mod accessibility;
mod animation;
mod interop;
mod island;
mod model;
mod rt;
mod tooltip;
mod ui;

use std::cell::RefCell;
use std::rc::Rc;

fn main() {
    rt::init();
    let settings = Rc::new(RefCell::new(di4w_core::AppSettings::load()));
    di4w_core::l::use_language(&settings.borrow().language);
    ui::colors::set_light(di4w_core::theme::is_light(&settings.borrow().theme));
    model::power::start();
    let manager = model::Manager::new();
    let island = island::Island::create(manager, settings);
    island.show();
    rt::run();
}
