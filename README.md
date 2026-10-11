# gosleep-timer

Rust TUI sleep timer for Linux desktop actions.

`gosleep-timer` waits for a configured duration, then runs post-countdown desktop commands such as switching workspace, stopping media, dimming brightness, muting audio, locking the session, turning the screen off, sending a desktop notification, killing selected processes, powering off/rebooting, or running custom shell commands.

## Status

- Release tags: `vMAJOR.MINOR.PATCH`
- Primary repository: <https://git.forust.xyz/forust/gosleep>
- SSH upstream: `ssh://git@gitssh.forust.xyz:2221/forust/gosleep.git`

## Features

- Single Rust binary, independent of Python.
- Ratatui/Crossterm terminal UI.
- YAML config at `~/.config/gosleep-timer/config.yaml` or `$XDG_CONFIG_HOME/gosleep-timer/config.yaml`.
- CLI commands for init, status, validate, edit, preview, run, history, and stats.
- Time-left display and progress bar.
- Wrapped command preview for smaller terminals.
- Linux desktop action support for niri, Hyprland, sway, i3, KDE, playerctl, brightnessctl/light, PipeWire/PulseAudio, systemd, desktop notifications, and custom shell commands.

## Install

### From source

```bash
git clone ssh://git@gitssh.forust.xyz:2221/forust/gosleep.git
cd gosleep
cargo build --release --locked
install -Dm755 target/release/gosleep-timer ~/.local/bin/gosleep-timer
```

### From release artifact

Download `gosleep-timer-<version>-linux-amd64` and its `.sha256` from the Gitea release page, then:

```bash
sha256sum -c gosleep-timer-<version>-linux-amd64.sha256
install -Dm755 gosleep-timer-<version>-linux-amd64 ~/.local/bin/gosleep-timer
```

## Usage

Open the TUI:

```bash
gosleep-timer
```

Initialize the default config:

```bash
gosleep-timer init
```

Preview commands that will run after the countdown:

```bash
gosleep-timer preview
```

Show the current config, enabled actions, and dangerous-action markers:

```bash
gosleep-timer status
```

Validate the config without starting the timer:

```bash
gosleep-timer validate
```

Open the config in `$VISUAL`, `$EDITOR`, or `vi`:

```bash
gosleep-timer edit
```

Run the saved timer config:

```bash
gosleep-timer run
```

Override the duration for one run:

```bash
gosleep-timer run 45m
gosleep-timer run 1h30m
```

Run the countdown without executing post-timer actions:

```bash
gosleep-timer run --dry-run 10s
gosleep-timer run --no-actions 10s
```

Inspect recorded sessions and aggregate stats:

```bash
gosleep-timer history
gosleep-timer stats
```

Use a custom config path:

```bash
gosleep-timer --config ./config.yaml preview
```

## TUI Keys

| Key | Action |
| --- | --- |
| `j` / `Down` | Move down |
| `k` / `Up` | Move up |
| `Tab` / `Shift+Tab` | Select next/previous settings group |
| `Space` / `Enter` | Toggle, cycle, or edit focused field |
| `1` / `2` / `3` | Set duration to 30 minutes / 1 hour / 2 hours |
| `l` | Switch between the action plan and results |
| `PageUp` / `PageDown` | Scroll the action plan or results |
| `Enter` while editing | Apply edit |
| `Esc` while editing | Cancel edit |
| `Left` / `Right`, `Home` / `End` while editing | Move the text cursor |
| `Backspace` / `Delete` while editing | Delete before/after the cursor |
| `r` | Start/restart timer |
| `p` | Pause/resume running timer |
| `x` | Stop timer |
| `s` | Save config |
| `q` / `Esc` | Quit |

Settings are grouped into **Timer**, **Audio**, **Screen**, **Session**, and **More**.
The selected field has a context hint. Text edits use a yellow background and a
visible cursor. Invalid durations and brightness values stay open for correction.

The idle screen shows `not running`. The running screen shows the remaining time,
progress, and estimated local end time. Pausing extends the end time. Small
terminals use a compact timer instead of cropped large digits. The minimum size
is 36 columns by 20 rows. Settings and the preview stack below 118 columns; wider
terminals show them side by side.

The action plan lists enabled actions in execution order. Each row pairs a cyan
action description on the left with its green shell command on the right. Long
text wraps within its column. Dangerous actions remain red. Use `PageUp` and
`PageDown` when the line count shows more content. Use
`l` for the current run's results, including each action's success or failure.
Results open automatically when the timer completes and remain until the next run.
Argument previews quote each argument so spaces and quotes are clear. Control
characters and invisible formatting marks appear as visible escapes. This
changes the display only; actions still use the configured values.

To finish at a clock time, edit **until local time** in the **Timer** group. Enter
`HH:MM`, such as `23:30`, then press `r`. The timer uses the next occurrence of that
local time, today or tomorrow. If a clock change repeats the time, the next future
occurrence is used. A time that does not exist due to a clock change is rejected.
The target is converted to a countdown when you press `r`; pausing delays it.
Later system clock changes do not change the countdown.

The clock target is for the current TUI session and is not saved. The saved
`duration` remains the default. Set a duration preset, edit `duration`, or clear
**until local time** to return to duration mode. Presets set the time; press `r`
to start. Numeric keys typed during editing enter text.

## Configuration

Default config:

```yaml
duration: 25m
actions:
  workspace:
    enabled: true
    backend: auto
    number: 3
  media:
    enabled: true
    action: stop
  brightness:
    enabled: false
    value: 30
  mute:
    enabled: false
  mute_input:
    enabled: false
  lock:
    enabled: false
    backend: auto
  kill:
    enabled: false
    processes: []
  power:
    mode: none
  custom:
    enabled: false
    commands: []
```

Supported values:

| Field | Values |
| --- | --- |
| `duration` | Go-style duration strings such as `25m`, `45m`, `1h30m`, `10s` |
| `actions.workspace.backend` | `auto`, `niri`, `hyprland`, `sway`, `i3`, `kde` |
| `actions.media.action` | `none`, `stop`, `pause`, `play-pause`, `next`, `previous` |
| `actions.monitor.backend` | `auto`, `hyprland`, `niri`, `sway`, `kde`, `x11` |
| `actions.power.mode` | `none`, `suspend`, `hibernate`, `hybrid-sleep`, `poweroff`, `reboot` |

Native GNOME monitor power-off is not supported. The `auto` backend reports an
action failure when it cannot find a supported backend. Use the custom monitor
command for a desktop-specific command.

## Commands Run After Countdown

`auto` workspace backend tries:

```sh
niri msg action focus-workspace <number>
hyprctl dispatch workspace <number>
swaymsg workspace number <number>
i3-msg workspace number <number>
qdbus org.kde.KWin /KWin org.kde.KWin.setCurrentDesktop <number>
```

Screen off (`monitor`) on KDE runs `kscreen-doctor --dpms off`, desktop notifications (`notify`) use `notify-send`, and `inhibit_sleep` maps to the TUI `prevent sleep` toggle.

Other actions use common Linux desktop tools:

- `playerctl`
- `brightnessctl` or `light`
- `wpctl` or `pactl`
- `loginctl`
- `kscreen-doctor`
- `notify-send`
- `pkill`
- `systemctl`

Install only the tools needed for the actions you enable.
Dangerous actions such as `pkill`, power actions, and common destructive custom commands are highlighted in red in the TUI preview.

## History

Completed timer sessions are appended to `history.yaml` next to the active config file.
If no poweroff or reboot action is configured, CLI `run` asks for wake confirmation after the timer finishes and records the elapsed sleep time.

## Discord Rich Presence

Timer runs can publish status to the local Discord desktop client through
Discord local IPC. No bot token, network service, or account credentials are
used.

### Application ID setup

1. Create an application in the Discord Developer Portal and copy its
   **Application ID**. The ID is public configuration, not a secret. Do not
   commit a personal ID.
2. Put the ID in the config file:

   ```yaml
   discord:
     enabled: true
     application_id: "123456789012345678"
   ```

3. Keep the Discord desktop client running on the same machine. No connection
   is attempted until a non-empty Application ID is configured.

### Configuration keys

| Key | Default | Meaning |
| --- | --- | --- |
| `discord.enabled` | `true` | Master switch for Rich Presence. |
| `discord.application_id` | `""` | Discord Application ID. Empty means disabled. |

Both fields are editable in the TUI **More** group as **discord enabled** and
**discord app id**. Existing config files without a `discord` section load
with the defaults above. To disable presence, set `discord.enabled: false` or
leave `application_id` empty; either way no IPC connection is attempted.

### Displayed states

`details` stays `Sleep timer`. `state` and timestamps change by phase:

| Phase | State | Timestamps |
| --- | --- | --- |
| Running | `<remaining> until sleep`, for example `25m0s until sleep` | Countdown end timestamp |
| Running with `actions.power.mode: poweroff` | `<remaining> until sleep • Power off after timer` | Countdown end timestamp |
| Paused | `paused` (plus the poweroff suffix only for exact `poweroff`) | None, so no stale countdown is shown |
| After successful real actions | `Sleeping for <duration>`, for example `Sleeping for 25m0s` | Start timestamp at the beginning of post-countdown actions, so Discord shows elapsed sleep time |
| Dry-run, `--no-actions`, action failures, or no enabled actions | `finished` | None, so no sleep interval is claimed |
| Stopped or cancelled | Presence is cleared | — |

The `Power off after timer` suffix appears only when `actions.power.mode` is
exactly `poweroff`. It is omitted for `none`, `suspend`, `hibernate`,
`hybrid-sleep`, `reboot`, and every other value.

### Lifetime and unavailable Discord

- The countdown status is published by both CLI `run` and TUI timer runs,
  including `--no-actions` and dry-run runs.
- After a successful real-action run, the elapsed `Sleeping for …` status
  stays active while the TUI remains open and during the CLI
  wake-confirmation wait. Discord clears it when the process exits or the
  machine powers off or suspends.
- Cancelling a run clears the presence. Quitting the TUI after a completed
  run clears the retained status; otherwise it is retained until the app
  exits.
- When Discord is closed, missing, or unreachable, the timer behaves exactly
  as without Discord. IPC connects in a background thread with short
  timeouts, updates never block the countdown or action runner, and all
  connection and send failures are ignored.

## Development

```bash
cargo fmt --check
cargo test --locked
cargo clippy --locked -- -D warnings
cargo build --release --locked
```

Local smoke test:

```bash
tmpdir="$(mktemp -d)"
cargo run -- --config "$tmpdir/config.yaml" init
cargo run -- --config "$tmpdir/config.yaml" preview
```

## Versioning

Version is stored in:

- `Cargo.toml`
- `Cargo.lock`
- `VERSION`
- `CHANGELOG.md`

Release tags must match the package version:

```bash
git tag -a vMAJOR.MINOR.PATCH -m "gosleep-timer vMAJOR.MINOR.PATCH"
git push origin main
git push origin vMAJOR.MINOR.PATCH
```

## Releases

Gitea Actions builds releases from tags matching `v*.*.*`.

Required runner tools:

- Rust stable toolchain with `cargo`, `rustfmt`, and `clippy`
- `curl`
- `jq`
- `sha256sum`

Release authentication:

- The workflow exposes Gitea Actions' built-in `${{ secrets.GITEA_TOKEN }}` as `GITEA_TOKEN`.
- The workflow declares `permissions: contents: write` so the token can create releases and upload assets.

See [RELEASING.md](RELEASING.md) for the full release checklist.

## License

MIT. See [LICENSE](LICENSE).
