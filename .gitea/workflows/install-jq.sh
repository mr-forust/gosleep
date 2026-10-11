#!/usr/bin/env bash
set -euo pipefail
version=1.8.1
bin_dir="${XDG_CACHE_HOME:-$HOME/.cache}/gitea-ci/bin"
mkdir -p "$bin_dir"
exec 9>"$bin_dir/jq-install.lock"
flock -w 300 9
case "$(uname -m)" in
  x86_64) arch=amd64 ;;
  aarch64) arch=arm64 ;;
  *) echo 'Unsupported jq architecture' >&2; exit 1 ;;
esac
if [[ ! -x "$bin_dir/jq" || "$("$bin_dir/jq" --version 2>/dev/null || true)" != "jq-$version" ]]; then
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT
  curl -fsSL --connect-timeout 15 --max-time 120 --retry 3 \
    "https://github.com/jqlang/jq/releases/download/jq-${version}/jq-linux-${arch}" -o "$tmp/jq"
  install -m 0755 "$tmp/jq" "$bin_dir/jq"
fi
"$bin_dir/jq" --version
if [[ -n "${GITHUB_PATH:-}" ]]; then
  printf '%s\n' "$bin_dir" >> "$GITHUB_PATH"
fi
