#!/usr/bin/env bash
set -euo pipefail
mode="${1:-full}"
case "$mode" in
  verify | full)
    package_version="$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -n 1)"
    test -n "$package_version"
    test "$(cat VERSION)" = "$package_version"
    cargo fmt --check
    cargo test --locked
    TZ=Europe/Bratislava cargo test --locked local_deadlines_handle_daylight_saving_transitions -- --ignored
    cargo clippy --locked -- -D warnings
    ;;
  build) ;;
  *) echo 'Usage: check-rust.sh [verify|build|full]' >&2; exit 2 ;;
esac
if [[ "$mode" != verify ]]; then
  cargo build --release --locked
  if [[ "${CARGO_TARGET_DIR:-target}" != target ]]; then
    mkdir -p target/release
    cp "$CARGO_TARGET_DIR/release/gosleep-timer" target/release/gosleep-timer
  fi
fi
