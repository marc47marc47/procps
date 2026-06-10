//! [PLATFORM:WINDOWS] Native Windows backend — reimplements the libproc2 functionality using Win32 APIs.
//!
//! Mapping between Linux /proc and Win32 (full table in PORTING.md):
//!
//! | Linux source         | Win32 API used here                                   |
//! |----------------------|-------------------------------------------------------|
//! | /proc/meminfo        | GlobalMemoryStatusEx, K32GetPerformanceInfo            |
//! | /proc/stat (cpu)     | GetSystemTimes                                         |
//! | /proc/uptime         | GetTickCount64                                         |
//! | /proc/[pid]/*        | Toolhelp32 snapshot + OpenProcess + GetProcessTimes/MemoryInfo |
//! | /proc/[pid]/cmdline  | NtQueryInformationProcess + ReadProcessMemory reading PEB |
//! | /proc/[pid]/cwd      | same as above (RTL_USER_PROCESS_PARAMETERS.CurrentDirectory) |
//! | /proc/[pid]/maps     | VirtualQueryEx + K32GetMappedFileNameW                 |
//! | kill(2)              | TerminateProcess (semantic differences in types::Signal comments) |
//! | utmp                 | WTSEnumerateSessionsW                                  |

#![allow(dead_code)]

mod cpu;
mod mem;
mod process;
mod sessions;

pub use cpu::*;
pub use mem::*;
pub use process::*;
pub use sessions::*;

use std::time::{Duration, SystemTime, UNIX_EPOCH};
use windows_sys::Win32::Foundation::FILETIME;

/// FILETIME (100ns ticks since 1601-01-01) → SystemTime.
/// The 1601 and 1970 epochs differ by 11_644_473_600 seconds.
pub(crate) fn filetime_to_systemtime(ft: FILETIME) -> Option<SystemTime> {
    let t = ((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64;
    const EPOCH_DIFF_100NS: u64 = 11_644_473_600 * 10_000_000;
    if t < EPOCH_DIFF_100NS {
        return None;
    }
    Some(UNIX_EPOCH + Duration::from_nanos((t - EPOCH_DIFF_100NS) * 100))
}

/// FILETIME duration (not a point in time) → milliseconds.
pub(crate) fn filetime_to_ms(ft: FILETIME) -> u64 {
    let t = ((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64;
    t / 10_000
}

/// UTF-16 (NUL-terminated) → String.
pub(crate) fn wide_to_string(buf: &[u16]) -> String {
    let len = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}
