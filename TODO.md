# gosleep-timer — Go TUI Timer

## Цель
Переписать `timer.py` на Go с модульной архитектурой, поддержкой нескольких WM/DE (niri, Hyprland, KDE), пользовательскими командами, профилями, историей, CLI-режимом и QR-экспортом.

---

## Структура проекта

```
gosleep-timer/
├── main.go                 # CLI entrypoint
├── go.mod / go.sum
├── .gitignore
├── TODO.md
├── cicd.yaml               # CI/CD pipeline
├── .golangci.yml           # linter config
├── config/
│   ├── config.go           # загрузка YAML
│   ├── defaults.go         # дефолтные значения
│   └── types.go            # структуры Config, Module, Profile, Stage
├── engine/
│   ├── timer.go            # ядро отсчёта
│   ├── executor.go         # запуск команд, таймауты, kill
│   └── stages.go           # фазы: pre → timer → post
├── modules/
│   ├── module.go           # интерфейс Module
│   ├── registry.go         # глобальный реестр модулей
│   ├── workspace/
│   │   └── workspace.go    # переключение workspace (niri/hyprland/kde)
│   ├── media/
│   │   └── media.go        # playerctl / MPRIS
│   ├── lock/
│   │   └── lock.go         # блокировка экрана
│   ├── kill/
│   │   └── kill.go         # pkill процессов
│   ├── notify/
│   │   └── notify.go       # notify-send / dunst
│   ├── sound/
│   │   └── sound.go        # звуковой сигнал
│   ├── brightness/
│   │   └── brightness.go   # управление яркостью (brightnessctl/light)
│   ├── mute/
│   │   └── mute.go         # mute/unmute (pactl/wpctl)
│   ├── custom/
│   │   └── custom.go       # пользовательские команды pre/post
│   └── script/
│       └── script.go       # запуск произвольного скрипта
├── profiles/
│   ├── profile.go          # структура Profile
│   └── manager.go          # CRUD профилей
├── history/
│   ├── history.go          # JSONL лог запусков
│   └── stats.go            # статистика
├── tui/
│   ├── app.go              # bubbletea App
│   ├── widgets.go          # кастомные виджеты
│   ├── styles.go           # темы
│   ├── keybinds.go         # горячие клавиши
│   └── qr.go               # QR код
```

---

## Очередность (от важного к мелкому)

### [x] 1. TODO.md
Этот файл.

### [ ] 2. Фундамент
- Инициализация Go модуля
- `.gitignore`
- Структура папок
- Первый коммит

### [ ] 3. Конфиг
- `config/types.go` — типы данных
- `config/defaults.go` — дефолты
- `config/config.go` — парсинг YAML
- Автоопределение WM (niri/hyprland/kde/none)

### [ ] 4. Ядро
- `engine/timer.go` — обратный отсчёт, тики
- `engine/executor.go` — exec.Command, таймауты, принудительное завершение
- `engine/stages.go` — фазы с callback-ами

### [ ] 5. Модули
- `modules/module.go` — интерфейс
- `modules/registry.go` — реестр
- workspace, media, lock, kill, notify, sound, brightness, mute, custom, script

### [ ] 6. Профили
- `profiles/profile.go`
- `profiles/manager.go`
- Загрузка/сохранение профилей, список, apply

### [ ] 7. История
- `history/history.go` — запись в JSONL
- `history/stats.go` — агрегация

### [ ] 8. TUI
- `tui/app.go`, `widgets.go`, `styles.go`, `keybinds.go`, `qr.go`

### [ ] 9. CLI
- `main.go` — cobra/флаги
- Режимы: `tui` (по умолчанию), `run`, `list-profiles`, `export`, `import`

### [ ] 10. Линтеры + CI/CD
- `.golangci.yml` (golint, staticcheck, gosec, revive, errcheck)
- `cicd.yaml` на основе `cicd.example.yaml`
- Go-специфичные линтеры

---

## Модули (детально)

### workspace
- Поле `Backend` (niri | hyprland | kde | auto)
- Поле `Workspace` (int)
- Автоопределение: `loginctl`, `$XDG_CURRENT_DESKTOP`, `hyprctl`, `niri msg`
- Команды:
  - niri: `niri msg action focus-workspace {n}`
  - hyprland: `hyprctl dispatch workspace {n}`
  - kde: `qdbus org.kde.KWin /KWin setCurrentDesktop {n}`

### media
- Поле `Action` (stop | pause | play-pause | next | previous | none)
- fallback: `playerctl`

### lock
- Поле `Command` (кастомная строка)
- fallback: `loginctl lock-session`

### kill
- Поле `Processes` ([]string)
- `pkill {name}` для каждого

### notify
- Поле `Sound` (путь к файлу или none)
- `notify-send` + опционально звук

### sound
- Поле `File` (путь к wav/oga)
- `paplay`, `aplay`, `ffplay`

### brightness
- Поле `Value` (int 0-100)
- `brightnessctl set {n}%` или `light -S {n}`

### mute
- Поле `Mute` (bool)
- `pactl set-sink-mute @DEFAULT_SINK@ 1` / `0`
- `wpctl set-mute @DEFAULT_AUDIO_SINK@ 1` / `0`

### custom
- Поля `PreCommands`, `PostCommands` ([]string)

### script
- Поле `Path` (путь к скрипту)
- Аргументы из контекста таймера

---

## CLI флаги (`main.go`)

```
gosleep-timer                   # TUI режим
gosleep-timer run 25m           # CLI режим
gosleep-timer run --profile work 25m
gosleep-timer list-profiles
gosleep-timer export > config.yaml
gosleep-timer import < config.yaml
gosleep-timer qr                # показать QR с конфигом
```

---

## TUI (bubbletea)

### Экраны
1. **Main** — ввод времени, чекбоксы модулей, выбор профиля, Start/Stop
2. **Running** — прогресс-бар, оставшееся время, текущий этап
3. **History** — последние запуски
4. **Stats** — статистика

### Горячие клавиши
- `s` — Start
- `S` — Stop
- `q` / `Esc` — Quit
- `h` — History
- `Tab` — фокус следующий элемент
- `Enter` — toggle/action

---

## Формат конфига (`~/.config/gosleep-timer/config.yaml`)

```yaml
profiles:
  default:
    modules:
      workspace:
        enabled: true
        backend: auto
        workspace: 12
      media:
        enabled: true
        action: stop
      lock:
        enabled: true
        command: ""
      kill:
        enabled: false
        processes: []
      notify:
        enabled: true
        sound: ""
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
  gaming:
    modules:
      kill:
        enabled: true
        processes: [firefox, slack]
      mute:
        enabled: true
```

---

## История (`~/.local/share/gosleep-timer/history.jsonl`)

```jsonl
{"ts":"2026-07-03T01:30:00Z","profile":"default","duration":"25m","status":"completed"}
{"ts":"2026-07-03T02:00:00Z","profile":"work","duration":"1h","status":"killed"}
```

---

## CI/CD Pipeline

На основе `cicd.example.yaml`:
- `lint-go` — golangci-lint (все линтеры)
- `lint-yaml` — yamllint
- `lint-dockerfile` — hadolint (если появится)
- `build` — go build
- `test` — go test ./...
- `publish` — docker образ (опционально)
