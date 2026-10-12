#!/usr/bin/env bash
# Validate CI definitions without a Docker socket or production environment files.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
bin_dir="$(bash "$here/install-ci-tools.sh" actionlint shellcheck)"
export PATH="$bin_dir:$PATH"
actionlint -config-file .gitea/actionlint.yaml -ignore 'unknown permission scope "(code|releases)"' .gitea/workflows/*.yaml
mapfile -t scripts < <(git ls-files '*.sh')
if (( ${#scripts[@]} )); then shellcheck --external-sources --source-path=SCRIPTDIR "${scripts[@]}"; fi
if [[ -f compose.yaml ]]; then
  bin_dir="$(bash "$here/install-ci-tools.sh" compose)"
  export PATH="$bin_dir:$PATH"
  environment="$(mktemp)"
  trap 'rm -f "$environment"' EXIT
  # Skip application env_file reads and interpolation; check the Compose schema offline.
  docker-compose --env-file "$environment" -f compose.yaml config --no-env-resolution --no-interpolate --quiet
fi
