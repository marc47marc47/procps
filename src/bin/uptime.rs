//! uptime — show system uptime, number of users, and load average. Corresponds to procps-v4.0.6/src/uptime.c
//!
//! Cross-platform: uptime / sessions / loadavg are all provided by platform.
//! [PLATFORM:WINDOWS] Windows has no load average; that field shows n/a.

use clap::Parser;
use procps::common::{unsupported_note, version_string};
use procps::platform;

#[derive(Parser)]
#[command(
    name = "uptime",
    about = "Show system uptime and load",
    disable_version_flag = true
)]
struct Args {
    /// Show version
    #[arg(short = 'V', long)]
    version: bool,
    /// Show only the elapsed time since boot, in compact form
    #[arg(short, long)]
    pretty: bool,
    /// Show the time the system booted
    #[arg(short, long)]
    since: bool,
    /// Output values in raw format (not prettified)
    #[arg(short, long)]
    raw: bool,
    /// Show container uptime (Linux only)
    #[arg(short, long)]
    container: bool,
}

fn main() {
    let args = Args::parse();
    if args.version {
        println!("{}", version_string("uptime"));
        return;
    }
    if args.container && !cfg!(target_os = "linux") {
        eprintln!("{}", unsupported_note("uptime", "-c/--container"));
    }

    let up = match platform::uptime() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("uptime: {e}");
            std::process::exit(1);
        }
    };

    if args.since {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
            .saturating_sub(up.as_secs());
        println!("{secs} (seconds, since Unix epoch)");
        return;
    }

    if args.pretty {
        println!("up {}", procps::units::format_uptime(up.as_secs()));
        return;
    }

    let users = platform::sessions().map(|s| s.len()).unwrap_or(0);
    let load = match platform::loadavg() {
        Ok(Some((a, b, c))) => format!("{a:.2}, {b:.2}, {c:.2}"),
        _ => "n/a (not supported on this platform)".to_string(),
    };

    if args.raw {
        // Raw format: seconds, user count, three load values
        let load_raw = match platform::loadavg() {
            Ok(Some((a, b, c))) => format!("{a} {b} {c}"),
            _ => "0 0 0".to_string(),
        };
        println!("uptime_secs={} users={} loadavg={}", up.as_secs(), users, load_raw);
        return;
    }

    println!(
        " up {},  {} user{},  load average: {}",
        procps::units::format_uptime(up.as_secs()),
        users,
        if users == 1 { "" } else { "s" },
        load,
    );
}
