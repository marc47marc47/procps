//! Shared helpers used across the tools: version string, cross-platform notice for Linux-only flags, pidfile reading.

/// Matches the original procps version source, marking this as a Rust port.
/// Example: `free 0.2.5 (procps-rust, ported from procps-ng 4.0.6)`
pub fn version_string(tool: &str) -> String {
    format!(
        "{tool} {} (procps-rust, ported from procps-ng {})",
        env!("CARGO_PKG_VERSION"),
        PROCPS_SOURCE_VERSION
    )
}

/// The original procps-ng version this project tracks.
pub const PROCPS_SOURCE_VERSION: &str = "4.0.6";

/// Print the version and exit with 0 (matching the original -V/--version behavior).
pub fn print_version_and_exit(tool: &str) -> ! {
    println!("{}", version_string(tool));
    std::process::exit(0);
}

/// Unified notice for Linux-only flags on Windows/macOS.
/// Returns an explanatory string; the caller decides whether to warn and continue or treat it as a fatal error.
pub fn unsupported_note(tool: &str, flag: &str) -> String {
    format!(
        "{tool}: flag {flag} is a Linux-only feature and is not supported on this platform ({}).",
        std::env::consts::OS
    )
}

/// Print an unsupported message for a Linux-only flag and exit nonzero (used when the flag cannot be approximated).
pub fn unsupported_exit(tool: &str, flag: &str) -> ! {
    eprintln!("{}", unsupported_note(tool, flag));
    std::process::exit(1);
}

/// Read a pidfile (matching pgrep/pkill's -F/--pidfile): take the first integer in the file as the PID.
pub fn read_pidfile(path: &str) -> std::io::Result<u32> {
    let text = std::fs::read_to_string(path)?;
    text.split_whitespace()
        .next()
        .and_then(|s| s.parse::<u32>().ok())
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "pidfile contains no valid PID"))
}
