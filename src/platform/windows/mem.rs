//! [PLATFORM:WINDOWS] Memory information — replaces /proc/meminfo.

use std::io;
use std::mem;

use windows_sys::Win32::System::ProcessStatus::{K32GetPerformanceInfo, PERFORMANCE_INFORMATION};
use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

use crate::platform::types::MemInfo;

pub fn mem_info() -> io::Result<MemInfo> {
    // SAFETY: zero-initialize the struct then set dwLength, as required by the documentation
    unsafe {
        let mut ms: MEMORYSTATUSEX = mem::zeroed();
        ms.dwLength = mem::size_of::<MEMORYSTATUSEX>() as u32;
        if GlobalMemoryStatusEx(&mut ms) == 0 {
            return Err(io::Error::last_os_error());
        }

        // SystemCache (file cache page count) ≈ Linux's Cached
        let mut pi: PERFORMANCE_INFORMATION = mem::zeroed();
        pi.cb = mem::size_of::<PERFORMANCE_INFORMATION>() as u32;
        let cached = if K32GetPerformanceInfo(&mut pi, pi.cb) != 0 {
            Some(pi.SystemCache as u64 * pi.PageSize as u64)
        } else {
            None
        };

        // Windows PageFile statistics include physical memory:
        // swap_total = TotalPageFile - TotalPhys (commit limit minus RAM)
        let swap_total = ms.ullTotalPageFile.saturating_sub(ms.ullTotalPhys);
        let swap_free = ms.ullAvailPageFile.saturating_sub(ms.ullAvailPhys).min(swap_total);

        Ok(MemInfo {
            total: ms.ullTotalPhys,
            // Windows has no "completely free vs available" distinction;
            // AvailPhys is the standby + free pages. free is a rough approximation of avail minus cache.
            free: ms.ullAvailPhys.saturating_sub(cached.unwrap_or(0)),
            available: ms.ullAvailPhys,
            buffers: None, // [PORT:WINDOWS] no equivalent concept
            cached,
            shared: None,
            swap_total,
            swap_free,
            // commit charge: TotalPageFile is the limit, used = limit - available
            commit_limit: Some(ms.ullTotalPageFile),
            committed: Some(ms.ullTotalPageFile.saturating_sub(ms.ullAvailPageFile)),
        })
    }
}
