#!/usr/bin/env bash
# Package the built procps binaries into a tar.gz.
# Run build.sh first. Works on Linux, macOS, WSL, or Git Bash / MSYS2 on Windows.
#
#   bash pack.sh             # package the release build
#   bash pack.sh debug       # package the debug build
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

PROFILE="release"
if [ "${1:-}" = "debug" ]; then
    PROFILE="debug"
fi
OUT_DIR="$SCRIPT_DIR/target/$PROFILE"

# Version from Cargo.toml ([package] version, anchored to line start)
VERSION="$(grep -m1 '^version' Cargo.toml | sed -E 's/.*"([^"]+)".*/\1/')"
ARCH="$(uname -m)"

case "$(uname -s)" in
    Linux)                OS="linux" ;;
    Darwin)               OS="macos" ;;
    MINGW*|MSYS*|CYGWIN*) OS="windows" ;;
    *)                    OS="$(uname -s | tr '[:upper:]' '[:lower:]')" ;;
esac

EXE=""
[ "$OS" = "windows" ] && EXE=".exe"

TOOLS=(free uptime w tload vmstat pgrep pkill pidof pidwait kill \
       ps pmap pwdx watch top sysctl slabtop hugetop)

if [ ! -d "$OUT_DIR" ]; then
    echo "error: $OUT_DIR not found. Run build.sh first." >&2
    exit 1
fi

PKG_NAME="procps-${VERSION}-${OS}-${ARCH}"
DIST_DIR="$SCRIPT_DIR/dist"
STAGE_DIR="$DIST_DIR/$PKG_NAME"
rm -rf "$STAGE_DIR"
mkdir -p "$STAGE_DIR/bin"

echo "==> collecting binaries"
found=0
for t in "${TOOLS[@]}"; do
    src="$OUT_DIR/$t$EXE"
    if [ -f "$src" ]; then
        cp "$src" "$STAGE_DIR/bin/"
        found=$((found + 1))
    else
        echo "warning: missing $src" >&2
    fi
done
if [ "$found" -eq 0 ]; then
    echo "error: no tool binaries found in $OUT_DIR. Run build.sh first." >&2
    exit 1
fi

# Strip debug symbols on Unix-like targets to shrink the package
if [ "$OS" != "windows" ] && command -v strip >/dev/null 2>&1; then
    strip "$STAGE_DIR/bin/"* 2>/dev/null || true
fi

# Bundle docs alongside the binaries
for doc in README.md PORTING.md TODO.md; do
    [ -f "$SCRIPT_DIR/$doc" ] && cp "$SCRIPT_DIR/$doc" "$STAGE_DIR/"
done

echo "==> creating archive"
tar -czf "$DIST_DIR/$PKG_NAME.tar.gz" -C "$DIST_DIR" "$PKG_NAME"

echo "==> done: $DIST_DIR/$PKG_NAME.tar.gz  ($found/${#TOOLS[@]} tools)"
tar -tzf "$DIST_DIR/$PKG_NAME.tar.gz" | grep -E 'bin/.' || true
du -h "$DIST_DIR/$PKG_NAME.tar.gz" | cut -f1
