# CI maintenance

Light checks run on VPS. Compiler builds and image builds run on workstation.
PR jobs use isolated containers without host Docker access or deployment secrets.
Each check has a separate job. CI summary must fail when a required check fails.

## Tool downloads

Versions are defined in `.gitea/workflows/tool-versions.env`.
Downloads must match a SHA256 in `.gitea/workflows/download-checksums.txt`.
Unknown URLs and mismatched content fail before installation. To update a tool,
review its release, download the official artifact, verify its upstream checksum
when available, update the version and reviewed SHA256 together, and run CI.
Existing tools at the configured version are reused.

## Changes and rollback

Run CI on a pull request before merging. Main CI validates the merged commit.
Revert the focused CI commit to restore previous configuration. Reverting source
files does not restore runner configuration; use the Homelab runner backup procedure.
Do not prune shared Docker images or volumes to fix a failed job.

## Required main checks

Main requires `ci / CI summary (pull_request_target)` on each PR. The summary requires all validation
jobs to succeed and allows only documented conditional image skips. Force push
to main is disabled. The owner's existing direct-push path and administrator
overrides remain available; normal changes should use checked PRs.

Runner post-task maintenance clears supported npm/uv download caches above
256 MiB each under the installer lock. Installed tools, runtimes, Rust caches,
Docker images and application data are outside that cleanup.
