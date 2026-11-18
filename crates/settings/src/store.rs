//! The settings the pages edit. Every change is saved straight away (like Windows Settings, no Save button);
//! the island watches the file and applies it.
//!
//! Only the changes made here are written, on top of the latest file, so values the island wrote while this
//! window was open (such as its install path after an update) are never overwritten with old ones. Saving waits
//! a moment, so dragging a slider doesn't write the file 60 times a second.

use di4w_core::AppSettings;
use std::cell::RefCell;
use std::time::Duration;

/// A change to the settings. It may run twice (on the copy shown here and on the latest file), so it must be
/// safe to repeat.
pub type Change = Box<dyn Fn(&mut AppSettings)>;

/// The settings as the pages see them, plus the changes not written yet.
pub struct Store {
    pub current: AppSettings,
    pending: Vec<Change>,
}

impl Store {
    pub fn new(current: AppSettings) -> Self {
        Self { current, pending: Vec::new() }
    }

    /// Applies a change here and remembers it for the next [`Store::save`].
    pub fn change(&mut self, change: Change) {
        change(&mut self.current);
        self.pending.push(change);
    }

    #[cfg(test)]
    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    /// Writes the pending changes on top of the latest file (under the lock both apps share) and picks up what
    /// the island changed meanwhile.
    pub fn save(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        let changes = std::mem::take(&mut self.pending);
        self.current = AppSettings::update(|latest| changes.iter().for_each(|c| c(latest)));
    }
}

const SAVE_DELAY: Duration = Duration::from_millis(250);

thread_local! {
    static STORE: RefCell<Store> = RefCell::new(Store::new(AppSettings::load()));
    static TIMER: slint::Timer = slint::Timer::default();
}

/// A copy of the current settings.
pub fn current() -> AppSettings {
    STORE.with(|s| s.borrow().current.clone())
}

/// Reads one value from the current settings.
pub fn with<R>(read: impl FnOnce(&AppSettings) -> R) -> R {
    STORE.with(|s| read(&s.borrow().current))
}

/// Applies a change and saves it shortly after.
pub fn change(change: impl Fn(&mut AppSettings) + 'static) {
    STORE.with(|s| s.borrow_mut().change(Box::new(change)));
    TIMER.with(|t| t.start(slint::TimerMode::SingleShot, SAVE_DELAY, save));
}

/// Writes pending changes now (on close, before a restart or an update).
pub fn flush() {
    TIMER.with(|t| t.stop());
    save();
}

fn save() {
    STORE.with(|s| s.borrow_mut().save());
}

#[cfg(test)]
mod tests {
    use super::*;
    use di4w_core::paths;

    #[test]
    fn saves_changes_on_top_of_what_the_island_wrote() {
        let folder = tempfile::tempdir().unwrap();
        paths::set_data_folder(Some(folder.path().to_path_buf()));
        AppSettings::default().save();

        let mut store = Store::new(AppSettings::load());
        store.change(Box::new(|s| s.theme = "Light".into()));
        store.change(Box::new(|s| s.top_offset = 20.0));
        store.change(Box::new(|s| s.top_offset = 24.0)); // a slider drag: only the last value counts

        // Meanwhile the island records where it lives.
        AppSettings::update(|s| s.island_path = Some(r"C:\Island\DynamicIsland.exe".into()));
        assert_eq!(store.current.island_path, None);

        store.save();
        assert!(!store.has_pending());
        let saved = AppSettings::try_load().unwrap();
        assert_eq!(saved.theme, "Light");
        assert_eq!(saved.top_offset, 24.0);
        assert_eq!(saved.island_path.as_deref(), Some(r"C:\Island\DynamicIsland.exe"));
        // The store picked up the island's change too.
        assert_eq!(store.current, saved);

        // Nothing pending: saving again doesn't touch the file.
        AppSettings::update(|s| s.sounds = false);
        store.save();
        assert!(!AppSettings::try_load().unwrap().sounds);
        paths::set_data_folder(None);
    }
}
