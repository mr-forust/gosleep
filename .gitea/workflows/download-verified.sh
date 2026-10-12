#!/usr/bin/env bash
# Download only a reviewed URL and atomically replace the destination after SHA256 verification.
set -euo pipefail
[[ $# -eq 2 ]] || { echo 'Usage: download-verified.sh HTTPS_URL DESTINATION' >&2; exit 2; }
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
url="$1"
destination="$2"
[[ "$url" == https://* ]] || { echo 'CI downloads require HTTPS.' >&2; exit 2; }
expected="$(awk -v url="$url" '$2 == url {print $1}' "$here/download-checksums.txt")"
[[ "$expected" =~ ^[0-9a-f]{64}$ ]] || { echo "No unique reviewed checksum for $url" >&2; exit 1; }
mkdir -p "$(dirname "$destination")"
temporary="$(mktemp "${destination}.download.XXXXXXXX")"
trap 'rm -f "$temporary"' EXIT
curl --connect-timeout 15 --max-time 180 --retry 3 -fsSL "$url" -o "$temporary"
printf '%s  %s\n' "$expected" "$temporary" | sha256sum --check --status || {
  echo "CI download checksum mismatch: $url" >&2
  exit 1
}
mv -f -- "$temporary" "$destination"
trap - EXIT
