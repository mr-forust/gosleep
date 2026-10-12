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
