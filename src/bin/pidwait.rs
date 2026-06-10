//! pidwait — wait for the given processes to exit. Corresponds to procps-v4.0.6/src/pidwait.c
//!
//! Cross-platform:
//! - Linux: poll /proc/[pid] (the C version uses pidfd)
//! - Windows: WaitForSingleObject (event-driven)
//! - macOS: polling (could switch to kqueue)

use clap::Parser;
use procps::common::version_string;
use procps::matcher::{Selection, select};
use procps::platform;

#[derive(Parser)]
#[command(
    name = "pidwait",
    about = "Wait for matching processes to exit",
    disable_version_flag = true
)]
struct Args {
    #[arg(short = 'V', long)]
    version: bool,
    /// Print the matching PIDs before waiting
    #[arg(short = 'e', long)]
    echo: bool,
    /// Report only the number of matches
    #[arg(short, long)]
    count: bool,

    // Pattern matching
    #[arg(short, long)]
    full: bool,
    #[arg(short, long)]
    ignore_case: bool,
    #[arg(short = 'x', long)]
    exact: bool,
    #[arg(short = 'v', long)]
    inverse: bool,

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

    /// Match pattern
    pattern: Option<String>,
}

fn main() {
    let args = Args::parse();
    if args.version {
        println!("{}", version_string("pidwait"));
        return;
    }

    let targets: Vec<u32> = if !args.pid.is_empty() && args.pattern.is_none() {
        args.pid.clone()
    } else {
        let sel = Selection {
            full: args.full,
            ignore_case: args.ignore_case,
            exact: args.exact,
            inverse: args.inverse,
            pids: args.pid.clone(),
            ppids: args.parent,
            euids: args.euid,
            ruids: args.uid,
            terminals: args.terminal,
            older: args.older,
            newest: args.newest,
            oldest: args.oldest,
            ..Default::default()
        };
        let procs = match platform::list_processes() {
            Ok(p) => p,
            Err(e) => {
                eprintln!("pidwait: {e}");
                std::process::exit(2);
            }
        };
        let self_pid = std::process::id();
        select(&procs, args.pattern.as_deref(), &sel)
            .iter()
            .map(|p| p.pid)
            .filter(|p| *p != self_pid)
            .collect()
    };

    if targets.is_empty() {
        std::process::exit(1);
    }
    if args.count {
        println!("{}", targets.len());
    }
    if args.echo {
        for pid in &targets {
            println!("{pid}");
        }
    }

    let mut threads = Vec::new();
    for pid in targets {
        threads.push(std::thread::spawn(move || {
            match platform::wait_process_exit(pid, None) {
                Ok(_) => {}
                Err(e) => eprintln!("pidwait: failed waiting for {pid}: {e}"),
            }
        }));
    }
    for t in threads {
        let _ = t.join();
    }
}
