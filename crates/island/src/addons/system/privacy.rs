//! Orange dot while an app uses the microphone, green while one uses the camera, next to the pill (like the
//! iPhone's status dots). Windows records "in use right now" per app in the capability consent store; this reads
//! that, and pops up "Teams is using the microphone" when an app starts.

use crate::addons::hud::Hud;
use crate::addons::{Addon, Base, Context};
use crate::island::Dot;
use crate::reg::Key;
use crate::rt::{Handle, Timer, WideNull};
use crate::ui::colors::{self, Color};
use di4w_core::{AddonSettings, l};
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;
use std::time::Duration;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
use windows::Win32::System::Registry::{REG_NOTIFY_CHANGE_LAST_SET, REG_NOTIFY_CHANGE_NAME, RegNotifyChangeKeyValue};
use windows::Win32::System::Threading::{CreateEventW, INFINITE, SetEvent, WaitForMultipleObjects};
use windows::core::PCWSTR;

const STORE: &str = r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore";

struct State {
    mic: BTreeSet<String>,
    cam: BTreeSet<String>,
    poll: Option<Timer>,
    stop: Option<isize>,
    events: Option<Handle<()>>,
}

pub struct Privacy {
    base: Base,
    st: RefCell<State>,
}

/// Apps currently using the capability: a start time without a stop time.
fn in_use(capability: &str) -> BTreeSet<String> {
    let mut apps = BTreeSet::new();
    let Some(root) = Key::current_user(&format!(r"{STORE}\{capability}")) else { return apps };
    let active = |k: &Key| k.qword("LastUsedTimeStop") == Some(0) && k.qword("LastUsedTimeStart").is_some_and(|s| s != 0);
    for name in root.subkeys() {
        let Some(key) = root.sub(&name) else { continue };
        if name == "NonPackaged" {
            for exe in key.subkeys() {
                if key.sub(&exe).is_some_and(|k| active(&k)) {
                    apps.insert(desktop_app_name(&exe));
                }
            }
        } else if active(&key) {
            apps.insert(packaged_app_name(&name));
        }
    }
    apps
}

/// "C:#Program Files#Discord#app-1.0#Discord.exe" -> its file description, else "Discord".
fn desktop_app_name(encoded: &str) -> String {
    let path = encoded.replace('#', "\\");
    file_description(&path).unwrap_or_else(|| std::path::Path::new(&path).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or(path.clone()))
}

fn file_description(path: &str) -> Option<String> {
    let wide = path.encode_wide_null();
    unsafe {
        let size = GetFileVersionInfoSizeW(PCWSTR(wide.as_ptr()), None);
        if size == 0 {
            return None;
        }
        let mut data = vec![0u8; size as usize];
        GetFileVersionInfoW(PCWSTR(wide.as_ptr()), None, size, data.as_mut_ptr().cast()).ok()?;
        let mut ptr: *mut core::ffi::c_void = std::ptr::null_mut();
        let mut len = 0u32;
        let translation = "\\VarFileInfo\\Translation".encode_wide_null();
        if !VerQueryValueW(data.as_ptr().cast(), PCWSTR(translation.as_ptr()), &mut ptr, &mut len).as_bool() || len < 4 {
            return None;
        }
        let lang = *(ptr as *const u16);
        let codepage = *(ptr as *const u16).add(1);
        let query = format!("\\StringFileInfo\\{lang:04x}{codepage:04x}\\FileDescription").encode_wide_null();
        if !VerQueryValueW(data.as_ptr().cast(), PCWSTR(query.as_ptr()), &mut ptr, &mut len).as_bool() || len == 0 {
            return None;
        }
        let text = String::from_utf16_lossy(std::slice::from_raw_parts(ptr as *const u16, len as usize));
        let text = text.trim_end_matches('\0').trim().to_string();
        (!text.is_empty()).then_some(text)
    }
}

/// "Microsoft.WindowsCamera_8wekyb3d8bbwe" -> "WindowsCamera"
fn packaged_app_name(family: &str) -> String {
    let name = family.split('_').next().unwrap_or(family);
    name.rsplit('.').next().unwrap_or(name).to_string()
}

impl Privacy {
    pub fn new(ctx: &Rc<Context>) -> Rc<Privacy> {
        Rc::new(Privacy { base: Base::new(ctx, "PrivacyIndicators", "Privacy indicators"), st: RefCell::new(State { mic: BTreeSet::new(), cam: BTreeSet::new(), poll: None, stop: None, events: None }) })
    }

    fn render(&self) {
        let st = self.st.borrow();
        let dot = |color: Color, apps: &BTreeSet<String>, label: &str| Dot {
            color,
            visible: !apps.is_empty(),
            tooltip: (!apps.is_empty()).then(|| format!("{}: {}", label, apps.iter().cloned().collect::<Vec<_>>().join(", "))),
        };
        let dots = vec![dot(colors::green(), &st.cam, l::t("Camera")), dot(colors::orange(), &st.mic, l::t("Microphone"))];
        drop(st);
        self.base.ctx.island.set_dots(dots);
    }

    fn check(&self) {
        let (mic, cam) = (in_use("microphone"), in_use("webcam"));
        let (new_mic, new_cam) = {
            let mut st = self.st.borrow_mut();
            let new_mic = mic.difference(&st.mic).next().cloned();
            let new_cam = cam.difference(&st.cam).next().cloned();
            st.mic = mic;
            st.cam = cam;
            (new_mic, new_cam)
        };
        self.render();
        if let Some(app) = new_cam {
            self.announce(&l::f("{0} is using the camera", &[&app]), "\u{E714}", colors::green());
        } else if let Some(app) = new_mic {
            self.announce(&l::f("{0} is using the microphone", &[&app]), "\u{E720}", colors::orange());
        }
    }

    fn announce(&self, text: &str, glyph: &str, color: Color) {
        let hud = Hud::new(glyph, color);
        hud.set(None, text, None);
        self.base.show(hud.alert.clone());
    }
}

impl Addon for Privacy {
    fn base(&self) -> &Base {
        &self.base
    }

    fn enabled(&self, o: &AddonSettings) -> bool {
        o.privacy_indicators
    }

    fn start(self: Rc<Self>) -> Result<(), String> {
        {
            let mut st = self.st.borrow_mut();
            st.mic = in_use("microphone");
            st.cam = in_use("webcam");
        }
        self.render();

        // Windows signals an event when anything under the consent store changes (an app starts or stops using the
        // mic/camera), so there's no need to poll. A slow check stays as a safety net.
        let weak = Rc::downgrade(&self);
        let events = Handle::new(move |()| {
            if let Some(p) = weak.upgrade() {
                p.check();
            }
        });
        let stop = unsafe { CreateEventW(None, true, false, PCWSTR::null()) }.map_err(|e| e.message())?;
        let stop_raw = stop.0 as isize;
        let sender = events.clone();
        std::thread::spawn(move || {
            let stop = HANDLE(stop_raw as *mut _);
            let Some(store) = Key::current_user(STORE) else { return };
            let Ok(changed) = (unsafe { CreateEventW(None, false, false, PCWSTR::null()) }) else { return };
            loop {
                let status = unsafe { RegNotifyChangeKeyValue(store.0, true, REG_NOTIFY_CHANGE_NAME | REG_NOTIFY_CHANGE_LAST_SET, Some(changed), true) };
                if status.is_err() {
                    break;
                }
                let which = unsafe { WaitForMultipleObjects(&[changed, stop], false, INFINITE) };
                if which.0 != 0 {
                    break;
                }
                sender.send(());
            }
            unsafe {
                let _ = CloseHandle(changed);
            }
        });
        let weak = Rc::downgrade(&self);
        let mut st = self.st.borrow_mut();
        st.poll = Some(Timer::every(Duration::from_secs(15), move || {
            if let Some(p) = weak.upgrade() {
                p.check();
            }
        }));
        st.stop = Some(stop_raw);
        st.events = Some(events);
        Ok(())
    }

    fn stop(&self) {
        let mut st = self.st.borrow_mut();
        st.poll = None;
        if let Some(stop) = st.stop.take() {
            unsafe {
                let _ = SetEvent(HANDLE(stop as *mut _));
            }
        }
        if let Some(e) = st.events.take() {
            e.close();
        }
        drop(st);
        self.base.ctx.island.set_dots(Vec::new());
    }

    fn preview(self: Rc<Self>) {
        self.announce(&l::f("{0} is using the microphone", &[&"Teams"]), "\u{E720}", colors::orange());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_apps() {
        assert_eq!(packaged_app_name("Microsoft.WindowsCamera_8wekyb3d8bbwe"), "WindowsCamera");
        assert_eq!(desktop_app_name("C:#Nowhere#Discord.exe"), "Discord");
    }
}
