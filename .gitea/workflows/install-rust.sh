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
  case "$(uname -m)" in
    x86_64 | aarch64) arch="$(uname -m)" ;;
    *) echo 'Unsupported rustup architecture' >&2; exit 1 ;;
  esac
  temporary="$(mktemp -d)"
  trap 'rm -rf "$temporary"' EXIT
  bash "$(dirname "${BASH_SOURCE[0]}")/download-verified.sh" \
    "https://static.rust-lang.org/rustup/archive/1.28.2/${arch}-unknown-linux-gnu/rustup-init" "$temporary/rustup-init"
  chmod 0755 "$temporary/rustup-init"
  "$temporary/rustup-init" -y --profile minimal --default-toolchain none --no-modify-path
  rm -rf "$temporary"
  trap - EXIT
fi

if [[ -n "${GITHUB_PATH:-}" ]]; then
  echo "$cargo_bin" >> "$GITHUB_PATH"
fi
"$cargo_bin/rustup" toolchain install "$toolchain" --profile minimal --component clippy --component rustfmt
