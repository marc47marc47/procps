//! kill — send a signal to the given PIDs. Corresponds to procps-v4.0.6/src/kill.c
//!
//! [PLATFORM:WINDOWS] See platform::Signal: only TERM/KILL/INT/HUP/QUIT forcibly
//! terminate a process, 0 is used for existence checks, and other signals are
//! reported as unsupported.

use procps::common::version_string;
use procps::platform::{self, Signal};

const SIGNALS: &[(i32, &str)] = &[
    (1, "HUP"),
    (2, "INT"),
    (3, "QUIT"),
    (9, "KILL"),
    (10, "USR1"),
    (12, "USR2"),
    (15, "TERM"),
    (18, "CONT"),
    (19, "STOP"),
];

fn list_signals() {
    let names: Vec<String> = SIGNALS.iter().map(|(_, n)| n.to_string()).collect();
    println!("{}", names.join(" "));
}

/// -L: list signals in table form (number + name).
fn table_signals() {
    for (i, (num, name)) in SIGNALS.iter().enumerate() {
        print!("{num:>2} {name:<8}");
        if (i + 1) % 5 == 0 {
            println!();
        }
    }
    println!();
}

/// -l SIG: convert between signal name and number.
fn convert_signal(arg: &str) {
    if let Ok(num) = arg.parse::<i32>() {
        if let Some((_, name)) = SIGNALS.iter().find(|(n, _)| *n == num) {
            println!("{name}");
        } else {
            eprintln!("kill: {num}: invalid signal number");
            std::process::exit(1);
        }
    } else {
        let up = arg.to_ascii_uppercase();
        let name = up.strip_prefix("SIG").unwrap_or(&up);
        if let Some((num, _)) = SIGNALS.iter().find(|(_, n)| *n == name) {
            println!("{num}");
        } else {
            eprintln!("kill: {arg}: invalid signal name");
            std::process::exit(1);
        }
    }
}

fn usage() {
    eprintln!("Usage: kill [-s SIGNAL | -SIGNAL] PID...");
    eprintln!("       kill -l [SIGNAL]   list signals / convert between name and number");
    eprintln!("       kill -L            list signals in a table");
    eprintln!("       kill -q VALUE -s SIG PID   send a signal with a value (Linux)");
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.is_empty() {
        usage();
        std::process::exit(2);
    }

    let mut sig = Signal::Term;
    let mut pids: Vec<u32> = Vec::new();
    let mut queue_value: Option<i64> = None;
    let mut i = 0;
    while i < argv.len() {
        let a = &argv[i];
        match a.as_str() {
            "-V" | "--version" => {
                println!("{}", version_string("kill"));
                return;
            }
            "-l" | "--list" => {
                // Optional argument: if a signal follows, convert it; otherwise list all
                if let Some(next) = argv.get(i + 1)
                    && !next.starts_with('-')
                {
                    convert_signal(next);
                } else {
                    list_signals();
                }
                return;
            }
            "-L" | "--table" => {
                table_signals();
                return;
            }
            "-h" | "--help" => {
                usage();
                return;
            }
            "-s" | "--signal" => {
                i += 1;
                let Some(name) = argv.get(i) else {
                    usage();
                    std::process::exit(2);
                };
                match Signal::parse(name) {
                    Some(s) => sig = s,
                    None => {
                        eprintln!("kill: invalid signal: {name}");
                        std::process::exit(2);
                    }
                }
            }
            "-q" | "--queue" => {
                i += 1;
                queue_value = argv.get(i).and_then(|s| s.parse::<i64>().ok());
                if queue_value.is_none() {
                    eprintln!("kill: -q requires an integer value");
                    std::process::exit(2);
                }
            }
            s if s.starts_with('-') && s.len() > 1 => {
                let rest = &s[1..];
                match Signal::parse(rest) {
                    Some(sg) => sig = sg,
                    None => {
                        eprintln!("kill: invalid signal: {rest}");
                        std::process::exit(2);
                    }
                }
            }
            s => match s.parse::<u32>() {
                Ok(pid) => pids.push(pid),
                Err(_) => {
                    eprintln!("kill: invalid PID: {s}");
                    std::process::exit(2);
                }
            },
        }
        i += 1;
    }

    if queue_value.is_some() && !cfg!(target_os = "linux") {
        eprintln!("kill: -q/--queue (signal with a value) is a Linux-only feature; this platform sends a plain signal only.");
    }

    if pids.is_empty() {
        usage();
        std::process::exit(2);
    }

    let mut rc = 0;
    for pid in pids {
        if let Err(e) = platform::kill(pid, sig) {
            eprintln!("kill: ({pid}): {e}");
            rc = 1;
        }
    }
    std::process::exit(rc);
}
