//! w — show who is currently logged in and what they are doing. Corresponds to procps-v4.0.6/src/w.c
//!
//! Cross-platform: sessions() provides the login list
//! (Linux=utmp, Windows=WTSEnumerateSessionsW, macOS pending).
//! [PLATFORM:WINDOWS] No per-tty idle/JCPU/PCPU or "current command" info; those fields show '-'.

use clap::Parser;
use procps::common::version_string;
use procps::platform;

#[derive(Parser)]
#[command(
    name = "w",
    about = "Show logged-in users and what they are doing",
    disable_help_flag = true,
    disable_version_flag = true
)]
struct Args {
    /// Show help
    #[arg(long, action = clap::ArgAction::Help)]
    help: Option<bool>,
    /// Show version
    #[arg(short = 'V', long)]
    version: bool,
    /// Do not print the header
    #[arg(short = 'h', long = "no-header")]
    no_header: bool,
    /// Do not show the WHAT column for the current process (compact)
    #[arg(short = 's', long)]
    short: bool,
    /// Show container uptime (Linux only)
    #[arg(short = 'c', long)]
    container: bool,
    /// Ignore the current user (Linux-specific semantics; skipped cross-platform)
    #[arg(short = 'u', long = "no-current")]
    no_current: bool,
    /// Show the terminal column
    #[arg(short = 't', long)]
    terminal: bool,
    /// Toggle the remote hostname column
    #[arg(short = 'f', long)]
    from: bool,
    /// Old-style output
    #[arg(short = 'o', long = "old-style")]
    old_style: bool,
    /// Show IP address instead of hostname
    #[arg(short = 'i', long = "ip-addr")]
    ip_addr: bool,
    /// Show process PIDs
    #[arg(short = 'p', long)]
    pids: bool,
}

fn main() {
    let args = Args::parse();
    if args.version {
        println!("{}", version_string("w"));
        return;
    }

    if !args.no_header {
        let up = platform::uptime().map(|d| procps::units::format_uptime(d.as_secs()));
        let users = platform::sessions().map(|s| s.len()).unwrap_or(0);
        let load = match platform::loadavg() {
            Ok(Some((a, b, c))) => format!("{a:.2}, {b:.2}, {c:.2}"),
            _ => "n/a".to_string(),
        };
        if let Ok(u) = up {
            println!(
                " up {},  {} user{},  load average: {}",
                u,
                users,
                if users == 1 { "" } else { "s" },
                load
            );
        }
    }

    if args.short {
        println!("{:<10} {:<10} {:<8}", "USER", "TTY", "IDLE");
    } else {
        println!(
            "{:<10} {:<12} {:<16} {:<8} {:<6} {:<6} {:<6} {}",
            "USER", "TTY", "FROM", "LOGIN@", "IDLE", "JCPU", "PCPU", "WHAT"
        );
    }

    match platform::sessions() {
        Ok(list) => {
            for s in list {
                let from = s.host.unwrap_or_else(|| "-".to_string());
                if args.short {
                    println!("{:<10} {:<10} {:<8}", s.user, s.line, "-");
                } else {
                    println!(
                        "{:<10} {:<12} {:<16} {:<8} {:<6} {:<6} {:<6} {}",
                        s.user, s.line, from, "-", "-", "-", "-", "-"
                    );
                }
            }
        }
        Err(e) => {
            eprintln!("w: failed to get sessions: {e}");
            std::process::exit(1);
        }
    }
}
