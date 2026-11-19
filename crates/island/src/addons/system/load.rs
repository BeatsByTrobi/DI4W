//! CPU, memory and GPU readings shared by the performance warnings and the game overlay.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{CloseHandle, FILETIME, HMODULE};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::System::Performance::*;
use windows::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows::Win32::System::Threading::{GetProcessTimes, GetSystemTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
use windows::core::{PCWSTR, s, w};

fn ticks(t: FILETIME) -> u64 {
    ((t.dwHighDateTime as u64) << 32) | t.dwLowDateTime as u64
}

/// Whole-system CPU use since this meter's previous reading. One meter per user, so readers don't interfere.
#[derive(Default)]
pub struct CpuMeter {
    last_idle: u64,
    last_total: u64,
}

impl CpuMeter {
    /// 0..100; the first reading is 0.
    pub fn read(&mut self) -> f64 {
        let (mut idle, mut kernel, mut user) = (FILETIME::default(), FILETIME::default(), FILETIME::default());
        if unsafe { GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user)) }.is_err() {
            return 0.0;
        }
        let (idle, total) = (ticks(idle), ticks(kernel) + ticks(user)); // kernel time includes idle time
        let (d_idle, d_total) = (idle.wrapping_sub(self.last_idle), total.wrapping_sub(self.last_total));
        let first = self.last_total == 0;
        self.last_idle = idle;
        self.last_total = total;
        if first || d_total == 0 { 0.0 } else { (100.0 * (d_total as f64 - d_idle as f64) / d_total as f64).clamp(0.0, 100.0) }
    }
}

fn cpu_times() -> HashMap<u32, (String, u64)> {
    let mut out = HashMap::new();
    for (pid, name) in crate::interop::processes() {
        if pid == 0 {
            continue;
        }
        unsafe {
            let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { continue };
            let (mut c, mut e, mut k, mut u) = Default::default();
            if GetProcessTimes(h, &mut c, &mut e, &mut k, &mut u).is_ok() {
                out.insert(pid, (name, ticks(k) + ticks(u)));
            }
            let _ = CloseHandle(h);
        }
    }
    out
}

/// The process using the most CPU over a one second window (`None` if none could be read). Blocks for a second.
pub fn top_cpu_process() -> Option<String> {
    let before = cpu_times();
    std::thread::sleep(Duration::from_secs(1));
    let after = cpu_times();
    after
        .iter()
        .filter_map(|(pid, (name, t))| before.get(pid).map(|(_, b)| (t.saturating_sub(*b), name)))
        .max_by_key(|(d, _)| *d)
        .map(|(_, n)| n.clone())
}

pub fn memory_percent() -> u32 {
    let mut status = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    if unsafe { GlobalMemoryStatusEx(&mut status) }.is_ok() { status.dwMemoryLoad } else { 0 }
}

pub fn top_memory_process() -> Option<String> {
    let mut best: Option<(usize, String)> = None;
    for (pid, name) in crate::interop::processes() {
        unsafe {
            let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { continue };
            let mut counters = PROCESS_MEMORY_COUNTERS { cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32, ..Default::default() };
            if K32GetProcessMemoryInfo(h, &mut counters, counters.cb).as_bool() && best.as_ref().is_none_or(|(m, _)| counters.WorkingSetSize > *m) {
                best = Some((counters.WorkingSetSize, name));
            }
            let _ = CloseHandle(h);
        }
    }
    best.map(|(_, n)| n)
}

// ───────────────────────────── GPU ─────────────────────────────

type NvmlInit = unsafe extern "C" fn() -> i32;
type NvmlHandle = unsafe extern "C" fn(u32, *mut isize) -> i32;
type NvmlTemp = unsafe extern "C" fn(isize, i32, *mut u32) -> i32;
type NvmlUtil = unsafe extern "C" fn(isize, *mut [u32; 2]) -> i32;

struct Nvml {
    device: isize,
    temperature: NvmlTemp,
    utilization: NvmlUtil,
}

// The NVML functions are plain C and thread-safe.
unsafe impl Send for Nvml {}
unsafe impl Sync for Nvml {}

/// The NVIDIA driver's management library, when there is one.
fn nvml() -> Option<&'static Nvml> {
    static NVML: OnceLock<Option<Nvml>> = OnceLock::new();
    NVML.get_or_init(|| unsafe {
        let lib: HMODULE = LoadLibraryW(w!("nvml.dll")).ok()?;
        let init: NvmlInit = std::mem::transmute(GetProcAddress(lib, s!("nvmlInit_v2"))?);
        let handle: NvmlHandle = std::mem::transmute(GetProcAddress(lib, s!("nvmlDeviceGetHandleByIndex_v2"))?);
        let temperature: NvmlTemp = std::mem::transmute(GetProcAddress(lib, s!("nvmlDeviceGetTemperature"))?);
        let utilization: NvmlUtil = std::mem::transmute(GetProcAddress(lib, s!("nvmlDeviceGetUtilizationRates"))?);
        if init() != 0 {
            return None;
        }
        let mut device = 0isize;
        (handle(0, &mut device) == 0).then_some(Nvml { device, temperature, utilization })
    })
    .as_ref()
}

/// GPU temperature in °C (NVIDIA cards only; `None` otherwise).
pub fn gpu_temperature() -> Option<i32> {
    let n = nvml()?;
    let mut t = 0u32;
    (unsafe { (n.temperature)(n.device, 0, &mut t) } == 0).then_some(t as i32)
}

struct Counters {
    query: PDH_HQUERY,
    counter: PDH_HCOUNTER,
    made: Instant,
}

thread_local! {
    static GPU: RefCell<(Option<Counters>, Option<f64>)> = const { RefCell::new((None, None)) };
}

/// GPU (3D engine) use 0..100: from the NVIDIA driver when available, else Windows' GPU counters.
pub fn gpu_percent() -> Option<f64> {
    if let Some(n) = nvml() {
        let mut u = [0u32; 2];
        if unsafe { (n.utilization)(n.device, &mut u) } == 0 {
            return Some(u[0] as f64);
        }
    }
    GPU.with(|g| {
        let mut g = g.borrow_mut();
        // Counter instances are per process and engine; refresh the set now and then as processes come and go.
        if g.0.as_ref().is_none_or(|c| c.made.elapsed() > Duration::from_secs(15)) {
            if let Some(old) = g.0.take() {
                unsafe { PdhCloseQuery(old.query) };
            }
            unsafe {
                let mut query = PDH_HQUERY::default();
                if PdhOpenQueryW(PCWSTR::null(), 0, &mut query) != 0 {
                    return None;
                }
                let mut counter = PDH_HCOUNTER::default();
                if PdhAddEnglishCounterW(query, w!("\\GPU Engine(*engtype_3D)\\Utilization Percentage"), 0, &mut counter) != 0 {
                    PdhCloseQuery(query);
                    return None;
                }
                PdhCollectQueryData(query); // the first reading is always 0
                g.0 = Some(Counters { query, counter, made: Instant::now() });
            }
            return g.1; // fresh counters read 0 the first time; repeat the previous value
        }
        let c = g.0.as_ref()?;
        unsafe {
            if PdhCollectQueryData(c.query) != 0 {
                return g.1;
            }
            let (mut size, mut count) = (0u32, 0u32);
            let _ = PdhGetFormattedCounterArrayW(c.counter, PDH_FMT_DOUBLE, &mut size, &mut count, None);
            if size == 0 {
                g.1 = Some(0.0);
                return g.1;
            }
            let mut buffer = vec![0u8; size as usize];
            let items = buffer.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W;
            if PdhGetFormattedCounterArrayW(c.counter, PDH_FMT_DOUBLE, &mut size, &mut count, Some(items)) != 0 {
                return g.1;
            }
            let sum: f64 = std::slice::from_raw_parts(items, count as usize).iter().map(|i| i.FmtValue.Anonymous.doubleValue).sum();
            g.1 = Some(sum.clamp(0.0, 100.0));
            g.1
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_system() {
        let mut cpu = CpuMeter::default();
        assert_eq!(cpu.read(), 0.0);
        std::thread::sleep(Duration::from_millis(50));
        assert!((0.0..=100.0).contains(&cpu.read()));
        assert!(memory_percent() > 0);
        assert!(top_memory_process().is_some());
    }
}
