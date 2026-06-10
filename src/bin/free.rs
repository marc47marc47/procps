//! free — display system memory usage. Corresponds to procps-v4.0.6/src/free.c
//!
//! Cross-platform: memory data is provided by procps::platform::mem_info()
//! (Linux=/proc/meminfo, Windows=GlobalMemoryStatusEx, macOS pending).

use clap::Parser;
use procps::common::version_string;
use procps::platform;

#[derive(Parser)]
#[command(
    name = "free",
    about = "Display free and used physical memory and swap space",
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

    // ---- Units (IEC, base 1024) ----
    /// Show output in bytes
    #[arg(short = 'b', long)]
    bytes: bool,
    /// Show output in KiB (default)
    #[arg(short, long)]
    kibi: bool,
    /// Show output in MiB
    #[arg(short, long)]
    mebi: bool,
    /// Show output in GiB
    #[arg(short, long)]
    gibi: bool,
    /// Show output in TiB
    #[arg(long)]
    tebi: bool,
    /// Show output in PiB
    #[arg(long)]
    pebi: bool,

    // ---- Units (SI, base 1000) ----
    /// Show output in KB (1000)
    #[arg(long)]
    kilo: bool,
    /// Show output in MB (1000)
    #[arg(long)]
    mega: bool,
    /// Show output in GB (1000)
    #[arg(long)]
    giga: bool,
    /// Show output in TB (1000)
    #[arg(long)]
    tera: bool,
    /// Show output in PB (1000)
    #[arg(long)]
    peta: bool,

    /// Human-readable output (auto-scaled)
    #[arg(short = 'h', long)]
    human: bool,
    /// Use base 1000 in human mode
    #[arg(long)]
    si: bool,

    /// Show low and high memory statistics
    #[arg(short = 'l', long)]
    lohi: bool,
    /// Single-line output
    #[arg(short = 'L', long)]
    line: bool,
    /// Show the total row
    #[arg(short = 't', long)]
    total: bool,
    /// Show committed memory
    #[arg(short = 'v', long)]
    committed: bool,
    /// Wide output (buffers and cache in separate columns)
    #[arg(short = 'w', long)]
    wide: bool,

    /// Refresh continuously every N seconds
    #[arg(short = 's', long, value_name = "N")]
    seconds: Option<f64>,
    /// With -s, stop after repeating N times
    #[arg(short = 'c', long, value_name = "COUNT")]
    count: Option<u64>,
}

enum Unit {
    Bytes,
    Fixed(u64), // divisor
    Human(u64),               // base 1024 or 1000
}

impl Args {
    fn unit(&self) -> Unit {
        // IEC
        if self.bytes {
            Unit::Bytes
        } else if self.mebi {
            Unit::Fixed(1 << 20)
        } else if self.gibi {
            Unit::Fixed(1 << 30)
        } else if self.tebi {
            Unit::Fixed(1 << 40)
        } else if self.pebi {
            Unit::Fixed(1 << 50)
        // SI
        } else if self.kilo {
            Unit::Fixed(1000)
        } else if self.mega {
            Unit::Fixed(1_000_000)
        } else if self.giga {
            Unit::Fixed(1_000_000_000)
        } else if self.tera {
            Unit::Fixed(1_000_000_000_000)
        } else if self.peta {
            Unit::Fixed(1_000_000_000_000_000)
        } else if self.human {
            Unit::Human(if self.si { 1000 } else { 1024 })
        } else {
            // Default KiB (-k)
            Unit::Fixed(1024)
        }
    }
}

fn human_base(bytes: u64, base: u64) -> String {
    const U: [&str; 7] = ["B", "K", "M", "G", "T", "P", "E"];
    if bytes < base {
        return format!("{bytes}B");
    }
    let mut v = bytes as f64;
    let mut i = 0;
    let b = base as f64;
    while v >= b && i < U.len() - 1 {
        v /= b;
        i += 1;
    }
    let suffix = if base == 1024 {
        ["B", "Ki", "Mi", "Gi", "Ti", "Pi", "Ei"][i]
    } else {
        U[i]
    };
    if v >= 100.0 {
        format!("{v:.0}{suffix}")
    } else {
        format!("{v:.1}{suffix}")
    }
}

fn fmt(bytes: u64, unit: &Unit) -> String {
    match unit {
        Unit::Bytes => bytes.to_string(),
        Unit::Fixed(div) => (bytes / div).to_string(),
        Unit::Human(base) => human_base(bytes, *base),
    }
}

fn print_once(args: &Args, unit: &Unit) -> std::io::Result<()> {
    let m = platform::mem_info()?;
    let buff = m.buffers.unwrap_or(0);
    let cache = m.cached.unwrap_or(0);
    let buff_cache = buff + cache;
    let used = m.total.saturating_sub(m.free).saturating_sub(buff_cache);
    let swap_used = m.swap_total.saturating_sub(m.swap_free);

    if args.line {
        // Single-line format (-L): compact display
        println!(
            "{:<7} {:>11} {:>11} {:>11} {:>11} {:>11}",
            "Mem:",
            fmt(m.total, unit),
            fmt(used, unit),
            fmt(m.free, unit),
            fmt(buff_cache, unit),
            fmt(m.available, unit),
        );
        return Ok(());
    }

    let w = 12;
    // Header
    print!("{:<7}", "");
    if args.wide {
        for h in ["total", "used", "free", "shared", "buffers", "cache", "available"] {
            print!("{h:>w$}");
        }
    } else {
        for h in ["total", "used", "free", "shared", "buff/cache", "available"] {
            print!("{h:>w$}");
        }
    }
    println!();

    // Mem row
    print!("{:<7}", "Mem:");
    print!("{:>w$}", fmt(m.total, unit));
    print!("{:>w$}", fmt(used, unit));
    print!("{:>w$}", fmt(m.free, unit));
    print!("{:>w$}", fmt(m.shared.unwrap_or(0), unit));
    if args.wide {
        print!("{:>w$}", fmt(buff, unit));
        print!("{:>w$}", fmt(cache, unit));
    } else {
        print!("{:>w$}", fmt(buff_cache, unit));
    }
    print!("{:>w$}", fmt(m.available, unit));
    println!();

    // Low/High (-l): on 64-bit systems High is usually 0 and Low approximately equals total
    if args.lohi {
        print!("{:<7}", "Low:");
        print!("{:>w$}", fmt(m.total, unit));
        print!("{:>w$}", fmt(used, unit));
        print!("{:>w$}", fmt(m.free, unit));
        println!();
        print!("{:<7}", "High:");
        print!("{:>w$}", fmt(0, unit));
        print!("{:>w$}", fmt(0, unit));
        print!("{:>w$}", fmt(0, unit));
        println!();
    }

    // Swap row
    print!("{:<7}", "Swap:");
    print!("{:>w$}", fmt(m.swap_total, unit));
    print!("{:>w$}", fmt(swap_used, unit));
    print!("{:>w$}", fmt(m.swap_free, unit));
    println!();

    // Total row (-t)
    if args.total {
        print!("{:<7}", "Total:");
        print!("{:>w$}", fmt(m.total + m.swap_total, unit));
        print!("{:>w$}", fmt(used + swap_used, unit));
        print!("{:>w$}", fmt(m.free + m.swap_free, unit));
        println!();
    }

    // Committed(-v)
    if args.committed {
        let limit = m.commit_limit.unwrap_or(0);
        let cmt = m.committed.unwrap_or(0);
        print!("{:<7}", "Comm:");
        print!("{:>w$}", fmt(limit, unit));
        print!("{:>w$}", fmt(cmt, unit));
        print!("{:>w$}", fmt(limit.saturating_sub(cmt), unit));
        println!();
    }
    Ok(())
}

fn main() {
    let args = Args::parse();
    if args.version {
        println!("{}", version_string("free"));
        return;
    }
    let unit = args.unit();

    if let Some(secs) = args.seconds {
        let mut n = 0u64;
        loop {
            if let Err(e) = print_once(&args, &unit) {
                eprintln!("free: {e}");
                std::process::exit(1);
            }
            n += 1;
            if let Some(c) = args.count
                && n >= c
            {
                break;
            }
            println!();
            std::thread::sleep(std::time::Duration::from_secs_f64(secs));
        }
    } else if let Err(e) = print_once(&args, &unit) {
        eprintln!("free: {e}");
        std::process::exit(1);
    }
}
