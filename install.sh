#!/usr/bin/env bash
#
# Builds Helios and puts it on your PATH.
#
#   ./install.sh              installs to ~/.local/bin
#   ./install.sh /usr/local/bin
#
# There is nothing else to set up: one binary, no runtime, no configuration
# file, no background service. Uninstalling is deleting the binary.

set -euo pipefail

DEST="${1:-$HOME/.local/bin}"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if ! command -v cargo >/dev/null 2>&1; then
    cat >&2 <<'MISSING'
error: cargo not found — Helios is built with Rust.

  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
  source "$HOME/.cargo/env"

Then run this script again.
MISSING
    exit 127
fi

echo "Building (this takes a minute the first time)…"
cargo build --release --manifest-path "$REPO_ROOT/Cargo.toml" -p helios-cli

mkdir -p "$DEST"
install -m 755 "$REPO_ROOT/target/release/helios" "$DEST/helios"

echo
echo "Installed $DEST/helios ($(du -h "$DEST/helios" | cut -f1))"

case ":$PATH:" in
    *":$DEST:"*)
        echo "Try it:  helios"
        ;;
    *)
        # Worth saying plainly: ~/.local/bin is not on every shell's PATH.
        echo "$DEST is not on your PATH. Add it:"
        echo "  echo 'export PATH=\"$DEST:\$PATH\"' >> ~/.zshrc && exec zsh"
        echo "Or run it directly:  $DEST/helios"
        ;;
esac
