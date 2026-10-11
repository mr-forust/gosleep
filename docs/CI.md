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

Rust checks and release compilation are separate jobs, with compilation
requiring successful Rust and YAML checks. CI runs on main pushes, manual dispatches and `pull_request_target`. PR workflows
come from the base branch and checkout the exact PR head SHA with
`persist-credentials: false` and read-only permissions. Non-main dispatches and
Rust checks and builds use `workstation-pr` for PRs and non-main dispatches,
and `workstation-build` for trusted main. YAML stays on the VPS.
Rust jobs allow 30 minutes and require 1,536 MiB free before starting. YAML
jobs allow 10 minutes and require 512 MiB. Superseded PR and branch runs are
cancelled; main and release runs are not cancelled by newer runs.

## Shared runner policy

| Work                                           | Runner label                           | Trust and resources                                                                           |
| ---------------------------------------------- | -------------------------------------- | --------------------------------------------------------------------------------------------- |
| Lint, manifests, small Python tests, summaries | `vps-light` / `vps-pr`                 | Trusted host / disposable PR container; VPS PR limit: 1 CPU, 768 MiB, 512 processes           |
| Compile, frontend build, container build       | `workstation-build` / `workstation-pr` | Trusted host / disposable PR container; workstation PR limit: 2 CPUs, 1536 MiB, 512 processes |

Each queue has capacity one. Workstation host jobs have a systemd CPU quota of
200%, MemoryMax 1536 MiB and lower scheduling priority. Docker children need
separate limits; a host cgroup does not cap containers created by Docker.
PR jobs have no host Docker socket, host cache mounts or publication secrets.
Trusted jobs retain pinned tools and project caches. Lockfiles, image digests,
explicit versions, disk preflights, timeouts and scoped cleanup remain required.
A tool version change must be reviewed; do not switch to a floating latest tag.

The CI graph separates checks and builds. `CI summary` reports every required
job result and fails if any required job failed or was cancelled. Skipped image
publication is expected for PRs. Credentials stay confined to publish steps;
CI routing does not authorize deployment. If workstation is unavailable, heavy
jobs wait there; do not silently fall back to the VPS or a trusted PR runner.

Rollback: revert the routing commit. Keep the previous VPS labels as aliases
while old workflows exist. No application data or volumes need to change.
The authoritative runner configs and installation/rollback instructions live
in homelab `.gitea/runner/`; per-project workflow files select the labels.

Workstation CI uses the separate `gitea-ci-runner` account. Only the existing
runner registration is copied; old SSH and Docker credentials are not imported.
PR containers additionally drop all capabilities and enable no-new-privileges.
