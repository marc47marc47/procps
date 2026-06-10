//! slabtop — display kernel slab cache information. Corresponds to procps-v4.0.6/src/slabtop.c
//!
//! [PLATFORM:LINUX] parse /proc/slabinfo (full content requires root).
//! [PLATFORM:WINDOWS] / [PLATFORM:MACOS] no user-readable slab allocator statistics → unsupported.
//! CLI flags are provided on all three platforms.

use clap::Parser;
use procps::common::version_string;

#[derive(Parser)]
#[command(
    name = "slabtop",
    about = "Display kernel slab cache statistics (Linux)",
    disable_version_flag = true
)]
struct Args {
    #[arg(short = 'V', long)]
    version: bool,
    /// Update delay in seconds
    #[arg(short = 'd', long, value_name = "SECS", default_value_t = 3.0)]
    delay: f64,
    /// Display once and then exit
    #[arg(short = 'o', long)]
    once: bool,
    /// Human-readable output
    #[arg(long)]
    human: bool,
    /// Sort criterion: c=cache size, o=object count, s=object size, n=name
    #[arg(short = 's', long, value_name = "CHAR", default_value = "c")]
    sort: char,
}

fn main() {
    let args = Args::parse();
    if args.version {
        println!("{}", version_string("slabtop"));
        return;
    }
    #[cfg(target_os = "linux")]
    imp::run(&args);
    #[cfg(not(target_os = "linux"))]
    {
        let _ = &args;
        eprintln!(
            "slabtop: this tool is only available on Linux (it parses kernel slab allocator statistics from /proc/slabinfo).\n\
             Current platform: {}.\n\
             [PLATFORM:WINDOWS] the Windows counterpart is the kernel pool, which requires a kernel debugger / ETW and is not available to ordinary tools.",
            std::env::consts::OS
        );
        std::process::exit(1);
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use super::Args;
    use std::fs;
    use std::thread::sleep;
    use std::time::Duration;

    struct Slab {
        name: String,
        active: u64,
        num: u64,
        objsize: u64,
    }

    fn human(bytes: u64) -> String {
        procps::units::human_bytes(bytes)
    }

    fn render(args: &Args) -> bool {
        let text = match fs::read_to_string("/proc/slabinfo") {
            Ok(t) => t,
            Err(e) => {
                eprintln!("slabtop: cannot read /proc/slabinfo: {e} (root may be required)");
                return false;
            }
        };
        let mut slabs = Vec::new();
        for line in text.lines() {
            if line.starts_with('#') || line.starts_with("slabinfo") { continue; }
            let f: Vec<&str> = line.split_whitespace().collect();
            if f.len() < 4 { continue; }
            let p = |s: &str| s.parse::<u64>().unwrap_or(0);
            slabs.push(Slab { name: f[0].to_string(), active: p(f[1]), num: p(f[2]), objsize: p(f[3]) });
        }
        match args.sort {
            'o' => slabs.sort_by(|a, b| b.num.cmp(&a.num)),
            's' => slabs.sort_by(|a, b| b.objsize.cmp(&a.objsize)),
            'n' => slabs.sort_by(|a, b| a.name.cmp(&b.name)),
            _ => slabs.sort_by(|a, b| (b.num * b.objsize).cmp(&(a.num * a.objsize))),
        }
        let total: u64 = slabs.iter().map(|s| s.num * s.objsize).sum();
        println!("Active / Total slab caches: {}", slabs.len());
        if args.human {
            println!("Total size: {}\n", human(total));
        } else {
            println!("Total size: {:.2} MB\n", total as f64 / 1024.0 / 1024.0);
        }
        println!("{:>8} {:>8} {:>9} {:>10}  {}", "OBJS", "ACTIVE", "OBJSIZE", "CACHE", "NAME");
        for s in slabs.iter().take(40) {
            let cache = s.num * s.objsize;
            let (osize, csize) = if args.human {
                (human(s.objsize), human(cache))
            } else {
                (format!("{}B", s.objsize), format!("{}K", cache / 1024))
            };
            println!("{:>8} {:>8} {:>9} {:>10}  {}", s.num, s.active, osize, csize, s.name);
        }
        true
    }

    pub fn run(args: &Args) {
        loop {
            if !render(args) {
                std::process::exit(1);
            }
            if args.once {
                break;
            }
            sleep(Duration::from_secs_f64(args.delay.max(1.0)));
            println!();
        }
    }
}
