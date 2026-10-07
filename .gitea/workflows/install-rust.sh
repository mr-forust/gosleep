#!/usr/bin/env bash
set -euo pipefail

cargo_bin="${CARGO_HOME:-$HOME/.cargo}/bin"
if [[ ! -x "$cargo_bin/rustup" ]] && command -v rustup >/dev/null 2>&1; then
  cargo_bin="$(dirname "$(command -v rustup)")"
fi
if [[ ! -x "$cargo_bin/rustup" ]]; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --profile minimal --default-toolchain stable
fi

echo "$cargo_bin" >> "$GITHUB_PATH"
"$cargo_bin/rustup" toolchain install stable --component clippy --component rustfmt
