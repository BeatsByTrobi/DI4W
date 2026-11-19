//! Downloads from any browser: size and speed in the pill while downloading, "Download complete" after. Browsers
//! write to a temporary file (Chrome/Edge ".crdownload", Firefox ".part") and rename it when done, so watching the
//! Downloads folder is enough, no browser extension needed.

use crate::addons::activity::AddonActivity;
use crate::addons::system::usb::format_size;
use crate::addons::{Addon, Base, Context};
use crate::model::Alert;
use crate::rt::{Change, Timer, Watcher};
use crate::ui::colors;
use crate::ui::el::{El, Len};
use crate::ui::{self, ICON_SIZE, NORMAL, SEMIBOLD};
use di4w_core::{AddonSettings, l};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;
use windows::Win32::Storage::FileSystem::{FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE, FILE_NOTIFY_CHANGE_SIZE};
use windows::Win32::UI::Shell::FOLDERID_Downloads;

const TEMP_TYPES: [&str; 4] = ["crdownload", "part", "partial", "download"];
const GLYPH: &str = "\u{E896}";

#[derive(Clone)]
struct Download {
    path: PathBuf,
    size: i64,
    speed: f64,
}

pub struct Downloads {
    base: Base,
    active: Rc<RefCell<Vec<Download>>>,
    activity: Rc<AddonActivity>,
    watcher: RefCell<Option<Watcher>>,
    tick: RefCell<Option<Timer>>,
}

fn is_temp(path: &Path) -> bool {
    path.extension().is_some_and(|e| TEMP_TYPES.iter().any(|t| e.to_string_lossy().eq_ignore_ascii_case(t)))
}

/// "photo.jpg.crdownload" -> "photo.jpg"; Chrome's placeholder "Unconfirmed 1234" -> "New download".
pub fn display_name(path: &str) -> String {
    let name = Path::new(path).file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    if name.to_lowercase().starts_with("unconfirmed ") { l::t("New download").to_string() } else { name }
}

impl Downloads {
    pub fn new(ctx: &Rc<Context>) -> Rc<Downloads> {
        let active: Rc<RefCell<Vec<Download>>> = Rc::new(RefCell::new(Vec::new()));
        let (a, b) = (active.clone(), active.clone());
        let activity = AddonActivity::new(
            ctx.manager.clone(),
            "Downloads",
            "Downloads",
            15,
            Box::new(|| ui::glyph(GLYPH, 15.0, colors::blue())),
            Box::new(move || {
                let list = a.borrow();
                let total: f64 = list.iter().map(|d| d.speed).sum();
                let text = if list.len() > 1 {
                    l::f("{0} files", &[&list.len()])
                } else if total > 0.0 {
                    format!("{}/s", format_size(total as i64))
                } else {
                    format_size(list.first().map_or(0, |d| d.size))
                };
                ui::digits(text, 14.0, SEMIBOLD, colors::blue())
            }),
            Box::new(|| ui::glyph(GLYPH, 13.0, colors::blue())),
            Box::new(move || {
                let list = b.borrow();
                let mut rows = vec![ui::text(l::t("Downloading"), 14.0, SEMIBOLD, colors::blue())];
                for d in list.iter().take(3) {
                    let speed = if d.speed > 0.0 { format!(" · {}/s", format_size(d.speed as i64)) } else { String::new() };
                    let detail = ui::digits(format!("{}{speed}", format_size(d.size)), 13.0, NORMAL, colors::grey()).margin(12.0, 0.0, 0.0, 0.0);
                    let name = ui::text(display_name(&d.path.to_string_lossy()), 14.0, NORMAL, colors::primary());
                    rows.push(El::grid(vec![Len::Star(1.0), Len::Auto], vec![], vec![(0, 0, name), (0, 1, detail)]).margin(0.0, 6.0, 0.0, 0.0));
                }
                El::vstack(rows).margin(18.0, 16.0, 20.0, 14.0)
            }),
            230.0,
            120.0,
            Some(colors::blue()),
        );
        Rc::new(Downloads { base: Base::new(ctx, "Downloads", "Downloads"), active, activity, watcher: RefCell::new(None), tick: RefCell::new(None) })
    }

    fn touched(self: &Rc<Self>, path: PathBuf) {
        if !is_temp(&path) || self.active.borrow().iter().any(|d| d.path == path) {
            return;
        }
        self.active.borrow_mut().push(Download { path, size: 0, speed: 0.0 });
        self.update(false);
    }

    fn renamed(self: &Rc<Self>, from: PathBuf, to: PathBuf) {
        let index = self.active.borrow().iter().position(|d| d.path == from);
        let Some(i) = index else {
            self.touched(to);
            return;
        };
        if is_temp(&to) {
            // Chrome: "Unconfirmed 1234.crdownload" -> "photo.jpg.crdownload"
            self.active.borrow_mut()[i].path = to;
        } else {
            self.active.borrow_mut().remove(i);
            self.finished(&to);
        }
        self.update(false);
    }

    fn removed(self: &Rc<Self>, path: PathBuf) {
        let before = self.active.borrow().len();
        self.active.borrow_mut().retain(|d| d.path != path); // cancelled
        if self.active.borrow().len() != before {
            self.update(false);
        }
    }

    fn update(self: &Rc<Self>, measure_speed: bool) {
        self.active.borrow_mut().retain_mut(|d| match std::fs::metadata(&d.path) {
            Ok(m) => {
                let size = m.len() as i64;
                if measure_speed {
                    d.speed = if d.size > 0 { (size - d.size).max(0) as f64 } else { 0.0 }; // bytes in the last second
                }
                d.size = size;
                true
            }
            Err(_) => false, // gone without a rename event
        });
        if self.active.borrow().is_empty() {
            self.activity.hide();
            *self.tick.borrow_mut() = None;
            return;
        }
        self.activity.show();
        // Ticking only while something is downloading, so an idle island has no timer running.
        if self.tick.borrow().is_none() {
            let me = Rc::downgrade(self);
            *self.tick.borrow_mut() = Some(Timer::every(Duration::from_secs(1), move || {
                if let Some(d) = me.upgrade() {
                    d.update(true);
                }
            }));
        }
    }

    fn finished(&self, path: &Path) {
        let size = std::fs::metadata(path).map(|m| m.len() as i64).unwrap_or(0);
        let p = path.to_path_buf();
        // Show it in Explorer rather than running it: downloads can be programs.
        self.show_complete(path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(), size, Some(Rc::new(move || crate::interop::show_in_folder(&p))));
    }

    fn show_complete(&self, name: String, size: i64, on_click: Option<Rc<dyn Fn()>>) {
        let detail = if size > 0 { format!("{name} · {}", format_size(size)) } else { name };
        let alert = Alert::new(move || ui::alert_view(ui::glyph_circle(GLYPH, colors::blue(), ICON_SIZE, 22.0), l::t("Download complete"), &detail, None, None))
            .seconds(4.0)
            .on_click_opt(on_click);
        self.base.show(alert.rc());
    }
}

impl Addon for Downloads {
    fn base(&self) -> &Base {
        &self.base
    }

    fn enabled(&self, o: &AddonSettings) -> bool {
        o.downloads
    }

    fn start(self: Rc<Self>) -> Result<(), String> {
        let folder = crate::interop::known_folder(&FOLDERID_Downloads).unwrap_or_else(|| PathBuf::from(std::env::var("USERPROFILE").unwrap_or_default()).join("Downloads"));
        let weak = Rc::downgrade(&self);
        let watcher = Watcher::new(&folder, FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_SIZE | FILE_NOTIFY_CHANGE_LAST_WRITE, move |changes| {
            let Some(d) = weak.upgrade() else { return };
            for change in changes {
                match change {
                    Change::Created(p) | Change::Modified(p) => d.touched(p),
                    Change::Deleted(p) => d.removed(p),
                    Change::Renamed(from, to) => d.renamed(from, to),
                }
            }
        })
        .map_err(|e| e.message())?;
        *self.watcher.borrow_mut() = Some(watcher);
        Ok(())
    }

    fn stop(&self) {
        *self.watcher.borrow_mut() = None;
        *self.tick.borrow_mut() = None;
        self.active.borrow_mut().clear();
        self.activity.hide();
    }

    fn preview(self: Rc<Self>) {
        self.show_complete(format!("{}.pdf", l::t("report")), 2_400_000, None);
    }
}

#[cfg(test)]
mod tests {
    use super::display_name;

    #[test]
    fn shows_download_names_without_the_temporary_extension() {
        assert_eq!(display_name(r"C:\Downloads\photo.jpg.crdownload"), "photo.jpg");
        assert_eq!(display_name(r"C:\Downloads\setup.exe.part"), "setup.exe");
        assert_eq!(display_name(r"C:\Downloads\Unconfirmed 123456.crdownload"), "New download");
    }
}
