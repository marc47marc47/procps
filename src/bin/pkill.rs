//! pkill — find processes by name/attributes and send them a signal. Corresponds to procps-v4.0.6/src/pgrep.c (pkill mode)
//!
//! [PLATFORM:WINDOWS] Signal semantics are limited: only TERM/KILL/INT/HUP/QUIT force termination (TerminateProcess);
//! other signals are reported as unsupported. See the platform::Signal comments and PORTING.md.

use clap::Parser;
use procps::common::{read_pidfile, unsupported_note, version_string};
use procps::matcher::{Selection, select};
use procps::platform::{self, Signal};

#[derive(Parser)]
#[command(
    name = "pkill",
    about = "Find processes by criteria and send them a signal",
    disable_version_flag = true
)]
struct Args {
    #[arg(short = 'V', long)]
    version: bool,
    /// Signal to send (name or number, default TERM)
    #[arg(long = "signal", value_name = "SIG")]
    signal: Option<String>,

    // Pattern matching
    #[arg(short, long)]
    full: bool,
    #[arg(short, long)]
    ignore_case: bool,
    #[arg(short = 'x', long)]
    exact: bool,
    #[arg(short = 'v', long)]
    inverse: bool,

    // Behavior
    #[arg(short = 'e', long)]
    echo: bool,
    #[arg(short = 'c', long)]
    count: bool,
    #[arg(short = 'H', long = "require-handler")]
    require_handler: bool,
    #[arg(short = 'q', long = "queue", value_name = "VALUE")]
    queue: Option<i64>,
    #[arg(short = 'm', long = "mrelease")]
    mrelease: bool,

    // Selection criteria
    #[arg(short = 'n', long)]
    newest: bool,
    #[arg(short = 'o', long)]
    oldest: bool,
    #[arg(short = 'O', long = "older", value_name = "SECS")]
    older: Option<u64>,
    #[arg(short = 'p', long = "pid", value_name = "PID", value_delimiter = ',')]
    pid: Vec<u32>,
    #[arg(short = 'P', long = "parent", value_name = "PPID", value_delimiter = ',')]
    parent: Vec<u32>,
    #[arg(short = 'u', long = "euid", value_name = "ID", value_delimiter = ',')]
    euid: Vec<String>,
    #[arg(short = 'U', long = "uid", value_name = "ID", value_delimiter = ',')]
    uid: Vec<String>,
    #[arg(short = 't', long = "terminal", value_name = "TTY", value_delimiter = ',')]
    terminal: Vec<String>,
    #[arg(short = 'F', long = "pidfile", value_name = "FILE")]
    pidfile: Option<String>,

    // Linux-only
    #[arg(short = 'g', long = "pgroup", value_name = "PGID", value_delimiter = ',')]
    pgroup: Vec<u32>,
    #[arg(short = 'G', long = "group", value_name = "GID", value_delimiter = ',')]
    group: Vec<String>,
    #[arg(short = 's', long = "session", value_name = "SID", value_delimiter = ',')]
    session: Vec<u32>,
    #[arg(short = 'r', long = "runstates", value_name = "STATES")]
    runstates: Option<String>,
    #[arg(long = "cgroup", value_name = "GRP", value_delimiter = ',')]
    cgroup: Vec<String>,

    /// Match pattern
    pattern: Option<String>,
}

fn main() {
    // Support the procps-style -SIGNAME / -SIGNUM prefix form (pre-scan argv first)
    let raw: Vec<String> = std::env::args().collect();
    let mut sig_override: Option<String> = None;
    let mut filtered: Vec<String> = Vec::with_capacity(raw.len());
    for (i, a) in raw.iter().enumerate() {
        if i == 0 {
            filtered.push(a.clone());
            continue;
        }
        if let Some(rest) = a.strip_prefix('-')
            && !rest.is_empty()
            && Signal::parse(rest).is_some()
            && a != "-s"
        {
            sig_override = Some(rest.to_string());
            continue;
        }
        filtered.push(a.clone());
    }

    let args = Args::parse_from(filtered);
    if args.version {
        println!("{}", version_string("pkill"));
        return;
    }

    if !cfg!(target_os = "linux") {
        for (used, flag) in [
            (!args.pgroup.is_empty(), "-g/--pgroup"),
            (!args.group.is_empty(), "-G/--group"),
            (!args.session.is_empty(), "-s/--session"),
            (args.runstates.is_some(), "-r/--runstates"),
            (!args.cgroup.is_empty(), "--cgroup"),
            (args.require_handler, "-H/--require-handler"),
            (args.queue.is_some(), "-q/--queue"),
            (args.mrelease, "-m/--mrelease"),
        ] {
            if used {
                eprintln!("{}", unsupported_note("pkill", flag));
            }
        }
    }

    let sig_str = args.signal.clone().or(sig_override).unwrap_or_else(|| "TERM".into());
    let sig = match Signal::parse(&sig_str) {
        Some(s) => s,
        None => {
            eprintln!("pkill: invalid signal: {sig_str}");
            std::process::exit(2);
        }
    };

    let mut pids = args.pid.clone();
    if let Some(pf) = &args.pidfile {
        match read_pidfile(pf) {
            Ok(p) => pids.push(p),
            Err(e) => {
                eprintln!("pkill: cannot read pidfile {pf}: {e}");
                std::process::exit(2);
            }
        }
    }

    let sel = Selection {
        full: args.full,
        ignore_case: args.ignore_case,
        exact: args.exact,
        inverse: args.inverse,
        pids,
        ppids: args.parent,
        euids: args.euid,
        ruids: args.uid,
        terminals: args.terminal,
        older: args.older,
        newest: args.newest,
        oldest: args.oldest,
        pgroups: args.pgroup,
        groups: args.group,
        sessions: args.session,
        runstates: args.runstates.as_deref().unwrap_or("").chars().collect(),
        cgroups: args.cgroup,
    };

    let procs = match platform::list_processes() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("pkill: cannot enumerate processes: {e}");
            std::process::exit(2);
        }
    };

    let self_pid = std::process::id();
    let mut matched = select(&procs, args.pattern.as_deref(), &sel);
    matched.retain(|p| p.pid != self_pid);
    matched.sort_by_key(|p| p.pid);

    if matched.is_empty() {
        std::process::exit(1);
    }
    if args.count {
        println!("{}", matched.len());
    }

    let mut killed = 0;
    for p in &matched {
        if args.echo {
            println!("{} {}", p.pid, p.name);
        }
        match platform::kill(p.pid, sig) {
            Ok(()) => killed += 1,
            Err(e) => eprintln!("pkill: cannot signal {} ({}): {e}", p.pid, p.name),
        }
    }
    std::process::exit(if killed > 0 { 0 } else { 1 });
}
