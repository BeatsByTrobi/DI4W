//! Thumbnail of a new screenshot. Watches Pictures\Screenshots, where Win+PrtScn and the Snipping Tool save them.
//! Click opens the image; the folder button shows it in Explorer.

use crate::addons::{Addon, Base, Context};
use crate::model::Alert;
use crate::rt::{Change, Watcher};
use crate::ui::colors;
use crate::ui::el::{El, key};
use crate::ui::image::{self, Decode, Image};
use crate::ui::{self, ICON_SIZE};
use di4w_core::{AddonSettings, l};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};
use windows::Win32::Storage::FileSystem::FILE_NOTIFY_CHANGE_FILE_NAME;
use windows::Win32::UI::Shell::FOLDERID_Screenshots;

const IMAGE_TYPES: [&str; 5] = ["png", "jpg", "jpeg", "bmp", "gif"];

pub struct Screenshots {
    base: Base,
    watcher: RefCell<Option<Watcher>>,
    last_file: RefCell<Option<PathBuf>>,
}

impl Screenshots {
    pub fn new(ctx: &Rc<Context>) -> Rc<Screenshots> {
        Rc::new(Screenshots { base: Base::new(ctx, "Screenshots", "Screenshots"), watcher: RefCell::new(None), last_file: RefCell::new(None) })
    }

    fn arrived(self: &Rc<Self>, path: PathBuf) {
        let is_image = path.extension().is_some_and(|e| IMAGE_TYPES.iter().any(|t| e.to_string_lossy().eq_ignore_ascii_case(t)));
        if !is_image || self.last_file.borrow().as_ref() == Some(&path) {
            return;
        }
        *self.last_file.borrow_mut() = Some(path.clone());
        super::clipboard::LAST_SCREENSHOT.with(|t| t.set(Some(Instant::now())));
        let me = Rc::downgrade(self);
        let p = path.clone();
        crate::rt::spawn(move || load_thumbnail(&p), move |thumb| {
            let (Some(s), Some(thumb)) = (me.upgrade(), thumb) else { return };
            let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let image = Image::new(thumb);
            let (folder, open) = (path.clone(), path.clone());
            s.show_saved(Rc::new(move || ui::image(Some(image.id), ICON_SIZE, 12.0)), name, Rc::new(move || crate::interop::show_in_folder(&folder)), Some(Rc::new(move || crate::interop::shell_open(&open.to_string_lossy()))));
        });
    }

    /// The pop-up for a saved screenshot; the settings app's preview passes a camera icon and no actions.
    fn show_saved(&self, picture: Rc<dyn Fn() -> El>, name: String, show_in_folder: Rc<dyn Fn()>, open: Option<Rc<dyn Fn()>>) {
        let alert = Alert::new(move || {
            let f = show_in_folder.clone();
            let button = ui::circle_button(key("screenshot.folder"), ui::glyph("\u{E8B7}", 15.0, colors::primary()), colors::button_bg(), 36.0, move || f(), Some(l::t("Show in folder").into()));
            ui::alert_view(picture(), l::t("Screenshot saved"), &name, None, Some(button))
        })
        .seconds(4.0)
        .on_click_opt(open);
        self.base.show(alert.rc());
    }
}

/// The capture tool may still be writing; retry for a moment.
fn load_thumbnail(path: &Path) -> Option<image::Bitmap> {
    for attempt in 0..10 {
        match image::decode_file(path, Decode { width: None, height: Some(120), square: false }) {
            Ok(b) => return Some(b),
            Err(e) if attempt == 9 => di4w_core::log::info(format!("Screenshot thumbnail failed: {}", e.message())),
            Err(_) => std::thread::sleep(Duration::from_millis(200)),
        }
    }
    None
}

impl Addon for Screenshots {
    fn base(&self) -> &Base {
        &self.base
    }

    fn enabled(&self, o: &AddonSettings) -> bool {
        o.screenshots
    }

    fn start(self: Rc<Self>) -> Result<(), String> {
        let folder = crate::interop::known_folder(&FOLDERID_Screenshots)
            .unwrap_or_else(|| PathBuf::from(std::env::var("USERPROFILE").unwrap_or_default()).join("Pictures").join("Screenshots"));
        std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
        let weak = Rc::downgrade(&self);
        let watcher = Watcher::new(&folder, FILE_NOTIFY_CHANGE_FILE_NAME, move |changes| {
            let Some(s) = weak.upgrade() else { return };
            for change in changes {
                match change {
                    Change::Created(p) | Change::Renamed(_, p) => s.arrived(p),
                    _ => {}
                }
            }
        })
        .map_err(|e| e.message())?;
        *self.watcher.borrow_mut() = Some(watcher);
        Ok(())
    }

    fn stop(&self) {
        *self.watcher.borrow_mut() = None;
    }

    fn preview(self: Rc<Self>) {
        let name = format!("{} {}.png", l::t("Screenshot"), chrono::Local::now().format("%Y-%m-%d %H%M%S"));
        self.show_saved(Rc::new(|| ui::glyph_circle("\u{E722}", colors::button_bg(), ICON_SIZE, 22.0)), name, Rc::new(|| {}), None);
    }
}
