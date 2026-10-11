# CI and release

The workflow uses the shared pinned Node image for PR jobs. The YAML job adds
Python venv support only inside its disposable container; the VPS image cache
does not grow.

## Pull request checks

`.gitea/workflows/ci.yaml` runs for pull requests, pushes to `main`, and manual
runs. PR checks use `pull_request_target` to load the base workflow, check out
the exact PR SHA, and disable persisted checkout credentials. They select the
shared `homelab-pr-container` label. The container image supplies Node for
Actions and Python venv support for the pinned YAML linter; Rust stays a
project tool installed by `install-rust.sh`.

Rust formatting, tests, clippy, and release build run with a 1,536 MiB disk
preflight. YAML lint needs 256 MiB and uses a temporary venv that is removed at
job exit. Cargo cache writes are limited to trusted pushes or manual runs on
`main`; PR runs do not populate shared build caches. Job timeouts and per-PR
cancellation bound stale work.

The PR job container has a one-CPU, 768 MiB memory, and 512-process limit. It
does not receive the Docker socket. The shared runner pauses new jobs below
384 MiB free and removes stale workspaces after 24 hours while idle. It does
not prune images, caches, or volumes.

## Releases

`.gitea/workflows/release.yaml` runs for version tags and manual dispatch on the
trusted runner. It verifies the tag and version, runs the required Rust checks,
then builds and uploads the binary. The Gitea token is now available only to
the release creation and asset upload steps. Release runs are serialized and
are not cancelled.

To roll back CI changes, revert the focused workflow commit. This does not
change application configuration or deployed release assets.
