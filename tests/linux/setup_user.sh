#!/usr/bin/env bash
# Per-user setup: Rust toolchain.
set -euo pipefail
if [ ! -x "$HOME/.cargo/bin/cargo" ]; then
  curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal -c clippy
fi
"$HOME/.cargo/bin/cargo" --version
echo "USER SETUP DONE"
