#!/usr/bin/env bash
# Install a pinned Python minor with uv and reuse its runtime and wheel cache.
set -euo pipefail

python_version="${1:?Usage: setup-python.sh 3.12}"
if [[ ! "$python_version" =~ ^3\.(12|13)$ ]]; then
  echo "Unsupported CI Python version: $python_version" >&2
  exit 2
fi
# Pin the patch version instead of resolving a moving Python minor.
case "$python_version" in
  3.12) python_version=3.12.14 ;;
  3.13) python_version=3.13.15 ;;
esac

cache_root="${CI_TOOL_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/gitea-ci}"
bin_dir="$cache_root/bin"
python_dir="$cache_root/python"
uv_cache_dir="$cache_root/uv-cache"
uv_version="0.12.17"
mkdir -p "$bin_dir" "$python_dir" "$uv_cache_dir"

case "$(uname -m)" in
  x86_64 | amd64) uv_arch=x86_64 ;;
  aarch64 | arm64) uv_arch=aarch64 ;;
  *) echo "Unsupported CI architecture: $(uname -m)" >&2; exit 1 ;;
esac

uv_bin="$bin_dir/uv"
if [[ ! -x "$uv_bin" || "$($uv_bin --version 2>/dev/null || true)" != "uv $uv_version ("* ]]; then
  tmp="$(mktemp -d "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/gosleep-uv.XXXXXXXX")"
  trap 'rm -rf "$tmp"' EXIT
  archive="uv-${uv_arch}-unknown-linux-gnu.tar.gz"
  bash "$(dirname "${BASH_SOURCE[0]}")/download-verified.sh" \
    "https://github.com/astral-sh/uv/releases/download/${uv_version}/${archive}" "$tmp/uv.tar.gz"
  tar --no-same-owner -xzf "$tmp/uv.tar.gz" -C "$tmp" --strip-components=1 \
    "uv-${uv_arch}-unknown-linux-gnu/uv"
  install -m 0755 "$tmp/uv" "$uv_bin"
  rm -rf "$tmp"
  trap - EXIT
fi

export UV_CACHE_DIR="$uv_cache_dir"
export UV_PYTHON_INSTALL_DIR="$python_dir"
"$uv_bin" python install "$python_version" --no-bin

job_dir="$(mktemp -d "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/gosleep-python.XXXXXXXX")"
venv="$job_dir/venv"
"$uv_bin" venv "$venv" --python "$python_version"

if [[ -n "${GITHUB_PATH:-}" ]]; then
  printf '%s\n%s\n' "$bin_dir" "$venv/bin" >> "$GITHUB_PATH"
fi
if [[ -n "${GITHUB_ENV:-}" ]]; then
  printf 'UV_CACHE_DIR=%s\nUV_PYTHON_INSTALL_DIR=%s\nCI_PYTHON_ENV=%s\n' \
    "$uv_cache_dir" "$python_dir" "$venv" >> "$GITHUB_ENV"
fi
printf 'Python %s ready; uv cache: %s\n' "$python_version" "$uv_cache_dir"
