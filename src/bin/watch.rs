//! watch — run a command periodically and display its output full-screen. Corresponds to procps-v4.0.6/src/watch.c
//!
//! Cross-platform terminal handling uses crossterm (works with both Windows Console and ANSI).
//! [PLATFORM:WINDOWS] If a POSIX shell is available (Git Bash / MSYS / Cygwin / `$SHELL`), the
//! command runs through it via `sh -c <command>` so pipes and quoting behave the way the user's
//! shell expects; otherwise it falls back to `cmd /C <command>`. The rest of the behavior
//! (-n/-d/-t/-x/-e/-g/-c/-b) is identical across the three platforms.
//!
//! Press q twice to exit: the first press shows a confirmation prompt on the bottom line, the
//! second press quits, and any other key cancels. Ctrl+C still exits immediately. The terminal
//! runs in raw mode between refreshes so keys are picked up without Enter.

use std::io::{Write, stdout};
use std::process::Command;
use std::time::{Duration, Instant};

use clap::Parser;
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
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

/// How the watched command should be invoked, resolved once before the loop starts.
enum Shell {
    /// -x/--exec: run the argument list directly without a shell.
    Exec,
    /// Run `<prog> -c <command>` through a POSIX shell.
    Posix(String),
    /// [PLATFORM:WINDOWS] Run `cmd /C <command>` (fallback when no POSIX shell is found).
    Cmd,
}

/// Locate an executable on `PATH` (appending `.exe` on Windows). A name containing a path
/// separator is tested directly. Returns the resolved path on success.
fn find_on_path(name: &str) -> Option<String> {
    use std::path::Path;
    let exts: &[&str] = if cfg!(windows) { &["", ".exe"] } else { &[""] };
    if name.contains('/') || name.contains('\\') {
        for ext in exts {
            let p = format!("{name}{ext}");
            if Path::new(&p).is_file() {
                return Some(p);
            }
        }
        return None;
    }
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        for ext in exts {
            let cand = dir.join(format!("{name}{ext}"));
            if cand.is_file() {
                return Some(cand.to_string_lossy().into_owned());
            }
        }
    }
    None
}

/// Find a usable POSIX shell: honor `$SHELL` (by basename) first, then fall back to `sh`/`bash`.
fn posix_shell() -> Option<String> {
    let mut cands: Vec<String> = Vec::new();
    if let Some(sh) = std::env::var_os("SHELL") {
        let sh = sh.to_string_lossy();
        let base = sh.rsplit(['/', '\\']).next().unwrap_or(&sh).to_string();
        if !base.is_empty() {
            cands.push(base);
        }
    }
    cands.push("sh".into());
    cands.push("bash".into());
    cands.into_iter().find_map(|c| find_on_path(&c))
}

/// Decide how to run the command once, before entering the loop.
fn resolve_shell(exec: bool) -> Shell {
    if exec {
        return Shell::Exec;
    }
    if cfg!(windows) {
        // Prefer a POSIX shell when one is present (Git Bash / MSYS / Cygwin), so pipes and
        // quoting match the shell the user invoked watch from; otherwise use cmd.
        match posix_shell() {
            Some(prog) => Shell::Posix(prog),
            None => Shell::Cmd,
        }
    } else {
        Shell::Posix("sh".into())
    }
}

fn run_command(command: &[String], shell: &Shell) -> (String, bool) {
    let output = match shell {
        Shell::Exec => {
            // Run directly: command[0] is the program, the rest are arguments
            let mut c = Command::new(&command[0]);
            c.args(&command[1..]);
            c.output()
        }
        Shell::Posix(prog) => {
            let joined = command.join(" ");
            Command::new(prog).arg("-c").arg(&joined).output()
        }
        Shell::Cmd => {
            let joined = command.join(" ");
            Command::new("cmd").arg("/C").arg(&joined).output()
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

/// Enables raw mode for the lifetime of the value so key presses (q to quit) are readable
/// without Enter; restores the terminal on drop. Raw mode also stops the console from
/// echoing typed characters into the display.
struct RawGuard {
    active: bool,
}

impl RawGuard {
    fn new() -> Self {
        RawGuard {
            active: terminal::enable_raw_mode().is_ok(),
        }
    }
    fn release(&mut self) {
        if self.active {
            let _ = terminal::disable_raw_mode();
            self.active = false;
        }
    }
}

impl Drop for RawGuard {
    fn drop(&mut self) {
        self.release();
    }
}

const QUIT_PROMPT: &str = "press q again will exit";

/// Show the quit-confirmation prompt on the bottom line of the terminal.
fn draw_quit_prompt(out: &mut impl Write) {
    let (_, rows) = terminal::size().unwrap_or((80, 24));
    out.queue(cursor::SavePosition).ok();
    out.queue(cursor::MoveTo(0, rows.saturating_sub(1))).ok();
    out.queue(Clear(ClearType::CurrentLine)).ok();
    out.queue(SetForegroundColor(Color::Red)).ok();
    out.queue(Print(QUIT_PROMPT)).ok();
    out.queue(ResetColor).ok();
    out.queue(cursor::RestorePosition).ok();
    out.flush().ok();
}

/// Erase the quit-confirmation prompt from the bottom line.
fn clear_quit_prompt(out: &mut impl Write) {
    let (_, rows) = terminal::size().unwrap_or((80, 24));
    out.queue(cursor::SavePosition).ok();
    out.queue(cursor::MoveTo(0, rows.saturating_sub(1))).ok();
    out.queue(Clear(ClearType::CurrentLine)).ok();
    out.queue(cursor::RestorePosition).ok();
    out.flush().ok();
}

/// Sleep until the next cycle while watching the keyboard. Quitting takes two presses of q:
/// the first sets `pending` and shows the confirmation prompt, the second returns true; any
/// other key cancels. Ctrl+C exits immediately (raw mode intercepts it as a key event, so it
/// must be handled here). `pending` survives across refresh cycles so the confirmation isn't
/// lost when the screen redraws mid-wait.
fn wait_or_quit(dur: Duration, pending: &mut bool, out: &mut impl Write) -> bool {
    let deadline = Instant::now() + dur;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return false;
        }
        match event::poll(remaining) {
            Ok(true) => {
                if let Ok(Event::Key(k)) = event::read() {
                    // [PLATFORM:WINDOWS] key-release events also arrive; only act on presses
                    if k.kind != KeyEventKind::Press {
                        continue;
                    }
                    match k.code {
                        KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                            return true;
                        }
                        KeyCode::Char('q') | KeyCode::Char('Q') => {
                            if *pending {
                                return true;
                            }
                            *pending = true;
                            draw_quit_prompt(out);
                        }
                        _ => {
                            if *pending {
                                *pending = false;
                                clear_quit_prompt(out);
                            }
                        }
                    }
                }
            }
            Ok(false) => return false,
            // stdin is not a terminal (e.g. redirected): fall back to a plain sleep
            Err(_) => {
                std::thread::sleep(remaining);
                return false;
            }
        }
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
    let shell = resolve_shell(args.exec);
    // Raw mode lets us react to a bare `q` key press; restored on exit via Drop
    let mut raw = RawGuard::new();
    // True after the first q press, awaiting confirmation with a second q
    let mut quit_pending = false;

    loop {
        let start = Instant::now();
        let (content, ok) = run_command(&args.command, &shell);

        // -f/--follow: do not clear the screen, just keep printing
        if !args.follow {
            out.queue(Clear(ClearType::All)).ok();
            out.queue(cursor::MoveTo(0, 0)).ok();
        }

        if !args.no_title {
            let (cols, _) = terminal::size().unwrap_or((80, 24));
            let left = format!("Every {interval:.1}s: {title_cmd}");
            let right = "procps watch";
            let pad = (cols as usize)
                .saturating_sub(left.len())
                .saturating_sub(right.len());
            out.queue(Print(format!("{left}{}{right}\r\n\r\n", " ".repeat(pad))))
                .ok();
        }

        render(&mut out, &content, prev.as_deref(), args.differences, use_color).ok();
        out.flush().ok();
        // The redraw wiped the confirmation prompt; restore it while a quit is pending
        if quit_pending {
            draw_quit_prompt(&mut out);
        }

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
            raw.release();
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
        // q pressed twice exits watch (Ctrl+C exits immediately)
        if wait_or_quit(wait, &mut quit_pending, &mut out) {
            break;
        }
    }
    raw.release();
}
