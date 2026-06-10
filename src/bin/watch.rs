//! watch — run a command periodically and display its output full-screen. Corresponds to procps-v4.0.6/src/watch.c
//!
//! Cross-platform terminal handling uses crossterm (works with both Windows Console and ANSI).
//! [PLATFORM:WINDOWS] The command runs via `cmd /C <command>` (the equivalent of Unix `sh -c`);
//! the rest of the behavior (-n/-d/-t/-x/-e/-g/-c/-b) is identical across the three platforms.

use std::io::{Write, stdout};
use std::process::Command;
use std::time::{Duration, Instant};

use clap::Parser;
use crossterm::{
    cursor,
    style::{Color, Print, ResetColor, SetForegroundColor},
    terminal::{self, Clear, ClearType},
    QueueableCommand,
};

#[derive(Parser)]
#[command(
    name = "watch",
    about = "Run a command periodically and display its output",
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
    /// Update interval in seconds (default 2.0)
    #[arg(short = 'n', long, value_name = "SECS", default_value_t = 2.0)]
    interval: f64,
    /// Highlight differences from the previous output
    #[arg(short = 'd', long)]
    differences: bool,
    /// Do not show the title line
    #[arg(short = 't', long)]
    no_title: bool,
    /// Do not run a shell; run the command directly from the argument list
    #[arg(short = 'x', long)]
    exec: bool,
    /// Exit when the command returns non-zero
    #[arg(short = 'e', long = "errexit")]
    errexit: bool,
    /// Exit when the output changes
    #[arg(short = 'g', long = "chgexit")]
    chgexit: bool,
    /// Exit when the output is unchanged for N consecutive cycles
    #[arg(short = 'q', long = "equexit", value_name = "CYCLES")]
    equexit: Option<u64>,
    /// Interpret ANSI color/style sequences
    #[arg(short = 'c', long)]
    color: bool,
    /// Do not interpret ANSI color/style sequences
    #[arg(short = 'C', long = "no-color")]
    no_color: bool,
    /// Follow output without clearing the screen
    #[arg(short = 'f', long)]
    follow: bool,
    /// Do not rerun when the window size changes
    #[arg(short = 'r', long = "no-rerun")]
    no_rerun: bool,
    /// Directory to save screenshots (accepted, currently a no-op)
    #[arg(short = 's', long = "shotsdir", value_name = "DIR")]
    shotsdir: Option<String>,
    /// Turn off line wrapping
    #[arg(short = 'w', long = "no-wrap")]
    no_wrap: bool,
    /// Beep when the output changes
    #[arg(short = 'b', long)]
    beep: bool,
    /// Precise timing (subtract the command's runtime)
    #[arg(short = 'p', long)]
    precise: bool,
    /// The command and arguments to run
    #[arg(trailing_var_arg = true)]
    command: Vec<String>,
}

fn run_command(args: &Args) -> (String, bool) {
    let output = if args.exec {
        // Run directly: command[0] is the program, the rest are arguments
        let mut c = Command::new(&args.command[0]);
        c.args(&args.command[1..]);
        c.output()
    } else {
        // Run the whole command string through a shell
        let joined = args.command.join(" ");
        if cfg!(windows) {
            // [PLATFORM:WINDOWS] use cmd /C (equivalent to Unix sh -c)
            Command::new("cmd").arg("/C").arg(&joined).output()
        } else {
            Command::new("sh").arg("-c").arg(&joined).output()
        }
    };

    match output {
        Ok(o) => {
            let mut s = String::from_utf8_lossy(&o.stdout).into_owned();
            if !o.stderr.is_empty() {
                s.push_str(&String::from_utf8_lossy(&o.stderr));
            }
            (s, o.status.success())
        }
        Err(e) => (format!("watch: failed to run command: {e}"), false),
    }
}

/// Simple per-character difference highlighting (watch -d): characters that differ from the previous output are shown in red.
fn render(
    out: &mut impl Write,
    content: &str,
    prev: Option<&str>,
    differences: bool,
    use_color: bool,
) -> std::io::Result<()> {
    let (cols, rows) = terminal::size().unwrap_or((80, 24));
    let prev_lines: Vec<&str> = prev.map(|p| p.lines().collect()).unwrap_or_default();

    for (li, line) in content.lines().enumerate() {
        if li as u16 >= rows.saturating_sub(2) {
            break;
        }
        let prev_line = prev_lines.get(li).copied().unwrap_or("");
        let mut col = 0u16;
        for (ci, ch) in line.chars().enumerate() {
            if col >= cols {
                break;
            }
            let changed = differences && prev_line.chars().nth(ci) != Some(ch);
            if changed {
                out.queue(SetForegroundColor(Color::Red))?;
                out.queue(Print(ch))?;
                out.queue(ResetColor)?;
            } else if use_color {
                // Output directly (keep the original ANSI; not filtered here, the terminal interprets it)
                out.queue(Print(ch))?;
            } else {
                out.queue(Print(ch))?;
            }
            col += 1;
        }
        out.queue(Print("\r\n"))?;
    }
    Ok(())
}

fn main() {
    let args = Args::parse();
    if args.version {
        println!("{}", procps::common::version_string("watch"));
        return;
    }
    if args.command.is_empty() {
        eprintln!("Usage: watch [options] command...");
        std::process::exit(1);
    }
    let interval = args.interval.max(0.1);
    // The following flags are included for CLI parity; their behavior is a no-op or covered by render's existing truncation
    let _ = (&args.no_rerun, &args.shotsdir, &args.no_wrap);
    // -C/--no-color takes precedence over -c/--color
    let use_color = args.color && !args.no_color;
    let mut out = stdout();

    let mut prev: Option<String> = None;
    let mut unchanged_streak = 0u64;
    let title_cmd = args.command.join(" ");

    loop {
        let start = Instant::now();
        let (content, ok) = run_command(&args);

        // -f/--follow: do not clear the screen, just keep printing
        if !args.follow {
            out.queue(Clear(ClearType::All)).ok();
            out.queue(cursor::MoveTo(0, 0)).ok();
        }

        if !args.no_title {
            let (cols, _) = terminal::size().unwrap_or((80, 24));
            let left = format!("Every {interval:.1}s: {title_cmd}");
            let right = "procps-rs watch";
            let pad = (cols as usize)
                .saturating_sub(left.len())
                .saturating_sub(right.len());
            out.queue(Print(format!("{left}{}{right}\r\n\r\n", " ".repeat(pad))))
                .ok();
        }

        render(&mut out, &content, prev.as_deref(), args.differences, use_color).ok();
        out.flush().ok();

        let changed = prev.as_deref() != Some(content.as_str());
        if args.beep && changed && prev.is_some() {
            print!("\x07");
            out.flush().ok();
        }
        if args.chgexit && changed && prev.is_some() {
            break;
        }
        // -q/--equexit: exit when the output is unchanged for N consecutive cycles
        if let Some(n) = args.equexit {
            if changed || prev.is_none() {
                unchanged_streak = 0;
            } else {
                unchanged_streak += 1;
                if unchanged_streak >= n {
                    break;
                }
            }
        }
        if args.errexit && !ok {
            // Wait for a key press before leaving (simplified to match watch's behavior: exit directly)
            eprintln!("\nwatch: command returned non-zero, exiting");
            std::process::exit(1);
        }
        prev = Some(content);

        let elapsed = start.elapsed();
        let wait = if args.precise {
            Duration::from_secs_f64(interval).saturating_sub(elapsed)
        } else {
            Duration::from_secs_f64(interval)
        };
        std::thread::sleep(wait);
    }
}
