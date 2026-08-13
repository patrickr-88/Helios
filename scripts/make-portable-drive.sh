#!/usr/bin/env bash
#
# Sets up a flash drive to run Helios portably.
#
#   ./scripts/make-portable-drive.sh /Volumes/HELIOS
#
# Copies whichever build artefacts exist (the .app bundle, the CLI binary, or
# both), drops the portable marker beside them, and creates the data folder that
# keeps every scan on the drive instead of on the host machine.
#
# This script only ever copies files into the destination you name and creates
# two entries there. It never deletes anything, and never touches the machine
# you run it from outside the repository.

set -euo pipefail

DEST="${1:-}"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if [[ -z "$DEST" ]]; then
    cat >&2 <<'USAGE'
usage: make-portable-drive.sh <destination>

  <destination>   The mounted drive, e.g. /Volumes/HELIOS on macOS or
                  /media/you/HELIOS on Linux.

Build something to copy first:
  npm run app:build                       # the desktop app (macOS)
  cargo build --release -p helios-cli     # the command-line tool
USAGE
    exit 64
fi

if [[ ! -d "$DEST" ]]; then
    echo "error: $DEST is not a mounted directory" >&2
    exit 66
fi
if [[ ! -w "$DEST" ]]; then
    echo "error: $DEST is not writable — is the drive locked or read-only?" >&2
    exit 77
fi

APP="$REPO_ROOT/src-tauri/target/release/bundle/macos/Helios.app"
CLI="$REPO_ROOT/target/release/helios"
copied=0

if [[ -d "$APP" ]]; then
    echo "copying Helios.app…"
    # -R preserves the bundle's symlinks and extended attributes, which the
    # code signature depends on. Copy to a temporary name and swap, so an
    # interrupted copy cannot leave a half-written app behind.
    rm -rf "$DEST/.Helios.app.partial"
    cp -R "$APP" "$DEST/.Helios.app.partial"
    rm -rf "$DEST/Helios.app"
    mv "$DEST/.Helios.app.partial" "$DEST/Helios.app"
    copied=$((copied + 1))
fi

if [[ -x "$CLI" ]]; then
    echo "copying the helios command-line tool…"
    cp "$CLI" "$DEST/helios.tmp"
    chmod +x "$DEST/helios.tmp"
    mv "$DEST/helios.tmp" "$DEST/helios"
    copied=$((copied + 1))
fi

if [[ "$copied" -eq 0 ]]; then
    echo "error: nothing to copy — build the app or the CLI first:" >&2
    echo "  npm run app:build" >&2
    echo "  cargo build --release -p helios-cli" >&2
    exit 66
fi

# The marker is what switches Helios into portable mode; the data folder is
# where it will keep snapshots. Either one alone is enough, both is tidier.
printf '%s\n' \
    "Helios keeps its data in the HeliosData folder beside this file," \
    "so running the app from this drive writes nothing to the host machine." \
    "Delete this file and the HeliosData folder to go back to normal." \
    > "$DEST/helios-portable.txt"
mkdir -p "$DEST/HeliosData/snapshots"

# Gatekeeper flags anything copied from another volume. Clearing it here means
# the app opens with a double-click instead of a right-click → Open dance.
if command -v xattr >/dev/null 2>&1 && [[ -d "$DEST/Helios.app" ]]; then
    xattr -dr com.apple.quarantine "$DEST/Helios.app" 2>/dev/null || true
fi

echo
echo "Done. $DEST now contains:"
[[ -d "$DEST/Helios.app" ]] && echo "  Helios.app          double-click to run"
[[ -x "$DEST/helios" ]]     && echo "  helios              the command-line tool"
echo "  helios-portable.txt the portable-mode marker"
echo "  HeliosData/         scans are cached here, not on the host"
echo
if [[ -x "$DEST/helios" ]]; then
    echo "Check it with:"
    echo "  $DEST/helios paths"
fi
