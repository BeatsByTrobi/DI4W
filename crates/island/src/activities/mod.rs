//! The built-in live activities: music, the timer (with its picker and the Recent list), the focus timer, the
//! stopwatch, notifications from chat apps, and the Island API for scripts.

pub mod api;
pub mod focus;
pub mod media;
pub mod notifications;
pub mod stopwatch;
pub mod timer;

use std::time::{Duration, Instant};

/// .NET's `Stopwatch`: elapsed time that can be paused and resumed.
#[derive(Clone, Copy, Debug, Default)]
pub struct Watch {
    banked: Duration,
    since: Option<Instant>,
}

impl Watch {
    pub fn elapsed(&self) -> Duration {
        self.banked + self.since.map_or(Duration::ZERO, |s| s.elapsed())
    }

    pub fn is_running(&self) -> bool {
        self.since.is_some()
    }

    pub fn start(&mut self) {
        if self.since.is_none() {
            self.since = Some(Instant::now());
        }
    }

    pub fn stop(&mut self) {
        if let Some(s) = self.since.take() {
            self.banked += s.elapsed();
        }
    }

    /// Zero, and running.
    pub fn restart(&mut self) {
        self.banked = Duration::ZERO;
        self.since = Some(Instant::now());
    }

    /// Zero, and stopped.
    pub fn reset(&mut self) {
        self.banked = Duration::ZERO;
        self.since = None;
    }

    pub fn toggle(&mut self) {
        if self.is_running() { self.stop() } else { self.start() }
    }
}

/// Sounds from `C:\Windows\Media`.
pub mod sound {
    use crate::rt::WideNull;
    use std::path::PathBuf;
    use windows::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_FILENAME, SND_LOOP, SND_NODEFAULT};
    use windows::Win32::System::Diagnostics::Debug::MessageBeep;
    use windows::Win32::UI::WindowsAndMessaging::MB_ICONASTERISK;
    use windows::core::PCWSTR;

    /// A file in Windows' Media folder.
    pub fn media(name: &str) -> PathBuf {
        let windows = std::env::var_os("SystemRoot").or_else(|| std::env::var_os("windir")).unwrap_or_else(|| "C:\\Windows".into());
        PathBuf::from(windows).join("Media").join(name)
    }

    fn play_file(path: &std::path::Path, looping: bool) -> bool {
        let wide = path.as_os_str().encode_wide_null();
        let flags = SND_FILENAME | SND_ASYNC | SND_NODEFAULT | if looping { SND_LOOP } else { Default::default() };
        unsafe { PlaySoundW(PCWSTR(wide.as_ptr()), None, flags).as_bool() }
    }

    /// Plays a file once (in the background).
    pub fn play(path: &std::path::Path) -> bool {
        play_file(path, false)
    }

    /// Plays a file over and over until [`stop`].
    pub fn play_looping(path: &std::path::Path) -> bool {
        play_file(path, true)
    }

    pub fn stop() {
        unsafe {
            let _ = PlaySoundW(PCWSTR::null(), None, Default::default());
        }
    }

    /// Windows' "Asterisk" system sound.
    pub fn asterisk() {
        unsafe {
            let _ = MessageBeep(MB_ICONASTERISK);
        }
    }
}

/// A time as Windows shows short times for the user's region ("14:05" or "2:05 PM").
pub fn short_time(time: chrono::DateTime<chrono::Local>) -> String {
    use chrono::{Datelike, Timelike};
    use windows::Win32::Foundation::SYSTEMTIME;
    use windows::Win32::Globalization::{GetTimeFormatEx, TIME_NOSECONDS};
    use windows::core::PCWSTR;
    let st = SYSTEMTIME {
        wYear: time.year() as u16,
        wMonth: time.month() as u16,
        wDayOfWeek: time.weekday().num_days_from_sunday() as u16,
        wDay: time.day() as u16,
        wHour: time.hour() as u16,
        wMinute: time.minute() as u16,
        wSecond: time.second() as u16,
        wMilliseconds: 0,
    };
    let mut buf = [0u16; 64];
    let n = unsafe { GetTimeFormatEx(PCWSTR::null(), TIME_NOSECONDS, Some(&st), PCWSTR::null(), Some(&mut buf)) };
    if n > 1 { String::from_utf16_lossy(&buf[..n as usize - 1]) } else { time.format("%H:%M").to_string() }
}

/// The user's decimal separator ("." or ","), as .NET formats numbers for the current culture.
pub fn decimal_separator() -> String {
    use windows::Win32::Globalization::{GetLocaleInfoEx, LOCALE_SDECIMAL};
    use windows::core::PCWSTR;
    let mut buf = [0u16; 8];
    let n = unsafe { GetLocaleInfoEx(PCWSTR::null(), LOCALE_SDECIMAL, Some(&mut buf)) };
    if n > 1 { String::from_utf16_lossy(&buf[..n as usize - 1]) } else { ".".into() }
}

/// Asks the island to draw again (a countdown changed).
pub fn redraw() {
    if let Some(i) = crate::island::get() {
        i.request_redraw();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watch_pauses_and_resumes() {
        let mut w = Watch::default();
        assert!(!w.is_running());
        w.restart();
        std::thread::sleep(Duration::from_millis(20));
        w.stop();
        let paused = w.elapsed();
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(w.elapsed(), paused);
        assert!(paused >= Duration::from_millis(20));
        w.reset();
        assert_eq!(w.elapsed(), Duration::ZERO);
    }
}
