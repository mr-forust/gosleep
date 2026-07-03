# Changelog

All notable changes to this project are documented here.

The project uses semantic versioning. Tags are formatted as `vMAJOR.MINOR.PATCH`.

## [1.1.3] - 2026-07-03

### Fixed

- Made the tag-based release workflow fail closed: lint and test gates now run before build and publish.
- Exposed Gitea Actions' built-in `${{ secrets.GITEA_TOKEN }}` to the release API steps so release creation and asset upload authenticate correctly.

## [1.1.2] - 2026-07-03

### Fixed

- Suppressed all background `stdout`/`stderr` writes while the Ratatui screen is active, including child command output from timer actions.
- Split `time left` out of the gauge label and reduced the gauge label to a compact percentage for narrow terminal safety.

## [1.1.1] - 2026-07-03

### Changed

- Rebuilt the application as a Rust terminal binary.
- Added a Ratatui/Crossterm TUI with command preview wrapping, timer progress, and time-left display.
- Added YAML configuration, CLI `init`, `preview`, and `run` commands.
- Added Gitea CI and tag-based release workflows.

[1.1.3]: https://gitea.forust.xyz/forust/gosleep/releases/tag/v1.1.3
[1.1.2]: https://gitea.forust.xyz/forust/gosleep/releases/tag/v1.1.2
[1.1.1]: https://gitea.forust.xyz/forust/gosleep/releases/tag/v1.1.1
