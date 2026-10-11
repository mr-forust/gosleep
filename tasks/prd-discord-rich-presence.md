[PRD]
# PRD: Discord Rich Presence for gosleep-timer

## Overview

Add Discord Rich Presence to CLI and TUI timer runs. Discord should show the timer phase and useful timing information while the timer runs, then change to a sleep-duration status after the countdown and configured actions. Presence must be optional and must never stop or delay the timer when Discord is unavailable.

## Goals

- Show `Sleep timer` with a countdown such as `45m until sleep` while the timer is active.
- Show a paused state without a misleading live countdown.
- Show `Sleeping for …` after the countdown and configured actions when the run represents a real sleep interval.
- Before activation, show `Power off after timer` only when the configured power action is explicitly `poweroff`.
- Support CLI and TUI runs, with a setting to disable Discord Rich Presence.
- Keep timer behavior independent of Discord availability and connection failures.

## Quality Gates

These commands must pass for every user story:

- `cargo fmt --check`
- `cargo test --locked`
- `cargo clippy --locked -- -D warnings`
- `cargo build --release --locked`
- Isolated CLI smoke checks from `AGENTS.md`: initialize a temporary config, then run `validate`, `status`, and `preview` against it.

Run the timezone transition test only if clock target behavior changes. Run YAML lint only if workflow or YAML files change.

## User Stories

### US-001: Configure Discord Rich Presence
**Description:** As a user, I want to configure and disable Discord Rich Presence so that I control whether gosleep-timer publishes my timer status.

**Acceptance Criteria:**
- [ ] Add a backward-compatible Discord configuration section with an enabled setting and Discord Application ID.
- [ ] Provide a usable default: presence is enabled, but no connection is attempted until a non-empty Application ID is configured.
- [ ] Expose the enable toggle and Application ID through the existing TUI configuration editor.
- [ ] Existing YAML configurations without the new section continue to load with defaults.
- [ ] Add focused tests for defaults, old configuration deserialization, and editing/applying the new fields.

### US-002: Add a non-blocking Discord presence client
**Description:** As a user, I want gosleep-timer to connect to Discord's local Rich Presence interface so that my timer status appears when the Discord desktop client is available.

**Acceptance Criteria:**
- [ ] Use Discord local IPC for Rich Presence; do not add a bot token, network service, or account credentials.
- [ ] Reuse or add the smallest maintained Rust dependency that supports the repository's Rust version; keep `Cargo.lock` committed.
- [ ] A missing Discord client, unavailable IPC socket, or connection failure does not fail or stall a timer run.
- [ ] Presence updates and cleanup do not block the countdown or action runner.
- [ ] Add deterministic tests for presence payload formatting and the disabled/unconfigured no-op behavior.

### US-003: Publish countdown status in CLI and TUI
**Description:** As a user, I want Discord to show how long remains until the timer activates so that others can see when my sleep actions are due.

**Acceptance Criteria:**
- [ ] While running, show `Sleep timer` and a state such as `45m until sleep`, with a countdown end timestamp where supported.
- [ ] When paused, show a paused state and do not leave a stale countdown timestamp.
- [ ] Show `Power off after timer` only when `actions.power.mode` is exactly `poweroff`; omit it for every other setting.
- [ ] Support both CLI `run` and TUI timer runs, including `--no-actions` and dry-run behavior.
- [ ] Clear countdown presence when a run is stopped; transition to the post-countdown state on successful completion instead of clearing it.
- [ ] Add tests for CLI/TUI status projection, paused status, remaining time, and poweroff visibility.

### US-004: Publish post-countdown sleep status and document setup
**Description:** As a user, I want Discord to show `Sleeping for …` after the timer activates so that the presence reflects the current phase.

**Acceptance Criteria:**
- [ ] After the countdown and configured actions complete, publish `Sleeping for …` with an elapsed-time timestamp measured from the start of post-countdown actions.
- [ ] Keep the status accurate when actions fail; do not claim a sleep interval for `--no-actions` or dry-run runs.
- [ ] Keep the elapsed status active in the TUI while it remains open and during the CLI wake-confirmation wait; let Discord clear it when the process exits or the machine powers off.
- [ ] Clear presence when the run is cancelled or the application exits cleanly; retain the post-countdown status while the TUI remains open.
- [ ] Document the Discord Application ID setup, configuration keys, disable option, displayed states, and behavior when Discord is unavailable in `README.md`.
- [ ] Add tests for transition to the post-countdown phase and cleanup on stop/completion.

## Functional Requirements

- FR-1: Read Discord Rich Presence settings from the existing YAML configuration while preserving compatibility with existing files.
- FR-2: Publish status through Discord's local IPC interface only when enabled and configured.
- FR-3: Update presence as the timer moves between running, paused, post-countdown, stopped, and completed states; retain the sleep-duration state after a successful countdown while the app is open.
- FR-4: Include a poweroff indication only for an explicit `poweroff` power action.
- FR-5: Treat Discord connection and update failures as non-fatal and keep all timer operations responsive.

## Non-Goals

- No Discord bot, bot token, cloud service, account login flow, or remote presence relay.
- No presence support for third-party/custom shell actions that might power off the machine.
- No automatic detection of sleep/wake events after the timer process exits or the machine suspends; use elapsed timestamps while the process is alive.
- No custom images/assets, buttons, party features, or user-defined status templates in the initial version.

## Technical Considerations

- Keep changes in `src/main.rs` unless the implementation has a concrete reason to split modules; this repository is a small Rust-first single-binary app.
- The timer worker already owns the countdown and executes configured actions; presence updates must not write into the TUI terminal.
- Discord Application ID is public configuration, not a secret. Do not commit a real user-specific ID.
- The repository currently has no Discord application ID. Keep it configurable and skip connection attempts when it is empty.
- The TUI settings are generated from `FieldKey`/field metadata and use the existing edit/apply/cancel flow.

## Success Metrics

- A configured Discord desktop client displays the correct countdown, pause, poweroff, and post-countdown states for CLI and TUI runs.
- With Discord closed, missing, or disabled, the timer has the same completion and action behavior as before.
- Existing config files load without migration or manual edits.
- All quality gates pass.

## Open Questions

- The Discord Application ID must be supplied or registered before live presence can be verified. The implementation must not invent or commit one.
- Confirm the exact wording after implementation against Discord's two-line Rich Presence layout and text-length limits.
[/PRD]
