# gosleep-timer

Rust TUI sleep timer for Linux desktop actions.

`gosleep-timer` waits for a configured duration, then runs post-countdown desktop commands such as switching workspace, stopping media, dimming brightness, muting audio, locking the session, killing selected processes, powering off/rebooting, or running custom shell commands.

## Status

- Current version: `1.1.1`
- Release tags: `vMAJOR.MINOR.PATCH`, for example `v1.1.1`
- Primary repository: <https://gitea.forust.xyz/forust/gosleep>
- SSH upstream: `ssh://git@gitssh.forust.xyz:2221/forust/gosleep.git`

## Features

- Single Rust binary, independent of Python.
- Ratatui/Crossterm terminal UI.
- YAML config at `~/.config/gosleep-timer/config.yaml` or `$XDG_CONFIG_HOME/gosleep-timer/config.yaml`.
- CLI commands for initialization, preview, and direct timer runs.
- Time-left display and progress bar.
- Wrapped command preview for smaller terminals.
- Linux desktop action support for niri, Hyprland, KDE, playerctl, brightnessctl/light, PipeWire/PulseAudio, systemd, and custom shell commands.

## Install

### From source

```bash
git clone ssh://git@gitssh.forust.xyz:2221/forust/gosleep.git
cd gosleep
cargo build --release --locked
install -Dm755 target/release/gosleep-timer ~/.local/bin/gosleep-timer
```

### From release artifact

Download `gosleep-timer-<version>-linux-amd64.tar.gz` from the Gitea release page, then:

```bash
tar -xzf gosleep-timer-1.1.1-linux-amd64.tar.gz
install -Dm755 gosleep-timer ~/.local/bin/gosleep-timer
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

Run the saved timer config:

```bash
gosleep-timer run
```

Override the duration for one run:

```bash
gosleep-timer run 45m
gosleep-timer run 1h30m
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
| `Space` / `Enter` | Toggle, cycle, or edit focused field |
| `Enter` while editing | Apply edit |
| `Esc` while editing | Cancel edit |
| `r` | Start/restart timer |
| `x` | Stop timer |
| `s` | Save config |
| `q` / `Esc` | Quit |

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
  lock:
    enabled: false
    command: loginctl lock-session
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
| `actions.workspace.backend` | `auto`, `niri`, `hyprland`, `kde` |
| `actions.media.action` | `none`, `stop`, `pause`, `play-pause`, `next`, `previous` |
| `actions.power.mode` | `none`, `poweroff`, `reboot` |

## Commands Run After Countdown

`auto` workspace backend tries:

```sh
niri msg action focus-workspace <number>
hyprctl dispatch workspace <number>
qdbus org.kde.KWin /KWin org.kde.KWin.setCurrentDesktop <number>
```

Other actions use common Linux desktop tools:

- `playerctl`
- `brightnessctl` or `light`
- `wpctl` or `pactl`
- `loginctl`
- `pkill`
- `systemctl`

Install only the tools needed for the actions you enable.

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
git tag -a v1.1.1 -m "gosleep-timer v1.1.1"
git push origin main --tags
```

## Releases

Gitea Actions builds releases from tags matching `v*.*.*`.

Required runner tools:

- Rust stable toolchain with `cargo`, `rustfmt`, and `clippy`
- `curl`
- `jq`
- `tar`
- `sha256sum`

Release authentication:

- The workflow uses Gitea Actions' built-in `GITEA_TOKEN`.
- The workflow declares `permissions: contents: write` so the token can create releases and upload assets.

See [RELEASING.md](RELEASING.md) for the full release checklist.

## License

MIT. See [LICENSE](LICENSE).
