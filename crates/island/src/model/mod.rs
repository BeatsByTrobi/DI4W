//! The island's model (the C# app's `Core` folder): live activities, alerts, the manager that decides what shows,
//! and battery saver.

pub mod activity;
pub mod manager;
pub mod power;

pub use activity::{Activity, Alert};
pub use manager::Manager;
