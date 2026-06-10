//! ps — a snapshot of process status. Corresponds to procps-v4.0.6/src/ps/
//!
//! Supports a practical subset of the three option styles (UNIX `-x` / BSD `x` / GNU `--x`),
//! plus `-o` custom format with core format keywords. Advanced flags/keywords are accepted but marked "TODO".
//!
//! Cross-platform: list_processes().
//! [PLATFORM:WINDOWS] STAT has no R/S/D/Z (shows '?'); TTY is usually '?';
//! %CPU is an approximation of "cumulative CPU time / process lifetime".

use std::time::SystemTime;

use procps::common::version_string;
use procps::platform::{self, ProcessInfo};

#[derive(Default)]
struct Options {
    all: bool,            // -e/-A/a/x/-a/-d
    full: bool,           // -f
    extra_full: bool,     // -F
    long: bool,           // -l
    user_format: bool,    // u (BSD)
    job_format: bool,     // -j/j
    forest: bool,         // -H/f/--forest
    no_headers: bool,     // --no-headers
    wide: bool,           // -w
    negate: bool,         // -N/--deselect
    format: Vec<String>,  // -o / --format (keyword list)
    sort: Option<String>, // --sort / -O / k
    // selection criteria
    sel_users: Vec<String>,
    sel_pids: Vec<u32>,
    sel_ppids: Vec<u32>,
    sel_cmd: Vec<String>,
    has_selection: bool,
}

fn split_keywords(s: &str, out: &mut Vec<String>) {
    for k in s.split([',', ' ']) {
        let k = k.trim();
        if !k.is_empty() {
            out.push(k.to_string());
        }
    }
}

/// Parse a mix of all three styles of argv. Returns None to indicate we should exit immediately (version/help already handled).
fn parse(argv: &[String]) -> Option<Options> {
    let mut o = Options::default();
    let mut i = 0;
    while i < argv.len() {
        let a = &argv[i];
        // ---- GNU long options ----
        if let Some(long) = a.strip_prefix("--") {
            let (key, val) = match long.split_once('=') {
                Some((k, v)) => (k, Some(v.to_string())),
                None => (long, None),
            };
            let take = |i: &mut usize| -> Option<String> {
                val.clone().or_else(|| {
                    *i += 1;
                    argv.get(*i).cloned()
                })
            };
            match key {
                "help" => {
                    print_help();
                    return None;
                }
                "version" => {
                    println!("{}", version_string("ps"));
                    return None;
                }
                "no-headers" | "no-heading" | "noheader" | "noheaders" => o.no_headers = true,
                "sort" => o.sort = take(&mut i),
                "forest" => o.forest = true,
                "format" => {
                    if let Some(v) = take(&mut i) {
                        split_keywords(&v, &mut o.format);
                    }
                }
                "pid" => {
                    if let Some(v) = take(&mut i) {
                        o.has_selection = true;
                        for p in v.split([',', ' ']).filter_map(|s| s.trim().parse().ok()) {
                            o.sel_pids.push(p);
                        }
                    }
                }
                "ppid" => {
                    if let Some(v) = take(&mut i) {
                        o.has_selection = true;
                        for p in v.split([',', ' ']).filter_map(|s| s.trim().parse().ok()) {
                            o.sel_ppids.push(p);
                        }
                    }
                }
                "user" | "User" => {
                    if let Some(v) = take(&mut i) {
                        o.has_selection = true;
                        split_keywords(&v, &mut o.sel_users);
                    }
                }
                "deselect" => o.negate = true,
                "cols" | "columns" | "width" | "rows" | "lines" => {
                    let _ = take(&mut i);
                    o.wide = true;
                }
                _ => { /* other long options: accepted but ignored (TODO) */ }
            }
            i += 1;
            continue;
        }

        // ---- UNIX short options (leading -) ----
        if let Some(cluster) = a.strip_prefix('-') {
            let chars: Vec<char> = cluster.chars().collect();
            let mut j = 0;
            while j < chars.len() {
                let c = chars[j];
                // Options that require an argument: take the rest of the cluster or the next argv
                let take_arg = |j: &mut usize, i: &mut usize| -> Option<String> {
                    let rest: String = chars[*j + 1..].iter().collect();
                    if !rest.is_empty() {
                        *j = chars.len();
                        Some(rest)
                    } else {
                        *i += 1;
                        argv.get(*i).cloned()
                    }
                };
                match c {
                    'e' | 'A' => o.all = true,
                    'a' => o.all = true,
                    'd' => o.all = true,
                    'f' => o.full = true,
                    'F' => o.extra_full = true,
                    'l' => o.long = true,
                    'j' => o.job_format = true,
                    'H' => o.forest = true,
                    'w' => o.wide = true,
                    'N' => o.negate = true,
                    'x' => o.all = true,
                    'L' | 'T' => { /* threads: accepted, but this implementation works per process */ }
                    'o' => {
                        if let Some(v) = take_arg(&mut j, &mut i) {
                            split_keywords(&v, &mut o.format);
                        }
                    }
                    'O' => {
                        if let Some(v) = take_arg(&mut j, &mut i) {
                            o.sort = Some(v);
                        }
                    }
                    'p' | 'q' => {
                        if let Some(v) = take_arg(&mut j, &mut i) {
                            o.has_selection = true;
                            for p in v.split([',', ' ']).filter_map(|s| s.trim().parse().ok()) {
                                o.sel_pids.push(p);
                            }
                        }
                    }
                    'u' | 'U' => {
                        if let Some(v) = take_arg(&mut j, &mut i) {
                            o.has_selection = true;
                            split_keywords(&v, &mut o.sel_users);
                        }
                    }
                    'C' => {
                        if let Some(v) = take_arg(&mut j, &mut i) {
                            o.has_selection = true;
                            split_keywords(&v, &mut o.sel_cmd);
                        }
                    }
                    'G' | 'g' | 't' | 's' => {
                        // group/tty/session selection: Linux concepts; the argument is accepted but this implementation skips the filtering
                        let _ = take_arg(&mut j, &mut i);
                    }
                    'V' => {
                        println!("{}", version_string("ps"));
                        return None;
                    }
                    'h' => o.no_headers = true,
                    'c' | 'y' | 'm' | 'n' | 'P' | 'M' | 'Z' => { /* accepted, ignored */ }
                    _ => { /* unknown: ignored */ }
                }
                j += 1;
            }
            i += 1;
            continue;
        }

        // ---- BSD options (no leading -) ----
        let chars: Vec<char> = a.chars().collect();
        if chars.iter().all(|c| c.is_ascii_digit()) && !a.is_empty() {
            // Pure digits = PID selection
            o.has_selection = true;
            if let Ok(p) = a.parse() {
                o.sel_pids.push(p);
            }
            i += 1;
            continue;
        }
        let mut j = 0;
        while j < chars.len() {
            let c = chars[j];
            let take_arg = |j: &mut usize, i: &mut usize| -> Option<String> {
                let rest: String = chars[*j + 1..].iter().collect();
                if !rest.is_empty() {
                    *j = chars.len();
                    Some(rest)
                } else {
                    *i += 1;
                    argv.get(*i).cloned()
                }
            };
            match c {
                'a' => o.all = true,
                'x' => o.all = true,
                'e' => o.all = true,
                'u' => o.user_format = true,
                'f' => o.forest = true,
                'l' => o.long = true,
                'j' => o.job_format = true,
                'w' => o.wide = true,
                'r' => { /* running only: accepted, skipped */ }
                'h' => o.no_headers = true,
                'o' => {
                    if let Some(v) = take_arg(&mut j, &mut i) {
                        split_keywords(&v, &mut o.format);
                    }
                }
                'k' | 'O' => {
                    if let Some(v) = take_arg(&mut j, &mut i) {
                        o.sort = Some(v);
                    }
                }
                'p' => {
                    if let Some(v) = take_arg(&mut j, &mut i) {
                        o.has_selection = true;
                        for p in v.split([',', ' ']).filter_map(|s| s.trim().parse().ok()) {
                            o.sel_pids.push(p);
                        }
                    }
                }
                'U' => {
                    if let Some(v) = take_arg(&mut j, &mut i) {
                        o.has_selection = true;
                        split_keywords(&v, &mut o.sel_users);
                    }
                }
                _ => { /* ignored */ }
            }
            j += 1;
        }
        i += 1;
    }
    Some(o)
}

fn print_help() {
    println!("Usage: ps [options]");
    println!("  Styles: UNIX (-e -f -l -o ...), BSD (aux), GNU (--sort --pid ...)");
    println!("  Selection: -e/-A all, -u USER, -p PID, -C name, --ppid PPID");
    println!("  Format: -o keyword[,...], -f full, -l long, u (BSD user format)");
    println!("  Other: --sort spec, --no-headers, -H/f forest, -w wide, -V version");
}

/// Field keyword -> column header.
fn header_for(kw: &str) -> &str {
    match kw {
        "pid" => "PID",
        "ppid" => "PPID",
        "user" | "euser" | "uname" => "USER",
        "ruser" => "RUSER",
        "uid" | "euid" => "UID",
        "ruid" => "RUID",
        "gid" | "egid" => "GID",
        "pgid" | "pgrp" => "PGID",
        "sid" | "sess" => "SID",
        "tty" | "tname" | "tt" => "TTY",
        "%cpu" | "pcpu" => "%CPU",
        "%mem" | "pmem" => "%MEM",
        "vsz" | "vsize" => "VSZ",
        "rss" | "rssize" => "RSS",
        "stat" | "state" | "s" => "STAT",
        "start" | "start_time" | "stime" => "START",
        "time" | "cputime" => "TIME",
        "etime" => "ELAPSED",
        "pri" => "PRI",
        "ni" | "nice" => "NI",
        "nlwp" | "thcount" => "NLWP",
        "comm" | "ucomm" => "COMMAND",
        "cmd" | "args" | "command" => "COMMAND",
        other => Box::leak(other.to_uppercase().into_boxed_str()),
    }
}

struct Ctx {
    mem_total: u64,
    now: SystemTime,
    current_user: String,
}

fn field_value(kw: &str, p: &ProcessInfo, ctx: &Ctx) -> String {
    let cpu_pct = || {
        let cpu_ms = p.utime_ms + p.stime_ms;
        let alive = p
            .start_time
            .and_then(|st| ctx.now.duration_since(st).ok())
            .map(|d| d.as_millis() as f64)
            .unwrap_or(0.0);
        if alive <= 0.0 { 0.0 } else { (cpu_ms as f64 / alive * 100.0).min(9999.0) }
    };
    let time_fmt = || {
        let s = (p.utime_ms + p.stime_ms) / 1000;
        let (h, m, sec) = (s / 3600, (s % 3600) / 60, s % 60);
        if h > 0 { format!("{h}:{m:02}:{sec:02}") } else { format!("{m:02}:{sec:02}") }
    };
    match kw {
        "pid" => p.pid.to_string(),
        "ppid" => p.ppid.to_string(),
        "user" | "euser" | "uname" | "ruser" => p.user.clone(),
        "uid" | "euid" => p.euid.map(|u| u.to_string()).unwrap_or_else(|| "-".into()),
        "ruid" => p.ruid.map(|u| u.to_string()).unwrap_or_else(|| "-".into()),
        "gid" | "egid" => p.egid.map(|g| g.to_string()).unwrap_or_else(|| "-".into()),
        "pgid" | "pgrp" => p.pgrp.map(|g| g.to_string()).unwrap_or_else(|| "-".into()),
        "sid" | "sess" => p.sid.map(|s| s.to_string()).unwrap_or_else(|| "-".into()),
        "tty" | "tname" | "tt" => p.tty.clone().unwrap_or_else(|| "?".into()),
        "%cpu" | "pcpu" => format!("{:.1}", cpu_pct()),
        "%mem" | "pmem" => format!("{:.1}", p.rss_bytes as f64 / ctx.mem_total as f64 * 100.0),
        "vsz" | "vsize" => (p.vsz_bytes / 1024).to_string(),
        "rss" | "rssize" => (p.rss_bytes / 1024).to_string(),
        "stat" | "state" | "s" => p.state.to_string(),
        "start" | "start_time" | "stime" => "-".into(),
        "time" | "cputime" => time_fmt(),
        "etime" => p
            .start_time
            .and_then(|st| ctx.now.duration_since(st).ok())
            .map(|d| procps::units::format_uptime(d.as_secs()))
            .unwrap_or_else(|| "-".into()),
        "pri" => p.priority.map(|v| v.to_string()).unwrap_or_else(|| "-".into()),
        "ni" | "nice" => p.nice.map(|v| v.to_string()).unwrap_or_else(|| "-".into()),
        "nlwp" | "thcount" => p.threads.to_string(),
        "comm" | "ucomm" => p.name.clone(),
        "cmd" | "args" | "command" => {
            if p.cmdline.is_empty() { format!("[{}]", p.name) } else { p.cmdline.join(" ") }
        }
        _ => "-".into(),
    }
}

/// Determine the column list (based on the style flags).
fn columns(o: &Options) -> Vec<String> {
    if !o.format.is_empty() {
        return o.format.clone();
    }
    let kw = |s: &[&str]| s.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    if o.user_format {
        kw(&["user", "pid", "%cpu", "%mem", "vsz", "rss", "tty", "stat", "start", "time", "command"])
    } else if o.long {
        kw(&["s", "uid", "pid", "ppid", "pri", "ni", "vsz", "rss", "stat", "tty", "time", "command"])
    } else if o.job_format {
        kw(&["pid", "pgid", "sid", "tty", "time", "command"])
    } else if o.full || o.extra_full {
        kw(&["uid", "pid", "ppid", "stime", "tty", "time", "cmd"])
    } else {
        kw(&["pid", "tty", "time", "cmd"])
    }
}

fn whoami() -> String {
    #[cfg(windows)]
    { std::env::var("USERNAME").unwrap_or_default() }
    #[cfg(unix)]
    { std::env::var("USER").unwrap_or_default() }
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let Some(o) = parse(&argv) else { return };

    let mut procs = match platform::list_processes() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("ps: {e}");
            std::process::exit(1);
        }
    };

    let ctx = Ctx {
        mem_total: platform::mem_info().map(|m| m.total).unwrap_or(1).max(1),
        now: SystemTime::now(),
        current_user: whoami(),
    };

    // Selection
    let keep = |p: &ProcessInfo| -> bool {
        if o.has_selection {
            let mut hit = false;
            if !o.sel_pids.is_empty() && o.sel_pids.contains(&p.pid) { hit = true; }
            if !o.sel_ppids.is_empty() && o.sel_ppids.contains(&p.ppid) { hit = true; }
            if !o.sel_users.is_empty()
                && o.sel_users.iter().any(|u| {
                    u.eq_ignore_ascii_case(&p.user) || u.parse::<u32>().ok() == p.euid
                })
            {
                hit = true;
            }
            if !o.sel_cmd.is_empty()
                && o.sel_cmd.iter().any(|c| p.name.eq_ignore_ascii_case(c)
                    || p.name.to_lowercase() == format!("{}.exe", c.to_lowercase()))
            {
                hit = true;
            }
            hit
        } else if o.all {
            true
        } else {
            // Default: processes of the current user
            p.user == ctx.current_user
        }
    };
    procs.retain(|p| {
        let k = keep(p);
        if o.negate { !k } else { k }
    });

    // Sorting
    if let Some(spec) = &o.sort {
        // A leading + / - indicates ascending/descending order, e.g. -%cpu, +pid
        let desc = spec.starts_with('-');
        let key = spec.trim_start_matches(['+', '-']).to_string();
        procs.sort_by(|a, b| {
            let va = field_value(&key, a, &ctx);
            let vb = field_value(&key, b, &ctx);
            // Compare numerically first, otherwise as strings
            let ord = match (va.parse::<f64>(), vb.parse::<f64>()) {
                (Ok(x), Ok(y)) => x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal),
                _ => va.cmp(&vb),
            };
            if desc { ord.reverse() } else { ord }
        });
    } else {
        procs.sort_by_key(|p| p.pid);
    }

    let cols = columns(&o);

    // Column width: fixed width for all columns except the last
    if !o.no_headers {
        let mut line = String::new();
        for (idx, kw) in cols.iter().enumerate() {
            let h = header_for(kw);
            if idx + 1 == cols.len() {
                line.push_str(h);
            } else {
                line.push_str(&format!("{h:>8} "));
            }
        }
        println!("{}", line.trim_end());
    }

    for p in &procs {
        let mut line = String::new();
        for (idx, kw) in cols.iter().enumerate() {
            let v = field_value(kw, p, &ctx);
            if idx + 1 == cols.len() {
                line.push_str(&v);
            } else {
                line.push_str(&format!("{v:>8} "));
            }
        }
        let out = if o.wide { line } else {
            // Not wide: limit line width to about 110 columns
            let max = 110;
            if line.chars().count() > max { line.chars().take(max).collect() } else { line }
        };
        println!("{}", out.trim_end());
    }
}
