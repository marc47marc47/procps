//! vmstat — report virtual memory statistics. Corresponds to procps-v4.0.6/src/vmstat.c
//!
//! [PLATFORM:WINDOWS] Most vmstat fields (si/so/bi/bo/in/cs) have no public Win32 equivalent
//! and show '-'. The memory/swap/cpu fields are available. Disk/slab modes are Linux only.

use std::time::Duration;

use clap::Parser;
use procps::common::{unsupported_exit, version_string};
use procps::platform;

#[derive(Parser)]
#[command(
    name = "vmstat",
    about = "Report virtual memory and system activity statistics",
    disable_version_flag = true
)]
struct Args {
    /// Show version
    #[arg(short = 'V', long)]
    version: bool,
    /// Update interval in seconds (omit to output only once)
    delay: Option<u64>,
    /// Number of updates to output
    count: Option<u64>,
    /// Show active and inactive memory
    #[arg(short, long)]
    active: bool,
    /// Show the number of forks since boot (Linux)
    #[arg(short, long)]
    forks: bool,
    /// Show slab info (Linux)
    #[arg(short = 'm', long)]
    slabs: bool,
    /// Print the header only once
    #[arg(short = 'n', long = "one-header")]
    one_header: bool,
    /// Show event counter statistics
    #[arg(short = 's', long)]
    stats: bool,
    /// Show disk statistics (Linux)
    #[arg(short = 'd', long)]
    disk: bool,
    /// Show a disk statistics summary (Linux)
    #[arg(short = 'D', long = "disk-sum")]
    disk_sum: bool,
    /// Statistics for a specific partition (Linux)
    #[arg(short = 'p', long, value_name = "DEV")]
    partition: Option<String>,
    /// Show memory in the given unit (k/K/m/M)
    #[arg(short = 'S', long, value_name = "UNIT", default_value = "K")]
    unit: String,
    /// Wide output
    #[arg(short, long)]
    wide: bool,
    /// Show timestamps
    #[arg(short, long)]
    timestamp: bool,
    /// Skip the first line of output
    #[arg(short = 'y', long = "no-first")]
    no_first: bool,
}

fn opt(v: Option<u64>) -> String {
    v.map(|n| n.to_string()).unwrap_or_else(|| "-".to_string())
}

fn print_header() {
    println!("procs -----------memory---------- ---swap-- -----io---- -system-- ------cpu-----");
    println!(" r  b   swpd   free   buff  cache   si   so    bi    bo   in   cs us sy id wa st");
}

/// -s mode: event counter overview.
fn print_stats() {
    let m = platform::mem_info().unwrap_or_default();
    let c = platform::cpu_times().unwrap_or_default();
    let vc = platform::vm_counters().unwrap_or_default();
    let k = |b: u64| b / 1024;
    println!("{:>14} K total memory", k(m.total));
    println!("{:>14} K used memory", k(m.total.saturating_sub(m.free)));
    println!("{:>14} K free memory", k(m.free));
    println!("{:>14} K buffer memory", k(m.buffers.unwrap_or(0)));
    println!("{:>14} K swap cache", k(m.cached.unwrap_or(0)));
    println!("{:>14} K total swap", k(m.swap_total));
    println!("{:>14} K used swap", k(m.swap_total.saturating_sub(m.swap_free)));
    println!("{:>14} K free swap", k(m.swap_free));
    println!("{:>14} non-nice user cpu ticks", c.user / 10);
    println!("{:>14} system cpu ticks", c.system / 10);
    println!("{:>14} idle cpu ticks", c.idle / 10);
    println!("{:>14} IO-wait cpu ticks", c.iowait / 10);
    println!("{:>14} interrupts", vc.interrupts.unwrap_or(0));
    println!("{:>14} CPU context switches", vc.context_switches.unwrap_or(0));
}

fn main() {
    let args = Args::parse();
    if args.version {
        println!("{}", version_string("vmstat"));
        return;
    }

    // Linux-only modes: explicitly unsupported on other platforms
    if !cfg!(target_os = "linux") {
        if args.disk || args.disk_sum {
            unsupported_exit("vmstat", "-d/-D (disk statistics)");
        }
        if args.partition.is_some() {
            unsupported_exit("vmstat", "-p (partition statistics)");
        }
        if args.slabs {
            unsupported_exit("vmstat", "-m (slab statistics)");
        }
    }

    if args.stats {
        print_stats();
        return;
    }

    let div = if args.unit.starts_with('M') || args.unit.starts_with('m') {
        1024 * 1024
    } else {
        1024
    };

    let mut printed_header = false;
    let print_hdr = |printed: &mut bool| {
        if !args.one_header || !*printed {
            print_header();
            *printed = true;
        }
    };
    print_hdr(&mut printed_header);

    let mut prev_cpu = platform::cpu_times().ok();
    let mut iter = 0u64;
    loop {
        let m = platform::mem_info().unwrap_or_default();
        let vc = platform::vm_counters().unwrap_or_default();
        let cpu = platform::cpu_times().ok();

        let (us, sy, id, wa, st) = match (&prev_cpu, &cpu) {
            (Some(a), Some(b)) => {
                let dt = b.total().saturating_sub(a.total()).max(1);
                let pct = |x: u64, y: u64| x.saturating_sub(y) * 100 / dt;
                (
                    pct(b.user + b.nice, a.user + a.nice),
                    pct(b.system + b.irq + b.softirq, a.system + a.irq + a.softirq),
                    pct(b.idle, a.idle),
                    pct(b.iowait, a.iowait),
                    pct(b.steal, a.steal),
                )
            }
            _ => (0, 0, 100, 0, 0),
        };

        let buff = m.buffers.unwrap_or(0) / div;
        let cache = m.cached.unwrap_or(0) / div;
        let swpd = m.swap_total.saturating_sub(m.swap_free) / div;

        // -y: skip the first line (the initial output)
        if !(args.no_first && iter == 0) {
            if args.timestamp {
                let secs = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                print!("{secs} ");
            }
            println!(
                "{:>2} {:>2} {:>6} {:>6} {:>6} {:>6} {:>4} {:>4} {:>5} {:>5} {:>4} {:>4} {:>2} {:>2} {:>2} {:>2} {:>2}",
                opt(vc.procs_running),
                opt(vc.procs_blocked),
                swpd,
                m.free / div,
                buff,
                cache,
                opt(vc.swap_in),
                opt(vc.swap_out),
                opt(vc.pages_in),
                opt(vc.pages_out),
                opt(vc.interrupts),
                opt(vc.context_switches),
                us, sy, id, wa, st,
            );
        }

        prev_cpu = cpu;
        iter += 1;
        match args.delay {
            Some(d) => {
                if let Some(c) = args.count
                    && iter >= c
                {
                    break;
                }
                std::thread::sleep(Duration::from_secs(d.max(1)));
            }
            None => break,
        }
    }
}
