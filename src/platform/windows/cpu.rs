//! [PLATFORM:WINDOWS] CPU time / uptime / loadavg — replaces /proc/stat, /proc/uptime, /proc/loadavg.

use std::io;
use std::mem;
use std::time::{Duration, SystemTime};

use windows_sys::Win32::Foundation::FILETIME;
use windows_sys::Win32::System::SystemInformation::{
    GetSystemInfo, GetTickCount64, SYSTEM_INFO,
};
use windows_sys::Win32::System::Threading::GetSystemTimes;

use super::filetime_to_ms;
use crate::platform::types::{CpuTimes, VmCounters};

/// GetSystemTimes returns cumulative system idle/kernel/user time; kernel includes idle.
pub fn cpu_times() -> io::Result<CpuTimes> {
    // SAFETY: all three output parameters are valid pointers
    unsafe {
        let mut idle: FILETIME = mem::zeroed();
        let mut kernel: FILETIME = mem::zeroed();
        let mut user: FILETIME = mem::zeroed();
        if GetSystemTimes(&mut idle, &mut kernel, &mut user) == 0 {
            return Err(io::Error::last_os_error());
        }
        let idle_ms = filetime_to_ms(idle);
        let kernel_ms = filetime_to_ms(kernel);
        let user_ms = filetime_to_ms(user);
        Ok(CpuTimes {
            user: user_ms,
            nice: 0, // [PORT:WINDOWS] Windows has no nice time category
            system: kernel_ms.saturating_sub(idle_ms),
            idle: idle_ms,
            iowait: 0, // [PORT:WINDOWS] Windows does not track iowait separately
            irq: 0,
            softirq: 0,
            steal: 0,
        })
    }
}

/// [PLATFORM:WINDOWS] Real per-core CPU time.
/// Uses NtQuerySystemInformation(SystemProcessorPerformanceInformation, class 8)
/// to get each logical processor's IdleTime/KernelTime/UserTime (100ns units; KernelTime already includes IdleTime).
pub fn per_cpu_times() -> io::Result<Vec<CpuTimes>> {
    use windows_sys::Wdk::System::SystemInformation::{
        NtQuerySystemInformation, SystemProcessorPerformanceInformation,
    };
    use windows_sys::Win32::System::WindowsProgramming::SYSTEM_PROCESSOR_PERFORMANCE_INFORMATION;

    let n = cpu_count().max(1);
    // SAFETY: allocate a buffer of n structures for NtQuerySystemInformation to fill, length passed in bytes
    unsafe {
        let mut buf: Vec<SYSTEM_PROCESSOR_PERFORMANCE_INFORMATION> = vec![mem::zeroed(); n];
        let size = (n * mem::size_of::<SYSTEM_PROCESSOR_PERFORMANCE_INFORMATION>()) as u32;
        let mut ret = 0u32;
        let status = NtQuerySystemInformation(
            SystemProcessorPerformanceInformation,
            buf.as_mut_ptr() as *mut _,
            size,
            &mut ret,
        );
        if status < 0 {
            // on failure, fall back to averaging the totals so top still works
            let total = cpu_times()?;
            let d = |v: u64| v / n as u64;
            return Ok((0..n)
                .map(|_| CpuTimes {
                    user: d(total.user),
                    system: d(total.system),
                    idle: d(total.idle),
                    ..Default::default()
                })
                .collect());
        }
        let to_ms = |t: i64| (t as u64) / 10_000; // 100ns → ms
        Ok(buf
            .iter()
            .map(|c| {
                let idle = to_ms(c.IdleTime);
                let kernel = to_ms(c.KernelTime);
                let user = to_ms(c.UserTime);
                CpuTimes {
                    user,
                    nice: 0,
                    system: kernel.saturating_sub(idle), // kernel includes idle; subtract it to get pure system time
                    idle,
                    iowait: 0,
                    irq: 0,
                    softirq: 0,
                    steal: 0,
                }
            })
            .collect())
    }
}

/// GetTickCount64: milliseconds since boot (including sleep time).
pub fn uptime() -> io::Result<Duration> {
    // SAFETY: parameterless pure query
    Ok(Duration::from_millis(unsafe { GetTickCount64() }))
}

pub fn boot_time() -> io::Result<SystemTime> {
    Ok(SystemTime::now() - uptime()?)
}

/// [PORT:WINDOWS] Windows has no concept of load average.
/// The closest metric is the PDH counter `\System\Processor Queue Length`, but its semantics differ → return None,
/// and the various tools (uptime, w, top) display n/a.
pub fn loadavg() -> io::Result<Option<(f64, f64, f64)>> {
    Ok(None)
}

pub fn cpu_count() -> usize {
    // SAFETY: SYSTEM_INFO is output-only
    unsafe {
        let mut si: SYSTEM_INFO = mem::zeroed();
        GetSystemInfo(&mut si);
        si.dwNumberOfProcessors as usize
    }
}

/// [PORT:WINDOWS] Most vmstat counters have no public Win32 equivalent:
/// - context switches / interrupts: NtQuerySystemInformation(SystemPerformanceInformation) (undocumented) or PDH
/// - pages in/out: PDH `\Memory\Pages Input/sec` cumulative value is unavailable
/// For now always return None; vmstat displays '-' and explains this in --help.
pub fn vm_counters() -> io::Result<VmCounters> {
    Ok(VmCounters::default())
}
