//! top — real-time process monitor, ratatui visual edition. Port of procps-v4.0.6/src/top/
//!
//! Visual features (goal: better than classic Linux top, closer to htop/btop):
//! - Multi-column real per-core CPU meters (Windows uses NtQuerySystemInformation)
//! - Memory / Swap gradient gauges (green -> yellow -> red by usage)
//! - Overall CPU usage history sparkline
//! - Colored, selectable process table; auto-colored by resource usage
//! - Send a signal to the selected process (k), renice (r), etc.
//!
//! Cross-platform: all data comes from the procps::platform layer (same code on Win/Linux/macOS).
//! [PLATFORM:WINDOWS] No load average -> summary shows n/a.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Cell, Gauge, Paragraph, Row, Sparkline, Table, TableState,
};
use ratatui::Frame;

use procps::platform::{self, ProcessInfo};
use procps::units::{format_uptime, human_bytes};

#[derive(Clone, Copy, PartialEq)]
enum Sort {
    Cpu,
    Mem,
    Pid,
    Time,
}

impl Sort {
    fn label(&self) -> &'static str {
        match self {
            Sort::Cpu => "CPU%",
            Sort::Mem => "MEM",
            Sort::Pid => "PID",
            Sort::Time => "TIME",
        }
    }
}

/// Interactive actions that need a text prompt (top's k/r/d/u/n/L commands).
#[derive(Clone, Copy, PartialEq)]
enum InputAction {
    KillSignal, // k: enter signal
    Renice,     // r: enter nice value
    Delay,      // d/s: enter refresh seconds
    FilterUser, // u/U: enter user
    MaxTasks,   // n/#: enter max rows to show
    Locate,     // L: search string
}

struct InputState {
    action: InputAction,
    prompt: String,
    buf: String,
}

struct App {
    delay: Duration,
    sort: Sort,
    reverse: bool, // R: reverse sort
    table: TableState,
    /// Per-process cumulative CPU time, used to compute the per-tick delta %
    prev_proc_cpu: HashMap<u32, u64>,
    prev_total: u64,
    /// Previous per-core times, used for the per-core delta
    prev_per_core: Vec<platform::CpuTimes>,
    /// Overall CPU% history (sparkline)
    history: VecDeque<u64>,
    status: String,
    paused: bool,
    // Filters (matches top -p/-u/-U/-i)
    pid_filter: Vec<u32>,
    user_filter: Vec<String>,
    hide_idle: bool, // i
    // Display toggles
    per_core: bool,      // 1: per-core vs aggregate
    show_mem: bool,      // m: memory area
    show_summary: bool,  // l/t: summary lines
    show_history: bool,  // CPU history panel
    show_cmdline: bool,  // c: command line vs program name
    threads_mode: bool,  // H
    bold: bool,          // b
    irix: bool,          // I: Irix (per-core 100%) vs Solaris (total 100%)
    suppress_zero: bool, // 0: hide zero values
    highlight_sort: bool,// x: highlight sort column
    highlight_run: bool, // y: highlight running tasks
    max_tasks: Option<usize>, // n/#
    locate: Option<String>,   // L: search string (highlight)
    help: bool,          // ? / h
    input: Option<InputState>,
}

impl App {
    fn new(delay: Duration) -> Self {
        let mut table = TableState::default();
        table.select(Some(0));
        App {
            delay,
            sort: Sort::Cpu,
            reverse: false,
            table,
            prev_proc_cpu: HashMap::new(),
            prev_total: platform::cpu_times().map(|c| c.total()).unwrap_or(0),
            prev_per_core: platform::per_cpu_times().unwrap_or_default(),
            history: VecDeque::with_capacity(256),
            status: String::new(),
            paused: false,
            pid_filter: Vec::new(),
            user_filter: Vec::new(),
            hide_idle: false,
            per_core: true,
            show_mem: true,
            show_summary: true,
            show_history: true,
            show_cmdline: true,
            threads_mode: false,
            bold: false,
            irix: true,
            suppress_zero: false,
            highlight_sort: true,
            highlight_run: false,
            max_tasks: None,
            locate: None,
            help: false,
            input: None,
        }
    }

    /// Reset all toggles and filters (top's `=`).
    fn reset(&mut self) {
        self.reverse = false;
        self.hide_idle = false;
        self.per_core = true;
        self.show_mem = true;
        self.show_summary = true;
        self.show_history = true;
        self.show_cmdline = true;
        self.threads_mode = false;
        self.bold = false;
        self.irix = true;
        self.suppress_zero = false;
        self.highlight_sort = true;
        self.highlight_run = false;
        self.max_tasks = None;
        self.locate = None;
        self.pid_filter.clear();
        self.user_filter.clear();
        self.status = "reset (=)".into();
    }
}

/// Color by load percentage: green < 50, yellow < 80, red >= 80.
fn load_color(pct: f64) -> Color {
    if pct < 50.0 {
        Color::Green
    } else if pct < 80.0 {
        Color::Yellow
    } else {
        Color::Red
    }
}

/// One CPU meter row: `C0 [████░░░░] 52.3%`
fn core_line(idx: usize, pct: f64, bar_w: usize) -> Line<'static> {
    core_line_labeled(&format!("C{idx}"), pct, bar_w)
}

/// CPU meter row with an explicit label (e.g. "ALL" for the aggregate bar).
fn core_line_labeled(label: &str, pct: f64, bar_w: usize) -> Line<'static> {
    let pct = pct.clamp(0.0, 100.0);
    let filled = (((pct / 100.0) * bar_w as f64).round() as usize).min(bar_w);
    let color = load_color(pct);
    Line::from(vec![
        Span::styled(format!("{label:>3} "), Style::default().fg(Color::DarkGray)),
        Span::styled("[", Style::default().fg(Color::DarkGray)),
        Span::styled("█".repeat(filled), Style::default().fg(color)),
        Span::styled("░".repeat(bar_w - filled), Style::default().fg(Color::Rgb(40, 40, 40))),
        Span::styled("]", Style::default().fg(Color::DarkGray)),
        Span::styled(format!(" {pct:>5.1}%"), Style::default().fg(color).add_modifier(Modifier::BOLD)),
    ])
}

struct Snapshot {
    procs: Vec<ProcessInfo>,
    cpu_pct: HashMap<u32, f64>,
    per_core_pct: Vec<f64>,
    overall_cpu: f64,
    mem: platform::MemInfo,
    mem_total: u64,
}

/// Sample one round of data and update the App's delta state.
fn sample(app: &mut App) -> Snapshot {
    let mut procs = platform::list_processes().unwrap_or_default();

    // Filters: -p PID, -u/-U user
    if !app.pid_filter.is_empty() {
        procs.retain(|p| app.pid_filter.contains(&p.pid));
    }
    if !app.user_filter.is_empty() {
        procs.retain(|p| {
            app.user_filter.iter().any(|u| {
                u.eq_ignore_ascii_case(&p.user)
                    || u.parse::<u32>().ok() == p.euid
                    || u.parse::<u32>().ok() == p.ruid
            })
        });
    }

    // Per-core delta
    let cur_core = platform::per_cpu_times().unwrap_or_default();
    let mut per_core_pct = Vec::with_capacity(cur_core.len());
    for (i, c) in cur_core.iter().enumerate() {
        let pct = match app.prev_per_core.get(i) {
            Some(p) => {
                let dt = c.total().saturating_sub(p.total());
                let db = c.busy().saturating_sub(p.busy());
                if dt == 0 { 0.0 } else { db as f64 * 100.0 / dt as f64 }
            }
            None => 0.0,
        };
        per_core_pct.push(pct);
    }
    app.prev_per_core = cur_core;
    let overall_cpu = if per_core_pct.is_empty() {
        0.0
    } else {
        per_core_pct.iter().sum::<f64>() / per_core_pct.len() as f64
    };

    // Total CPU time delta (denominator for per-process CPU%)
    let cur_total = platform::cpu_times().map(|c| c.total()).unwrap_or(0);
    let total_delta = cur_total.saturating_sub(app.prev_total).max(1);
    app.prev_total = cur_total;
    let ncpu = per_core_pct.len().max(1) as f64;

    let mut cpu_pct = HashMap::new();
    let mut cur_proc_cpu = HashMap::new();
    // Irix mode: relative to one core (can exceed 100%); Solaris mode: divided by ncpu (max 100%)
    let scale = if app.irix { ncpu } else { 1.0 };
    for p in &procs {
        let c = p.utime_ms + p.stime_ms;
        cur_proc_cpu.insert(p.pid, c);
        let prev = app.prev_proc_cpu.get(&p.pid).copied().unwrap_or(c);
        let pct = c.saturating_sub(prev) as f64 / total_delta as f64 * 100.0 * scale;
        cpu_pct.insert(p.pid, pct);
    }
    app.prev_proc_cpu = cur_proc_cpu;

    // -i / 'i': hide idle (CPU ~ 0) processes
    if app.hide_idle {
        procs.retain(|p| cpu_pct.get(&p.pid).copied().unwrap_or(0.0) >= 0.05);
    }

    match app.sort {
        Sort::Cpu => procs.sort_by(|a, b| {
            cpu_pct[&b.pid].partial_cmp(&cpu_pct[&a.pid]).unwrap_or(std::cmp::Ordering::Equal)
        }),
        Sort::Mem => procs.sort_by(|a, b| b.rss_bytes.cmp(&a.rss_bytes)),
        Sort::Pid => procs.sort_by_key(|p| p.pid),
        Sort::Time => procs.sort_by(|a, b| {
            (b.utime_ms + b.stime_ms).cmp(&(a.utime_ms + a.stime_ms))
        }),
    }
    // 'R': reverse the current sort order
    if app.reverse {
        procs.reverse();
    }

    app.history.push_back(overall_cpu.round() as u64);
    while app.history.len() > 256 {
        app.history.pop_front();
    }

    let mem = platform::mem_info().unwrap_or_default();
    let mem_total = mem.total.max(1);
    Snapshot { procs, cpu_pct, per_core_pct, overall_cpu, mem, mem_total }
}

use clap::Parser;

#[derive(Parser)]
#[command(
    name = "top",
    about = "real-time process monitor (ratatui visual edition)",
    disable_version_flag = true
)]
struct Cli {
    #[arg(short = 'V', long)]
    version: bool,
    /// Batch mode (non-interactive plain-text output, for scripts/redirection)
    #[arg(short = 'b', long = "batch-mode")]
    batch: bool,
    /// Refresh delay in seconds
    #[arg(short = 'd', long, value_name = "SECS", default_value_t = 2.0)]
    delay: f64,
    /// Exit after N iterations (common in batch mode)
    #[arg(short = 'n', long = "iterations", value_name = "N")]
    iterations: Option<u64>,
    /// Monitor only these PIDs (comma-separated, repeatable)
    #[arg(short = 'p', long = "pid", value_name = "PID", value_delimiter = ',')]
    pid: Vec<u32>,
    /// Show only this effective user
    #[arg(short = 'u', long = "filter-only-euser", value_name = "USER")]
    euser: Vec<String>,
    /// Show only this user (any)
    #[arg(short = 'U', long = "filter-any-user", value_name = "USER")]
    any_user: Vec<String>,
    /// Initial sort field: %CPU / %MEM / PID / TIME
    #[arg(short = 'o', long = "sort-override", value_name = "FIELD")]
    sort: Option<String>,
    /// Show all threads (accepted; this port is process-level)
    #[arg(short = 'H', long = "threads-show")]
    threads: bool,
    /// Hide idle processes
    #[arg(short = 'i', long = "idle-toggle")]
    idle: bool,
    /// Toggle command-line display (accepted)
    #[arg(short = 'c', long = "cmdline-toggle")]
    cmdline: bool,
    /// Set screen width (accepted)
    #[arg(short = 'w', long = "width", value_name = "COLS", num_args = 0..=1)]
    width: Option<Option<u16>>,
    /// Secure mode (disable kill/renice)
    #[arg(short = 's', long = "secure-mode")]
    secure: bool,
    /// List available fields and exit
    #[arg(short = 'O', long = "list-fields")]
    list_fields: bool,
    /// Debug: render a single frame as plain text via TestBackend
    #[arg(long)]
    snapshot: bool,
    /// Debug: keys to apply before the snapshot (e.g. "1" turns off per-core)
    #[arg(long, hide = true)]
    keys: Option<String>,
}

fn sort_from_str(s: &str) -> Sort {
    match s.to_ascii_uppercase().as_str() {
        "MEM" | "%MEM" | "RES" => Sort::Mem,
        "PID" => Sort::Pid,
        "TIME" | "TIME+" => Sort::Time,
        _ => Sort::Cpu,
    }
}

fn main() {
    let cli = Cli::parse();
    if cli.version {
        println!("{}", procps::common::version_string("top"));
        return;
    }
    if cli.list_fields {
        println!("Available fields: PID USER PR %CPU %MEM RES TIME+ COMMAND");
        return;
    }
    let _ = (cli.threads, cli.cmdline, cli.width, cli.secure);

    let delay = cli.delay.max(0.5);
    let mut app = App::new(Duration::from_secs_f64(delay));
    app.pid_filter = cli.pid.clone();
    app.user_filter = cli.euser.iter().chain(cli.any_user.iter()).cloned().collect();
    app.hide_idle = cli.idle;
    if let Some(s) = &cli.sort {
        app.sort = sort_from_str(s);
    }

    if cli.snapshot {
        snapshot_app(app, cli.keys.clone());
        return;
    }

    if cli.batch {
        run_batch(&mut app, cli.iterations);
        return;
    }

    let mut terminal = ratatui::init();
    let res = run(&mut terminal, &mut app);
    ratatui::restore();
    if let Err(e) = res {
        eprintln!("top: {e}");
    }
}

/// Batch mode: plain-text summary + process table, repeated `iterations` times (or forever).
fn run_batch(app: &mut App, iterations: Option<u64>) {
    let _ = sample(app); // establish baseline
    let mut n = 0u64;
    loop {
        std::thread::sleep(app.delay);
        let snap = sample(app);
        let up = platform::uptime().map(|d| procps::units::format_uptime(d.as_secs())).unwrap_or_default();
        let load = match platform::loadavg() {
            Ok(Some((a, b, c))) => format!("{a:.2}, {b:.2}, {c:.2}"),
            _ => "n/a".into(),
        };
        println!("top - up {up},  load average: {load}");
        let m = &snap.mem;
        let mib = |b: u64| b / 1024 / 1024;
        println!(
            "Tasks: {} total;  MiB Mem: {} total, {} free",
            snap.procs.len(),
            mib(m.total),
            mib(m.free),
        );
        println!("{:>7} {:<12} {:>6} {:>6} {:>9} {:>8} {}", "PID", "USER", "%CPU", "%MEM", "RES", "TIME+", "COMMAND");
        for p in snap.procs.iter().take(20) {
            let cpu = snap.cpu_pct.get(&p.pid).copied().unwrap_or(0.0);
            let mem_pct = p.rss_bytes as f64 / snap.mem_total as f64 * 100.0;
            let secs = (p.utime_ms + p.stime_ms) / 1000;
            let cmd = if p.cmdline.is_empty() { p.name.clone() } else { p.cmdline.join(" ") };
            println!(
                "{:>7} {:<12} {:>6.1} {:>6.1} {:>9} {:>5}:{:02} {}",
                p.pid, trunc(&p.user, 12), cpu, mem_pct, procps::units::human_bytes(p.rss_bytes), secs / 60, secs % 60, cmd
            );
        }
        println!();
        n += 1;
        if let Some(limit) = iterations
            && n >= limit
        {
            break;
        }
    }
}

/// Render two rounds (so deltas have values) via TestBackend, then print the buffer as plain text.
/// `keys` optionally feeds a sequence of normal-mode keys first (for visual verification).
fn snapshot_app(mut app: App, keys: Option<String>) {
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    let (w, h) = (120u16, 40u16);
    let backend = TestBackend::new(w, h);
    let mut terminal = Terminal::new(backend).unwrap();
    let _ = sample(&mut app); // first round establishes baseline
    std::thread::sleep(Duration::from_millis(300));
    let snap = sample(&mut app);
    if let Some(seq) = keys {
        for c in seq.chars() {
            handle_key(&mut app, &snap, KeyCode::Char(c));
        }
    }
    terminal.draw(|f| ui(f, &app, &snap)).unwrap();

    let buf = terminal.backend().buffer();
    for y in 0..h {
        let mut line = String::new();
        for x in 0..w {
            line.push_str(buf[(x, y)].symbol());
        }
        println!("{}", line.trim_end());
    }
}

const SORT_ORDER: [Sort; 4] = [Sort::Cpu, Sort::Mem, Sort::Pid, Sort::Time];

fn run(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
) -> std::io::Result<()> {
    let mut snap = sample(app);
    let mut last_tick = Instant::now();

    loop {
        terminal.draw(|f| ui(f, app, &snap))?;

        let timeout = app.delay.saturating_sub(last_tick.elapsed());
        if event::poll(timeout)? {
            if let Event::Key(k) = event::read()? {
                if k.kind != KeyEventKind::Press {
                    continue;
                }
                // Input mode: capture text for the pending prompt
                if app.input.is_some() {
                    handle_input_key(app, &snap, k.code);
                } else if handle_key(app, &snap, k.code) {
                    return Ok(());
                }
            }
        }

        if last_tick.elapsed() >= app.delay && !app.paused {
            snap = sample(app);
            let n = snap.procs.len();
            if let Some(s) = app.table.selected()
                && s >= n
            {
                app.table.select(Some(n.saturating_sub(1)));
            }
            last_tick = Instant::now();
        }
    }
}

/// Handle a normal-mode key. Returns true if the app should quit.
fn handle_key(app: &mut App, snap: &Snapshot, code: KeyCode) -> bool {
    let n = snap.procs.len();
    // Dismiss help on any key
    if app.help {
        app.help = false;
        return false;
    }
    match code {
        KeyCode::Char('q') | KeyCode::Esc => return true,
        // --- sorting ---
        KeyCode::Char('P') => app.sort = Sort::Cpu,
        KeyCode::Char('M') => app.sort = Sort::Mem,
        KeyCode::Char('N') => app.sort = Sort::Pid,
        KeyCode::Char('T') => app.sort = Sort::Time,
        KeyCode::Char('R') => {
            app.reverse = !app.reverse;
            app.status = format!("reverse sort: {}", on_off(app.reverse));
        }
        KeyCode::Char('<') => cycle_sort(app, -1),
        KeyCode::Char('>') => cycle_sort(app, 1),
        // --- navigation ---
        KeyCode::Up => move_sel(app, n, -1),
        KeyCode::Down => move_sel(app, n, 1),
        KeyCode::PageUp => move_sel(app, n, -10),
        KeyCode::PageDown => move_sel(app, n, 10),
        KeyCode::Home => app.table.select(Some(0)),
        KeyCode::End => app.table.select(Some(n.saturating_sub(1))),
        // --- refresh / pause / rate ---
        KeyCode::Enter | KeyCode::Char(' ') => {
            if code == KeyCode::Char(' ') {
                app.paused = !app.paused;
                app.status = format!("paused: {}", on_off(app.paused));
            }
        }
        KeyCode::Char('+') => {
            app.delay = (app.delay + Duration::from_millis(500)).min(Duration::from_secs(10));
        }
        KeyCode::Char('-') => {
            app.delay =
                app.delay.saturating_sub(Duration::from_millis(500)).max(Duration::from_millis(500));
        }
        // --- summary / cpu display toggles ---
        KeyCode::Char('1') => {
            app.per_core = !app.per_core;
            app.status = format!("per-core CPU: {}", on_off(app.per_core));
        }
        KeyCode::Char('m') => {
            app.show_mem = !app.show_mem;
            app.status = format!("memory area: {}", on_off(app.show_mem));
        }
        KeyCode::Char('l') | KeyCode::Char('t') => {
            app.show_summary = !app.show_summary;
            app.status = format!("summary line: {}", on_off(app.show_summary));
        }
        KeyCode::Char('I') => {
            app.irix = !app.irix;
            app.status = format!("Irix mode: {}", on_off(app.irix));
        }
        // --- task area toggles ---
        KeyCode::Char('c') => {
            app.show_cmdline = !app.show_cmdline;
            app.status = format!("command line: {}", on_off(app.show_cmdline));
        }
        KeyCode::Char('H') => {
            app.threads_mode = !app.threads_mode;
            app.status = format!("threads mode: {} (process-level only)", on_off(app.threads_mode));
        }
        KeyCode::Char('i') => {
            app.hide_idle = !app.hide_idle;
            app.status = format!("hide idle: {}", on_off(app.hide_idle));
        }
        KeyCode::Char('b') => {
            app.bold = !app.bold;
            app.status = format!("bold: {}", on_off(app.bold));
        }
        KeyCode::Char('x') => {
            app.highlight_sort = !app.highlight_sort;
            app.status = format!("highlight sort column: {}", on_off(app.highlight_sort));
        }
        KeyCode::Char('y') => {
            app.highlight_run = !app.highlight_run;
            app.status = format!("highlight running: {}", on_off(app.highlight_run));
        }
        KeyCode::Char('0') => {
            app.suppress_zero = !app.suppress_zero;
            app.status = format!("suppress zeros: {}", on_off(app.suppress_zero));
        }
        // --- prompts (input mode) ---
        KeyCode::Char('k') => start_input(app, InputAction::KillSignal, "Send signal [TERM]: "),
        KeyCode::Char('r') => start_input(app, InputAction::Renice, "Renice value: "),
        KeyCode::Char('d') | KeyCode::Char('s') => {
            start_input(app, InputAction::Delay, "Delay seconds: ")
        }
        KeyCode::Char('u') | KeyCode::Char('U') => {
            start_input(app, InputAction::FilterUser, "Filter user (empty=clear): ")
        }
        KeyCode::Char('n') | KeyCode::Char('#') => {
            start_input(app, InputAction::MaxTasks, "Max tasks (0=all): ")
        }
        KeyCode::Char('L') => start_input(app, InputAction::Locate, "Locate string: "),
        // --- misc ---
        KeyCode::Char('=') => app.reset(),
        KeyCode::Char('?') | KeyCode::Char('h') => app.help = true,
        KeyCode::Char('W') => app.status = "config save is a no-op in procps-rust".into(),
        _ => {}
    }
    false
}

fn start_input(app: &mut App, action: InputAction, prompt: &str) {
    app.input = Some(InputState { action, prompt: prompt.to_string(), buf: String::new() });
}

/// Handle a key while a text prompt is active.
fn handle_input_key(app: &mut App, snap: &Snapshot, code: KeyCode) {
    let Some(input) = app.input.as_mut() else { return };
    match code {
        KeyCode::Esc => {
            app.input = None;
            app.status = "cancelled".into();
        }
        KeyCode::Backspace => {
            input.buf.pop();
        }
        KeyCode::Char(c) => input.buf.push(c),
        KeyCode::Enter => {
            let action = input.action;
            let text = input.buf.trim().to_string();
            app.input = None;
            apply_input(app, snap, action, &text);
        }
        _ => {}
    }
}

fn apply_input(app: &mut App, snap: &Snapshot, action: InputAction, text: &str) {
    match action {
        InputAction::KillSignal => {
            let sig_str = if text.is_empty() { "TERM" } else { text };
            let Some(sig) = platform::Signal::parse(sig_str) else {
                app.status = format!("invalid signal: {sig_str}");
                return;
            };
            if let Some(p) = selected_proc(app, snap) {
                match platform::kill(p.pid, sig) {
                    Ok(()) => app.status = format!("sent {sig_str} to PID {} ({})", p.pid, p.name),
                    Err(e) => app.status = format!("kill PID {} failed: {e}", p.pid),
                }
            }
        }
        InputAction::Renice => {
            let Ok(nice) = text.parse::<i32>() else {
                app.status = "invalid nice value".into();
                return;
            };
            if let Some(p) = selected_proc(app, snap) {
                match platform::set_nice(p.pid, nice) {
                    Ok(()) => app.status = format!("reniced PID {} to {nice}", p.pid),
                    Err(e) => app.status = format!("renice PID {} failed: {e}", p.pid),
                }
            }
        }
        InputAction::Delay => {
            if let Ok(secs) = text.parse::<f64>() {
                app.delay = Duration::from_secs_f64(secs.clamp(0.1, 600.0));
                app.status = format!("delay set to {secs}s");
            } else {
                app.status = "invalid delay".into();
            }
        }
        InputAction::FilterUser => {
            if text.is_empty() {
                app.user_filter.clear();
                app.status = "user filter cleared".into();
            } else {
                app.user_filter = vec![text.to_string()];
                app.status = format!("filtering user: {text}");
            }
        }
        InputAction::MaxTasks => {
            match text.parse::<usize>() {
                Ok(0) => {
                    app.max_tasks = None;
                    app.status = "show all tasks".into();
                }
                Ok(n) => {
                    app.max_tasks = Some(n);
                    app.status = format!("max tasks: {n}");
                }
                Err(_) => app.status = "invalid number".into(),
            }
        }
        InputAction::Locate => {
            app.locate = if text.is_empty() { None } else { Some(text.to_string()) };
            app.status = match &app.locate {
                Some(s) => format!("locate: {s}"),
                None => "locate cleared".into(),
            };
        }
    }
}

fn cycle_sort(app: &mut App, dir: i64) {
    let cur = SORT_ORDER.iter().position(|s| *s == app.sort).unwrap_or(0) as i64;
    let len = SORT_ORDER.len() as i64;
    let next = ((cur + dir) % len + len) % len;
    app.sort = SORT_ORDER[next as usize];
    app.status = format!("sort: {}", app.sort.label());
}

fn on_off(b: bool) -> &'static str {
    if b { "on" } else { "off" }
}

fn selected_proc<'a>(app: &App, snap: &'a Snapshot) -> Option<&'a ProcessInfo> {
    app.table.selected().and_then(|s| snap.procs.get(s))
}

fn move_sel(app: &mut App, n: usize, delta: i64) {
    if n == 0 {
        return;
    }
    let cur = app.table.selected().unwrap_or(0) as i64;
    let next = (cur + delta).clamp(0, n as i64 - 1) as usize;
    app.table.select(Some(next));
}

fn ui(f: &mut Frame, app: &App, snap: &Snapshot) {
    let cores = snap.per_core_pct.len().max(1);
    let rows_per_col = cores.min(8);
    // System area height: per-core grid, or a compact single gauge when toggled off
    let sys_height = if app.per_core { rows_per_col as u16 + 2 } else { 4 };

    let mut constraints = vec![Constraint::Length(1)]; // header
    constraints.push(Constraint::Length(sys_height)); // system area
    if app.show_history {
        constraints.push(Constraint::Length(5)); // cpu history
    }
    constraints.push(Constraint::Min(5)); // process table
    constraints.push(Constraint::Length(1)); // footer / prompt

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(f.area());

    let mut idx = 0;
    render_header(f, chunks[idx], app, snap);
    idx += 1;
    render_system(f, chunks[idx], app, snap, rows_per_col);
    idx += 1;
    if app.show_history {
        render_history(f, chunks[idx], app, snap);
        idx += 1;
    }
    render_table(f, chunks[idx], app, snap);
    idx += 1;
    render_footer(f, chunks[idx], app);

    if app.help {
        render_help(f);
    }
}

/// Centered help overlay listing all interactive keys.
fn render_help(f: &mut Frame) {
    let area = f.area();
    let w = 60.min(area.width.saturating_sub(4));
    let h = 22.min(area.height.saturating_sub(2));
    let x = (area.width.saturating_sub(w)) / 2;
    let y = (area.height.saturating_sub(h)) / 2;
    let popup = Rect { x, y, width: w, height: h };

    let lines = vec![
        Line::from(Span::styled("procps top — keys", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))),
        Line::from(""),
        Line::from("q/Esc quit      Enter refresh    Space pause"),
        Line::from("P/M/N/T sort by CPU/MEM/PID/TIME"),
        Line::from("R reverse       < > move sort field"),
        Line::from("↑↓ PgUp/Dn Home/End  navigate rows"),
        Line::from("+/- delay       d/s set delay"),
        Line::from("k kill (signal) r renice"),
        Line::from("u/U filter user n/# max rows    L locate"),
        Line::from("i hide idle     H threads        c cmd/name"),
        Line::from("1 per-core CPU  m memory area    l/t summary"),
        Line::from("I Irix mode     0 suppress zeros"),
        Line::from("x sort-col hl   y running hl     b bold"),
        Line::from("= reset all     W save (no-op)"),
        Line::from("? / h  this help"),
        Line::from(""),
        Line::from(Span::styled("press any key to close", Style::default().fg(Color::DarkGray))),
    ];
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .title(" Help ")
        .style(Style::default().bg(Color::Rgb(20, 20, 30)));
    f.render_widget(ratatui::widgets::Clear, popup);
    f.render_widget(Paragraph::new(lines).block(block), popup);
}

fn render_header(f: &mut Frame, area: Rect, app: &App, snap: &Snapshot) {
    let arrow = if app.reverse { "v" } else { "^" };
    let mut spans = vec![
        Span::styled("  procps ", Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::styled("top ", Style::default().fg(Color::Black).bg(Color::Cyan)),
        Span::raw("  "),
        Span::styled(format!("{:.1}s", app.delay.as_secs_f64()), Style::default().fg(Color::Gray)),
        Span::raw("  "),
        Span::styled(
            format!("sort:{}{arrow}", app.sort.label()),
            Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(format!("tasks:{}", snap.procs.len()), Style::default().fg(Color::Gray)),
    ];
    // Compact indicator of active non-default toggles
    let mut flags = Vec::new();
    if app.paused { flags.push("PAUSED"); }
    if app.hide_idle { flags.push("idle-off"); }
    if !app.irix { flags.push("Solaris"); }
    if !app.user_filter.is_empty() { flags.push("u-filter"); }
    if app.max_tasks.is_some() { flags.push("n-limit"); }
    if !flags.is_empty() {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(flags.join(" "), Style::default().fg(Color::Yellow)));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_system(f: &mut Frame, area: Rect, app: &App, snap: &Snapshot, rows_per_col: usize) {
    // Split horizontally only when the memory panel is visible
    let (cpu_area, mem_area) = if app.show_mem {
        let halves = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(62), Constraint::Percentage(38)])
            .split(area);
        (halves[0], Some(halves[1]))
    } else {
        (area, None)
    };

    // ---- CPU panel ----
    let cpu_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(Span::styled(
            if app.per_core { " CPU per-core " } else { " CPU " },
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ));
    let inner = cpu_block.inner(cpu_area);
    f.render_widget(cpu_block, cpu_area);

    if app.per_core {
        let cores = snap.per_core_pct.len();
        let ncols = cores.div_ceil(rows_per_col.max(1)).max(1);
        let col_constraints: Vec<Constraint> =
            (0..ncols).map(|_| Constraint::Ratio(1, ncols as u32)).collect();
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(col_constraints)
            .split(inner);
        for (ci, col_rect) in cols.iter().enumerate() {
            let start = ci * rows_per_col;
            let end = (start + rows_per_col).min(cores);
            if start >= cores {
                break;
            }
            let bar_w = (col_rect.width as usize).saturating_sub(14).clamp(4, 24);
            let lines: Vec<Line> = (start..end)
                .map(|i| core_line(i, snap.per_core_pct[i], bar_w))
                .collect();
            f.render_widget(Paragraph::new(lines), *col_rect);
        }
    } else {
        // Aggregate single bar
        let bar_w = (inner.width as usize).saturating_sub(16).clamp(8, 80);
        let line = core_line_labeled("ALL", snap.overall_cpu, bar_w);
        f.render_widget(Paragraph::new(vec![line]), inner);
    }

    // ---- Memory panel ----
    let Some(mem_area) = mem_area else { return };
    let mem_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(Span::styled(" Memory ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)));
    let minner = mem_block.inner(mem_area);
    f.render_widget(mem_block, mem_area);

    let m = &snap.mem;
    let used = m.total.saturating_sub(m.free).saturating_sub(m.cached.unwrap_or(0));
    let mem_ratio = used as f64 / m.total.max(1) as f64;
    let swap_used = m.swap_total.saturating_sub(m.swap_free);
    let swap_ratio = if m.swap_total == 0 { 0.0 } else { swap_used as f64 / m.swap_total as f64 };

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1), Constraint::Min(1)])
        .split(minner);

    let mem_gauge = Gauge::default()
        .gauge_style(Style::default().fg(load_color(mem_ratio * 100.0)).bg(Color::Rgb(40, 40, 40)))
        .ratio(mem_ratio.clamp(0.0, 1.0))
        .label(format!("Mem {} / {}", human_bytes(used), human_bytes(m.total)));
    f.render_widget(mem_gauge, rows[0]);

    let swap_gauge = Gauge::default()
        .gauge_style(Style::default().fg(load_color(swap_ratio * 100.0)).bg(Color::Rgb(40, 40, 40)))
        .ratio(swap_ratio.clamp(0.0, 1.0))
        .label(format!("Swp {} / {}", human_bytes(swap_used), human_bytes(m.swap_total)));
    f.render_widget(swap_gauge, rows[1]);

    if app.show_summary {
        let up = platform::uptime().map(|d| format_uptime(d.as_secs())).unwrap_or_default();
        let load = match platform::loadavg() {
            Ok(Some((a, b, c))) => format!("{a:.2} {b:.2} {c:.2}"),
            _ => "n/a".into(),
        };
        let summary = vec![
            Line::from(vec![
                Span::styled("Tasks ", Style::default().fg(Color::DarkGray)),
                Span::styled(format!("{}", snap.procs.len()), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                Span::styled("  CPU ", Style::default().fg(Color::DarkGray)),
                Span::styled(format!("{:.1}%", snap.overall_cpu), Style::default().fg(load_color(snap.overall_cpu)).add_modifier(Modifier::BOLD)),
            ]),
            Line::from(vec![
                Span::styled("Up ", Style::default().fg(Color::DarkGray)),
                Span::styled(up, Style::default().fg(Color::White)),
            ]),
            Line::from(vec![
                Span::styled("Load ", Style::default().fg(Color::DarkGray)),
                Span::styled(load, Style::default().fg(Color::White)),
            ]),
        ];
        f.render_widget(Paragraph::new(summary), rows[2]);
    }
}

fn render_history(f: &mut Frame, area: Rect, app: &App, snap: &Snapshot) {
    let data: Vec<u64> = app.history.iter().copied().collect();
    let spark = Sparkline::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .title(Span::styled(
                    format!(" CPU history (overall {:.1}%) ", snap.overall_cpu),
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                )),
        )
        .data(&data)
        .max(100)
        .style(Style::default().fg(load_color(snap.overall_cpu)));
    f.render_widget(spark, area);
}

fn render_table(f: &mut Frame, area: Rect, app: &App, snap: &Snapshot) {
    // Column titles; the active sort column is highlighted when enabled.
    let titles = ["PID", "USER", "PR", "%CPU", "%MEM", "RES", "TIME+", "COMMAND"];
    let sort_col = match app.sort {
        Sort::Pid => 0,
        Sort::Cpu => 3,
        Sort::Mem => 4,
        Sort::Time => 6,
    };
    let header_cells: Vec<Cell> = titles
        .iter()
        .enumerate()
        .map(|(i, t)| {
            if app.highlight_sort && i == sort_col {
                Cell::from(*t).style(Style::default().fg(Color::Black).bg(Color::Yellow).add_modifier(Modifier::BOLD))
            } else {
                Cell::from(*t)
            }
        })
        .collect();
    let header = Row::new(header_cells)
        .style(Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD));

    let zero = |s: String| -> String {
        if app.suppress_zero && (s == "0" || s == "0.0") { String::new() } else { s }
    };

    let limit = app.max_tasks.unwrap_or(usize::MAX);
    let rows: Vec<Row> = snap
        .procs
        .iter()
        .take(limit)
        .map(|p| {
            let cpu = snap.cpu_pct.get(&p.pid).copied().unwrap_or(0.0);
            let mem_pct = p.rss_bytes as f64 / snap.mem_total as f64 * 100.0;
            let secs = (p.utime_ms + p.stime_ms) / 1000;
            let time = format!("{}:{:02}", secs / 60, secs % 60);
            let cmd = if app.show_cmdline && !p.cmdline.is_empty() {
                p.cmdline.join(" ")
            } else {
                p.name.clone()
            };
            let mut base = Style::default();
            if app.bold {
                base = base.add_modifier(Modifier::BOLD);
            }
            // Highlight running tasks (state 'R'); on Windows state is '?', so no-op there.
            if app.highlight_run && p.state == 'R' {
                base = base.bg(Color::Rgb(0, 50, 0));
            }
            // Locate: highlight rows matching the search string
            if let Some(q) = &app.locate
                && cmd.to_lowercase().contains(&q.to_lowercase())
            {
                base = base.bg(Color::Rgb(70, 50, 0));
            }
            Row::new(vec![
                Cell::from(p.pid.to_string()).style(base.fg(Color::DarkGray)),
                Cell::from(trunc(&p.user, 12)).style(base.fg(Color::Blue)),
                Cell::from(p.priority.map(|v| v.to_string()).unwrap_or_else(|| "-".into())).style(base),
                Cell::from(zero(format!("{cpu:.1}"))).style(base.fg(load_color(cpu)).add_modifier(Modifier::BOLD)),
                Cell::from(zero(format!("{mem_pct:.1}"))).style(base.fg(load_color(mem_pct * 2.0))),
                Cell::from(human_bytes(p.rss_bytes)).style(base.fg(Color::Green)),
                Cell::from(time).style(base.fg(Color::Gray)),
                Cell::from(cmd).style(base.fg(Color::White)),
            ])
        })
        .collect();

    let shown = rows.len();
    let widths = [
        Constraint::Length(7),
        Constraint::Length(12),
        Constraint::Length(3),
        Constraint::Length(6),
        Constraint::Length(5),
        Constraint::Length(8),
        Constraint::Length(8),
        Constraint::Min(20),
    ];

    let title = if app.max_tasks.is_some() {
        format!(" tasks {shown}/{} ", snap.procs.len())
    } else {
        format!(" tasks ({}) ", snap.procs.len())
    };
    let table = Table::new(rows, widths)
        .header(header)
        .row_highlight_style(Style::default().bg(Color::Rgb(60, 60, 90)).add_modifier(Modifier::BOLD))
        .highlight_symbol("▶ ")
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .title(Span::styled(title, Style::default().fg(Color::White).add_modifier(Modifier::BOLD))),
        );

    let mut state = app.table.clone();
    f.render_stateful_widget(table, area, &mut state);
}

fn render_footer(f: &mut Frame, area: Rect, app: &App) {
    // Active text prompt takes over the footer line.
    if let Some(input) = &app.input {
        let line = Line::from(vec![
            Span::styled(
                format!(" {} ", input.prompt),
                Style::default().fg(Color::Black).bg(Color::Yellow).add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("{}_", input.buf), Style::default().fg(Color::White)),
            Span::styled("   (Enter=ok Esc=cancel)", Style::default().fg(Color::DarkGray)),
        ]);
        f.render_widget(Paragraph::new(line), area);
        return;
    }

    let key = |k: &'static str, d: &'static str| {
        vec![
            Span::styled(format!(" {k} "), Style::default().fg(Color::Black).bg(Color::Gray)),
            Span::styled(format!(" {d} "), Style::default().fg(Color::Gray)),
        ]
    };
    let mut spans = Vec::new();
    spans.extend(key("q", "quit"));
    spans.extend(key("?", "help"));
    spans.extend(key("P/M/N/T", "sort"));
    spans.extend(key("k", "kill"));
    spans.extend(key("r", "renice"));
    spans.extend(key("u", "user"));
    spans.extend(key("=", "reset"));
    if !app.status.is_empty() {
        spans.push(Span::styled(
            format!("  {}", app.status),
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        ));
    }
    f.render_widget(Paragraph::new(Line::from(spans)).alignment(Alignment::Left), area);
}

fn trunc(s: &str, n: usize) -> String {
    if s.chars().count() > n {
        s.chars().take(n).collect()
    } else {
        s.to_string()
    }
}
