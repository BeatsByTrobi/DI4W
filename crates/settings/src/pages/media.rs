//! Music: show what's playing, any player or only Spotify, and how long it stays after pausing.

use crate::{MainWindow, Media, store};
use slint::ComponentHandle;

pub fn load(app: &MainWindow) {
    let m = app.global::<Media>();
    let s = store::current();
    m.set_enabled(s.media_enabled);
    m.set_any_player(s.media_any_player);
    m.set_hide_after(s.media_hide_after_pause_seconds.clamp(1, 600));
}

pub fn wire(app: &MainWindow) {
    let m = app.global::<Media>();
    m.on_enabled_toggled(|on| store::change(move |s| s.media_enabled = on));
    m.on_any_player_toggled(|on| store::change(move |s| s.media_any_player = on));
    m.on_hide_after_changed(|seconds| {
        let seconds = seconds.clamp(1, 600);
        store::change(move |s| s.media_hide_after_pause_seconds = seconds);
    });
}
