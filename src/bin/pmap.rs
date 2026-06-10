//! pmap — display a process's memory map. Corresponds to procps-v4.0.6/src/pmap.c
//!
//! Cross-platform:
//! - Linux: parse /proc/[pid]/maps
//! - Windows: scan region by region with VirtualQueryEx + K32GetMappedFileNameW for the mapped file name
//! - macOS: mach_vm_region (TODO)

use clap::Parser;
use procps::common::version_string;
use procps::platform;

#[derive(Parser)]
#[command(
    name = "pmap",
    about = "Display a process's memory map",
    disable_version_flag = true
)]
struct Args {
    #[arg(short = 'V', long)]
    version: bool,
    /// Show extended format (permissions column)
    #[arg(short = 'x', long)]
    extended: bool,
    /// Show more details (same as extended)
    #[arg(short = 'X')]
    more_extended: bool,
    /// Show all details provided by the kernel
    #[arg(long = "XX")]
    max_extended: bool,
    /// Device format
    #[arg(short = 'd', long)]
    device: bool,
    /// Do not show header and footer
    #[arg(short = 'q', long)]
    quiet: bool,
    /// Show the full path of mappings
    #[arg(short = 'p', long = "show-path")]
    show_path: bool,
    /// Use the name provided by the kernel
    #[arg(short = 'k', long = "use-kernel-name")]
    use_kernel_name: bool,
    /// Limit to address range [low][,high]
    #[arg(short = 'A', long = "range", value_name = "LOW,HIGH")]
    range: Option<String>,
    /// SunOS compatibility (ignored)
    #[arg(short = 'r')]
    _sunos: bool,
    /// PIDs to inspect (one or more)
    pids: Vec<u32>,
}

fn parse_range(s: &str) -> (Option<usize>, Option<usize>) {
    let parse = |x: &str| usize::from_str_radix(x.trim().trim_start_matches("0x"), 16).ok();
    match s.split_once(',') {
        Some((lo, hi)) => (parse(lo), parse(hi)),
        None => (parse(s), None),
    }
}

fn main() {
    let args = Args::parse();
    if args.version {
        println!("{}", version_string("pmap"));
        return;
    }
    if args.pids.is_empty() {
        eprintln!("Usage: pmap [options] PID...");
        std::process::exit(1);
    }
    let extended = args.extended || args.more_extended || args.max_extended;
    let (lo, hi) = args.range.as_deref().map(parse_range).unwrap_or((None, None));

    let mut rc = 0;
    for pid in &args.pids {
        let regions = match platform::process_maps(*pid) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("pmap: {pid}: {e}");
                rc = 1;
                continue;
            }
        };

        if !args.quiet {
            let name = platform::process_info(*pid)
                .map(|p| p.cmdline.join(" "))
                .unwrap_or_default();
            println!("{pid}:   {name}");
        }

        let mut total_kb = 0u64;
        if extended && !args.quiet {
            println!("{:>16} {:>8} {:<5} {}", "Address", "Kbytes", "Mode", "Mapping");
        }
        for r in &regions {
            if let Some(l) = lo
                && r.base < l
            {
                continue;
            }
            if let Some(h) = hi
                && r.base > h
            {
                continue;
            }
            let kb = (r.size / 1024) as u64;
            total_kb += kb;
            let mapping = r.mapping.clone().unwrap_or_else(|| "[ anon ]".to_string());
            // -p/--show-path: show the full path (in this implementation `mapping` is already the full path, so keep it)
            if extended {
                println!("{:016x} {:>8} {:<5} {}", r.base, kb, r.perms, mapping);
            } else {
                println!("{:016x} {:>8}K {} {}", r.base, kb, r.perms, mapping);
            }
        }
        if !args.quiet {
            println!("{:>16} {:>8}K", "total", total_kb);
        }
    }
    std::process::exit(rc);
}
