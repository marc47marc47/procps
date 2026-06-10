//! tload — display the system load average as a text graph. Corresponds to procps-v4.0.6/src/tload.c
//!
//! [PLATFORM:WINDOWS] Windows has no load average, so "CPU usage %" is used as a
//! substitute graphing source, with a notice printed at startup. Linux/macOS use the real loadavg.

use std::io::Write;
use std::time::Duration;

use clap::Parser;
use procps::common::version_string;
use procps::platform;

#[derive(Parser)]
#[command(
    name = "tload",
    about = "Display the system load as an ASCII graph",
    disable_version_flag = true
)]
struct Args {
    /// Show version
    #[arg(short = 'V', long)]
    version: bool,
    /// Update delay in seconds
    #[arg(short = 'd', long, value_name = "SECS", default_value_t = 5u64)]
    delay: u64,
    /// Vertical scale (load per cell; larger values flatten the graph)
    #[arg(short = 's', long, value_name = "NUM")]
    scale: Option<f64>,
}

fn sample_value() -> f64 {
    match platform::loadavg() {
        Ok(Some((a, _, _))) => a,
        _ => cpu_busy_percent().unwrap_or(0.0) / 100.0 * platform::cpu_count() as f64,
    }
}

/// Measure the CPU busy percentage (%) over a short interval.
fn cpu_busy_percent() -> Option<f64> {
    let a = platform::cpu_times().ok()?;
    std::thread::sleep(Duration::from_millis(200));
    let b = platform::cpu_times().ok()?;
    let dt = b.total().saturating_sub(a.total());
    if dt == 0 {
        return Some(0.0);
    }
    let busy = b.busy().saturating_sub(a.busy());
    Some(busy as f64 * 100.0 / dt as f64)
}

fn main() {
    let args = Args::parse();
    if args.version {
        println!("{}", version_string("tload"));
        return;
    }
    if cfg!(windows) && matches!(platform::loadavg(), Ok(None)) {
        eprintln!("tload: [PLATFORM:WINDOWS] no load average; graphing a value derived from CPU usage instead");
    }

    let width = 60usize;
    // scale: load represented per cell. When unspecified, auto-scale by the maximum value.
    let fixed_scale = args.scale;
    let mut max = 1.0f64;
    loop {
        let v = sample_value();
        if v > max {
            max = v;
        }
        let per_cell = fixed_scale.unwrap_or(max / width as f64).max(f64::MIN_POSITIVE);
        let bars = ((v / per_cell).round() as usize).min(width);
        print!("{v:6.2} ");
        let mut line = String::new();
        for i in 0..width {
            line.push(if i < bars { '*' } else { ' ' });
        }
        println!("|{line}|");
        std::io::stdout().flush().ok();
        std::thread::sleep(Duration::from_secs(args.delay.max(1)));
    }
}
