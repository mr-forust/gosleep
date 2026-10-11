# VPS CI

Trusted main and release jobs use the existing Node build image pinned by
digest. It supplies the C compiler missing from the VPS host. Docker runs only
the Rust build commands, with one CPU, 768 MiB RAM and 512 processes. It receives
the checkout and this project's cache, with no Docker socket or release token.
`rust-toolchain.toml` pins the compiler version; the installer follows that file.

The runner stores Rustup, Cargo downloads and build outputs under
`~/.cache/gitea-ci/gosleep`. A lock covers each build and its cleanup. Debug
information and incremental compilation are disabled in CI to reduce disk use.
The target cache is retained up to 768 MiB and removed after a build if larger.
Rustup and Cargo downloads remain available for the next build. No Docker image
or volume cleanup is part of this workflow. A cancellation removes only the
wrapper's own build container before releasing the lock.

PR checks run inside the isolated runner container. They execute the same Rust
checks directly, without host cache mounts, Docker socket or secrets. Python
and yamllint use the same pinned uv installer and runner cache layout as the
other projects; a PR job has only its disposable container cache.

Release publication requires a matching version tag whose commit is in main.
The release token is exposed only to the create and upload steps. Validation
does not publish a release. jq is a cached static binary; it needs no tool image
or package installation on the host.

Rollback: revert the CI commits and rerun validation. The generated GoSleep
cache can remain; the old workflow does not use that path. Removing it is an
optional, separately approved cleanup. Production services are unaffected.

CI runs on main pushes, manual dispatches and `pull_request_target`. PR workflows
come from the base branch and checkout the exact PR head SHA with
`persist-credentials: false` and read-only permissions. Non-main dispatches and
PRs use `homelab-pr-container`; only main uses the trusted host runner.
Rust jobs allow 30 minutes and require 1,536 MiB free before starting. YAML
jobs allow 10 minutes and require 512 MiB. Superseded PR and branch runs are
cancelled; main and release runs are not cancelled by newer runs.
