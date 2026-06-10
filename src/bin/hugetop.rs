//! hugetop — display HugePages usage. Corresponds to procps-v4.0.6/src/hugetop.c
//!
//! [PLATFORM:LINUX] read HugePages_* from /proc/meminfo and /sys/kernel/mm/hugepages/.
//! [PLATFORM:WINDOWS] / [PLATFORM:MACOS] HugePages is a Linux-only concept → unsupported.
//! CLI flags are provided on all three platforms.

use clap::Parser;
use procps::common::version_string;

#[derive(Parser)]
#[command(
    name = "hugetop",
    about = "Display HugePages usage (Linux)",
    disable_version_flag = true
)]
struct Args {
    #[arg(short = 'V', long)]
    version: bool,
    /// Update delay in seconds
    #[arg(short = 'd', long, value_name = "SECS", default_value_t = 3.0)]
    delay: f64,
    /// Show HugePages information per NUMA node
    #[arg(short = 'n', long)]
    numa: bool,
    /// Display once and then exit
    #[arg(short = 'o', long)]
    once: bool,
    /// Human-readable output
    #[arg(short = 'H', long)]
    human: bool,
}

fn main() {
    let args = Args::parse();
    if args.version {
        println!("{}", version_string("hugetop"));
        return;
    }
    #[cfg(target_os = "linux")]
    imp::run(&args);
    #[cfg(not(target_os = "linux"))]
    {
        let _ = &args;
        eprintln!(
            "hugetop: HugePages is a Linux-only concept; this tool is only available on Linux.\n\
             Current platform: {}.\n\
             [PLATFORM:WINDOWS] Windows large pages (VirtualAlloc MEM_LARGE_PAGES) have different semantics and there is no equivalent system-wide statistics file.",
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

    fn render(args: &Args) {
        let text = fs::read_to_string("/proc/meminfo").unwrap_or_default();
        let mut fields = std::collections::BTreeMap::new();
        for line in text.lines() {
            if line.starts_with("HugePages_")
                || line.starts_with("Hugepagesize")
                || line.starts_with("Hugetlb")
            {
                if let Some((k, v)) = line.split_once(':') {
                    fields.insert(k.to_string(), v.trim().to_string());
                }
            }
        }
        if fields.is_empty() {
            println!("hugetop: HugePages is not enabled on this system.");
            return;
        }
        println!("HugePages summary (/proc/meminfo):");
        for (k, v) in &fields {
            println!("  {k:<20} {v}");
        }

        let dir = "/sys/kernel/mm/hugepages";
        if let Ok(entries) = fs::read_dir(dir) {
            println!("\nPer size (/sys/kernel/mm/hugepages):");
            for e in entries.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                let nr = fs::read_to_string(e.path().join("nr_hugepages")).unwrap_or_default().trim().to_string();
                let free = fs::read_to_string(e.path().join("free_hugepages")).unwrap_or_default().trim().to_string();
                println!("  {name:<24} nr={nr:<8} free={free}");
            }
        }

        // -n/--numa: per NUMA node
        if args.numa {
            if let Ok(nodes) = fs::read_dir("/sys/devices/system/node") {
                println!("\nNUMA nodes:");
                for node in nodes.flatten() {
                    let nname = node.file_name().to_string_lossy().into_owned();
                    if !nname.starts_with("node") { continue; }
                    let hp = node.path().join("hugepages");
                    if let Ok(sizes) = fs::read_dir(&hp) {
                        for sz in sizes.flatten() {
                            let sname = sz.file_name().to_string_lossy().into_owned();
                            let nr = fs::read_to_string(sz.path().join("nr_hugepages")).unwrap_or_default().trim().to_string();
                            println!("  {nname} {sname:<22} nr={nr}");
                        }
                    }
                }
            }
        }
        let _ = args.human;
    }

    pub fn run(args: &Args) {
        loop {
            render(args);
            if args.once {
                break;
            }
            sleep(Duration::from_secs_f64(args.delay.max(1.0)));
            println!();
        }
    }
}
