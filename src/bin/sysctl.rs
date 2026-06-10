//! sysctl — read and write kernel parameters. Corresponds to procps-v4.0.6/src/sysctl.c
//!
//! [PLATFORM:LINUX] accessed through the /proc/sys file tree (key a.b.c ↔ /proc/sys/a/b/c).
//! [PLATFORM:WINDOWS] / [PLATFORM:MACOS] no equivalent unified kernel-parameter file tree → print an unsupported message.
//! CLI flags are provided on all three platforms (--help matches the original).

use clap::Parser;
use procps::common::version_string;

#[derive(Parser)]
#[command(
    name = "sysctl",
    about = "Read and write kernel parameters (Linux /proc/sys)",
    disable_version_flag = true
)]
struct Args {
    #[arg(short = 'V', long)]
    version: bool,
    /// Show all variables
    #[arg(short = 'a', long, visible_short_alias = 'A')]
    all: bool,
    /// Include deprecated parameters when listing
    #[arg(long)]
    deprecated: bool,
    /// Print key/value but do not write
    #[arg(long = "dry-run")]
    dry_run: bool,
    /// Print the value without a trailing newline
    #[arg(short = 'b', long)]
    binary: bool,
    /// Ignore errors about unknown variables
    #[arg(short = 'e', long)]
    ignore: bool,
    /// Print variable names only
    #[arg(short = 'N', long = "names")]
    names: bool,
    /// Print variable values only
    #[arg(short = 'n', long = "values")]
    values: bool,
    /// Load settings from a file (optional path)
    #[arg(short = 'p', long = "load", value_name = "FILE", num_args = 0..=1, default_missing_value = "/etc/sysctl.conf")]
    load: Option<String>,
    /// Load from all system directories
    #[arg(long)]
    system: bool,
    /// Select only settings matching the regular expression
    #[arg(short = 'r', long = "pattern", value_name = "EXPR")]
    pattern: Option<String>,
    /// Do not echo the variables that are set
    #[arg(short = 'q', long)]
    quiet: bool,
    /// Enable writing
    #[arg(short = 'w', long)]
    write: bool,
    /// BSD compatibility (no effect)
    #[arg(short = 'o', short_alias = 'x')]
    _bsd_compat: bool,
    /// Kernel parameters to read or write, key or key=value (one or more)
    variables: Vec<String>,
}

fn main() {
    let args = Args::parse();
    if args.version {
        println!("{}", version_string("sysctl"));
        return;
    }
    #[cfg(target_os = "linux")]
    imp::run(&args);
    #[cfg(not(target_os = "linux"))]
    {
        let _ = &args;
        eprintln!(
            "sysctl: this tool is fully functional only on Linux (the /proc/sys kernel-parameter tree).\n\
             Current platform: {}.\n\
             [PLATFORM:WINDOWS] Windows kernel settings are spread across the registry and various APIs, with no equivalent unified key/value tree.\n\
             [PORT:MACOS] macOS has a native sysctl(8) with key names that differ from Linux; this could be bridged later.",
            std::env::consts::OS
        );
        std::process::exit(1);
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use super::Args;
    use regex::Regex;
    use std::fs;
    use std::path::{Path, PathBuf};

    fn key_to_path(key: &str) -> PathBuf {
        Path::new("/proc/sys").join(key.replace('.', "/"))
    }
    fn path_to_key(path: &Path) -> String {
        path.strip_prefix("/proc/sys")
            .unwrap_or(path)
            .to_string_lossy()
            .trim_start_matches('/')
            .replace('/', ".")
    }

    fn read_one(args: &Args, key: &str) {
        match fs::read_to_string(key_to_path(key)) {
            Ok(v) => print_kv(args, key, v.trim_end()),
            Err(e) => {
                if !args.ignore {
                    eprintln!("sysctl: cannot read {key}: {e}");
                }
            }
        }
    }

    fn print_kv(args: &Args, key: &str, val: &str) {
        if args.names {
            println!("{key}");
        } else if args.values {
            if args.binary { print!("{val}"); } else { println!("{val}"); }
        } else if args.binary {
            print!("{val}");
        } else {
            println!("{key} = {val}");
        }
    }

    fn write_one(args: &Args, key: &str, val: &str) {
        if args.dry_run {
            if !args.quiet { println!("{key} = {val}"); }
            return;
        }
        match fs::write(key_to_path(key), val) {
            Ok(()) => if !args.quiet { print_kv(args, key, val); },
            Err(e) => eprintln!("sysctl: cannot set {key}: {e}"),
        }
    }

    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        if let Ok(entries) = fs::read_dir(dir) {
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() { walk(&p, out); } else { out.push(p); }
            }
        }
    }

    fn list_all(args: &Args) {
        let re = args.pattern.as_ref().and_then(|p| Regex::new(p).ok());
        let mut files = Vec::new();
        walk(Path::new("/proc/sys"), &mut files);
        files.sort();
        for f in files {
            let key = path_to_key(&f);
            if let Some(re) = &re && !re.is_match(&key) { continue; }
            if let Ok(v) = fs::read_to_string(&f) {
                let t = v.trim_end();
                if !t.contains('\n') { print_kv(args, &key, t); }
            }
        }
    }

    fn load_file(args: &Args, path: &str) {
        let text = match fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) => { eprintln!("sysctl: cannot read {path}: {e}"); return; }
        };
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') { continue; }
            if let Some((k, v)) = line.split_once('=') {
                write_one(args, k.trim(), v.trim());
            }
        }
    }

    pub fn run(args: &Args) {
        if let Some(path) = &args.load {
            load_file(args, path);
            return;
        }
        if args.system {
            for dir in ["/run/sysctl.d", "/etc/sysctl.d", "/etc/sysctl.conf"] {
                if Path::new(dir).is_file() {
                    load_file(args, dir);
                }
            }
            return;
        }
        if args.all {
            list_all(args);
            return;
        }
        if args.variables.is_empty() {
            eprintln!("Usage: sysctl [options] [-w] key[=value] ...");
            std::process::exit(1);
        }
        for v in &args.variables {
            if let Some((k, val)) = v.split_once('=') {
                write_one(args, k, val);
            } else {
                read_one(args, v);
            }
        }
    }
}
