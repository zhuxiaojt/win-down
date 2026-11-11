use std::path::Path;

use windows_sys::Win32::Storage::FileSystem::GetDriveTypeW;
use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows_sys::Win32::System::Threading::{
    GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_ABOVE_NORMAL, THREAD_PRIORITY_BELOW_NORMAL,
    THREAD_PRIORITY_NORMAL,
};

use crate::core::Policy;

// ---------------- path & string helpers ----------------

pub fn wide(s: &str) -> Vec<u16> {
    let mut v: Vec<u16> = s.encode_utf16().collect();
    v.push(0);
    v
}

pub fn is_remote_volume(path: &Path) -> bool {
    let s = path.to_string_lossy();
    let stripped = s.strip_prefix(r"\\?\").unwrap_or(&s);
    if stripped.starts_with(r"\\") {
        return true;
    }
    let root: String = stripped.chars().take(3).collect();
    if root.len() < 2 {
        return false;
    }
    let w = wide(&root);
    let ok = unsafe { GetDriveTypeW(w.as_ptr()) };
    ok == 4
}

// ---------------- cpu & memory sampling ----------------

pub fn mem_available_pct() -> f64 {
    let mut st: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    st.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
    let ok = unsafe { GlobalMemoryStatusEx(&mut st) };
    if ok == 0 || st.ullTotalPhys == 0 {
        return 100.0;
    }
    (st.ullAvailPhys as f64 / st.ullTotalPhys as f64) * 100.0
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct FileTimeRaw {
    pub low: u32,
    pub high: u32,
}

impl FileTimeRaw {
    pub fn as_u64(&self) -> u64 {
        (self.high as u64) << 32 | self.low as u64
    }
}

#[link(name = "kernel32")]
extern "system" {
    fn GetSystemTimes(
        idle: *mut FileTimeRaw,
        kernel: *mut FileTimeRaw,
        user: *mut FileTimeRaw,
    ) -> i32;
}

pub fn system_times() -> (u64, u64, u64) {
    let mut idle = FileTimeRaw::default();
    let mut kernel = FileTimeRaw::default();
    let mut user = FileTimeRaw::default();
    let ok = unsafe { GetSystemTimes(&mut idle, &mut kernel, &mut user) };
    if ok == 0 {
        (0, 0, 0)
    } else {
        (idle.as_u64(), kernel.as_u64(), user.as_u64())
    }
}

// ---------------- thread priority ----------------

pub fn apply_thread_priority(policy: Policy) {
    let p = match policy {
        Policy::FavorOtherApps => THREAD_PRIORITY_BELOW_NORMAL,
        Policy::MaxPerformance => THREAD_PRIORITY_ABOVE_NORMAL,
        Policy::Balanced => THREAD_PRIORITY_NORMAL,
    };
    unsafe {
        SetThreadPriority(GetCurrentThread(), p);
    }
}
