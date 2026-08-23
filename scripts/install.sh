#!/usr/bin/env bash
# Install ligh + lighd from source (release build).
# Requires: macOS, Xcode, Rust (https://rustup.rs)
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

echo "==> LIGH installer (MIT · open source)"
echo "    repo: $ROOT"
echo

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "error: LIGH requires macOS (Apple Silicon recommended)" >&2
  exit 1
fi

if ! xcode-select -p &>/dev/null; then
  echo "error: Xcode / CLT missing — run: xcode-select --install" >&2
  echo "       then open Xcode once and install an iOS Simulator runtime" >&2
  exit 1
fi

if ! command -v cargo &>/dev/null; then
  echo "error: Rust/cargo required" >&2
  echo "       curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh" >&2
  exit 1
fi

BIN_DIR="${CARGO_HOME:-$HOME/.cargo}/bin"
mkdir -p "$BIN_DIR"

echo "==> building release (first run can take a few minutes)…"
( unset CARGO_TARGET_DIR; cargo build --release --locked )

echo "==> installing binaries → $BIN_DIR"
cargo install --path crates/ligh-cli --force --locked --root "${CARGO_HOME:-$HOME/.cargo}"
cargo install --path crates/ligh-daemon --force --locked --root "${CARGO_HOME:-$HOME/.cargo}"

if ! echo ":$PATH:" | grep -q ":$BIN_DIR:"; then
  echo
  echo "⚠  add cargo bin to your PATH (then re-open the terminal):"
  echo "   export PATH=\"$BIN_DIR:\$PATH\""
  echo
  case "${SHELL:-}" in
    */zsh)  echo "   # or: echo 'export PATH=\"$BIN_DIR:\$PATH\"' >> ~/.zshrc" ;;
    */bash) echo "   # or: echo 'export PATH=\"$BIN_DIR:\$PATH\"' >> ~/.bashrc" ;;
  esac
fi

export PATH="$BIN_DIR:$PATH"

echo
echo "✓ installed"
command -v ligh >/dev/null && ligh --version || echo "  ligh → $BIN_DIR/ligh"
command -v lighd >/dev/null && echo "  lighd → $(command -v lighd)" || true
echo
echo "Next:"
echo "  ./scripts/developer-trial.sh   # developer smoke + MCP (start here)"
echo "  ligh doctor"
echo "  ligh daemon start"
echo "  ligh up"
echo "  docs/DEVELOPER_TRIAL.md"
echo
