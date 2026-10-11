#!/usr/bin/env bash
set -euo pipefail

usage() {
  echo "Usage: ci-disk-check MIN_FREE_MIB [PATH]" >&2
}

if [[ $# -lt 1 || $# -gt 2 || ! $1 =~ ^[0-9]+$ ]]; then
  usage
  exit 2
fi

minimum_mib="$1"
target_path="${2:-${GITHUB_WORKSPACE:-$PWD}}"
if [[ ! -d "$target_path" ]]; then
  printf 'Disk check path is not a directory: %s\n' "$target_path" >&2
  exit 2
fi

available_bytes="$(LC_ALL=C df -B1 --output=avail -- "$target_path" | tail -n 1 | tr -d '[:space:]')"
if [[ ! $available_bytes =~ ^[0-9]+$ ]]; then
  printf 'Could not read available disk space for: %s\n' "$target_path" >&2
  exit 2
fi

minimum_bytes=$((minimum_mib * 1024 * 1024))
printf 'CI disk check: available=%s bytes (%s MiB), required=%s bytes (%s MiB), path=%s\n' \
  "$available_bytes" "$((available_bytes / 1024 / 1024))" \
  "$minimum_bytes" "$minimum_mib" "$target_path"

if ((available_bytes < minimum_bytes)); then
  echo 'Not enough disk space for this CI job. The runner will keep existing images and caches.' >&2
  exit 78
fi
