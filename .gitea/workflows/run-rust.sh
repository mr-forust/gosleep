#!/usr/bin/env bash
# Trusted builds reuse runner-owned Rust data in the existing compiler image.
# PR builds execute directly inside the isolated job, with no host mounts.
set -euo pipefail

export CARGO_BUILD_JOBS=1
export CARGO_INCREMENTAL=0
export CARGO_PROFILE_DEV_DEBUG=0
export CARGO_PROFILE_TEST_DEBUG=0

if [[ "${CI_TRUSTED_BUILD:-0}" == 1 ]]; then
  bash .gitea/workflows/ci-disk-check.sh 1536
  cache_root="${XDG_CACHE_HOME:-$HOME/.cache}/gitea-ci/gosleep"
  mkdir -p "$cache_root"
  exec 9>"$cache_root/build.lock"
  flock -w 600 9
  container="gosleep-ci-${GITHUB_RUN_ID:-local}-$$"
  cleanup() {
    docker rm -f "$container" >/dev/null 2>&1 || true
    # This directory contains only this wrapper's generated Cargo target data.
    # The lock remains held until cleanup completes; no active build is pruned.
    if [[ -d "$cache_root/target" && ! -L "$cache_root/target" ]]; then
      size_kib="$(du -sk "$cache_root/target" | cut -f1)"
      if ((size_kib > 786432)); then
        echo 'Cargo target cache exceeds 768 MiB; remove it after the build.' >&2
        rm -rf -- "$cache_root/target"
      fi
    fi
  }
  trap cleanup EXIT
  trap 'exit 130' INT
  trap 'exit 143' TERM
  docker run --rm --init --name "$container" \
    --cpus=1 --memory=768m --pids-limit=512 \
    --cap-drop=ALL --security-opt=no-new-privileges \
    --user "$(id -u):$(id -g)" \
    -v "$PWD:/work" -v "$cache_root:/ci-cache" -w /work \
    -e HOME=/tmp/gosleep-home -e CARGO_HOME=/ci-cache/cargo \
    -e RUSTUP_HOME=/ci-cache/rustup -e CARGO_TARGET_DIR=/ci-cache/target \
    node@sha256:0e5f906573693feaa1e21057ebdcfdb5bd5021f050b2dc7c9deceb629c7da2a8 \
    bash .gitea/workflows/run-rust.sh "${1:-full}" &
  build_pid=$!
  wait "$build_pid"
else
  mkdir -p "$HOME"
  export CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"
  export PATH="$CARGO_HOME/bin:$PATH"
  command -v cc >/dev/null || { echo 'The Rust build environment requires cc.' >&2; exit 1; }
  bash .gitea/workflows/install-rust.sh
  bash .gitea/workflows/check-rust.sh "${1:-full}"
fi
