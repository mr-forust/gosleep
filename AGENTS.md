# AGENTS.md

Instructions for coding agents working in this repository.

## Project Shape

`gosleep-timer` is a Rust-first, single-binary Linux terminal app.

- Main code: `src/main.rs`
- Package metadata: `Cargo.toml`
- Locked dependency graph: `Cargo.lock`
- Version files: `Cargo.toml`, `Cargo.lock`, `VERSION`, `CHANGELOG.md`
- CI: `.gitea/workflows/ci.yaml`
- Release workflow: `.gitea/workflows/release.yaml`
- Release docs: `RELEASING.md`

Do not reintroduce Go, Python, Node, Docker packaging, or multi-language runtime dependencies unless the user explicitly asks for that migration.

## Required Checks

Run these before committing code changes:

```bash
cargo fmt --check
cargo test --locked
cargo clippy --locked -- -D warnings
cargo build --release --locked
```

Run YAML lint when editing workflow or YAML files:

```bash
yamllint -c .yamllint .
```

If `yamllint` is not installed, the CI-compatible fallback is:

```bash
docker run --rm -v "$PWD:/work" -w /work cytopia/yamllint:latest -c .yamllint .
```

## Local Smoke Tests

Use a temporary config path so local tests do not mutate the user's real config:

```bash
tmpdir="$(mktemp -d)"
cargo run -- --config "$tmpdir/config.yaml" init
cargo run -- --config "$tmpdir/config.yaml" validate
cargo run -- --config "$tmpdir/config.yaml" status
cargo run -- --config "$tmpdir/config.yaml" preview
```

For `run` smoke tests, pass `--no-actions` or disable actions first so the command does not call `playerctl`, `systemctl`, `pkill`, workspace tools, or custom shell commands.

## Versioning

The project uses semantic versioning. Tags must be annotated and formatted as `vMAJOR.MINOR.PATCH`.

When bumping a version, update all of:

- `Cargo.toml`
- `Cargo.lock`
- `VERSION`
- `CHANGELOG.md`

Verify version consistency with:

```bash
test "$(cat VERSION)" = "$(cargo metadata --no-deps --format-version 1 | jq -r '.packages[0].version')"
```

## Releases

Release workflow is tag-based:

```bash
git tag -a vMAJOR.MINOR.PATCH -m "gosleep-timer vMAJOR.MINOR.PATCH"
git push origin main
git push origin vMAJOR.MINOR.PATCH
```

The Gitea release workflow:

- runs formatting, tests, clippy, and YAML lint before building
- builds `target/release/gosleep-timer`
- copies `gosleep-timer-<version>-linux-amd64`
- writes a `.sha256`
- creates or reuses the matching Gitea release
- replaces release assets with matching names

The workflow exposes `${{ secrets.GITEA_TOKEN }}` as `GITEA_TOKEN` and declares `permissions: contents: write`.

## Git Remote

Expected upstream:

```text
origin ssh://git@gitssh.forust.xyz:2221/forust/gosleep.git
```

HTTPS remote URL:

```text
https://gitea.forust.xyz/forust/gosleep.git
```

Before pushing, confirm:

```bash
git status --short --branch
git branch -vv
git remote -v
```

## Repository Hygiene

- Keep `target/`, `dist/`, `.env*`, logs, temp files, and local artifacts out of git.
- Keep `Cargo.lock` committed; this is an application, not a library.
- Prefer small, direct Rust changes in `src/main.rs` until the code genuinely needs splitting.
- Do not add broad abstractions just to prepare for hypothetical modules.
- Do not add shelling out during tests unless the test fully controls the command.
- Keep docs synchronized with actual behavior and command names.
- Keep release assets and generated archives out of the repo.

## TUI Notes

The TUI uses Ratatui and Crossterm. Be careful with terminal-size assumptions.

For layout changes, check both:

- narrow terminal behavior, where settings and preview stack vertically
- wide terminal behavior, where settings and preview are side by side

Avoid truncating command previews without a visible indication. Preview wrapping is expected behavior.

## Safety

The app can run destructive or disruptive post-countdown commands:

- `pkill`
- `systemctl poweroff`
- `systemctl reboot`
- arbitrary custom shell commands

Never run `gosleep-timer run` against the real user config during verification unless the user explicitly asks for it. Use an isolated config and disable actions for smoke tests.
