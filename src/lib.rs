//! procps — a Rust port of procps-ng 4.0.6.
//!
//! Architecture mapping:
//! - The C `library/libproc2` (which parses Linux /proc) maps to this crate's
//!   [`platform`] module, selecting a backend based on the target OS:
//!   - Linux: parses `/proc` (same data source as the C version)
//!   - Windows: rewritten with native Win32 APIs (Toolhelp32, GlobalMemoryStatusEx, WTS, PEB reads, ...)
//!   - macOS: interface defined, implementation pending (sysctl / libproc / host_statistics64); see PORTING.md
//!
//! Platform tagging convention (searchable during porting):
//! - `[PLATFORM:WINDOWS]` — Win32-specific implementation
//! - `[PLATFORM:LINUX]`   — Linux /proc-specific implementation
//! - `[PLATFORM:MACOS]`   — macOS-specific implementation
//! - `[PORT:MACOS]` / `[PORT:...]` — porting points, noting the recommended API to use

pub mod common;
pub mod matcher;
pub mod platform;
pub mod units;
