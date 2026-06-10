//! pgrep — find processes by name/attributes and list their PIDs. Corresponds to procps-v4.0.6/src/pgrep.c
//!
//! Cross-platform: the process list is provided by platform::list_processes(); selection logic lives in procps::matcher.
//! [PLATFORM:WINDOWS/MACOS] Linux-only selection criteria (-g/-G/-s/-r/--cgroup/--ns etc.)
//! print an unsupported notice on other platforms (the fields are unavailable).

use clap::Parser;
use procps::common::{read_pidfile, unsupported_note, version_string};
use procps::matcher::{Selection, select};
use procps::platform;

#[derive(Parser)]
#[command(
    name = "pgrep",
    about = "Find processes by criteria and output their PIDs",
    disable_version_flag = true
)]
struct Args {
    #[arg(short = 'V', long)]
    version: bool,

    // Pattern matching
    #[arg(short, long)]
    full: bool,
    #[arg(short, long)]
    ignore_case: bool,
    #[arg(short = 'x', long)]
    exact: bool,
    #[arg(short = 'v', long)]
    inverse: bool,

    // Output format
    #[arg(short = 'l', long = "list-name")]
    list_name: bool,
    #[arg(short = 'a', long = "list-full")]
    list_full: bool,
    #[arg(short, long)]
    count: bool,
    #[arg(short = 'd', long = "delimiter", value_name = "STR", default_value = "\n")]
    delimiter: String,
    #[arg(long)]
    quiet: bool,
    #[arg(short = 'w', long = "lightweight")]
    lightweight: bool,
    #[arg(short = 'Q', long = "shell-quote")]
    shell_quote: bool,

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
    #[arg(short = 'A', long = "ignore-ancestors")]
    ignore_ancestors: bool,

    // Linux-only selection
    #[arg(short = 'g', long = "pgroup", value_name = "PGID", value_delimiter = ',')]
    pgroup: Vec<u32>,
    #[arg(short = 'G', long = "group", value_name = "GID", value_delimiter = ',')]
    group: Vec<String>,
    #[arg(short = 's', long = "session", value_name = "SID", value_delimiter = ',')]
    session: Vec<u32>,
    #[arg(short = 'r', long = "runstates", value_name = "STATES")]
    runstates: Option<String>,
    #[arg(short = 'L', long = "logpidfile")]
    logpidfile: bool,
    #[arg(long = "cgroup", value_name = "GRP", value_delimiter = ',')]
    cgroup: Vec<String>,
    #[arg(long = "ns", value_name = "PID")]
    ns: Option<u32>,
    #[arg(long = "nslist", value_name = "NS")]
    nslist: Option<String>,
    #[arg(long = "env", value_name = "VAL")]
    env: Option<String>,
    #[arg(long = "signal", value_name = "SIG")]
    signal: Option<String>,

    /// Match pattern (ERE)
    pattern: Option<String>,
}

fn main() {
    let args = Args::parse();
    if args.version {
        println!("{}", version_string("pgrep"));
        return;
    }

    // Notify about Linux-only flags on other platforms
    if !cfg!(target_os = "linux") {
        let mut used = Vec::new();
        if !args.pgroup.is_empty() { used.push("-g/--pgroup"); }
        if !args.group.is_empty() { used.push("-G/--group"); }
        if !args.session.is_empty() { used.push("-s/--session"); }
        if args.runstates.is_some() { used.push("-r/--runstates"); }
        if !args.cgroup.is_empty() { used.push("--cgroup"); }
        if args.ns.is_some() { used.push("--ns"); }
        for f in used {
            eprintln!("{}", unsupported_note("pgrep", f));
        }
    }

    let mut pids = args.pid.clone();
    if let Some(pf) = &args.pidfile {
        match read_pidfile(pf) {
            Ok(p) => pids.push(p),
            Err(e) => {
                eprintln!("pgrep: cannot read pidfile {pf}: {e}");
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
            eprintln!("pgrep: cannot enumerate processes: {e}");
            std::process::exit(2);
        }
    };

    let self_pid = std::process::id();
    let mut matched = select(&procs, args.pattern.as_deref(), &sel);
    matched.retain(|p| p.pid != self_pid);
    matched.sort_by_key(|p| p.pid);

    if args.count {
        if !args.quiet {
            println!("{}", matched.len());
        }
        std::process::exit(if matched.is_empty() { 1 } else { 0 });
    }

    if !args.quiet {
        let lines: Vec<String> = matched
            .iter()
            .map(|p| {
                if args.list_full {
                    let cmd = if p.cmdline.is_empty() { p.name.clone() } else { p.cmdline.join(" ") };
                    format!("{} {}", p.pid, cmd)
                } else if args.list_name {
                    format!("{} {}", p.pid, p.name)
                } else if args.shell_quote {
                    format!("'{}'", p.pid)
                } else {
                    p.pid.to_string()
                }
            })
            .collect();
        // -d delimiter (defaults to newline)
        print!("{}", lines.join(&args.delimiter));
        if !lines.is_empty() {
            println!();
        }
    }
    std::process::exit(if matched.is_empty() { 1 } else { 0 });
}
