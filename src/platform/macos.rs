//! [PLATFORM:MACOS] [PORT:MACOS] macOS backend skeleton.
//!
//! Most functions currently return `Unsupported`, so the project compiles on macOS.
//! Each function notes the recommended native API; fill them in one by one when porting (the interface matches the Linux/Windows backends).
//!
//! Planned primary data sources:
//! - Memory: `host_statistics64(HOST_VM_INFO64)` + `sysctl hw.memsize`
//! - CPU: `host_processor_info(PROCESSOR_CPU_LOAD_INFO)`
//! - uptime: `sysctl kern.boottime`
//! - loadavg: `getloadavg(3)` (natively supported on macOS)
//! - Process enumeration: `sysctl({CTL_KERN, KERN_PROC, KERN_PROC_ALL})` + `proc_pidinfo`
//! - Command line: `sysctl({CTL_KERN, KERN_PROCARGS2, pid})`
//! - cwd: `proc_pidinfo(PROC_PIDVNODEPATHINFO)`
//! - maps: `mach_vm_region` / `proc_regionfilename`
//! - kill/wait: `kill(2)` / `kqueue(EVFILT_PROC, NOTE_EXIT)`
//! - sessions: `utmpx`

#![allow(dead_code, unused_variables)]

use std::io;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use super::types::*;

fn todo<T>(api: &str) -> io::Result<T> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        format!("[PORT:MACOS] not yet implemented, recommended API: {api}"),
    ))
}

pub fn mem_info() -> io::Result<MemInfo> {
    todo("host_statistics64(HOST_VM_INFO64) + sysctl hw.memsize")
}

pub fn cpu_times() -> io::Result<CpuTimes> {
    todo("host_processor_info(PROCESSOR_CPU_LOAD_INFO)")
}

pub fn per_cpu_times() -> io::Result<Vec<CpuTimes>> {
    todo("host_processor_info per-CPU")
}

pub fn uptime() -> io::Result<Duration> {
    todo("sysctl kern.boottime")
}

pub fn boot_time() -> io::Result<SystemTime> {
    todo("sysctl kern.boottime")
}

/// macOS has a native getloadavg(3).
pub fn loadavg() -> io::Result<Option<(f64, f64, f64)>> {
    // SAFETY: getloadavg writes into an array of length 3
    let mut avg = [0f64; 3];
    let n = unsafe { libc::getloadavg(avg.as_mut_ptr(), 3) };
    if n == 3 {
        Ok(Some((avg[0], avg[1], avg[2])))
    } else {
        Ok(None)
    }
}

pub fn cpu_count() -> usize {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1)
}

pub fn list_processes() -> io::Result<Vec<ProcessInfo>> {
    todo("sysctl KERN_PROC_ALL + proc_pidinfo")
}

pub fn process_info(pid: u32) -> io::Result<ProcessInfo> {
    todo("proc_pidinfo(PROC_PIDTBSDINFO/TASKINFO)")
}

pub fn process_cwd(pid: u32) -> io::Result<PathBuf> {
    todo("proc_pidinfo(PROC_PIDVNODEPATHINFO)")
}

pub fn process_maps(pid: u32) -> io::Result<Vec<MemRegion>> {
    todo("mach_vm_region + proc_regionfilename")
}

pub fn kill(pid: u32, sig: Signal) -> io::Result<()> {
    // kill(2) is the same on macOS as on Linux
    let r = unsafe { libc::kill(pid as libc::pid_t, sig.number()) };
    if r == 0 { Ok(()) } else { Err(io::Error::last_os_error()) }
}

pub fn process_exists(pid: u32) -> bool {
    kill(pid, Signal::Check).is_ok()
}

/// renice: setpriority(PRIO_PROCESS, pid, nice) (same as Linux).
pub fn set_nice(pid: u32, nice: i32) -> io::Result<()> {
    let r = unsafe { libc::setpriority(libc::PRIO_PROCESS, pid as libc::id_t, nice) };
    if r == 0 { Ok(()) } else { Err(io::Error::last_os_error()) }
}

pub fn wait_process_exit(pid: u32, timeout: Option<Duration>) -> io::Result<bool> {
    // [PORT:MACOS] Could use kqueue(EVFILT_PROC, NOTE_EXIT); using polling for now
    let start = std::time::Instant::now();
    loop {
        if !process_exists(pid) {
            return Ok(true);
        }
        if let Some(t) = timeout
            && start.elapsed() >= t
        {
            return Ok(false);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

pub fn sessions() -> io::Result<Vec<SessionInfo>> {
    todo("utmpx getutxent")
}

pub fn vm_counters() -> io::Result<VmCounters> {
    todo("host_statistics64(HOST_VM_INFO64)")
}
