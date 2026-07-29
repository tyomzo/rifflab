#!/usr/bin/env bash
# Install RiffLab into the user's XDG paths so it shows up in the GNOME launcher.
#
# Usage:
#   packaging/linux/install.sh                # build release + install
#   packaging/linux/install.sh --no-build     # use existing target/release/rifflab
#   packaging/linux/install.sh --system       # install to /usr/local (needs sudo)
set -euo pipefail

cd "$(dirname "$0")/../.."

BUILD=1
PREFIX="$HOME/.local"
for arg in "$@"; do
    case "$arg" in
        --no-build) BUILD=0 ;;
        --system)   PREFIX="/usr/local" ;;
        -h|--help)
            sed -n '2,7p' "$0"; exit 0 ;;
        *) echo "Unknown arg: $arg" >&2; exit 1 ;;
    esac
done

BIN_DIR="$PREFIX/bin"
APPS_DIR="$PREFIX/share/applications"
ICON_DIR="$PREFIX/share/icons/hicolor/scalable/apps"

if [[ "$BUILD" == "1" ]]; then
    echo ">> cargo build --release -p rifflab-app"
    cargo build --release -p rifflab-app
fi

BIN_SRC="target/release/rifflab"
if [[ ! -x "$BIN_SRC" ]]; then
    echo "error: $BIN_SRC not found (run without --no-build)" >&2
    exit 1
fi

install -d "$BIN_DIR" "$APPS_DIR" "$ICON_DIR"

echo ">> install binary -> $BIN_DIR/rifflab"
install -m 0755 "$BIN_SRC" "$BIN_DIR/rifflab"

echo ">> install icon   -> $ICON_DIR/rifflab.svg"
install -m 0644 packaging/linux/rifflab.svg "$ICON_DIR/rifflab.svg"

echo ">> install .desktop -> $APPS_DIR/rifflab.desktop"
install -m 0644 packaging/linux/rifflab.desktop "$APPS_DIR/rifflab.desktop"

# Refresh GNOME's caches so the new entry/icon show up without a logout.
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$APPS_DIR" >/dev/null 2>&1 || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -f -t "$PREFIX/share/icons/hicolor" >/dev/null 2>&1 || true
fi

echo
echo "Installed. Make sure $BIN_DIR is on your PATH:"
echo "    echo \$PATH | tr ':' '\\n' | grep -qx '$BIN_DIR' || echo '  -> add $BIN_DIR to PATH'"
echo
echo "Launch from the GNOME activities menu (search 'RiffLab') or run: rifflab"
