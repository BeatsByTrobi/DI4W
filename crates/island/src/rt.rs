//! The island's runtime: one UI thread with a Win32 message loop (WPF's Dispatcher), plus helpers to get back to
//! it from other threads, timers (WPF's DispatcherTimer), background work and folder watching.
//!
//! Everything that touches views, activities or the alert queue runs on the UI thread. WinRT events, audio
//! callbacks, file watchers and the Island API server run elsewhere and hand their results over with [`post`].

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Storage::FileSystem::*;
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, COINIT_MULTITHREADED, CoInitializeEx};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{CreateEventW, INFINITE, SetEvent, WaitForMultipleObjects};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};

/// Runs queued closures (posted from any thread).
pub const WM_RUN: u32 = WM_APP + 1;

static WINDOW: AtomicIsize = AtomicIsize::new(0);
static QUEUE: Mutex<VecDeque<Box<dyn FnOnce() + Send>>> = Mutex::new(VecDeque::new());
static WAKE_PENDING: AtomicBool = AtomicBool::new(false);

thread_local! {
    static LOCAL: RefCell<VecDeque<Box<dyn FnOnce()>>> = RefCell::new(VecDeque::new());
    static TIMERS: RefCell<BTreeMap<u64, TimerEntry>> = const { RefCell::new(BTreeMap::new()) };
    static NEXT_TIMER: Cell<u64> = const { Cell::new(1) };
    static WAITING: RefCell<HashMap<u64, Box<dyn FnOnce(Box<dyn Any + Send>)>>> = RefCell::new(HashMap::new());
    static NEXT_WAIT: Cell<u64> = const { Cell::new(1) };
}

struct TimerEntry {
    due: Instant,
    interval: Option<Duration>,
    action: Rc<RefCell<dyn FnMut()>>,
}

/// Starts the runtime on the calling thread (the UI thread): COM in a single-threaded apartment and the hidden
/// window that receives posted work and timer ticks.
pub fn init() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let instance = GetModuleHandleW(None).unwrap_or_default();
        let class = WNDCLASSW { lpfnWndProc: Some(proc), hInstance: instance.into(), lpszClassName: w!("DI4W.Runtime"), ..Default::default() };
        RegisterClassW(&class);
        let hwnd = CreateWindowExW(WINDOW_EX_STYLE(0), w!("DI4W.Runtime"), PCWSTR::null(), WINDOW_STYLE(0), 0, 0, 0, 0, Some(HWND_MESSAGE), None, Some(instance.into()), None)
            .unwrap_or_default();
        WINDOW.store(hwnd.0 as isize, Ordering::SeqCst);
    }
}


fn window() -> HWND {
    HWND(WINDOW.load(Ordering::SeqCst) as *mut _)
}

fn wake() {
    let hwnd = window();
    if !hwnd.is_invalid() && !WAKE_PENDING.swap(true, Ordering::SeqCst) {
        unsafe {
            let _ = PostMessageW(Some(hwnd), WM_RUN, WPARAM(0), LPARAM(0));
        }
    }
}

/// Runs `f` on the UI thread soon (WPF's `Dispatcher.BeginInvoke`). Callable from any thread.
pub fn post(f: impl FnOnce() + Send + 'static) {
    QUEUE.lock().unwrap_or_else(|e| e.into_inner()).push_back(Box::new(f));
    wake();
}

/// From the UI thread: runs `f` after the current message has been handled.
pub fn defer(f: impl FnOnce() + 'static) {
    LOCAL.with(|l| l.borrow_mut().push_back(Box::new(f)));
    wake();
}

/// Runs `work` on a background thread (COM in the multithreaded apartment, as WinRT likes it), then `done` with its
/// result on the UI thread. Call from the UI thread.
pub fn spawn<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static, done: impl FnOnce(T) + 'static) {
    let id = NEXT_WAIT.with(|n| {
        let id = n.get();
        n.set(id + 1);
        id
    });
    WAITING.with(|w| {
        w.borrow_mut().insert(
            id,
            Box::new(move |any: Box<dyn Any + Send>| {
                if let Ok(value) = any.downcast::<T>() {
                    done(*value);
                }
            }),
        )
    });
    background(move || {
        let result = work();
        post(move || {
            if let Some(done) = WAITING.with(|w| w.borrow_mut().remove(&id)) {
                done(Box::new(result));
            }
        });
    });
}

/// Runs `work` on a new background thread with COM initialized; panics are logged, never fatal.
pub fn background(work: impl FnOnce() + Send + 'static) {
    let _ = std::thread::Builder::new().spawn(move || {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)).is_err() {
            di4w_core::log::error("A background task failed (see the previous error)");
        }
    });
}

/// Runs a UI callback, logging a panic instead of taking the island down (WPF's `DispatcherUnhandledException`
/// with `Handled = true`).
pub fn guard<R>(f: impl FnOnce() -> R) -> Option<R> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(r) => Some(r),
        Err(_) => {
            di4w_core::log::error("Unhandled UI exception (see the previous error)");
            None
        }
    }
}

fn run_queued() {
    WAKE_PENDING.store(false, Ordering::SeqCst);
    loop {
        let next: Option<Box<dyn FnOnce() + Send>> = QUEUE.lock().unwrap_or_else(|e| e.into_inner()).pop_front();
        match next {
            Some(f) => {
                guard(f);
            }
            None => break,
        }
    }
    loop {
        let next = LOCAL.with(|l| l.borrow_mut().pop_front());
        match next {
            Some(f) => {
                guard(f);
            }
            None => break,
        }
    }
}

/// A UI-thread callback that other threads can send values to, as often as they like (a WinRT event, a watcher).
pub struct Handle<T> {
    id: u64,
    _t: std::marker::PhantomData<fn(T)>,
}

impl<T> Clone for Handle<T> {
    fn clone(&self) -> Self {
        Handle { id: self.id, _t: std::marker::PhantomData }
    }
}

thread_local! {
    static HANDLES: RefCell<HashMap<u64, Rc<dyn Fn(Box<dyn Any>)>>> = RefCell::new(HashMap::new());
}

impl<T: Send + 'static> Handle<T> {
    /// Call on the UI thread; `f` runs there with each value sent.
    pub fn new(f: impl Fn(T) + 'static) -> Handle<T> {
        let id = NEXT_WAIT.with(|n| {
            let id = n.get();
            n.set(id + 1);
            id
        });
        HANDLES.with(|h| {
            h.borrow_mut().insert(
                id,
                Rc::new(move |any: Box<dyn Any>| {
                    if let Ok(v) = any.downcast::<T>() {
                        f(*v);
                    }
                }),
            )
        });
        Handle { id, _t: std::marker::PhantomData }
    }

    /// From any thread.
    pub fn send(&self, value: T) {
        let id = self.id;
        post(move || {
            if let Some(f) = HANDLES.with(|h| h.borrow().get(&id).cloned()) {
                f(Box::new(value));
            }
        });
    }

    /// Forgets the callback (UI thread).
    pub fn close(&self) {
        HANDLES.with(|h| h.borrow_mut().remove(&self.id));
    }
}

// ───────────────────────────── timers ─────────────────────────────

/// A timer on the UI thread; stops when dropped (unless [`Timer::forget`] is called).
pub struct Timer(u64);

impl Timer {
    /// Calls `f` every `interval` (first after one interval), like a started DispatcherTimer.
    pub fn every(interval: Duration, f: impl FnMut() + 'static) -> Timer {
        Timer(add(Instant::now() + interval, Some(interval), Rc::new(RefCell::new(f))))
    }

    /// Calls `f` once after `delay`.
    pub fn once(delay: Duration, f: impl FnOnce() + 'static) -> Timer {
        let mut f = Some(f);
        let id = add(Instant::now() + delay, None, Rc::new(RefCell::new(move || {
            if let Some(f) = f.take() {
                f();
            }
        })));
        Timer(id)
    }

    /// Changes the interval of a repeating timer; the next tick comes one new interval from now.
    pub fn set_interval(&self, interval: Duration) {
        TIMERS.with(|t| {
            if let Some(e) = t.borrow_mut().get_mut(&self.0)
                && e.interval != Some(interval) {
                    e.interval = Some(interval);
                    e.due = Instant::now() + interval;
                }
        });
        arm();
    }


    /// Keeps the timer running after the handle is dropped (a one-shot that must still fire).
    pub fn forget(self) {
        std::mem::forget(self);
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        let _ = TIMERS.try_with(|t| t.borrow_mut().remove(&self.0));
    }
}

/// Runs `f` once after `delay` (fire and forget).
pub fn after(delay: Duration, f: impl FnOnce() + 'static) {
    Timer::once(delay, f).forget();
}

fn add(due: Instant, interval: Option<Duration>, action: Rc<RefCell<dyn FnMut()>>) -> u64 {
    let id = NEXT_TIMER.with(|n| {
        let id = n.get();
        n.set(id + 1);
        id
    });
    TIMERS.with(|t| t.borrow_mut().insert(id, TimerEntry { due, interval, action }));
    arm();
    id
}

const TICK_TIMER: usize = 1;

/// Sets the one Win32 timer to the earliest due timer.
fn arm() {
    let hwnd = window();
    if hwnd.is_invalid() {
        return;
    }
    let next = TIMERS.with(|t| t.borrow().values().map(|e| e.due).min());
    unsafe {
        match next {
            Some(due) => {
                let ms = due.saturating_duration_since(Instant::now()).as_millis().clamp(1, u32::MAX as u128) as u32;
                SetTimer(Some(hwnd), TICK_TIMER, ms, None);
            }
            None => {
                let _ = KillTimer(Some(hwnd), TICK_TIMER);
            }
        }
    }
}

fn run_timers() {
    let now = Instant::now() + Duration::from_millis(1);
    let due: Vec<(u64, bool, Rc<RefCell<dyn FnMut()>>)> = TIMERS.with(|t| {
        let mut t = t.borrow_mut();
        let ids: Vec<u64> = t.iter().filter(|(_, e)| e.due <= now).map(|(id, _)| *id).collect();
        ids.into_iter()
            .filter_map(|id| {
                let e = t.get_mut(&id)?;
                let action = e.action.clone();
                let once = e.interval.is_none();
                match e.interval {
                    Some(i) => e.due = Instant::now() + i,
                    None => {
                        t.remove(&id);
                    }
                }
                Some((id, once, action))
            })
            .collect()
    });
    for (id, once, action) in due {
        // A repeating timer stopped by an earlier one in this batch doesn't run.
        let alive = once || TIMERS.with(|t| t.borrow().contains_key(&id));
        if alive && let Ok(mut f) = action.try_borrow_mut() {
            guard(&mut *f);
        }
    }
    arm();
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_RUN => {
            run_queued();
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == TICK_TIMER => {
            run_timers();
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// Runs the message loop until [`quit`].
pub fn run() {
    unsafe {
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

/// Ends the message loop (Application.Shutdown).
pub fn quit() {
    unsafe { PostQuitMessage(0) };
}

// ───────────────────────────── folder watching ─────────────────────────────

/// What happened to a file in a watched folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    Created(PathBuf),
    Deleted(PathBuf),
    Modified(PathBuf),
    /// Old name, new name.
    Renamed(PathBuf, PathBuf),
}

/// Watches a folder (FileSystemWatcher) until dropped. `on_change` runs on the UI thread.
pub struct Watcher {
    stop: HANDLE,
}

unsafe impl Send for Watcher {}

impl Watcher {
    /// `filter`: which changes count (file names, sizes, last writes).
    pub fn new(folder: &Path, filter: FILE_NOTIFY_CHANGE, on_change: impl Fn(Vec<Change>) + 'static) -> windows::core::Result<Watcher> {
        let wide: Vec<u16> = folder.as_os_str().encode_wide_null();
        unsafe {
            let dir = CreateFileW(
                PCWSTR(wide.as_ptr()),
                FILE_LIST_DIRECTORY.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED,
                None,
            )?;
            let stop = CreateEventW(None, true, false, PCWSTR::null())?;
            let io_event = CreateEventW(None, true, false, PCWSTR::null())?;
            let folder = folder.to_path_buf();
            let callback = Rc::new(on_change);
            // The callback isn't Send: keep it on the UI thread and let the watcher thread refer to it by id.
            let id = NEXT_WAIT.with(|n| {
                let id = n.get();
                n.set(id + 1);
                id
            });
            WATCH_CALLBACKS.with(|c| c.borrow_mut().insert(id, callback));
            let (dir_raw, stop_raw, io_raw) = (dir.0 as isize, stop.0 as isize, io_event.0 as isize);
            std::thread::spawn(move || {
                let (dir, stop, io_event) = (HANDLE(dir_raw as *mut _), HANDLE(stop_raw as *mut _), HANDLE(io_raw as *mut _));
                let mut buffer = vec![0u32; 16 * 1024];
                loop {
                    let mut overlapped = OVERLAPPED { hEvent: io_event, ..Default::default() };
                    let _ = windows::Win32::System::Threading::ResetEvent(io_event);
                    if ReadDirectoryChangesW(dir, buffer.as_mut_ptr().cast(), (buffer.len() * 4) as u32, false, filter, None, Some(&mut overlapped), None).is_err() {
                        break;
                    }
                    let which = WaitForMultipleObjects(&[io_event, stop], false, INFINITE);
                    if which.0 != 0 {
                        let _ = CancelIoEx(dir, Some(&overlapped));
                        let mut n = 0;
                        let _ = GetOverlappedResult(dir, &overlapped, &mut n, true);
                        break;
                    }
                    let mut bytes = 0u32;
                    if GetOverlappedResult(dir, &overlapped, &mut bytes, false).is_err() {
                        break;
                    }
                    let changes = parse_changes(&buffer, bytes as usize, &folder);
                    if !changes.is_empty() {
                        post(move || {
                            if let Some(cb) = WATCH_CALLBACKS.with(|c| c.borrow().get(&id).cloned()) {
                                cb(changes);
                            }
                        });
                    }
                }
                let _ = CloseHandle(dir);
                let _ = CloseHandle(io_event);
                post(move || {
                    WATCH_CALLBACKS.with(|c| c.borrow_mut().remove(&id));
                });
            });
            Ok(Watcher { stop })
        }
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        unsafe {
            let _ = SetEvent(self.stop);
            // The watcher thread still waits on it for a moment; the handle is tiny, so it's left open.
        }
    }
}

thread_local! {
    static WATCH_CALLBACKS: RefCell<HashMap<u64, Rc<dyn Fn(Vec<Change>)>>> = RefCell::new(HashMap::new());
}

fn parse_changes(buffer: &[u32], len: usize, folder: &Path) -> Vec<Change> {
    let bytes = unsafe { std::slice::from_raw_parts(buffer.as_ptr().cast::<u8>(), len.min(buffer.len() * 4)) };
    let mut out = Vec::new();
    let mut offset = 0usize;
    let mut old_name: Option<PathBuf> = None;
    while offset + 12 <= bytes.len() {
        let next = u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
        let action = u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().unwrap());
        let name_len = u32::from_le_bytes(bytes[offset + 8..offset + 12].try_into().unwrap()) as usize;
        let start = offset + 12;
        let end = (start + name_len).min(bytes.len());
        let units: Vec<u16> = bytes[start..end].as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
        let path = folder.join(String::from_utf16_lossy(&units));
        match FILE_ACTION(action) {
            FILE_ACTION_ADDED => out.push(Change::Created(path)),
            FILE_ACTION_REMOVED => out.push(Change::Deleted(path)),
            FILE_ACTION_MODIFIED => out.push(Change::Modified(path)),
            FILE_ACTION_RENAMED_OLD_NAME => old_name = Some(path),
            FILE_ACTION_RENAMED_NEW_NAME => out.push(Change::Renamed(old_name.take().unwrap_or_default(), path)),
            _ => {}
        }
        if next == 0 {
            break;
        }
        offset += next;
    }
    out
}

/// UTF-16 with a terminating NUL, for Win32 calls.
pub trait WideNull {
    fn encode_wide_null(&self) -> Vec<u16>;
}

impl WideNull for std::ffi::OsStr {
    fn encode_wide_null(&self) -> Vec<u16> {
        use std::os::windows::ffi::OsStrExt;
        self.encode_wide().chain(std::iter::once(0)).collect()
    }
}

impl WideNull for str {
    fn encode_wide_null(&self) -> Vec<u16> {
        self.encode_utf16().chain(std::iter::once(0)).collect()
    }
}
