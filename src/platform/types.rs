//! Cross-platform shared data types (equivalent to the return structs of libproc2's meminfo/stat/pids APIs).
//!
//! All field semantics are based on Linux /proc; other platform backends are responsible for "translating" native data into these fields.
//! Fields a platform cannot provide are represented as `Option` or 0; each tool decides whether to show `N/A` or omit them.

use std::path::PathBuf;
use std::time::SystemTime;

/// System memory information (in bytes).
/// - Linux: /proc/meminfo
/// - Windows: GlobalMemoryStatusEx + GetPerformanceInfo
/// - macOS: host_statistics64 (vm_statistics64) + sysctl hw.memsize
#[derive(Debug, Clone, Default)]
pub struct MemInfo {
    pub total: u64,
    pub free: u64,
    /// Estimated available memory (Linux MemAvailable; Windows ullAvailPhys)
    pub available: u64,
    /// Filesystem buffers (no equivalent concept on Windows/macOS → None)
    pub buffers: Option<u64>,
    /// Page cache (Windows maps to SystemCache; macOS maps to file-backed pages)
    pub cached: Option<u64>,
    /// Shared memory (Linux Shmem; None on other platforms)
    pub shared: Option<u64>,
    pub swap_total: u64,
    pub swap_free: u64,
    /// Committed memory (Linux Committed_AS; Windows commit charge) — used by free -v
    pub committed: Option<u64>,
    /// Commit limit (Linux CommitLimit; Windows TotalPageFile) — used by free -v
    pub commit_limit: Option<u64>,
}

/// Cumulative CPU time, in milliseconds.
/// - Linux: /proc/stat (jiffies × 1000 / CLK_TCK)
/// - Windows: GetSystemTimes (FILETIME 100ns / 10_000)
/// - macOS: host_processor_info (CPU_STATE_*)
#[derive(Debug, Clone, Copy, Default)]
pub struct CpuTimes {
    pub user: u64,
    pub nice: u64,
    pub system: u64,
    pub idle: u64,
    /// Windows has no iowait concept → 0; tools note this when displaying
    pub iowait: u64,
    pub irq: u64,
    pub softirq: u64,
    pub steal: u64,
}

impl CpuTimes {
    pub fn total(&self) -> u64 {
        self.user + self.nice + self.system + self.idle + self.iowait + self.irq + self.softirq + self.steal
    }
    pub fn busy(&self) -> u64 {
        self.total() - self.idle - self.iowait
    }
}

/// Information about a single process (a subset of the libproc2 pids API).
#[derive(Debug, Clone, Default)]
pub struct ProcessInfo {
    pub pid: u32,
    pub ppid: u32,
    /// Short program name (Linux comm; on Windows the image name, e.g. notepad.exe)
    pub name: String,
    pub exe: Option<PathBuf>,
    /// Full command line. On Windows obtained by reading the target process's PEB; may fail due to permissions → empty
    pub cmdline: Vec<String>,
    /// Process state: R/S/D/Z/T... (Linux); no equivalent on Windows → '?'
    pub state: char,
    pub user: String,
    /// Cumulative user-mode CPU time (milliseconds)
    pub utime_ms: u64,
    /// Cumulative kernel-mode CPU time (milliseconds)
    pub stime_ms: u64,
    pub start_time: Option<SystemTime>,
    /// Physical memory usage (Linux RSS; Windows WorkingSetSize; macOS resident_size)
    pub rss_bytes: u64,
    /// Virtual memory size (Windows PagefileUsage + WorkingSet approximates VSZ, with differences; see PORTING.md)
    pub vsz_bytes: u64,
    pub threads: u32,
    pub nice: Option<i32>,
    pub priority: Option<i32>,
    /// Controlling terminal name (Linux); none on Windows → None
    pub tty: Option<String>,
    // ---- Selection fields (advanced filtering for pgrep/pkill/pidwait/ps) ----
    /// Session ID (Linux session; not yet available on Windows/macOS → None)
    pub sid: Option<u32>,
    /// Process group ID (Linux pgrp; not yet available on Windows/macOS → None)
    pub pgrp: Option<u32>,
    /// Real / effective user and group IDs (Linux /proc/[pid]/status; None on other platforms)
    pub ruid: Option<u32>,
    pub euid: Option<u32>,
    pub rgid: Option<u32>,
    pub egid: Option<u32>,
    /// cgroup v2 path (Linux /proc/[pid]/cgroup; None on other platforms)
    pub cgroup: Option<String>,
}

/// Login session (the user count for w / uptime).
/// - Linux: utmp (/var/run/utmp)
/// - Windows: WTSEnumerateSessionsW
/// - macOS: utmpx
#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub user: String,
    /// tty name (Linux) or WinStation name (Windows, e.g. "Console", "rdp-tcp#0")
    pub line: String,
    pub host: Option<String>,
    pub login_time: Option<SystemTime>,
    pub idle: Option<std::time::Duration>,
}

/// A process memory region (pmap).
#[derive(Debug, Clone)]
pub struct MemRegion {
    pub base: usize,
    pub size: usize,
    /// "r-x" etc.; on Windows translated from PAGE_* protection flags
    pub perms: String,
    pub mapping: Option<String>,
}

/// vmstat counters. Fields a platform cannot obtain → None, and vmstat shows '-'.
#[derive(Debug, Clone, Default)]
pub struct VmCounters {
    pub procs_running: Option<u64>,
    pub procs_blocked: Option<u64>,
    pub pages_in: Option<u64>,
    pub pages_out: Option<u64>,
    pub swap_in: Option<u64>,
    pub swap_out: Option<u64>,
    pub interrupts: Option<u64>,
    pub context_switches: Option<u64>,
}

/// Cross-platform signal abstraction.
/// [PLATFORM:WINDOWS] Windows has no POSIX signals: Term/Kill/Int all map to
/// TerminateProcess (forced termination, equivalent to SIGKILL, cannot be intercepted); other signals report as unsupported.
/// See "Signal semantics" in PORTING.md for details.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    /// kill -0: only check process existence and permissions, send no signal
    Check,
    Hup,
    Int,
    Quit,
    Kill,
    Term,
    Usr1,
    Usr2,
    Cont,
    Stop,
    /// Native number on other platforms
    Other(i32),
}

impl Signal {
    /// Parse forms like "TERM" / "SIGTERM" / "15".
    pub fn parse(s: &str) -> Option<Signal> {
        if let Ok(n) = s.parse::<i32>() {
            return Some(match n {
                0 => Signal::Check,
                1 => Signal::Hup,
                2 => Signal::Int,
                3 => Signal::Quit,
                9 => Signal::Kill,
                10 => Signal::Usr1,
                12 => Signal::Usr2,
                15 => Signal::Term,
                18 => Signal::Cont,
                19 => Signal::Stop,
                other => Signal::Other(other),
            });
        }
        let up = s.to_ascii_uppercase();
        let name = up.strip_prefix("SIG").unwrap_or(&up);
        Some(match name {
            "HUP" => Signal::Hup,
            "INT" => Signal::Int,
            "QUIT" => Signal::Quit,
            "KILL" => Signal::Kill,
            "TERM" => Signal::Term,
            "USR1" => Signal::Usr1,
            "USR2" => Signal::Usr2,
            "CONT" => Signal::Cont,
            "STOP" => Signal::Stop,
            _ => return None,
        })
    }

    /// POSIX number (Linux x86_64 convention).
    pub fn number(&self) -> i32 {
        match self {
            Signal::Check => 0,
            Signal::Hup => 1,
            Signal::Int => 2,
            Signal::Quit => 3,
            Signal::Kill => 9,
            Signal::Usr1 => 10,
            Signal::Usr2 => 12,
            Signal::Term => 15,
            Signal::Cont => 18,
            Signal::Stop => 19,
            Signal::Other(n) => *n,
        }
    }
}
