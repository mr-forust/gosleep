#!/usr/bin/env bash
set -euo pipefail
toolchain="$(sed -n 's/^channel = "\([^"]*\)"/\1/p' rust-toolchain.toml)"
if [[ ! "$toolchain" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo 'CI requires an explicit Rust toolchain version.' >&2
  exit 1
fi

cargo_bin="${CARGO_HOME:-$HOME/.cargo}/bin"
if [[ ! -x "$cargo_bin/rustup" ]] && command -v rustup >/dev/null 2>&1; then
  cargo_bin="$(dirname "$(command -v rustup)")"
fi
if [[ ! -x "$cargo_bin/rustup" ]]; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --profile minimal --default-toolchain none --no-modify-path
fi

if [[ -n "${GITHUB_PATH:-}" ]]; then
  echo "$cargo_bin" >> "$GITHUB_PATH"
fi
"$cargo_bin/rustup" toolchain install "$toolchain" --profile minimal --component clippy --component rustfmt
