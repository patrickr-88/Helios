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

Build the binary first:
  cargo build --release
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

CLI="$REPO_ROOT/target/release/helios"

if [[ ! -x "$CLI" ]]; then
    echo "error: $CLI not found — build it first with 'cargo build --release'" >&2
    exit 66
fi

echo "copying helios…"
# Copy to a temporary name and rename, so an interrupted copy cannot leave a
# half-written binary that looks runnable.
cp "$CLI" "$DEST/helios.tmp"
chmod +x "$DEST/helios.tmp"
mv "$DEST/helios.tmp" "$DEST/helios"

# The marker is what switches Helios into portable mode; the data folder is
# where it will keep snapshots. Either one alone is enough, both is tidier.
printf '%s\n' \
    "Helios keeps its data in the HeliosData folder beside this file," \
    "so running it from this drive writes nothing to the host machine." \
    "Delete this file and the HeliosData folder to go back to normal." \
    > "$DEST/helios-portable.txt"
mkdir -p "$DEST/HeliosData/snapshots"

# macOS flags anything copied from another volume; clearing it here saves the
# "unidentified developer" refusal later.
if command -v xattr >/dev/null 2>&1; then
    xattr -d com.apple.quarantine "$DEST/helios" 2>/dev/null || true
fi

echo
echo "Done. $DEST now contains:"
echo "  helios              the program"
echo "  helios-portable.txt the portable-mode marker"
echo "  HeliosData/         scans are cached here, not on the host"
echo
echo "Check it with:"
echo "  $DEST/helios --info" 
