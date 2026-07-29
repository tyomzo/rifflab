#!/usr/bin/env bash
# Remove RiffLab from XDG paths. Mirror of install.sh.
set -euo pipefail

PREFIX="$HOME/.local"
[[ "${1:-}" == "--system" ]] && PREFIX="/usr/local"

rm -f "$PREFIX/bin/rifflab"
rm -f "$PREFIX/share/applications/rifflab.desktop"
rm -f "$PREFIX/share/icons/hicolor/scalable/apps/rifflab.svg"

if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$PREFIX/share/applications" >/dev/null 2>&1 || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -f -t "$PREFIX/share/icons/hicolor" >/dev/null 2>&1 || true
fi

echo "Removed RiffLab from $PREFIX"
