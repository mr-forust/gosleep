# Ralph Progress Log

This file tracks progress across iterations. Agents update this file
after each iteration and it's included in prompts for context.

## Codebase Patterns (Study These First)

- Single-binary Rust app: all logic in `src/main.rs` until a concrete reason to split.
- Config structs use `#[serde(default)]` + explicit `Default` impls for backward compat.
- TUI settings are generated from `FIELDS: &[Field]` (`FieldKey` + `FieldKind`); edit flow is `activate` -> `edit` -> `apply_edit` -> `normalize_config`; toggle via `toggle_bool`, choices via `cycle`.
- `field_group` maps keys to `GROUPS` (Timer, Audio, Screen, Session, More); new generic settings go to group 4 (More) via `_ => 4`.
- `normalize_config` fills empty-string/zero defaults; `validate_config` rejects invalid enums.
- Tests live in `mod tests` at bottom of `src/main.rs`; use `quiet_config()`, `render_app()` with TestBackend, isolated temp paths.

---

## US-001: Configure Discord Rich Presence — PASS

Implemented backward-compatible Discord config section with TUI editor exposure.

Files changed:
- `src/main.rs`: added `DiscordConfig { enabled (default true), application_id (default "") }`, `discord` field on `Config`, `should_connect()` helper (enabled && non-empty id), normalize trims app id, new `FieldKey::DiscordEnabled` (Bool) + `FieldKey::DiscordApplicationId` (Edit) in `FIELDS` (More group), `field_value`/`toggle_bool`/`apply_edit`/`field_visible` support, 3 focused tests.

Check results:
- `cargo fmt --check` — PASS (exit 0)
- `cargo test --locked` — PASS (36 passed, 1 ignored)
- `cargo clippy --locked -- -D warnings` — PASS (exit 0)
- `cargo build --release --locked` — PASS
- Isolated CLI smoke (`init`, `validate`, `status`, `preview` on temp config) — PASS; new config serializes `discord: { enabled: true, application_id: "" }`
- Old-config check (YAML without `discord` section validates/loads) — PASS

Learnings:
- `should_connect()` is only used by tests until US-002 wires the IPC client; marked `#[allow(dead_code)]` to keep `clippy -D warnings` green.
- New TUI fields fall into `field_group` `_ => 4` (More group) with no extra mapping needed.

---

## US-002: Add a non-blocking Discord presence client — PASS

Implemented local IPC Rich Presence client with background worker; no bot token, network service, or credentials.

Files changed:
- `Cargo.toml`, `Cargo.lock`: added `serde_json 1.0.153` (smallest maintained dep for IPC JSON frames; evaluated `discord-rich-presence 1.1.0` — pulls `serde_repr` + duplicate `uuid 0.8` vs existing `uuid 1.23`, heavier effective tree; `serde_json` reuses already-locked `itoa`/`ryu`/`memchr`).
- `src/main.rs`: added `PresenceActivity`, `presence_activity_payload` / `presence_clear_payload` (SET_ACTIVITY JSON, timestamps omitted when None so paused never leaves stale countdown), `encode_discord_ipc_frame` (opcode+len LE), `discord_ipc_candidates` (`XDG_RUNTIME_DIR`/`TMPDIR`/`/tmp` x `discord-ipc-0..9`, deduped) + pure `discord_ipc_candidates_from` for tests, `DiscordPresence` (`start` returns instant no-op when `!should_connect`; otherwise spawns worker; `update`/`clear` only `send` on unbounded channel; `Drop` does best-effort `clear` then closes channel with no join), `discord_presence_worker` (connects with 2s timeouts, swallows all I/O errors, drops stream on send failure, final clear on channel close), `discord_connect`/`discord_handshake` (opcode 0 `{"v":1,"client_id"}`, best-effort READY read)/`discord_send_activity` (opcode 1), `next_presence_nonce`; wired `run_timer` with `let _discord = DiscordPresence::start(&config.discord)` guard so missing Discord never fails/stalls CLI+TUI and cleanup is automatic. Removed `#[allow(dead_code)]` from `should_connect`; `update`/`is_active` kept `allow(dead_code)` until US-003 publishes countdown updates.
- 5 new deterministic tests: payload formatting + paused-no-timestamps + clear, frame encoding LE, candidates layout, disabled/unconfigured no-op timing (<2s), missing-client `start`/`update`/`clear` + `run_timer --no-actions` success without stall.

Check results:
- `cargo fmt --check` — PASS (exit 0)
- `cargo test --locked` — PASS (41 passed, 1 ignored)
- `cargo clippy --locked -- -D warnings` — PASS (exit 0)
- `cargo build --release --locked` — PASS
- Isolated CLI smoke (`init`, `validate`, `status`, `preview` on temp config) — PASS; config serializes `discord: { enabled: true, application_id: "" }`
- Manual `run --no-actions 1s` with fake app id `123456789012345678` and no Discord socket — PASS (completed ~1s, exit 0, no stall)

Learnings:
- `let Some(socket) = stream.as_mut()` shadowing pitfall: inner `stream` becomes `&mut`, so `stream = None` fails; use distinct `socket` name and assign to outer `stream`.
- `clippy -D warnings` flags test-only `is_active`/`update` and `Clear` variant; fix by making `Drop` call `clear()` (makes `clear`+`Clear` live) and `allow(dead_code)` only on truly future `is_active`/`update` until US-003.
- `UnixStream::connect` missing socket fails fast with ENOENT; real non-blocking guarantee comes from doing handshake in spawned worker + 2s read/write timeouts + unbounded channel sends.

---

## US-003: Publish countdown status in CLI and TUI — PASS

Implemented countdown presence projection and wired it into `run_timer` (shared by CLI `run` and TUI timer thread, including `--no-actions` and dry-run).

Files changed:
- `src/main.rs`: added pure `should_show_poweroff` (exact `poweroff` match), `countdown_presence` (`Sleep timer` + `{remaining} until sleep` + end timestamp, `Power off after timer` suffix only for exact poweroff), `paused_presence` (`paused`, no timestamps), `post_countdown_presence` (`finished`, no timestamps); `DiscordPresence` gained `clear_on_drop` + `disarm()` (success disarms drop-clear so the countdown transitions instead of clearing); removed worker's redundant final clear (Drop already sends explicit Clear on stop; disarmed success retains post-countdown until process exit); `run_timer` publishes initial countdown, throttled running updates every 5s, paused updates on pause entry, countdown re-publish on resume, explicit `clear()` on all three stop paths, `update(post_countdown)+disarm` on success. Removed `#[allow(dead_code)]` from `update` (now live); kept on `is_active` (test-only).
- 5 new deterministic tests: countdown details/state/end-timestamp + payload, paused no-timestamps + payload, poweroff exact-match visibility (running+paused, 8 negative modes), stopped-run clears without stalling, success transitions (disarm + CLI no-actions/dry-run + TUI `run_timer` success without stall).

Check results:
- `cargo fmt --check` — PASS (exit 0)
- `cargo test --locked` — PASS (46 passed, 1 ignored)
- `cargo clippy --locked -- -D warnings` — PASS (exit 0)
- `cargo build --release --locked` — PASS
- Isolated CLI smoke (`init`, `validate`, `status`, `preview` on temp config) — PASS
- Manual `run --no-actions 1s` on temp config — PASS (exit 0, no stall)

Learnings:
- `Drop`-always-clears conflicts with retain-on-success; fix with `clear_on_drop` flag + `disarm()` on success and no worker final clear (explicit Clear only on stop paths).
- Throttle Discord updates (>=5s) inside the 250ms countdown loop; publish immediately on pause entry and on resume to avoid stale countdown text.
- End timestamp must be recomputed as `now_unix + remaining` per publish so the deadline stays fixed as remaining shrinks.
