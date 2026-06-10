//! [PLATFORM:LINUX] Linux backend — parses /proc, using the same data source as the C libproc2.
//!
//! The comment above each function notes the corresponding /proc file and C libproc2 module, for easy cross-reference with the source:
//! procps-v4.0.6/library/{meminfo.c,stat.c,uptime.c,pids.c,...}

#![allow(dead_code)]

use std::fs;
use std::io;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::types::*;

fn read_proc(path: &str) -> io::Result<String> {
    fs::read_to_string(path)
}

fn clk_tck() -> u64 {
    // SAFETY: sysconf is a pure query
    unsafe { libc::sysconf(libc::_SC_CLK_TCK) as u64 }
}

fn page_size() -> u64 {
    unsafe { libc::sysconf(libc::_SC_PAGESIZE) as u64 }
}

/// /proc/meminfo (corresponds to library/meminfo.c)
pub fn mem_info() -> io::Result<MemInfo> {
    let text = read_proc("/proc/meminfo")?;
    let mut m = MemInfo::default();
    for line in text.lines() {
        let mut it = line.split_whitespace();
        let (Some(key), Some(val)) = (it.next(), it.next()) else { continue };
        let kb: u64 = val.parse().unwrap_or(0);
        let bytes = kb * 1024;
        match key {
            "MemTotal:" => m.total = bytes,
            "MemFree:" => m.free = bytes,
            "MemAvailable:" => m.available = bytes,
            "Buffers:" => m.buffers = Some(bytes),
            "Cached:" => m.cached = Some(m.cached.unwrap_or(0) + bytes),
            "SReclaimable:" => m.cached = Some(m.cached.unwrap_or(0) + bytes),
            "Shmem:" => m.shared = Some(bytes),
            "SwapTotal:" => m.swap_total = bytes,
            "SwapFree:" => m.swap_free = bytes,
            "CommitLimit:" => m.commit_limit = Some(bytes),
            "Committed_AS:" => m.committed = Some(bytes),
            _ => {}
        }
    }
    Ok(m)
}

fn parse_cpu_line(line: &str) -> CpuTimes {
    let tick_ms = 1000 / clk_tck().max(1);
    let v: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .map(|s| s.parse::<u64>().unwrap_or(0) * tick_ms)
        .collect();
    let g = |i: usize| v.get(i).copied().unwrap_or(0);
    CpuTimes {
        user: g(0),
        nice: g(1),
        system: g(2),
        idle: g(3),
        iowait: g(4),
        irq: g(5),
        softirq: g(6),
        steal: g(7),
    }
}

/// First "cpu" aggregate line of /proc/stat (corresponds to library/stat.c)
pub fn cpu_times() -> io::Result<CpuTimes> {
    let text = read_proc("/proc/stat")?;
    let line = text
        .lines()
        .find(|l| l.starts_with("cpu "))
        .ok_or_else(|| io::Error::other("no cpu line in /proc/stat"))?;
    Ok(parse_cpu_line(line))
}

/// /proc/stat cpu0..cpuN
pub fn per_cpu_times() -> io::Result<Vec<CpuTimes>> {
    let text = read_proc("/proc/stat")?;
    Ok(text
        .lines()
        .filter(|l| l.starts_with("cpu") && !l.starts_with("cpu "))
        .map(parse_cpu_line)
        .collect())
}

/// /proc/uptime (corresponds to library/uptime.c)
pub fn uptime() -> io::Result<Duration> {
    let text = read_proc("/proc/uptime")?;
    let secs: f64 = text
        .split_whitespace()
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| io::Error::other("bad /proc/uptime"))?;
    Ok(Duration::from_secs_f64(secs))
}

pub fn boot_time() -> io::Result<SystemTime> {
    Ok(SystemTime::now() - uptime()?)
}

/// /proc/loadavg
pub fn loadavg() -> io::Result<Option<(f64, f64, f64)>> {
    let text = read_proc("/proc/loadavg")?;
    let v: Vec<f64> = text
        .split_whitespace()
        .take(3)
        .filter_map(|s| s.parse().ok())
        .collect();
    if v.len() == 3 { Ok(Some((v[0], v[1], v[2]))) } else { Ok(None) }
}

pub fn cpu_count() -> usize {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1)
}

fn uid_to_name(uid: u32) -> String {
    // getpwuid_r is more cumbersome; use getpwuid here (acceptable for single-threaded query scenarios)
    unsafe {
        let pw = libc::getpwuid(uid);
        if pw.is_null() {
            return uid.to_string();
        }
        std::ffi::CStr::from_ptr((*pw).pw_name)
            .to_string_lossy()
            .into_owned()
    }
}

/// /proc/[pid]/{stat,status,cmdline,exe} (corresponds to library/pids.c + readproc)
pub fn process_info(pid: u32) -> io::Result<ProcessInfo> {
    let base = format!("/proc/{pid}");
    let stat = read_proc(&format!("{base}/stat"))?;
    // comm may contain spaces and parentheses; locate the last ')'
    let open = stat.find('(').ok_or_else(|| io::Error::other("bad stat"))?;
    let close = stat.rfind(')').ok_or_else(|| io::Error::other("bad stat"))?;
    let name = stat[open + 1..close].to_string();
    let rest: Vec<&str> = stat[close + 2..].split_whitespace().collect();
    let f = |i: usize| rest.get(i).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
    // rest[0]=state (field 3), rest[1]=ppid (field 4) ... see proc(5) for the numbering
    let state = rest.first().and_then(|s| s.chars().next()).unwrap_or('?');
    let ppid = f(1) as u32;
    let tick_ms = 1000 / clk_tck().max(1);
    let utime_ms = f(11) * tick_ms; // field 14 utime
    let stime_ms = f(12) * tick_ms; // field 15 stime
    let priority = rest.get(15).and_then(|s| s.parse::<i32>().ok());
    let nice = rest.get(16).and_then(|s| s.parse::<i32>().ok());
    let threads = f(17) as u32; // field 20 num_threads
    let starttime_ticks = f(19); // field 22 starttime (jiffies since boot)
    let vsz_bytes = f(20); // field 23 vsize (bytes)
    let rss_bytes = f(21) * page_size(); // field 24 rss (pages)

    let start_time = boot_time()
        .ok()
        .map(|bt| bt + Duration::from_millis(starttime_ticks * tick_ms));

    let cmdline: Vec<String> = fs::read(format!("{base}/cmdline"))
        .unwrap_or_default()
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect();

    let exe = fs::read_link(format!("{base}/exe")).ok();

    // Process group / session: stat field 5 pgrp, field 6 session (rest[2], rest[3])
    let pgrp = Some(f(2) as u32);
    let sid = Some(f(3) as u32);

    // Owner and uid/gid: the Uid:/Gid: lines in status (real eff saved fs)
    let status = read_proc(&format!("{base}/status")).unwrap_or_default();
    let parse_id_line = |prefix: &str| -> (Option<u32>, Option<u32>) {
        status
            .lines()
            .find(|l| l.starts_with(prefix))
            .map(|l| {
                let mut it = l.split_whitespace().skip(1);
                let real = it.next().and_then(|s| s.parse::<u32>().ok());
                let eff = it.next().and_then(|s| s.parse::<u32>().ok());
                (real, eff)
            })
            .unwrap_or((None, None))
    };
    let (ruid, euid) = parse_id_line("Uid:");
    let (rgid, egid) = parse_id_line("Gid:");
    let user = ruid.map(uid_to_name).unwrap_or_default();

    // cgroup v2: take the path from the line starting with "0::"
    let cgroup = read_proc(&format!("{base}/cgroup")).ok().and_then(|t| {
        t.lines()
            .find(|l| l.starts_with("0::"))
            .map(|l| l[3..].to_string())
    });

    // Controlling terminal: stat field 7 tty_nr; simplified handling: only give a value other than "?" when a tty exists
    let tty_nr = f(4);
    let tty = if tty_nr != 0 {
        // major 136 = pts
        let major = (tty_nr >> 8) & 0xfff;
        let minor = (tty_nr & 0xff) | ((tty_nr >> 12) & 0xfff00);
        Some(if major == 136 {
            format!("pts/{minor}")
        } else if major == 4 {
            format!("tty{minor}")
        } else {
            format!("{major}:{minor}")
        })
    } else {
        None
    };

    Ok(ProcessInfo {
        pid,
        ppid,
        name,
        exe,
        cmdline,
        state,
        user,
        utime_ms,
        stime_ms,
        start_time,
        rss_bytes,
        vsz_bytes,
        threads,
        nice,
        priority,
        tty,
        sid,
        pgrp,
        ruid,
        euid,
        rgid,
        egid,
        cgroup,
    })
}

pub fn list_processes() -> io::Result<Vec<ProcessInfo>> {
    let mut out = Vec::new();
    for entry in fs::read_dir("/proc")? {
        let entry = entry?;
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else { continue };
        // A process may disappear between reads; ignore individual errors
        if let Ok(info) = process_info(pid) {
            out.push(info);
        }
    }
    Ok(out)
}

/// /proc/[pid]/cwd
pub fn process_cwd(pid: u32) -> io::Result<PathBuf> {
    fs::read_link(format!("/proc/{pid}/cwd"))
}

/// /proc/[pid]/maps
pub fn process_maps(pid: u32) -> io::Result<Vec<MemRegion>> {
    let text = read_proc(&format!("/proc/{pid}/maps"))?;
    let mut out = Vec::new();
    for line in text.lines() {
        let mut it = line.split_whitespace();
        let (Some(range), Some(perms)) = (it.next(), it.next()) else { continue };
        let mut bounds = range.split('-');
        let (Some(lo), Some(hi)) = (bounds.next(), bounds.next()) else { continue };
        let (Ok(lo), Ok(hi)) = (usize::from_str_radix(lo, 16), usize::from_str_radix(hi, 16)) else {
            continue;
        };
        let mapping = line.split_whitespace().nth(5).map(str::to_string);
        out.push(MemRegion {
            base: lo,
            size: hi - lo,
            perms: perms.chars().take(3).collect(),
            mapping,
        });
    }
    Ok(out)
}

pub fn kill(pid: u32, sig: Signal) -> io::Result<()> {
    let r = unsafe { libc::kill(pid as libc::pid_t, sig.number()) };
    if r == 0 { Ok(()) } else { Err(io::Error::last_os_error()) }
}

pub fn process_exists(pid: u32) -> bool {
    kill(pid, Signal::Check).is_ok()
        || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// renice: setpriority(PRIO_PROCESS, pid, nice).
pub fn set_nice(pid: u32, nice: i32) -> io::Result<()> {
    let r = unsafe { libc::setpriority(libc::PRIO_PROCESS, pid, nice) };
    if r == 0 { Ok(()) } else { Err(io::Error::last_os_error()) }
}

/// Wait for a process (non-child) to exit: poll for the existence of /proc/[pid].
/// [PORT:LINUX] Could switch to pidfd_open(2) + poll for an event-driven version (as the C pidwait does).
pub fn wait_process_exit(pid: u32, timeout: Option<Duration>) -> io::Result<bool> {
    let start = std::time::Instant::now();
    loop {
        if !std::path::Path::new(&format!("/proc/{pid}")).exists() {
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

/// utmp login sessions (corresponds to the utmp reading in src/w.c).
pub fn sessions() -> io::Result<Vec<SessionInfo>> {
    let mut out = Vec::new();
    // SAFETY: the utmpx family are standard glibc APIs; used single-threaded
    unsafe {
        libc::setutxent();
        loop {
            let ent = libc::getutxent();
            if ent.is_null() {
                break;
            }
            let e = &*ent;
            if e.ut_type != libc::USER_PROCESS {
                continue;
            }
            let cstr = |buf: &[libc::c_char]| -> String {
                let bytes: Vec<u8> = buf.iter().take_while(|c| **c != 0).map(|c| *c as u8).collect();
                String::from_utf8_lossy(&bytes).into_owned()
            };
            let login_time =
                UNIX_EPOCH + Duration::from_secs(e.ut_tv.tv_sec as u64);
            out.push(SessionInfo {
                user: cstr(&e.ut_user),
                line: cstr(&e.ut_line),
                host: Some(cstr(&e.ut_host)).filter(|s| !s.is_empty()),
                login_time: Some(login_time),
                idle: None,
            });
        }
        libc::endutxent();
    }
    Ok(out)
}

/// /proc/vmstat + /proc/stat (used by vmstat)
pub fn vm_counters() -> io::Result<VmCounters> {
    let mut c = VmCounters::default();
    if let Ok(text) = read_proc("/proc/vmstat") {
        for line in text.lines() {
            let mut it = line.split_whitespace();
            let (Some(k), Some(v)) = (it.next(), it.next()) else { continue };
            let v: u64 = v.parse().unwrap_or(0);
            match k {
                "pgpgin" => c.pages_in = Some(v),
                "pgpgout" => c.pages_out = Some(v),
                "pswpin" => c.swap_in = Some(v),
                "pswpout" => c.swap_out = Some(v),
                _ => {}
            }
        }
    }
    if let Ok(text) = read_proc("/proc/stat") {
        for line in text.lines() {
            let mut it = line.split_whitespace();
            let (Some(k), Some(v)) = (it.next(), it.next()) else { continue };
            match k {
                "intr" => c.interrupts = v.parse().ok(),
                "ctxt" => c.context_switches = v.parse().ok(),
                "procs_running" => c.procs_running = v.parse().ok(),
                "procs_blocked" => c.procs_blocked = v.parse().ok(),
                _ => {}
            }
        }
    }
    Ok(c)
}
