#!/usr/bin/env bash
set -euo pipefail
trap 'exit 130' INT
trap 'exit 143' TERM

bash "$(dirname "${BASH_SOURCE[0]}")/ci-disk-check.sh" 512 >&2

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=tool-versions.env
. "$here/tool-versions.env"

case "$(uname -m)" in
  x86_64 | amd64) goarch=amd64; sharch=x86_64; hadolintarch=x86_64 ;;
  aarch64 | arm64) goarch=arm64; sharch=aarch64; hadolintarch=arm64 ;;
  *)
    echo "Unsupported CI runner architecture: $(uname -m)" >&2
    exit 1
    ;;
esac

tools_dir="${CI_TOOL_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/gitea-ci}"
bin_dir="$tools_dir/bin"
mkdir -p "$bin_dir"
if command -v flock >/dev/null 2>&1; then
  exec 9>"$tools_dir/install.lock"
  flock -w 300 9
fi

fetch() {
  bash "$here/download-verified.sh" "$1" "$2"
}

at_version() {
  local tool="$1" expected="$2" output
  [ -x "$bin_dir/$tool" ] || return 1
  if [ "$tool" = kubeconform ]; then
    output="$("$bin_dir/$tool" -v 2>&1 || true)"
  else
    output="$("$bin_dir/$tool" --version 2>&1 || true)"
  fi
  if [[ "$output" =~ (^|[^0-9.])v?([0-9]+(\.[0-9]+)+) ]]; then
    [ "${BASH_REMATCH[2]}" = "$expected" ]
  else
    return 1
  fi
}

for tool in "$@"; do
  case "$tool" in
    actionlint) expected="$ACTIONLINT_VERSION" ;;
    shellcheck) expected="$SHELLCHECK_VERSION" ;;
    hadolint) expected="$HADOLINT_VERSION" ;;
    kubeconform) expected="$KUBECONFORM_VERSION" ;;
    promtool) expected="$PROMETHEUS_VERSION" ;;
    compose) expected="$COMPOSE_VERSION" ;;
    uv) expected="$UV_VERSION" ;;
    *) echo "Unknown CI tool: $tool" >&2; exit 1 ;;
  esac
  binary_tool="$tool"
  if [[ "$tool" == compose ]]; then binary_tool=docker-compose; fi
  if at_version "$binary_tool" "$expected"; then
    continue
  fi
  tmp="$(mktemp -d "${TMPDIR:-/tmp}/ci-tool.XXXXXX")"
  trap 'rm -rf "$tmp"' EXIT
  case "$tool" in
    actionlint)
      fetch \
        "https://github.com/rhysd/actionlint/releases/download/v${ACTIONLINT_VERSION}/actionlint_${ACTIONLINT_VERSION}_linux_${goarch}.tar.gz" \
        "$tmp/tool.tar.gz"
      tar --no-same-owner -xzf "$tmp/tool.tar.gz" -C "$tmp" actionlint
      install -m 0755 "$tmp/actionlint" "$bin_dir/actionlint"
      ;;
    shellcheck)
      fetch \
        "https://github.com/koalaman/shellcheck/releases/download/v${SHELLCHECK_VERSION}/shellcheck-v${SHELLCHECK_VERSION}.linux.${sharch}.tar.xz" \
        "$tmp/tool.tar.xz"
      tar --no-same-owner -xJf "$tmp/tool.tar.xz" -C "$tmp" --strip-components=1 \
        "shellcheck-v${SHELLCHECK_VERSION}/shellcheck"
      install -m 0755 "$tmp/shellcheck" "$bin_dir/shellcheck"
      ;;
    hadolint)
      fetch \
        "https://github.com/hadolint/hadolint/releases/download/v${HADOLINT_VERSION}/hadolint-linux-${hadolintarch}" \
        "$tmp/hadolint"
      install -m 0755 "$tmp/hadolint" "$bin_dir/hadolint"
      ;;
    kubeconform)
      fetch \
        "https://github.com/yannh/kubeconform/releases/download/v${KUBECONFORM_VERSION}/kubeconform-linux-${goarch}.tar.gz" \
        "$tmp/tool.tar.gz"
      tar --no-same-owner -xzf "$tmp/tool.tar.gz" -C "$tmp" kubeconform
      install -m 0755 "$tmp/kubeconform" "$bin_dir/kubeconform"
      ;;
    promtool)
      fetch \
        "https://github.com/prometheus/prometheus/releases/download/v${PROMETHEUS_VERSION}/prometheus-${PROMETHEUS_VERSION}.linux-${goarch}.tar.gz" \
        "$tmp/tool.tar.gz"
      tar --no-same-owner -xzf "$tmp/tool.tar.gz" -C "$tmp" --strip-components=1 \
        "prometheus-${PROMETHEUS_VERSION}.linux-${goarch}/promtool"
      install -m 0755 "$tmp/promtool" "$bin_dir/promtool"
      ;;
    compose)
      fetch "https://github.com/docker/compose/releases/download/v${COMPOSE_VERSION}/docker-compose-linux-${sharch}" "$tmp/docker-compose"
      install -m 0755 "$tmp/docker-compose" "$bin_dir/docker-compose"
      ;;
    uv)
      fetch \
        "https://github.com/astral-sh/uv/releases/download/${UV_VERSION}/uv-${sharch}-unknown-linux-gnu.tar.gz" \
        "$tmp/tool.tar.gz"
      tar --no-same-owner -xzf "$tmp/tool.tar.gz" -C "$tmp" --strip-components=1 \
        "uv-${sharch}-unknown-linux-gnu/uv"
      install -m 0755 "$tmp/uv" "$bin_dir/uv"
      ;;
    *)
      echo "Unknown CI tool: $tool" >&2
      rm -rf "$tmp"
      exit 1
      ;;
  esac
  rm -rf "$tmp"
  trap - EXIT
done

printf '%s\n' "$bin_dir"
