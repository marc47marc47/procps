//! pwdx — print the current working directory of a process. Corresponds to procps-v4.0.6/src/pwdx.c
//!
//! Cross-platform:
//! - Linux: readlink /proc/[pid]/cwd
//! - Windows: read CurrentDirectory from the target process PEB (requires privileges)
//! - macOS: proc_pidinfo (TODO)

use procps::common::version_string;
use procps::platform;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("Usage: pwdx PID...");
        std::process::exit(2);
    }
    if args[0] == "--help" || args[0] == "-h" {
        eprintln!("Usage: pwdx PID...");
        return;
    }
    if args[0] == "-V" || args[0] == "--version" {
        println!("{}", version_string("pwdx"));
        return;
    }

    let mut rc = 0;
    for arg in args {
        let pid: u32 = match arg.trim_start_matches('/').parse() {
            Ok(p) => p,
            Err(_) => {
                eprintln!("pwdx: invalid PID: {arg}");
                rc = 1;
                continue;
            }
        };
        match platform::process_cwd(pid) {
            Ok(path) => println!("{pid}: {}", path.display()),
            Err(e) => {
                eprintln!("pwdx: {pid}: {e}");
                rc = 1;
            }
        }
    }
    std::process::exit(rc);
}
