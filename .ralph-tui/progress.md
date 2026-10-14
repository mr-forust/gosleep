# Ralph Progress Log

This file tracks progress across iterations. Agents update this file
after each iteration and it's included in prompts for context.

## Codebase Patterns (Study These First)

* Single-binary Rust app: all code in `src/main.rs` (now `src/main.rs` + `src/config.rs`); package metadata in `Cargo.toml`, locked deps in `Cargo.lock`.
* Config uses `noyalib` for YAML (`from_str`/`to_string`); `#[serde(default)]` on every config struct preserves backward-compat YAML shape.
* `normalize_config` fills empty/out-of-range values; `validate_config` checks duration + power/workspace/monitor/lock/media enums; `parse_duration` accepts Go-style `1h30m` with `h/m/s/ms/us/µs/ns`.
* History I/O (`HistoryEntry`, `read_history`, `append_history`, `history_path`) stays in CLI layer (`main.rs`) per US-001; shared history types not moved.
* Cross-module visibility is `pub(crate)` only for items named outside `config` (Config, DiscordConfig, Lock/Monitor/WorkspaceAction + 6 fns); action field types stay `pub(crate)` because they appear in `pub(crate)` fields.

---

## US-001 - Extract configuration and core data models (2026-10-14)

Implemented:
* Created `src/config.rs` with `Config`, `Actions`, `WorkspaceAction`, `MediaAction`, `BrightnessAction`, `ToggleAction`, `LockAction`, `MonitorAction`, `NotifyAction`, `KillAction`, `PowerAction`, `CustomAction`, `DiscordConfig` (+ `should_connect`), all `Default` impls, and `default_config_path`, `ensure_config`, `save_config`, `normalize_config`, `validate_config`, `parse_duration`.
* `src/main.rs`: added `mod config;` + `use crate::config::{...}` (only items named in main), removed moved structs/impls/fns, kept history I/O and CLI dispatch untouched.
* Moved 5 focused tests into `config::tests`: `parse_duration_accepts_go_style_values`, `parse_duration_rejects_overflow_and_rounds_that_become_zero`, `ensure_config_validates_loaded_config`, `discord_defaults_to_enabled_without_application_id`, `old_config_without_discord_section_loads_defaults`. Removed duplicates from `main.rs` tests.
* No new dependencies, no circular deps (`config` imports only std/anyhow/serde; `main` depends on `config` one-way).
* Files changed: `src/config.rs` (new), `src/main.rs` (extract + import trim + test dedup).

Checks (all PASS):
* `cargo fmt --check` -> PASS (exit 0, no diff).
* `cargo test --locked` -> PASS (51 passed, 1 ignored; includes 5 `config::tests::*`).
* `cargo clippy --locked -- -D warnings` -> PASS (after trimming unused `CustomAction/KillAction/NotifyAction/PowerAction` imports).
* `cargo build --release --locked` -> PASS.
* Isolated smoke: `tmpdir=$(mktemp -d); cargo run -- --config $tmpdir/config.yaml init/validate/status/preview` -> PASS (valid, status shows workspace+media, preview matches).

Learnings:
* `use super::*` in `config::tests` correctly re-exports parent `fs` import for the `ensure_config` test; no extra import needed.
* Clippy `-D warnings` fails on unused config type imports in `main.rs`; import only `Config, DiscordConfig, LockAction, MonitorAction, WorkspaceAction` + fns.
* `pub(crate)` on all config field types is required (field type visibility must cover `pub(crate)` fields), even for types not directly named in `main.rs`.
