#!/usr/bin/env bash
# Build the procps tools (Rust port of procps-ng 4.0.6).
# Runs wherever bash + cargo exist: Linux, macOS, WSL, or Git Bash / MSYS2 on Windows.
#
#   bash build.sh            # optimized release build of all 18 tools
#   bash build.sh debug      # quick unoptimized build
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

PROFILE="release"
if [ "${1:-}" = "debug" ]; then
    PROFILE="debug"
fi

# Make sure cargo is reachable (fall back to the default rustup location)
if ! command -v cargo >/dev/null 2>&1; then
    if [ -x "$HOME/.cargo/bin/cargo" ]; then
        export PATH="$HOME/.cargo/bin:$PATH"
    else
        echo "error: cargo not found. Install the Rust toolchain from https://rustup.rs" >&2
        exit 1
    fi
fi

echo "==> cargo build ($PROFILE) — all tool binaries"
if [ "$PROFILE" = "release" ]; then
    cargo build --release --bins
else
    cargo build --bins
fi

OUT_DIR="$SCRIPT_DIR/target/$PROFILE"
echo "==> done. Binaries in: $OUT_DIR"
# List the tool binaries that were produced (.exe suffix on Windows)
ls "$OUT_DIR" 2>/dev/null | grep -E \
    '^(free|uptime|w|tload|vmstat|pgrep|pkill|pidof|pidwait|kill|ps|pmap|pwdx|watch|top|sysctl|slabtop|hugetop)(\.exe)?$' \
    || true
