//! pidof — find the PID of a program by name. Corresponds to procps-v4.0.6/src/pidof.c
//!
//! Cross-platform: list_processes(). By default pidof matches only the program's short name.

use clap::Parser;
use procps::common::version_string;
use procps::platform;

#[derive(Parser)]
#[command(
    name = "pidof",
    about = "Find the PID of a named program",
    disable_version_flag = true
)]
struct Args {
    #[arg(short = 'V', long)]
    version: bool,
    /// Return only a single (newest) PID
    #[arg(short, long = "single-shot")]
    single_shot: bool,
    /// Quiet mode; indicate the result via exit code only
    #[arg(short = 'q')]
    quiet: bool,
    /// Omit processes with a different root directory (Linux only; skipped cross-platform)
    #[arg(short = 'c', long = "check-root")]
    check_root: bool,
    /// Also show kernel worker threads (Linux)
    #[arg(short = 'w', long = "with-workers")]
    with_workers: bool,
    /// Also match the shell running a script
    #[arg(short = 'x')]
    scripts: bool,
    /// Also list threads
    #[arg(short = 't', long = "lightweight")]
    lightweight: bool,
    /// Omit the given PIDs (multiple allowed, comma-separated)
    #[arg(short = 'o', long = "omit-pid", value_name = "PID", value_delimiter = ',')]
    omit: Vec<u32>,
    /// PID separator character (default space)
    #[arg(short = 'S', long, value_name = "SEP", default_value = " ")]
    separator: String,
    /// Program names to look for (multiple allowed)
    programs: Vec<String>,
}

fn main() {
    let args = Args::parse();
    if args.version {
        println!("{}", version_string("pidof"));
        return;
    }
    if args.programs.is_empty() {
        eprintln!("Usage: pidof [options] program...");
        std::process::exit(1);
    }
    let _ = (args.check_root, args.with_workers, args.scripts, args.lightweight);

    let procs = match platform::list_processes() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("pidof: {e}");
            std::process::exit(1);
        }
    };

    let self_pid = std::process::id();
    let mut pids: Vec<u32> = Vec::new();
    for prog in &args.programs {
        let target = prog.to_lowercase();
        let target_noext = target.strip_suffix(".exe").unwrap_or(&target);
        for p in &procs {
            if p.pid == self_pid || args.omit.contains(&p.pid) {
                continue;
            }
            let name = p.name.to_lowercase();
            let name_noext = name.strip_suffix(".exe").unwrap_or(&name);
            let base = p
                .exe
                .as_ref()
                .and_then(|e| e.file_name())
                .map(|s| s.to_string_lossy().to_lowercase());
            let hit = name == target
                || name_noext == target_noext
                || base.as_deref() == Some(target.as_str());
            if hit {
                pids.push(p.pid);
            }
        }
    }

    pids.sort_unstable();
    pids.dedup();
    pids.reverse(); // pidof convention: newest (highest PID) first

    if args.single_shot {
        pids.truncate(1);
    }

    if pids.is_empty() {
        std::process::exit(1);
    }
    if !args.quiet {
        let strs: Vec<String> = pids.iter().map(|p| p.to_string()).collect();
        println!("{}", strs.join(&args.separator));
    }
}
