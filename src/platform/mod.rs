//! Platform abstraction layer: the Rust counterpart to libproc2.
//!
//! The public API is always used through this module (`procps::platform::mem_info()`, etc.),
//! with the backend chosen at compile time via cfg. To add a platform, implement the
//! functions below; the tool layer needs no changes:
//!
//! | Function           | Linux source          | Windows source (Win32)         | Recommended macOS API           |
//! |--------------------|----------------------|--------------------------------|--------------------------------|
//! | mem_info           | /proc/meminfo        | GlobalMemoryStatusEx etc.      | host_statistics64              |
//! | cpu_times          | /proc/stat           | GetSystemTimes                 | host_processor_info            |
//! | uptime             | /proc/uptime         | GetTickCount64                 | sysctl kern.boottime           |
//! | loadavg            | /proc/loadavg        | (no equivalent → None)         | getloadavg(3)                  |
//! | list_processes     | /proc/[pid]/*        | Toolhelp32 + OpenProcess + PEB | sysctl KERN_PROC + libproc     |
//! | process_cwd        | /proc/[pid]/cwd      | PEB ProcessParameters          | proc_pidinfo(PROC_PIDVNODEPATHINFO) |
//! | process_maps       | /proc/[pid]/maps     | VirtualQueryEx                 | mach_vm_region                 |
//! | kill / wait        | kill(2)              | TerminateProcess / WaitForSingleObject | kill(2) / kqueue EVFILT_PROC |
//! | sessions           | utmp                 | WTSEnumerateSessionsW          | utmpx                          |
//! | vm_counters        | /proc/vmstat,/proc/stat | (mostly no equivalent → None) | host_statistics64              |

pub mod types;
pub use types::*;

// ---- Backend selection ----

// [PLATFORM:WINDOWS] native Win32 API implementation
#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use self::windows::*;

// [PLATFORM:LINUX] /proc parsing implementation (same data source as the C libproc2)
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use self::linux::*;

// [PLATFORM:MACOS] [PORT:MACOS] interface skeleton, implementation pending (see comments within each function)
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use self::macos::*;
