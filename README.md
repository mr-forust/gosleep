# ⏱ gosleep-timer

TUI sleep timer with modular WM/DE support. Go rewrite of `timer.py`.

## Quick start

```bash
# Run TUI
gosleep-timer

# First time? Interactive setup wizard
gosleep-timer init
```

## Installation

```bash
go install github.com/forust/gosleep-timer@latest
# or
git clone https://github.com/forust/gosleep-timer
cd gosleep-timer
go build -o gosleep-timer .
sudo cp gosleep-timer /usr/local/bin/
```

## Usage

### TUI mode (default)

```bash
gosleep-timer
```

Keys:
| Key | Action |
|-----|--------|
| `s` | Start timer |
| `S` | Stop timer |
| `Tab` | Next field |
| `↑`/`↓` | Previous/next profile |
| `1`-`0` | Toggle modules |
| `h` | History |
| `?` | Help |
| `q`/`Esc` | Quit |

### CLI mode

```bash
# Run with defaults (25m)
gosleep-timer run

# Custom duration
gosleep-timer run 45m
gosleep-timer run 1h30m

# With profile
gosleep-timer run 20m --profile work
```

### Other commands

```bash
# Interactive setup
gosleep-timer init

# List profiles
gosleep-timer list-profiles

# Export/import config
gosleep-timer export > config-backup.yaml
gosleep-timer import < config-backup.yaml

# History and stats
gosleep-timer history
gosleep-timer stats
```

## Configuration

File: `~/.config/gosleep-timer/config.yaml`

### Profiles

```yaml
profiles:
  default:
    modules:
      workspace:
        enabled: true
        backend: auto        # auto, niri, hyprland, kde
        workspace: 12
      media:
        enabled: true
        action: stop         # stop, pause, play-pause, next, previous
      lock:
        enabled: true
        command: ""          # custom command or auto-detect
      kill:
        enabled: false
        processes: [firefox, telegram-desktop]
      notify:
        enabled: true
        sound: ""            # path or empty
      sound:
        enabled: true
        file: /usr/share/sounds/freedesktop/stereo/complete.oga
      brightness:
        enabled: false
        value: 0
      mute:
        enabled: false
      custom:
        enabled: false
        pre: []
        post: []
      script:
        enabled: false
        path: ""

  work:
    extends: default
    modules:
      workspace:
        workspace: 5
```

### Profile inheritance

Profiles can extend others. Modules from the base profile are merged and overlaid.

## Modules

| Module | Stage | Description |
|--------|-------|-------------|
| **workspace** | pre | Switch WM workspace (niri/Hyprland/KDE) |
| **media** | pre | Control music player via playerctl |
| **lock** | post | Lock screen |
| **kill** | pre | Kill processes by name |
| **notify** | post | Desktop notification |
| **sound** | post | Play audio file |
| **brightness** | pre/post | Dim/restore brightness |
| **mute** | pre/post | Mute/unmute audio |
| **custom** | pre/post | User-defined shell commands |
| **script** | pre/post | Run external script with args |

### Workspace backends

| Backend | Command |
|---------|---------|
| niri | `niri msg action focus-workspace {n}` |
| Hyprland | `hyprctl dispatch workspace {n}` |
| KDE | `qdbus org.kde.KWin /KWin setCurrentDesktop {n}` |

Auto-detection checks `$XDG_CURRENT_DESKTOP`, then tries `hyprctl` and `niri msg`.

## Project structure

```
gosleep-timer/
├── main.go                 # CLI entrypoint
├── cmd_init.go             # Interactive setup wizard
├── config/                 # YAML config load/save/types
├── engine/                 # Timer core, executor, stages
├── modules/                # Module interface + 10 implementations
├── profiles/               # Profile manager
├── history/                # JSONL log + statistics
├── tui/                    # Bubbletea TUI app
├── cicd.yaml               # CI/CD pipeline
└── Dockerfile              # Multi-stage build
```

## Development

```bash
# Build
go build -o gosleep-timer .

# Test
go test ./...

# Lint
golangci-lint run --config .golangci.yml

# Static analysis
staticcheck ./...
gosec ./...
revive -config .revive.toml ./...
```

## CI/CD

Pipeline (`.github/workflows/cicd.yaml`):
- **lint-go**: golangci-lint, staticcheck, gosec, revive
- **lint-yaml**: yamllint
- **build**: go build + test with race detector
- **publish**: Docker image to registry (main/dev branches)

## License

MIT
