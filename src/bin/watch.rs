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
//!
//! Unless -f/--follow is given the display lives in the alternate screen and is repainted in
//! place (move-to + erase-to-end-of-line per row) rather than cleared wholesale. Nothing is ever
//! written past the last row and no newline follows the last line, so the viewport never scrolls
//! and old frames cannot stack up.
//!
//! stdout and stderr are kept apart. Upstream watch(1) merges them, which lets a command that
//! chatters on stderr (`du` on unreadable directories, for one) push the real output around from
//! cycle to cycle. Here stderr gets its own pane pinned to the bottom of the screen, sized to the
//! error text and capped at a third of the window; stdout keeps the rest.

use std::io::{BufWriter, Write, stdout};
use std::process::Command;
use std::time::{Duration, Instant};

use clap::Parser;
use crossterm::{
    QueueableCommand, cursor,
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    style::{Color, Print, ResetColor, SetForegroundColor},
    terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
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

/// Run the command once. stdout and stderr are returned separately so the renderer can keep them
/// in separate panes; the bool is the exit status for -e/--errexit.
fn run_command(command: &[String], shell: &Shell) -> (String, String, bool) {
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
        Ok(o) => (
            String::from_utf8_lossy(&o.stdout).into_owned(),
            String::from_utf8_lossy(&o.stderr).into_owned(),
            o.status.success(),
        ),
        Err(e) => (
            String::new(),
            format!("watch: failed to run command: {e}"),
            false,
        ),
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

/// Switches to the alternate screen buffer and hides the cursor, restoring both on release.
/// Painting into the alternate buffer keeps the user's scrollback intact, and combined with the
/// bounded repaint in `draw_frame` it means a frame can never scroll the viewport.
///
/// `release` is idempotent and must be called explicitly on any path that reaches
/// `std::process::exit`, which does not run destructors.
struct ScreenGuard {
    active: bool,
}

impl ScreenGuard {
    fn enter() -> Self {
        let mut out = stdout();
        let ok = out.queue(EnterAlternateScreen).is_ok()
            && out.queue(cursor::Hide).is_ok()
            && out.flush().is_ok();
        ScreenGuard { active: ok }
    }
    /// -f/--follow keeps the normal screen: a guard that owns nothing.
    fn disabled() -> Self {
        ScreenGuard { active: false }
    }
    fn release(&mut self) {
        if self.active {
            let mut out = stdout();
            let _ = out.queue(cursor::Show);
            let _ = out.queue(LeaveAlternateScreen);
            let _ = out.flush();
            self.active = false;
        }
    }
}

impl Drop for ScreenGuard {
    fn drop(&mut self) {
        self.release();
    }
}

const QUIT_PROMPT: &str = "press q again will exit";

/// Show the quit-confirmation prompt on the bottom line of the terminal. It overwrites whatever
/// occupies that row; the next repaint restores it.
fn draw_quit_prompt(out: &mut impl Write) {
    let (_, rows) = terminal::size().unwrap_or((80, 24));
    out.queue(cursor::MoveTo(0, rows.saturating_sub(1))).ok();
    out.queue(Clear(ClearType::CurrentLine)).ok();
    out.queue(SetForegroundColor(Color::Red)).ok();
    out.queue(Print(QUIT_PROMPT)).ok();
    out.queue(ResetColor).ok();
    out.flush().ok();
}

/// Erase the quit-confirmation prompt from the bottom line.
fn clear_quit_prompt(out: &mut impl Write) {
    let (_, rows) = terminal::size().unwrap_or((80, 24));
    out.queue(cursor::MoveTo(0, rows.saturating_sub(1))).ok();
    out.queue(Clear(ClearType::CurrentLine)).ok();
    out.flush().ok();
}

/// Why the wait between cycles ended.
enum Wake {
    /// The interval elapsed: run the command again.
    Timeout,
    /// The window changed size: repaint now rather than leaving a half-drawn frame up.
    Resize,
    /// The user asked to leave.
    Quit,
}

/// Sleep until the next cycle while watching the keyboard. Quitting takes two presses of q:
/// the first sets `pending` and shows the confirmation prompt, the second returns `Quit`; any
/// other key cancels. Ctrl+C exits immediately (raw mode intercepts it as a key event, so it
/// must be handled here). `pending` survives across refresh cycles so the confirmation isn't
/// lost when the screen redraws mid-wait.
///
/// A resize cuts the wait short unless `rerun_on_resize` is false (-r/--no-rerun), in which case
/// the event is swallowed and the current frame stays up until the interval elapses.
fn wait_or_quit(
    dur: Duration,
    pending: &mut bool,
    out: &mut impl Write,
    rerun_on_resize: bool,
) -> Wake {
    let deadline = Instant::now() + dur;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Wake::Timeout;
        }
        match event::poll(remaining) {
            Ok(true) => match event::read() {
                Ok(Event::Key(k)) => {
                    // [PLATFORM:WINDOWS] key-release events also arrive; only act on presses
                    if k.kind != KeyEventKind::Press {
                        continue;
                    }
                    match k.code {
                        KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                            return Wake::Quit;
                        }
                        KeyCode::Char('q') | KeyCode::Char('Q') => {
                            if *pending {
                                return Wake::Quit;
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
                Ok(Event::Resize(_, _)) if rerun_on_resize => return Wake::Resize,
                Ok(_) => continue,
                Err(_) => return Wake::Timeout,
            },
            Ok(false) => return Wake::Timeout,
            // stdin is not a terminal (e.g. redirected): fall back to a plain sleep
            Err(_) => {
                std::thread::sleep(remaining);
                return Wake::Timeout;
            }
        }
    }
}

/// Write `text` at the start of `row`, clipped to `cols` columns, erasing the rest of the line.
/// Nothing is written past the last column, so the cursor never wraps onto the next row.
fn draw_row(out: &mut impl Write, row: u16, cols: u16, text: &str) -> std::io::Result<()> {
    out.queue(cursor::MoveTo(0, row))?;
    out.queue(Clear(ClearType::UntilNewLine))?;
    let clipped: String = text.chars().take(cols as usize).collect();
    out.queue(Print(clipped))?;
    Ok(())
}

/// Erase rows `from..to` (exclusive) left over by a taller previous frame.
fn clear_rows(out: &mut impl Write, from: u16, to: u16) -> std::io::Result<()> {
    for row in from..to {
        out.queue(cursor::MoveTo(0, row))?;
        out.queue(Clear(ClearType::UntilNewLine))?;
    }
    Ok(())
}

/// The `Every 2.0s: <cmd>` header, right-aligning the brand when it fits. `draw_row` clips it to
/// the window, so a long command truncates instead of wrapping onto a second row and pushing the
/// body down.
fn title_line(interval: f64, cmd: &str, cols: u16) -> String {
    let left = format!("Every {interval:.1}s: {cmd}");
    let right = "procps watch";
    let width = cols as usize;
    let used = left.chars().count() + right.len();
    if used < width {
        format!("{left}{}{right}", " ".repeat(width - used))
    } else {
        left
    }
}

/// Paint the stdout body into rows `top..limit`, per-character difference highlighting (watch -d)
/// showing characters that differ from the previous output in red. Lines beyond the region and
/// columns beyond the window are dropped; unused rows in the region are erased.
fn draw_body(
    out: &mut impl Write,
    content: &str,
    prev: Option<&str>,
    differences: bool,
    cols: u16,
    top: u16,
    limit: u16,
) -> std::io::Result<()> {
    let prev_lines: Vec<&str> = prev.map(|p| p.lines().collect()).unwrap_or_default();
    let mut row = top;
    for (li, line) in content.lines().enumerate() {
        if row >= limit {
            break;
        }
        out.queue(cursor::MoveTo(0, row))?;
        out.queue(Clear(ClearType::UntilNewLine))?;
        let mut prev_chars = prev_lines.get(li).copied().unwrap_or("").chars();
        for (col, ch) in line.chars().enumerate() {
            if col as u16 >= cols {
                break;
            }
            // `prev_chars` walks in lockstep with `line`; short-circuiting when -d is off just
            // means we never consume it.
            if differences && prev_chars.next() != Some(ch) {
                out.queue(SetForegroundColor(Color::Red))?;
                out.queue(Print(ch))?;
                out.queue(ResetColor)?;
            } else {
                out.queue(Print(ch))?;
            }
        }
        row += 1;
    }
    clear_rows(out, row, limit)
}

/// Height of the stderr pane (separator + error lines) for `err_lines` lines of stderr, or `None`
/// when stderr is empty or the window is too short to spare the rows. Capped at a third of the
/// window so a flood of errors can never crowd out the command's real output.
fn stderr_pane_height(err_lines: usize, rows: u16) -> Option<u16> {
    if err_lines == 0 {
        return None;
    }
    let cap = (rows / 3).max(1);
    let height = (err_lines as u16).min(cap) + 1; // + separator
    // Leave at least one row for the body.
    (height < rows).then_some(height)
}

/// Paint the stderr pane across the bottom `height` rows: a red rule naming the line count, then
/// the tail of stderr (the newest lines, which is what a command still emitting errors is saying
/// now).
fn draw_stderr(
    out: &mut impl Write,
    err_lines: &[&str],
    cols: u16,
    rows: u16,
    height: u16,
) -> std::io::Result<()> {
    let top = rows - height;
    let shown = (height - 1) as usize;
    let hidden = err_lines.len().saturating_sub(shown);

    let label = if hidden > 0 {
        format!("stderr ({} lines, {hidden} not shown) ", err_lines.len())
    } else {
        format!("stderr ({} lines) ", err_lines.len())
    };
    let rule = format!("── {label}");
    let fill = (cols as usize).saturating_sub(rule.chars().count());

    out.queue(SetForegroundColor(Color::Red))?;
    draw_row(out, top, cols, &format!("{rule}{}", "─".repeat(fill)))?;
    for (i, line) in err_lines[hidden..].iter().enumerate() {
        draw_row(out, top + 1 + i as u16, cols, line)?;
    }
    out.queue(ResetColor)?;
    Ok(())
}

/// Repaint the whole window: title, then stdout in the rows above the stderr pane, then the pane
/// itself. Every row is addressed absolutely and nothing is written below the last one, so the
/// frame always lands inside the viewport no matter how much the command printed.
fn draw_frame(
    out: &mut impl Write,
    args: &Args,
    interval: f64,
    title_cmd: &str,
    content: &str,
    stderr: &str,
    prev: Option<&str>,
) -> std::io::Result<()> {
    let (cols, rows) = terminal::size().unwrap_or((80, 24));
    if cols == 0 || rows == 0 {
        return Ok(());
    }

    let err_lines: Vec<&str> = stderr.lines().collect();
    let pane = stderr_pane_height(err_lines.len(), rows);
    let body_limit = rows - pane.unwrap_or(0);

    let mut top = 0u16;
    if !args.no_title {
        draw_row(out, 0, cols, &title_line(interval, title_cmd, cols))?;
        draw_row(out, 1, cols, "")?;
        top = 2;
    }
    if top < body_limit {
        draw_body(out, content, prev, args.differences, cols, top, body_limit)?;
    }
    if let Some(height) = pane {
        draw_stderr(out, &err_lines, cols, rows, height)?;
    }
    out.flush()
}

/// -f/--follow: keep the normal screen and stream each cycle's output, tagging stderr so it stays
/// distinguishable without a pane to put it in. -d still highlights, as it did before stderr got
/// its own pane.
fn draw_follow(
    out: &mut impl Write,
    content: &str,
    stderr: &str,
    prev: Option<&str>,
    differences: bool,
) -> std::io::Result<()> {
    let prev_lines: Vec<&str> = prev.map(|p| p.lines().collect()).unwrap_or_default();
    for (li, line) in content.lines().enumerate() {
        let mut prev_chars = prev_lines.get(li).copied().unwrap_or("").chars();
        for ch in line.chars() {
            if differences && prev_chars.next() != Some(ch) {
                out.queue(SetForegroundColor(Color::Red))?;
                out.queue(Print(ch))?;
                out.queue(ResetColor)?;
            } else {
                out.queue(Print(ch))?;
            }
        }
        out.queue(Print("\r\n"))?;
    }
    if !stderr.is_empty() {
        out.queue(SetForegroundColor(Color::Red))?;
        for line in stderr.lines() {
            out.queue(Print(format!("stderr: {line}\r\n")))?;
        }
        out.queue(ResetColor)?;
    }
    out.flush()
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
    // Accepted for CLI parity: -s/--shotsdir is a no-op, and -w/--no-wrap is implicit because
    // draw_row clips every line to the window width. -c/-C likewise: ANSI sequences in the
    // command's output reach the terminal either way.
    let _ = (&args.shotsdir, &args.no_wrap, &args.color, &args.no_color);
    // Buffer the frame: `Stdout` is line-buffered, which would flush a syscall per row and make
    // the repaint visibly tear.
    let mut out = BufWriter::new(stdout());

    let mut prev: Option<String> = None;
    let mut unchanged_streak = 0u64;
    let title_cmd = args.command.join(" ");
    let shell = resolve_shell(args.exec);
    // Raw mode lets us react to a bare `q` key press; restored on exit via Drop
    let mut raw = RawGuard::new();
    // -f/--follow streams into the normal screen; otherwise repaint in the alternate buffer
    let mut screen = if args.follow {
        ScreenGuard::disabled()
    } else {
        ScreenGuard::enter()
    };
    // True after the first q press, awaiting confirmation with a second q
    let mut quit_pending = false;

    loop {
        let start = Instant::now();
        let (content, stderr, ok) = run_command(&args.command, &shell);

        if args.follow {
            draw_follow(
                &mut out,
                &content,
                &stderr,
                prev.as_deref(),
                args.differences,
            )
            .ok();
        } else {
            draw_frame(
                &mut out,
                &args,
                interval,
                &title_cmd,
                &content,
                &stderr,
                prev.as_deref(),
            )
            .ok();
        }
        // The repaint wiped the confirmation prompt; restore it while a quit is pending
        if quit_pending {
            draw_quit_prompt(&mut out);
        }

        let changed = prev.as_deref() != Some(content.as_str());
        if args.beep && changed && prev.is_some() {
            out.queue(Print("\x07")).ok();
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
            // std::process::exit skips Drop, so restore the terminal by hand before leaving, then
            // echo stderr — the pane it was in is about to disappear with the alternate screen.
            out.flush().ok();
            raw.release();
            screen.release();
            if !stderr.is_empty() {
                eprint!("{stderr}");
            }
            eprintln!("watch: command returned non-zero, exiting");
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
        match wait_or_quit(wait, &mut quit_pending, &mut out, !args.no_rerun) {
            Wake::Quit => break,
            Wake::Timeout | Wake::Resize => {}
        }
    }
    out.flush().ok();
    raw.release();
    screen.release();
}
